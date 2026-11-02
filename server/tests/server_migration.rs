use iroh::SecretKey;
use serein_connect::{
	protocol::*,
	server,
	server_migration::{self, Migration},
	server_store::ServerStore,
};
#[test]
fn signed_import_is_target_bound_atomic_and_preserves_historical_authors() {
	let root = tempfile::tempdir().unwrap();
	let data = root.path().join("server");
	let setup = server::initialize(
		&data,
		server::ServerConfig::local("@new/room", "New").unwrap(),
	)
	.unwrap();
	let old = SecretKey::generate();
	let target = setup.server_id.parse().unwrap();
	let snapshot = Snapshot {
		name: "Old".into(),
		channels: vec![Channel {
			id: "old".into(),
			name: "Old".into(),
			kind: ChannelKind::Text,
		}],
		members: vec![],
		messages: vec![Message {
			id: 12,
			channel: "old".into(),
			author_id: old.public().to_string(),
			author: Profile {
				name: "Old author".into(),
				accent: [1, 2, 3],
				card: None,
			},
			text: "Saved history".into(),
			sent_at: 1,
		}],
	};
	let migration = Migration::sign(snapshot, target, &old).unwrap();
	let path = root.path().join("migration.json");
	std::fs::write(&path, serde_json::to_vec(&migration).unwrap()).unwrap();
	server_migration::import(&data, &path).unwrap();
	let store = ServerStore::open(&data.join("server.db"), "New").unwrap();
	assert_eq!(
		store.snapshot().unwrap().messages[0].author_id,
		old.public().to_string()
	);
	assert!(store.snapshot().unwrap().members.is_empty());
	assert!(server_migration::import(&data, &path).is_err());
	let mut changed = migration.clone();
	changed.snapshot.messages[0].text = "Tampered".into();
	assert!(changed.verify().is_err());
	let other = root.path().join("other");
	server::initialize(
		&other,
		server::ServerConfig::local("@new/other", "Other").unwrap(),
	)
	.unwrap();
	assert!(server_migration::import(&other, &path).is_err());
	assert!(
		ServerStore::open(&other.join("server.db"), "Other")
			.unwrap()
			.snapshot()
			.unwrap()
			.messages
			.is_empty()
	);
}
