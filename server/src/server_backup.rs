//! Versioned, bounded server snapshots encrypted with the standard streaming age format.
//! These blocking operations belong on a dedicated worker, never a media executor.
use crate::{
	server::{self, ServerConfig},
	server_store::ServerStore,
};
use age::secrecy::ExposeSecret;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
	collections::BTreeSet,
	fs::{self, File, OpenOptions},
	io::{Read, Write},
	path::{Path, PathBuf},
};
const MAGIC: &[u8] = b"CONNECT-BACKUP-1\n";
const MAX_TOTAL: u64 = 20 * 1024 * 1024 * 1024;
const MAX_ASSET: u64 = 50 * 1024 * 1024;
const MAX_MANIFEST: usize = 32768;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
	version: u32,
	server_id: String,
	entries: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
	path: String,
	size: u64,
}
/// Returns (private recovery identity, public backup recipient). Save these separately.
pub fn generate_recovery_key() -> (String, String) {
	let key = age::x25519::Identity::generate();
	(
		key.to_string().expose_secret().to_owned(),
		key.to_public().to_string(),
	)
}
fn new_file(path: &Path) -> Result<File> {
	let mut options = OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}
	Ok(options.open(path)?)
}
/// Creates a separate recovery file; never changes permissions on its parent directory.
pub fn save_recovery_key(path: &Path, private: &str) -> Result<()> {
	let _: age::x25519::Identity = private
		.parse()
		.map_err(|_| anyhow::anyhow!("Ogiltig återställningsnyckel."))?;
	let mut file = new_file(path)?;
	let result = (|| -> Result<()> {
		#[cfg(windows)]
		{
			use std::os::windows::process::CommandExt;
			let output = std::process::Command::new("whoami.exe")
				.args(["/user", "/fo", "csv", "/nh"])
				.creation_flags(0x08000000)
				.output()?;
			ensure!(output.status.success(), "Kontot kunde inte identifieras.");
			let text = String::from_utf8(output.stdout)?;
			let sid = text
				.trim()
				.split(',')
				.next_back()
				.context("Kontots SID saknas.")?
				.trim_matches('"');
			ensure!(
				sid.starts_with("S-1-")
					&& sid
						.bytes()
						.all(|b| b.is_ascii_digit() || b == b'S' || b == b'-'),
				"Ogiltigt konto."
			);
			let output = std::process::Command::new("icacls.exe")
				.arg(path)
				.args([
					"/inheritance:r",
					"/grant:r",
					&format!("*{sid}:F"),
					"*S-1-5-18:F",
					"*S-1-5-32-544:F",
				])
				.creation_flags(0x08000000)
				.output()?;
			ensure!(
				output.status.success(),
				"Återställningsfilen kunde inte skyddas."
			);
		}
		file.write_all(private.as_bytes())?;
		file.write_all(b"\n")?;
		file.sync_all()?;
		Ok(())
	})();
	drop(file);
	if result.is_err() {
		let _ = fs::remove_file(path);
	}
	result
}
// Every temporary directory is newly created under a canonical, operator-selected parent.
// Cleanup is confined to that exact generated sibling and rejects replacement symlinks.
struct Stage {
	path: PathBuf,
}
impl Stage {
	fn new(parent: &Path) -> Result<Self> {
		let parent = fs::canonicalize(parent)?;
		let path = parent.join(format!(
			".connect-backup-{}",
			iroh::SecretKey::generate().public()
		));
		fs::create_dir(&path)?;
		let stage = Self { path };
		server::protect_directory(&stage.path)?;
		Ok(stage)
	}
}
impl Drop for Stage {
	fn drop(&mut self) {
		if fs::symlink_metadata(&self.path).is_ok_and(|m| m.is_dir() && !is_link(&m))
			&& fs::canonicalize(&self.path).is_ok_and(|p| p == self.path)
		{
			let _ = fs::remove_dir_all(&self.path);
		}
	}
}
fn is_link(metadata: &fs::Metadata) -> bool {
	#[cfg(windows)]
	{
		use std::os::windows::fs::MetadataExt;
		metadata.file_attributes() & 0x400 != 0
	}
	#[cfg(not(windows))]
	{
		metadata.file_type().is_symlink()
	}
}
fn regular(path: &Path) -> Result<u64> {
	let m = fs::symlink_metadata(path)?;
	ensure!(
		m.is_file() && !is_link(&m),
		"Backup tillåter bara vanliga filer."
	);
	Ok(m.len())
}
fn open_regular(path: &Path) -> Result<File> {
	regular(path)?;
	#[cfg(target_os = "linux")]
	let file = File::from(rustix::fs::open(
		path,
		rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
		rustix::fs::Mode::empty(),
	)?);
	#[cfg(windows)]
	let file = {
		use std::os::windows::fs::OpenOptionsExt;
		OpenOptions::new()
			.read(true)
			.custom_flags(0x00200000)
			.open(path)?
	};
	#[cfg(not(any(target_os = "linux", windows)))]
	let file = File::open(path)?;
	let metadata = file.metadata()?;
	ensure!(
		metadata.is_file() && !is_link(&metadata),
		"Backup tillåter bara vanliga filer."
	);
	Ok(file)
}
fn parent(path: &Path) -> Result<PathBuf> {
	Ok(fs::canonicalize(
		path.parent()
			.filter(|p| !p.as_os_str().is_empty())
			.unwrap_or(Path::new(".")),
	)?)
}
fn target(path: &Path) -> Result<PathBuf> {
	Ok(parent(path)?.join(path.file_name().context("Målets filnamn saknas.")?))
}
fn absent(path: &Path) -> Result<()> {
	match fs::symlink_metadata(path) {
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
		_ => anyhow::bail!("Målet finns redan eller kan inte kontrolleras."),
	}
}
fn asset_name(name: &str) -> bool {
	let Some((hash, extension)) = name.split_once('.') else {
		return false;
	};
	hash.len() == 64
		&& hash
			.bytes()
			.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
		&& ["png", "jpeg", "webp", "gif", "mp4"].contains(&extension)
}
fn validate_manifest(m: &Manifest) -> Result<()> {
	ensure!(
		m.version == 1
			&& m.server_id.len() <= 128
			&& m.entries.len() >= 3
			&& m.entries.len() <= 134,
		"Ogiltigt backupmanifest."
	);
	let mut paths = BTreeSet::new();
	let mut total = 0u64;
	let mut assets = 0;
	for (i, e) in m.entries.iter().enumerate() {
		ensure!(
			e.path.len() <= 100 && paths.insert(&e.path),
			"Dubblerad eller ogiltig backupsökväg."
		);
		let max = match (i, e.path.as_str()) {
			(0, "identity.key") => {
				ensure!(e.size == 32, "Ogiltig servernyckel.");
				32
			}
			(1, "server.json") => 16384,
			(2, "server.db") => MAX_TOTAL,
			(_, "directory-v2.json") => 16384,
			(_, p) if p.starts_with("assets/") && asset_name(&p[7..]) => {
				assets += 1;
				MAX_ASSET
			}
			_ => anyhow::bail!("Otillåten backupsökväg."),
		};
		ensure!(
			e.size > 0 && e.size <= max,
			"Backupfilen är för stor eller tom."
		);
		total = total
			.checked_add(e.size)
			.context("Backupstorleken är ogiltig.")?;
		ensure!(total <= MAX_TOTAL, "Backupen är för stor.");
	}
	ensure!(assets <= 130, "För många profiltillgångar.");
	Ok(())
}
fn validate_config(root: &Path) -> Result<ServerConfig> {
	let config = ServerConfig::load(root)?;
	let _: std::net::SocketAddr = config.panel_bind.parse().context("Ogiltig paneladress.")?;
	let origin = url::Url::parse(&config.panel_origin)?;
	ensure!(
		["http", "https"].contains(&origin.scheme())
			&& origin.host_str().is_some()
			&& origin.username().is_empty()
			&& origin.password().is_none()
			&& origin.path() == "/"
			&& origin.query().is_none()
			&& origin.fragment().is_none(),
		"Ogiltigt panelursprung."
	);
	if let Some(directory) = &config.directory_url {
		crate::directory::validate_url(directory)?;
	}
	Ok(config)
}
fn copy_exact(
	reader: &mut impl Read,
	writer: &mut impl Write,
	size: u64,
	asset: Option<&str>,
) -> Result<()> {
	let mut remaining = size;
	let mut buffer = [0u8; 64 * 1024];
	let mut digest = Sha256::new();
	while remaining > 0 {
		let count = reader.read(&mut buffer[..remaining.min(65536) as usize])?;
		ensure!(count > 0, "Backupfilen är avklippt.");
		writer.write_all(&buffer[..count])?;
		if asset.is_some() {
			digest.update(&buffer[..count]);
		}
		remaining -= count as u64;
	}
	if let Some(name) = asset {
		ensure!(
			format!("{:x}", digest.finalize()) == name[..64],
			"Profilfilens kontrollsumma stämmer inte."
		);
	}
	Ok(())
}
/// Exports a consistent online SQLite snapshot without taking the running instance lock.
pub fn export(root: &Path, destination: &Path, recipient: &str) -> Result<()> {
	let recipient: age::x25519::Recipient = recipient
		.parse()
		.map_err(|_| anyhow::anyhow!("Ogiltig backupmottagare."))?;
	let root = fs::canonicalize(root)?;
	let destination = target(destination)?;
	absent(&destination)?;
	ensure!(
		regular(&root.join("identity.key"))? == 32,
		"Ogiltig servernyckel."
	);
	ensure!(
		regular(&root.join("server.json"))? <= 16384,
		"Konfigurationen är för stor."
	);
	ensure!(
		regular(&root.join("server.db"))? <= MAX_TOTAL,
		"Databasen är för stor."
	);
	let config = validate_config(&root)?;
	let key = server::load_key(&root)?;
	let stage = Stage::new(destination.parent().unwrap())?;
	let snapshot = stage.path.join("server.db");
	let store = ServerStore::open(&root.join("server.db"), &config.name)?;
	store.export_database(&snapshot)?;
	drop(store);
	let mut files = vec![
		("identity.key".to_owned(), root.join("identity.key")),
		("server.json".to_owned(), root.join("server.json")),
		("server.db".to_owned(), snapshot),
	];
	let publication = root.join("directory-v2.json");
	if publication.try_exists()? {
		files.push(("directory-v2.json".into(), publication));
	}
	let assets = root.join("assets");
	if assets.try_exists()? {
		let metadata = fs::symlink_metadata(&assets)?;
		ensure!(
			metadata.is_dir() && !is_link(&metadata),
			"Ogiltig tillgångskatalog."
		);
		for entry in fs::read_dir(assets)? {
			let entry = entry?;
			let name = entry
				.file_name()
				.into_string()
				.map_err(|_| anyhow::anyhow!("Ogiltigt tillgångsnamn."))?;
			ensure!(asset_name(&name), "Ogiltigt tillgångsnamn.");
			ensure!(files.len() < 134, "För många profiltillgångar.");
			files.push((format!("assets/{name}"), entry.path()));
		}
	}
	let manifest = Manifest {
		version: 1,
		server_id: key.public().to_string(),
		entries: files
			.iter()
			.map(|(name, path)| {
				Ok(Entry {
					path: name.clone(),
					size: regular(path)?,
				})
			})
			.collect::<Result<_>>()?,
	};
	validate_manifest(&manifest)?;
	let bytes = serde_json::to_vec(&manifest)?;
	ensure!(bytes.len() <= MAX_MANIFEST, "Manifestet är för stort.");
	let output = stage.path.join("encrypted.age");
	let mut writer =
		age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))?
			.wrap_output(new_file(&output)?)?;
	writer.write_all(MAGIC)?;
	writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
	writer.write_all(&bytes)?;
	for ((name, path), entry) in files.iter().zip(&manifest.entries) {
		let mut file = open_regular(path)?;
		copy_exact(
			&mut file,
			&mut writer,
			entry.size,
			name.strip_prefix("assets/"),
		)?;
		ensure!(
			file.read(&mut [0u8; 1])? == 0,
			"Källfilen ändrades under backup."
		);
	}
	writer.finish()?.sync_all()?;
	// A hard link publishes a completed ciphertext atomically without replacing any destination.
	fs::hard_link(output, &destination)
		.context("Backupmålet kunde inte skapas utan överskrivning.")?;
	Ok(())
}
fn validate_database(path: &Path) -> Result<()> {
	let db =
		rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
	ensure!(
		db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))? == 1,
		"Okänd databasschemaversion."
	);
	ensure!(
		db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))? == "ok",
		"Databasen är skadad."
	);
	ensure!(
		!db.prepare("PRAGMA foreign_key_check")?.exists([])?,
		"Databasrelationerna är skadade."
	);
	let mut q = db.prepare("SELECT name,type FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'")?;
	let mut tables = BTreeSet::new();
	for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
		let (name, kind) = row?;
		ensure!(
			["table", "index"].contains(&kind.as_str()),
			"Otillåtet databasschema."
		);
		if kind == "table" {
			tables.insert(name);
		}
	}
	let expected: BTreeSet<String> = [
		"settings", "members", "channels", "invites", "messages", "requests", "audit",
	]
	.into_iter()
	.map(String::from)
	.collect();
	if tables.remove("legacy_import") {
		let columns: Vec<String> = db
			.prepare("PRAGMA table_info(legacy_import)")?
			.query_map([], |row| row.get(1))?
			.collect::<rusqlite::Result<_>>()?;
		ensure!(columns == ["completed"], "Ogiltig migrationsmarkör.");
		let valid: i64 = db.query_row(
			"SELECT count(*) FROM legacy_import WHERE completed=1",
			[],
			|r| r.get(0),
		)?;
		let total: i64 = db.query_row("SELECT count(*) FROM legacy_import", [], |r| r.get(0))?;
		ensure!(valid == 1 && total == 1, "Ogiltig migrationsmarkör.");
	}
	ensure!(tables == expected, "Okänt databasschema.");
	drop(q);
	// Browser sessions and online presence are memory-only. Durable membership remains intact.
	db.execute("UPDATE settings SET setup_hash=NULL,setup_expiry=0", [])?;
	db.execute_batch("PRAGMA synchronous=FULL;")?;
	drop(db);
	Ok(())
}
fn publish_directory(source: &Path, destination: &Path) -> Result<()> {
	#[cfg(target_os = "linux")]
	{
		rustix::fs::renameat_with(
			rustix::fs::CWD,
			source,
			rustix::fs::CWD,
			destination,
			rustix::fs::RenameFlags::NOREPLACE,
		)?;
	}
	#[cfg(windows)]
	{
		fs::rename(source, destination)?;
	}
	#[cfg(not(any(target_os = "linux", windows)))]
	{
		let _ = (source, destination);
		anyhow::bail!("Återställning stöds endast på Windows och Linux.");
	}
	Ok(())
}
/// Authenticates every age chunk before publishing a validated, inactive new server root.
pub fn restore(backup: &Path, destination: &Path, identity: &str) -> Result<()> {
	let identity: age::x25519::Identity = identity
		.parse()
		.map_err(|_| anyhow::anyhow!("Ogiltig återställningsnyckel."))?;
	ensure!(
		regular(backup)? <= MAX_TOTAL + 64 * 1024 * 1024,
		"Backupen är för stor."
	);
	let destination = target(destination)?;
	absent(&destination)?;
	let stage = Stage::new(destination.parent().unwrap())?;
	let lock = server::lock_directory(&stage.path)?;
	let decryptor = age::Decryptor::new(open_regular(backup)?)?;
	let mut reader = decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity))?;
	let mut magic = [0u8; 17];
	reader.read_exact(&mut magic[..MAGIC.len()])?;
	ensure!(&magic[..MAGIC.len()] == MAGIC, "Okänt backupformat.");
	let mut length = [0u8; 4];
	reader.read_exact(&mut length)?;
	let length = u32::from_le_bytes(length) as usize;
	ensure!(length <= MAX_MANIFEST, "Manifestet är för stort.");
	let mut bytes = vec![0; length];
	reader.read_exact(&mut bytes)?;
	let manifest: Manifest = serde_json::from_slice(&bytes)?;
	validate_manifest(&manifest)?;
	for entry in &manifest.entries {
		let path = stage.path.join(&entry.path);
		if entry.path.starts_with("assets/") {
			fs::create_dir_all(stage.path.join("assets"))?;
		}
		let mut file = new_file(&path)?;
		copy_exact(
			&mut reader,
			&mut file,
			entry.size,
			entry.path.strip_prefix("assets/"),
		)?;
		file.sync_all()?;
	}
	// This read is mandatory: the age final chunk authenticates EOF, including exact-boundary truncation.
	ensure!(
		reader.read(&mut [0u8; 1])? == 0,
		"Oväntade bytes efter backupen."
	);
	drop(reader);
	let config = validate_config(&stage.path)?;
	let key = server::load_key(&stage.path)?;
	ensure!(
		key.public().to_string() == manifest.server_id,
		"Backupens serveridentitet stämmer inte."
	);
	let publication = stage.path.join("directory-v2.json");
	if publication.exists() {
		let p: crate::directory_v2::Publication = crate::storage::read_json(&publication, 16384)?;
		p.delegation.verify()?;
		crate::directory::validate_url(&p.directory)?;
		ensure!(
			p.delegation.server == key.public()
				&& format!("@{}/{}", p.delegation.owner, p.delegation.slug) == config.alias,
			"Namnfullmakten tillhör en annan server."
		);
	}
	validate_database(&stage.path.join("server.db"))?;
	let store = ServerStore::open(&stage.path.join("server.db"), &config.name)?;
	store.snapshot()?;
	drop(store);
	OpenOptions::new()
		.read(true)
		.write(true)
		.open(stage.path.join("server.db"))?
		.sync_all()?;
	drop(lock);
	// No data is visible at the requested root until all validation succeeds.
	publish_directory(&stage.path, &destination)?;
	Ok(())
}
