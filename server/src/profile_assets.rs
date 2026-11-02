//! Hash-checked profile assets, separate from room snapshots and chat streams.
use crate::{
	profile_media::{ASSET_LIMIT, validate_asset},
	protocol::{ProfileAssetRef, Snapshot},
};
use anyhow::{Context, Result, ensure};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use sha2::{Digest, Sha256};
use std::{
	collections::{BTreeMap, BTreeSet},
	path::{Path, PathBuf},
	sync::{Arc, Mutex},
	time::Duration,
};
use tokio::{
	io::{AsyncReadExt, AsyncWriteExt},
	sync::{Semaphore, watch},
	time::timeout,
};

const CACHE_LIMIT: u64 = 512 * 1024 * 1024;
const CACHE_FILES: usize = 128;
const DEADLINE: Duration = Duration::from_secs(120);
fn hash(bytes: &[u8]) -> String {
	format!("{:x}", Sha256::digest(bytes))
}

pub struct AssetCache {
	root: PathBuf,
	lock: Mutex<BTreeSet<String>>,
	temporary: bool,
}
impl AssetCache {
	pub fn open(root: impl AsRef<Path>) -> Result<Arc<Self>> {
		std::fs::create_dir_all(root.as_ref())?;
		let cache = Arc::new(Self {
			root: root.as_ref().to_path_buf(),
			lock: Mutex::new(BTreeSet::new()),
			temporary: false,
		});
		cache.prune(0, &BTreeSet::new())?;
		Ok(cache)
	}
	pub fn temporary() -> Result<Arc<Self>> {
		let root = std::env::temp_dir().join(format!(
			"serein-profile-assets-{}",
			iroh::SecretKey::generate().public()
		));
		std::fs::create_dir_all(&root)?;
		Ok(Arc::new(Self {
			root,
			lock: Mutex::new(BTreeSet::new()),
			temporary: true,
		}))
	}
	/// Keep the currently saved avatar and cover while evicting older fetched media.
	pub fn pin(&self, references: &[String]) -> Result<()> {
		let mut pins = self.lock.lock().unwrap();
		let next: BTreeSet<_> = references
			.iter()
			.filter_map(|v| ProfileAssetRef::parse(v).ok())
			.map(|r| self.filename(&r))
			.collect();
		ensure!(next.len() <= 4, "För många sparade profilbilder.");
		*pins = next;
		Ok(())
	}
	fn filename(&self, reference: &ProfileAssetRef) -> String {
		format!("{}.{}", reference.hash, reference.format)
	}
	fn prune(&self, reserve: u64, pins: &BTreeSet<String>) -> Result<()> {
		let mut entries = Vec::new();
		let mut total = 0;
		for entry in std::fs::read_dir(&self.root)? {
			let entry = entry?;
			let name = entry.file_name().to_string_lossy().into_owned();
			if !valid_filename(&name) {
				continue;
			}
			let metadata = entry.metadata()?;
			ensure!(
				!entry.file_type()?.is_symlink() && metadata.is_file(),
				"Ogiltig mediecache."
			);
			total += metadata.len();
			entries.push((metadata.modified()?, entry.path(), name, metadata.len()));
		}
		entries.sort_by_key(|e| e.0);
		let mut count = entries.len();
		for (_, path, name, size) in entries {
			if total + reserve <= CACHE_LIMIT && count + usize::from(reserve > 0) <= CACHE_FILES {
				break;
			}
			if pins.contains(&name) {
				continue;
			}
			std::fs::remove_file(path)?;
			total -= size;
			count -= 1;
		}
		ensure!(
			total + reserve <= CACHE_LIMIT && count + usize::from(reserve > 0) <= CACHE_FILES,
			"Cachen för profilmedia är full."
		);
		Ok(())
	}
	pub fn put(&self, mime: &str, bytes: &[u8]) -> Result<String> {
		validate_asset(mime, bytes)?;
		let format = match mime {
			"image/png" => "png",
			"image/jpeg" => "jpeg",
			"image/webp" => "webp",
			"image/gif" => "gif",
			"video/mp4" => "mp4",
			_ => unreachable!(),
		};
		let reference = ProfileAssetRef {
			format: format.into(),
			hash: hash(bytes),
			size: bytes.len(),
		};
		self.store(&reference, bytes)?;
		Ok(reference.to_string())
	}
	fn store(&self, reference: &ProfileAssetRef, bytes: &[u8]) -> Result<()> {
		ensure!(
			bytes.len() == reference.size && hash(bytes) == reference.hash,
			"Profilfilens kontrollsumma stämmer inte."
		);
		let pins = self.lock.lock().unwrap();
		let path = self.root.join(self.filename(reference));
		if path.exists() && std::fs::read(&path).is_ok_and(|stored| stored == bytes) {
			return Ok(());
		}
		self.prune(bytes.len() as u64, &pins)?;
		let pending = self.root.join("pending");
		std::fs::write(&pending, bytes)?;
		if path.exists() {
			std::fs::remove_file(&path)?;
		}
		std::fs::rename(pending, path)?;
		Ok(())
	}
	pub fn get(&self, value: &str) -> Result<Option<Vec<u8>>> {
		let reference = ProfileAssetRef::parse(value)?;
		let _guard = self.lock.lock().unwrap();
		let path = self.root.join(self.filename(&reference));
		let metadata = match std::fs::symlink_metadata(&path) {
			Ok(m) => m,
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
			Err(e) => return Err(e.into()),
		};
		ensure!(
			metadata.is_file() && !metadata.is_symlink() && metadata.len() == reference.size as u64,
			"Ogiltig cachad profilfil."
		);
		let bytes = std::fs::read(path)?;
		ensure!(
			hash(&bytes) == reference.hash,
			"Profilfilens kontrollsumma stämmer inte."
		);
		Ok(Some(bytes))
	}
}
fn valid_filename(value: &str) -> bool {
	let Some((hash, format)) = value.split_once('.') else {
		return false;
	};
	hash.len() == 64
		&& hash
			.bytes()
			.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
		&& ["png", "jpeg", "webp", "gif", "mp4"].contains(&format)
}
impl Drop for AssetCache {
	fn drop(&mut self) {
		if self.temporary {
			let _ = std::fs::remove_dir_all(&self.root);
		}
	}
}

pub struct ProfileAssets {
	pub cache: Arc<AssetCache>,
	snapshots: watch::Receiver<Snapshot>,
	upstream: Option<String>,
	connections: Mutex<BTreeMap<String, Connection>>,
	downloads: tokio::sync::Mutex<()>,
	slots: Arc<Semaphore>,
}
impl ProfileAssets {
	pub fn new(
		cache: Arc<AssetCache>,
		snapshots: watch::Receiver<Snapshot>,
		upstream: Option<String>,
	) -> Arc<Self> {
		Arc::new(Self {
			cache,
			snapshots,
			upstream,
			connections: Mutex::new(BTreeMap::new()),
			downloads: tokio::sync::Mutex::new(()),
			slots: Arc::new(Semaphore::new(4)),
		})
	}
	pub fn attach(&self, connection: Connection) {
		self.connections
			.lock()
			.unwrap()
			.insert(connection.remote_id().to_string(), connection);
	}
	pub fn detach(&self, peer: &str) {
		self.connections.lock().unwrap().remove(peer);
	}
	fn owners(&self, value: &str) -> Vec<String> {
		let snapshot = self.snapshots.borrow();
		let advertises = |profile: &crate::protocol::Profile| {
			profile
				.card
				.as_ref()
				.is_some_and(|c| c.avatar == value || c.cover == value)
		};
		snapshot
			.members
			.iter()
			.filter(|m| advertises(&m.profile))
			.map(|m| m.id.clone())
			.chain(
				snapshot
					.messages
					.iter()
					.filter(|m| advertises(&m.author))
					.map(|m| m.author_id.clone()),
			)
			.collect::<BTreeSet<_>>()
			.into_iter()
			.collect()
	}
	fn member(&self, peer: &str) -> bool {
		self.snapshots.borrow().members.iter().any(|m| m.id == peer)
	}
	pub async fn resolve(&self, value: &str) -> Result<Vec<u8>> {
		let reference = ProfileAssetRef::parse(value)?;
		let owners = self.owners(value);
		ensure!(
			!owners.is_empty(),
			"Profilfilen är inte publicerad i rummet."
		);
		let _download = self.downloads.lock().await;
		let cache = self.cache.clone();
		let key = value.to_string();
		if let Some(bytes) = tokio::task::spawn_blocking(move || cache.get(&key)).await?? {
			return Ok(bytes);
		}
		let connection = {
			let connections = self.connections.lock().unwrap();
			if let Some(host) = &self.upstream {
				connections.get(host).cloned()
			} else {
				owners
					.iter()
					.find_map(|peer| connections.get(peer).cloned())
			}
		}
		.context("Profilbildens avsändare är inte ansluten.")?;
		let bytes = timeout(DEADLINE, fetch(&connection, value, &reference)).await??;
		ensure!(
			!self.owners(value).is_empty(),
			"Profilbilden är inte längre publicerad."
		);
		let cache = self.cache.clone();
		let mime = reference.mime().to_string();
		let expected = value.to_string();
		tokio::task::spawn_blocking(move || {
			ensure!(
				hash(&bytes) == reference.hash,
				"Profilfilens kontrollsumma stämmer inte."
			);
			ensure!(
				cache.put(&mime, &bytes)? == expected,
				"Ogiltig mediereferens."
			);
			Ok(bytes)
		})
		.await?
	}
	/// Run only after room admission / initial snapshot, alongside chat and media tasks.
	pub async fn serve_connection(self: Arc<Self>, connection: Connection) -> Result<()> {
		let mut tasks = tokio::task::JoinSet::new();
		loop {
			tokio::select! {
				accepted = connection.accept_bi() => {
					let (mut send, mut recv) = accepted?;
					let Ok(slot) = self.slots.clone().try_acquire_owned() else {
						let _ = send.reset(1u32.into());
						let _ = recv.stop(1u32.into());
						continue;
					};
					let assets = self.clone();
					let peer = connection.remote_id().to_string();
					tasks.spawn(async move {
						let _slot = slot;
						let _ = timeout(DEADLINE, assets.serve_stream(&peer, send, recv)).await;
					});
				}
				_ = tasks.join_next(), if !tasks.is_empty() => {}
			}
		}
	}
	async fn serve_stream(
		&self,
		peer: &str,
		mut send: SendStream,
		mut recv: RecvStream,
	) -> Result<()> {
		ensure!(self.member(peer), "Deltagaren är inte ansluten.");
		let length = recv.read_u16().await? as usize;
		ensure!(length <= 112, "Ogiltig medieförfrågan.");
		let mut request = vec![0; length];
		recv.read_exact(&mut request).await?;
		let value = String::from_utf8(request)?;
		ProfileAssetRef::parse(&value)?;
		ensure!(
			!self.owners(&value).is_empty(),
			"Profilfilen är inte publicerad i rummet."
		);
		// Guests serve local bytes only: a missing owner asset must never loop via the host.
		let bytes = if self.upstream.is_some() {
			let cache = self.cache.clone();
			tokio::task::spawn_blocking(move || cache.get(&value))
				.await??
				.context("Profilfilen saknas.")?
		} else {
			self.resolve(&value).await?
		};
		ensure!(self.member(peer), "Deltagaren har lämnat rummet.");
		send.write_u32(bytes.len() as u32).await?;
		send.write_all(&bytes).await?;
		send.finish()?;
		Ok(())
	}
}
async fn fetch(
	connection: &Connection,
	value: &str,
	reference: &ProfileAssetRef,
) -> Result<Vec<u8>> {
	let (mut send, mut recv) = connection.open_bi().await?;
	send.write_u16(value.len() as u16).await?;
	send.write_all(value.as_bytes()).await?;
	send.finish()?;
	let size = recv.read_u32().await? as usize;
	ensure!(
		size == reference.size && size <= ASSET_LIMIT,
		"Ogiltig profilfillängd."
	);
	let bytes = recv.read_to_end(size).await?;
	ensure!(bytes.len() == size, "Ogiltig profilfillängd.");
	Ok(bytes)
}
