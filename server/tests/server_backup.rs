use serein_connect::{
	server::{self, ServerConfig},
	server_backup as backup,
};

#[test]
fn recovery_key_is_create_new_and_roundtrips() {
	let t = tempfile::tempdir().unwrap();
	let (private, _) = backup::generate_recovery_key();
	let file = t.path().join("recovery.key");
	backup::save_recovery_key(&file, &private).unwrap();
	assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), private);
	assert!(backup::save_recovery_key(&file, &private).is_err());
}
#[test]
fn migrated_history_remains_restorable() {
	use serein_connect::{protocol::*, server_store::ServerStore};
	let t = tempfile::tempdir().unwrap();
	let root = t.path().join("source");
	server::initialize(
		&root,
		ServerConfig::local("@test/migrated", "Migrated").unwrap(),
	)
	.unwrap();
	let store = ServerStore::open(&root.join("server.db"), "Migrated").unwrap();
	let snapshot = Snapshot {
		name: "Old".into(),
		channels: vec![Channel {
			id: "old".into(),
			name: "Old".into(),
			kind: ChannelKind::Text,
		}],
		members: vec![],
		messages: vec![Message {
			id: 42,
			channel: "old".into(),
			author_id: "historic-author".into(),
			author: Profile {
				name: "Historic".into(),
				accent: [1, 2, 3],
				card: None,
			},
			text: "Preserved".into(),
			sent_at: 1,
		}],
	};
	store.import_legacy(&snapshot).unwrap();
	let (private, public) = backup::generate_recovery_key();
	let file = t.path().join("saved.age");
	backup::export(&root, &file, &public).unwrap();
	let dest = t.path().join("restored");
	backup::restore(&file, &dest, &private).unwrap();
	let restored = ServerStore::open(&dest.join("server.db"), "Restored").unwrap();
	assert_eq!(restored.snapshot().unwrap().messages[0].text, "Preserved");
	assert!(restored.import_legacy(&snapshot).is_err());
}
#[test]
fn encrypted_roundtrip_rejects_wrong_key_corruption_and_existing_destination() {
	let t = tempfile::tempdir().unwrap();
	let source = t.path().join("source");
	server::initialize(
		&source,
		ServerConfig::local("@test/backup", "Backup").unwrap(),
	)
	.unwrap();
	let (private, public) = backup::generate_recovery_key();
	let archive = t.path().join("saved.age");
	backup::export(&source, &archive, &public).unwrap();
	let dest = t.path().join("restored");
	backup::restore(&archive, &dest, &private).unwrap();
	assert_eq!(
		std::fs::read(source.join("identity.key")).unwrap(),
		std::fs::read(dest.join("identity.key")).unwrap()
	);
	assert!(backup::restore(&archive, &dest, &private).is_err());
	let (wrong, _) = backup::generate_recovery_key();
	assert!(backup::restore(&archive, &t.path().join("wrong"), &wrong).is_err());
	let bytes = std::fs::read(&archive).unwrap();
	for (i, mut bad) in [bytes[..bytes.len() - 1].to_vec(), bytes.clone()]
		.into_iter()
		.enumerate()
	{
		if i == 1 {
			let n = bad.len();
			bad[n - 10] ^= 1;
		}
		let path = t.path().join(format!("bad{i}"));
		std::fs::write(&path, bad).unwrap();
		let output = t.path().join(format!("out{i}"));
		assert!(backup::restore(&path, &output, &private).is_err());
		assert!(!output.exists());
	}
}
#[test]
fn roundtrip_preserves_history_owner_and_revokes_setup() {
	use serein_connect::{
		protocol::Profile,
		server_store::{Role, ServerStore},
	};
	let t = tempfile::tempdir().unwrap();
	let root = t.path().join("source");
	let setup = server::initialize(
		&root,
		ServerConfig::local("@test/history", "History").unwrap(),
	)
	.unwrap();
	let store = ServerStore::open(&root.join("server.db"), "History").unwrap();
	store.pair("owner", &setup.pairing_token, 1).unwrap();
	store
		.admit(
			"owner",
			Profile {
				name: "Owner".into(),
				accent: [1, 2, 3],
				card: None,
			},
			None,
			2,
		)
		.unwrap();
	store
		.send_message("owner", "allmant", "Saved history", "req-1", 3)
		.unwrap();
	let pending = setup.pairing_token.clone();
	let (private, public) = backup::generate_recovery_key();
	let archive = t.path().join("saved.age");
	let lock = server::lock_directory(&root).unwrap();
	backup::export(&root, &archive, &public).unwrap();
	drop(lock);
	let dest = t.path().join("restored");
	backup::restore(&archive, &dest, &private).unwrap();
	let restored = ServerStore::open(&dest.join("server.db"), "History").unwrap();
	assert_eq!(restored.role("owner").unwrap(), Role::Owner);
	assert_eq!(
		restored
			.history("owner", "allmant", None, 100)
			.unwrap()
			.messages[0]
			.text,
		"Saved history"
	);
	assert!(restored.pair("other", &pending, 5).is_err());
	assert!(restored.snapshot().unwrap().members.is_empty());
}
fn malicious_archive(
	t: &std::path::Path,
	public: &str,
	manifest: serde_json::Value,
	payload: &[u8],
) -> std::path::PathBuf {
	use std::io::Write;
	let recipient: age::x25519::Recipient = public.parse().unwrap();
	let path = t.join("malicious.age");
	let file = std::fs::File::create(&path).unwrap();
	let mut writer =
		age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
			.unwrap()
			.wrap_output(file)
			.unwrap();
	let bytes = serde_json::to_vec(&manifest).unwrap();
	writer.write_all(b"CONNECT-BACKUP-1\n").unwrap();
	writer
		.write_all(&(bytes.len() as u32).to_le_bytes())
		.unwrap();
	writer.write_all(&bytes).unwrap();
	writer.write_all(payload).unwrap();
	writer.finish().unwrap();
	path
}
#[test]
fn rejects_traversal_duplicates_and_oversized_files() {
	let t = tempfile::tempdir().unwrap();
	let (private, public) = backup::generate_recovery_key();
	for (n, extra) in [
		serde_json::json!({"path":"../escape","size":1}),
		serde_json::json!({"path":"identity.key","size":32}),
		serde_json::json!({"path":"assets/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png","size":52428801}),
	].into_iter().enumerate() {
		let entries = serde_json::json!([
			{"path":"identity.key","size":32},
			{"path":"server.json","size":2},
			{"path":"server.db","size":1}, extra
		]);
		let manifest=serde_json::json!({"version":1,"server_id":"wrong-server-id","entries":entries});
		let archive=malicious_archive(t.path(),&public,manifest,&[0;64]);
		let dest=t.path().join(format!("restored-{n}"));
		let error=backup::restore(&archive,&dest,&private).unwrap_err().to_string();
		assert!(if n<2 {error.contains("sökväg")} else {error.contains("stor")});
		assert!(!dest.exists());
		assert!(!t.path().join("escape").exists());
	}
}
#[test]
fn rejects_source_assets_with_arbitrary_names() {
	let t = tempfile::tempdir().unwrap();
	let root = t.path().join("source");
	server::initialize(
		&root,
		ServerConfig::local("@test/assets", "Assets").unwrap(),
	)
	.unwrap();
	std::fs::create_dir(root.join("assets")).unwrap();
	std::fs::write(root.join("assets/secret.txt"), b"not an asset").unwrap();
	let (_, recipient) = backup::generate_recovery_key();
	let dest = t.path().join("saved.age");
	assert!(backup::export(&root, &dest, &recipient).is_err());
	assert!(!dest.exists());
}
#[test]
fn preserves_hash_checked_assets() {
	use sha2::{Digest, Sha256};
	let t = tempfile::tempdir().unwrap();
	let root = t.path().join("source");
	server::initialize(&root, ServerConfig::local("@test/media", "Media").unwrap()).unwrap();
	std::fs::create_dir(root.join("assets")).unwrap();
	let content = b"synthetic asset bytes";
	let name = format!("{:x}.png", Sha256::digest(content));
	std::fs::write(root.join("assets").join(&name), content).unwrap();
	let (private, recipient) = backup::generate_recovery_key();
	let archive = t.path().join("saved.age");
	backup::export(&root, &archive, &recipient).unwrap();
	let dest = t.path().join("restored");
	backup::restore(&archive, &dest, &private).unwrap();
	assert_eq!(
		std::fs::read(dest.join("assets").join(name)).unwrap(),
		content
	);
}
#[test]
fn unconsumed_setup_token_is_revoked() {
	use serein_connect::server_store::ServerStore;
	let t = tempfile::tempdir().unwrap();
	let root = t.path().join("source");
	let setup =
		server::initialize(&root, ServerConfig::local("@test/setup", "Setup").unwrap()).unwrap();
	let (private, public) = backup::generate_recovery_key();
	let archive = t.path().join("saved.age");
	backup::export(&root, &archive, &public).unwrap();
	let dest = t.path().join("restored");
	backup::restore(&archive, &dest, &private).unwrap();
	let store = ServerStore::open(&dest.join("server.db"), "Setup").unwrap();
	assert!(store.pair("other", &setup.pairing_token, 1).is_err());
}
#[test]
fn rejects_mismatched_server_identity_after_authenticated_payload() {
	let t = tempfile::tempdir().unwrap();
	let (private, public) = backup::generate_recovery_key();
	let key = iroh::SecretKey::generate();
	let config =
		serde_json::to_vec(&ServerConfig::local("@test/mismatch", "Mismatch").unwrap()).unwrap();
	let manifest = serde_json::json!({"version":1,"server_id":iroh::SecretKey::generate().public().to_string(),"entries":[{"path":"identity.key","size":32},{"path":"server.json","size":config.len()},{"path":"server.db","size":1}]});
	let mut payload = key.to_bytes().to_vec();
	payload.extend(config);
	payload.push(0);
	let archive = malicious_archive(t.path(), &public, manifest, &payload);
	let dest = t.path().join("restored");
	let error = backup::restore(&archive, &dest, &private).unwrap_err();
	assert!(error.to_string().contains("serveridentitet"));
	assert!(!dest.exists());
}
#[test]
fn refuses_unknown_database_schema_before_activation() {
	let t = tempfile::tempdir().unwrap();
	let (private, public) = backup::generate_recovery_key();
	let key = iroh::SecretKey::generate();
	let config =
		serde_json::to_vec(&ServerConfig::local("@test/schema", "Schema").unwrap()).unwrap();
	let path = t.path().join("unknown.db");
	let db = rusqlite::Connection::open(&path).unwrap();
	db.execute_batch("CREATE TABLE unexpected(value TEXT); PRAGMA user_version=99;")
		.unwrap();
	drop(db);
	let database = std::fs::read(path).unwrap();
	let manifest = serde_json::json!({"version":1,"server_id":key.public().to_string(),"entries":[{"path":"identity.key","size":32},{"path":"server.json","size":config.len()},{"path":"server.db","size":database.len()}]});
	let mut payload = key.to_bytes().to_vec();
	payload.extend(config);
	payload.extend(database);
	let archive = malicious_archive(t.path(), &public, manifest, &payload);
	let dest = t.path().join("restored");
	let error = backup::restore(&archive, &dest, &private).unwrap_err();
	assert!(error.to_string().contains("schemaversion"));
	assert!(!dest.exists());
	assert!(!std::fs::read_dir(t.path()).unwrap().any(|entry| {
		entry
			.unwrap()
			.file_name()
			.to_string_lossy()
			.starts_with(".connect-backup-")
	}));
}
#[test]
fn preserves_signed_directory_delegation() {
	use serein_connect::{
		directory_v2::{Delegation, Publication},
		protocol::Alias,
		storage,
	};
	let t = tempfile::tempdir().unwrap();
	let root = t.path().join("source");
	server::initialize(
		&root,
		ServerConfig::local("@test/publish", "Published").unwrap(),
	)
	.unwrap();
	let raw: [u8; 32] = std::fs::read(root.join("identity.key"))
		.unwrap()
		.try_into()
		.unwrap();
	let key = iroh::SecretKey::from_bytes(&raw);
	let publication = Publication {
		directory: "https://directory.example.com".into(),
		delegation: Delegation::sign(
			&Alias::parse("@test/publish").unwrap(),
			key.public(),
			&iroh::SecretKey::generate(),
		)
		.unwrap(),
	};
	storage::write_json(&root.join("directory-v2.json"), &publication, 16384).unwrap();
	let (private, public) = backup::generate_recovery_key();
	let archive = t.path().join("saved.age");
	backup::export(&root, &archive, &public).unwrap();
	let dest = t.path().join("restored");
	backup::restore(&archive, &dest, &private).unwrap();
	assert_eq!(
		std::fs::read(root.join("directory-v2.json")).unwrap(),
		std::fs::read(dest.join("directory-v2.json")).unwrap()
	);
}
