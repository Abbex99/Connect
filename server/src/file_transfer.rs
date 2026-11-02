//! Consent-gated peer file streams. The room carries metadata only; file bytes use a
//! separate authenticated Iroh connection directly between the two participants.
use crate::{
	network::write_frame,
	protocol::{Command, Snapshot},
};
use anyhow::{Context, Result, ensure};
use iroh::{Endpoint, EndpointAddr, SecretKey, endpoint::Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
	collections::BTreeMap,
	path::PathBuf,
	sync::{Arc, Mutex},
	time::Duration,
};
use tokio::{
	io::{AsyncReadExt, AsyncWriteExt},
	sync::{Semaphore, mpsc, watch},
	time::timeout,
};
pub const FILE_ALPN: &[u8] = b"serein-connect/file/1";
pub const MAX_FILE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_PENDING: usize = 8;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Offer {
	pub id: String,
	pub name: String,
	pub size: u64,
	pub address: EndpointAddr,
	pub capability: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Signal {
	Offer(Offer),
	Decline { id: String },
	Cancel { id: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Directed {
	pub recipient: String,
	pub signal: Signal,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
	pub sender: String,
	pub sender_name: String,
	pub signal: Signal,
}
#[derive(Clone, Debug, Serialize)]
pub struct Transfer {
	pub id: String,
	pub peer: String,
	pub sender_name: String,
	pub name: String,
	pub size: u64,
	pub done: u64,
	pub incoming: bool,
	pub status: String,
	pub path: Option<String>,
	pub error: Option<String>,
}
struct Entry {
	view: Transfer,
	offer: Offer,
	file: Option<tokio::fs::File>,
	cancel: watch::Sender<bool>,
	created: std::time::Instant,
}
pub struct FileTransfers {
	endpoint: Endpoint,
	commands: mpsc::Sender<Command>,
	snapshots: watch::Receiver<Snapshot>,
	entries: Mutex<BTreeMap<String, Entry>>,
	stop: watch::Sender<bool>,
	slots: Arc<Semaphore>,
}
fn random_id() -> String {
	SecretKey::generate()
		.to_bytes()
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect()
}
fn valid_id(id: &str) -> bool {
	id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn validate_filename(name: &str) -> Result<()> {
	ensure!(
		!name.is_empty()
			&& name.len() <= 180
			&& !name.ends_with(['.', ' '])
			&& !name
				.chars()
				.any(|c| c.is_control() || "/\\:<>\"|?*".contains(c)),
		"Ogiltigt filnamn."
	);
	let stem = name
		.split('.')
		.next()
		.unwrap_or_default()
		.to_ascii_uppercase();
	ensure!(
		![
			"CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
			"COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"
		]
		.contains(&stem.as_str()),
		"Ogiltigt filnamn."
	);
	Ok(())
}
impl Signal {
	pub fn validate(&self, sender: &str) -> Result<()> {
		match self {
			Self::Offer(o) => {
				validate_filename(&o.name)?;
				ensure!(
					o.size <= MAX_FILE && valid_id(&o.id) && valid_id(&o.capability),
					"Ogiltigt filerbjudande (högst 8 GiB)."
				);
				ensure!(
					o.address.id.to_string() == sender
						&& serde_json::to_vec(&o.address)?.len() <= 8192,
					"Ogiltig avsändaradress."
				);
			}
			Self::Decline { id } | Self::Cancel { id } => {
				ensure!(valid_id(id), "Ogiltig överföring.")
			}
		};
		Ok(())
	}
}
fn active(status: &str) -> bool {
	matches!(status, "offered" | "waiting" | "transferring")
}
impl FileTransfers {
	pub(crate) fn new(
		endpoint: Endpoint,
		commands: mpsc::Sender<Command>,
		snapshots: watch::Receiver<Snapshot>,
	) -> Arc<Self> {
		let (stop, _) = watch::channel(false);
		Arc::new(Self {
			endpoint,
			commands,
			snapshots,
			entries: Mutex::new(BTreeMap::new()),
			stop,
			slots: Arc::new(Semaphore::new(2)),
		})
	}
	pub fn list(&self) -> Vec<Transfer> {
		self.entries
			.lock()
			.unwrap()
			.values()
			.map(|e| e.view.clone())
			.collect()
	}
	fn paired(&self, peer: &str) -> Result<()> {
		let snapshot = self.snapshots.borrow();
		let me = self.endpoint.id().to_string();
		let own = snapshot
			.members
			.iter()
			.find(|m| m.id == me)
			.and_then(|m| m.voice.as_ref());
		let other = snapshot
			.members
			.iter()
			.find(|m| m.id == peer)
			.and_then(|m| m.voice.as_ref());
		ensure!(
			me != peer && own.zip(other).is_some_and(|(a, b)| a.channel == b.channel),
			"Välj en annan deltagare i ditt samtal."
		);
		ensure!(!*self.stop.borrow(), "Anslutningen är stängd.");
		Ok(())
	}
	fn signal(&self, peer: String, signal: Signal) -> Result<()> {
		self.commands
			.try_send(Command::FileSignal(Directed {
				recipient: peer,
				signal,
			}))
			.map_err(|_| anyhow::anyhow!("Anslutningen är upptagen eller stängd."))
	}
	fn reserve(entries: &mut BTreeMap<String, Entry>) -> Result<()> {
		ensure!(
			entries.values().filter(|e| active(&e.view.status)).count() < MAX_PENDING,
			"Högst åtta väntande filöverföringar."
		);
		if entries.len() >= 32 {
			let old = entries
				.iter()
				.find(|(_, e)| !active(&e.view.status))
				.map(|(id, _)| id.clone());
			if let Some(id) = old {
				entries.remove(&id);
			}
		}
		Ok(())
	}
	pub async fn offer(self: &Arc<Self>, peer: String, path: PathBuf) -> Result<String> {
		self.paired(&peer)?;
		let name = path
			.file_name()
			.and_then(|v| v.to_str())
			.context("Ogiltigt filnamn.")?
			.to_string();
		validate_filename(&name)?;
		let file = tokio::fs::File::open(&path).await?;
		let metadata = file.metadata().await?;
		ensure!(
			metadata.is_file() && metadata.len() <= MAX_FILE,
			"Välj en fil på högst 8 GiB."
		);
		// An outbound room connection can succeed before this guest has its own
		// advertised relay address. Never send an offer the receiver cannot dial.
		let mut address = self.endpoint.addr();
		if address.addrs.is_empty() {
			timeout(Duration::from_secs(15), self.endpoint.online()).await
				.context("Din filanslutning är inte redo. Reläadressen kunde inte hämtas inom 15 sekunder. Försök igen.")?;
			address = self.endpoint.addr();
		}
		self.paired(&peer)?;
		ensure!(
			!address.addrs.is_empty(),
			"Din filanslutning saknar en nåbar adress. Anslut igen och försök på nytt."
		);
		let offer = Offer {
			id: random_id(),
			name: name.clone(),
			size: metadata.len(),
			address,
			capability: random_id(),
		};
		let id = offer.id.clone();
		let (cancel, _) = watch::channel(false);
		{
			let mut entries = self.entries.lock().unwrap();
			Self::reserve(&mut entries)?;
			entries.insert(
				id.clone(),
				Entry {
					view: Transfer {
						id: id.clone(),
						peer: peer.clone(),
						sender_name: String::new(),
						name,
						size: offer.size,
						done: 0,
						incoming: false,
						status: "waiting".into(),
						path: None,
						error: None,
					},
					offer: offer.clone(),
					file: Some(file),
					cancel,
					created: std::time::Instant::now(),
				},
			);
		}
		if let Err(e) = self.signal(peer, Signal::Offer(offer)) {
			self.finish(&id, Err(anyhow::anyhow!(e.to_string())));
			return Err(e);
		}
		Ok(id)
	}
	pub(crate) fn receive(&self, event: Envelope) -> Result<()> {
		event.signal.validate(&event.sender)?;
		let declined = matches!(&event.signal, Signal::Decline { .. });
		match event.signal {
			Signal::Offer(offer) => {
				self.paired(&event.sender)?;
				let mut entries = self.entries.lock().unwrap();
				ensure!(
					!entries.contains_key(&offer.id),
					"Duplicerat filerbjudande."
				);
				Self::reserve(&mut entries)?;
				let (cancel, _) = watch::channel(false);
				entries.insert(
					offer.id.clone(),
					Entry {
						view: Transfer {
							id: offer.id.clone(),
							peer: event.sender,
							sender_name: event.sender_name,
							name: offer.name.clone(),
							size: offer.size,
							done: 0,
							incoming: true,
							status: "offered".into(),
							path: None,
							error: None,
						},
						offer,
						file: None,
						cancel,
						created: std::time::Instant::now(),
					},
				);
			}
			Signal::Decline { id } | Signal::Cancel { id } => {
				let mut entries = self.entries.lock().unwrap();
				if let Some(e) = entries.get_mut(&id) {
					ensure!(e.view.peer == event.sender, "Fel avsändare.");
					if active(&e.view.status) || (!declined && e.view.status == "failed") {
						e.view.error = None;
						e.cancel.send_replace(true);
						e.file = None;
						e.view.status = if declined { "declined" } else { "cancelled" }.into();
					}
				}
			}
		}
		Ok(())
	}
	pub fn decline(&self, id: &str) -> Result<()> {
		self.end(id, "declined", true)
	}
	pub fn cancel(&self, id: &str) -> Result<()> {
		self.end(id, "cancelled", false)
	}
	fn end(&self, id: &str, status: &str, decline: bool) -> Result<()> {
		let peer = {
			let mut entries = self.entries.lock().unwrap();
			let e = entries.get_mut(id).context("Överföringen finns inte.")?;
			if decline {
				ensure!(
					e.view.incoming && e.view.status == "offered",
					"Erbjudandet gäller inte längre."
				);
			}
			ensure!(active(&e.view.status), "Överföringen är redan avslutad.");
			e.cancel.send_replace(true);
			e.file = None;
			e.view.status = status.into();
			e.view.peer.clone()
		};
		self.signal(
			peer,
			if decline {
				Signal::Decline { id: id.into() }
			} else {
				Signal::Cancel { id: id.into() }
			},
		)
	}
	pub fn close(&self) {
		self.stop.send_replace(true);
		let mut entries = self.entries.lock().unwrap();
		for e in entries.values_mut() {
			if active(&e.view.status) {
				e.cancel.send_replace(true);
				e.file = None;
				e.view.status = "cancelled".into();
			}
		}
	}
	pub(crate) async fn maintain(self: Arc<Self>) {
		let mut interval = tokio::time::interval(Duration::from_secs(1));
		loop {
			interval.tick().await;
			if *self.stop.borrow() {
				break;
			}
			let ids: Vec<_> = self
				.entries
				.lock()
				.unwrap()
				.iter()
				.filter(|(_, e)| {
					active(&e.view.status)
						&& (self.paired(&e.view.peer).is_err()
							|| (e.view.status != "transferring"
								&& e.created.elapsed() > Duration::from_secs(300)))
				})
				.map(|(id, _)| id.clone())
				.collect();
			for id in ids {
				let _ = self.cancel(&id);
			}
		}
	}
	fn progress(&self, id: &str, done: u64) {
		if let Some(e) = self.entries.lock().unwrap().get_mut(id) {
			e.view.done = done;
		}
	}
	fn finish(&self, id: &str, result: Result<Option<PathBuf>>) {
		if let Some(e) = self.entries.lock().unwrap().get_mut(id) {
			if !active(&e.view.status) {
				return;
			}
			e.file = None;
			match result {
				Ok(path) => {
					e.view.status = "complete".into();
					e.view.path = path.map(|p| p.to_string_lossy().into_owned());
				}
				Err(error) => {
					e.view.status = "failed".into();
					e.view.error = Some(error.to_string());
				}
			}
		}
	}
	pub async fn accept(self: &Arc<Self>, id: &str, directory: PathBuf) -> Result<()> {
		let permit = self
			.slots
			.clone()
			.try_acquire_owned()
			.context("Två filer överförs redan. Vänta tills en är klar.")?;
		let (offer, peer, mut cancelled) = {
			let mut entries = self.entries.lock().unwrap();
			let e = entries.get_mut(id).context("Erbjudandet finns inte.")?;
			ensure!(
				e.view.incoming && e.view.status == "offered",
				"Erbjudandet gäller inte längre."
			);
			self.paired(&e.view.peer)?;
			e.view.status = "transferring".into();
			(e.offer.clone(), e.view.peer.clone(), e.cancel.subscribe())
		};
		let this = self.clone();
		let id = id.to_string();
		let mut stop = self.stop.subscribe();
		tokio::spawn(async move {
			let _permit = permit;
			let result = tokio::select! {result=this.download(&offer,&peer,directory)=>result,_=cancelled.changed()=>Err(anyhow::anyhow!("Överföringen avbröts.")),_=stop.changed()=>Err(anyhow::anyhow!("Anslutningen stängdes."))};
			this.finish(&id, result);
		});
		Ok(())
	}
	async fn download(
		&self,
		offer: &Offer,
		peer: &str,
		directory: PathBuf,
	) -> Result<Option<PathBuf>> {
		tokio::fs::create_dir_all(&directory).await?;
		let target = directory.join(format!("{}-{}", &offer.id[..16], offer.name));
		let part = directory.join(format!(".{}.part", offer.id));
		let cleanup = PartialFile::create(part)?;
		let connection = timeout(
			Duration::from_secs(30),
			self.endpoint.connect(offer.address.clone(), FILE_ALPN),
		)
		.await??;
		ensure!(connection.remote_id().to_string() == peer, "Fel mottagare.");
		let result = self
			.download_stream(offer, &connection, cleanup, &target)
			.await;
		connection.close(0u32.into(), b"file finished");
		result
	}
	async fn download_stream(
		&self,
		offer: &Offer,
		connection: &Connection,
		mut partial: PartialFile,
		target: &std::path::Path,
	) -> Result<Option<PathBuf>> {
		let (mut send, mut recv) = connection.open_bi().await?;
		write_frame(
			&mut send,
			&Request {
				id: offer.id.clone(),
				capability: offer.capability.clone(),
			},
		)
		.await?;
		let ready: bool = timeout(Duration::from_secs(15), read_control(&mut recv)).await??;
		ensure!(ready, "Avsändaren kunde inte skicka filen.");
		let mut buffer = vec![0u8; 64 * 1024];
		let mut total = 0u64;
		let mut hash = Sha256::new();
		while total < offer.size {
			let n = (offer.size - total).min(buffer.len() as u64) as usize;
			timeout(Duration::from_secs(60), recv.read_exact(&mut buffer[..n])).await??;
			partial
				.file
				.as_mut()
				.unwrap()
				.write_all(&buffer[..n])
				.await?;
			hash.update(&buffer[..n]);
			total += n as u64;
			self.progress(&offer.id, total);
		}
		let mut expected = [0u8; 32];
		timeout(Duration::from_secs(60), recv.read_exact(&mut expected)).await??;
		ensure!(
			hash.finalize().as_slice() == expected,
			"Filens kontrollsumma stämmer inte."
		);
		let mut extra = [0u8; 1];
		ensure!(
			timeout(Duration::from_secs(10), recv.read(&mut extra))
				.await??
				.is_none(),
			"Filen innehåller oväntade extra byte."
		);
		let file = partial.file.take().unwrap();
		file.sync_all().await?;
		drop(file);
		self.commit(&offer.id, &partial.path, target)?;
		// Once committed, a lost acknowledgement must not misreport the saved file.
		if write_frame(&mut send, &true).await.is_ok() && send.finish().is_ok() {
			let _ = timeout(Duration::from_secs(10), send.stopped()).await;
		}
		Ok(Some(target.to_path_buf()))
	}
	// Cancellation and publication share one lock, with no await between creating
	// the final directory entry and making its path visible to the recipient.
	fn commit(&self, id: &str, partial: &std::path::Path, target: &std::path::Path) -> Result<()> {
		let mut entries = self.entries.lock().unwrap();
		let entry = entries.get_mut(id).context("Överföringen finns inte.")?;
		ensure!(
			entry.view.status == "transferring" && !*self.stop.borrow(),
			"Överföringen avbröts."
		);
		// Atomic no-overwrite publication. Synchronous metadata operations prevent a
		// cancelled async filesystem job from publishing a file after cleanup.
		std::fs::hard_link(partial, target)
			.context("Filen kunde inte sparas utan att skriva över en befintlig fil.")?;
		entry.view.status = "complete".into();
		entry.view.path = Some(target.to_string_lossy().into_owned());
		Ok(())
	}
	pub(crate) async fn serve(self: Arc<Self>, connection: Connection) -> Result<()> {
		let result = self.serve_inner(&connection).await;
		connection.close(0u32.into(), b"file finished");
		result
	}
	async fn serve_inner(&self, connection: &Connection) -> Result<()> {
		let _permit = self.slots.clone().try_acquire_owned()?;
		let peer = connection.remote_id().to_string();
		self.paired(&peer)?;
		let (mut send, mut recv) =
			timeout(Duration::from_secs(10), connection.accept_bi()).await??;
		let request: Request = timeout(Duration::from_secs(10), read_control(&mut recv)).await??;
		let (mut file, size, mut cancelled) = {
			let mut entries = self.entries.lock().unwrap();
			let e = entries.get_mut(&request.id).context("Okänd överföring.")?;
			ensure!(
				!e.view.incoming
					&& e.view.peer == peer
					&& e.view.status == "waiting"
					&& e.offer.capability == request.capability
					&& e.created.elapsed() < Duration::from_secs(300),
				"Filen har inte godkänts för denna mottagare."
			);
			let file = e.file.take().context("Filen skickas redan.")?;
			e.view.status = "transferring".into();
			(file, e.offer.size, e.cancel.subscribe())
		};
		let mut stop = self.stop.subscribe();
		let transfer = async {
			write_frame(&mut send, &true).await?;
			let mut buffer = vec![0u8; 64 * 1024];
			let mut hash = Sha256::new();
			let mut total = 0u64;
			while total < size {
				let n = (size - total).min(buffer.len() as u64) as usize;
				file.read_exact(&mut buffer[..n]).await?;
				hash.update(&buffer[..n]);
				timeout(Duration::from_secs(60), send.write_all(&buffer[..n])).await??;
				total += n as u64;
				self.progress(&request.id, total);
			}
			let mut extra = [0u8; 1];
			ensure!(
				file.read(&mut extra).await? == 0,
				"Filen ändrades under överföringen."
			);
			timeout(Duration::from_secs(60), send.write_all(&hash.finalize())).await??;
			send.finish()?;
			let received: bool =
				timeout(Duration::from_secs(60), read_control(&mut recv)).await??;
			ensure!(received, "Mottagaren kunde inte spara filen.");
			Ok(None)
		};
		let result = tokio::select! {result=transfer=>result,_=cancelled.changed()=>Err(anyhow::anyhow!("Överföringen avbröts.")),_=stop.changed()=>Err(anyhow::anyhow!("Anslutningen stängdes."))};
		self.finish(&request.id, result);
		Ok(())
	}
}
#[derive(Serialize, Deserialize)]
struct Request {
	id: String,
	capability: String,
}
struct PartialFile {
	file: Option<tokio::fs::File>,
	path: PathBuf,
}
impl PartialFile {
	fn create(path: PathBuf) -> Result<Self> {
		// Construct the cleanup guard before any async cancellation point.
		let file = std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&path)?;
		Ok(Self {
			file: Some(tokio::fs::File::from_std(file)),
			path,
		})
	}
}
impl Drop for PartialFile {
	fn drop(&mut self) {
		drop(self.file.take());
		let _ = std::fs::remove_file(&self.path);
	}
}

// File control messages never inherit the much larger profile/snapshot frame budget.
async fn read_control<T: serde::de::DeserializeOwned>(
	recv: &mut iroh::endpoint::RecvStream,
) -> Result<T> {
	let mut length = [0u8; 4];
	recv.read_exact(&mut length).await?;
	let length = u32::from_be_bytes(length) as usize;
	ensure!(length > 0 && length <= 1024, "Ogiltigt filkontrollpaket.");
	let mut bytes = vec![0u8; length];
	recv.read_exact(&mut bytes).await?;
	Ok(serde_json::from_slice(&bytes)?)
}
#[cfg(test)]
mod tests {
	use super::*;
	use crate::protocol::{Member, Profile, VoiceState};
	#[test]
	fn partial_guard_cleans_up_without_an_async_creation_gap() {
		let tmp = tempfile::tempdir().unwrap();
		let path = tmp.path().join("partial");
		let guard = PartialFile::create(path.clone()).unwrap();
		assert!(path.exists());
		drop(guard);
		assert!(!path.exists());
		std::fs::write(&path, b"existing").unwrap();
		assert!(PartialFile::create(path.clone()).is_err());
		assert_eq!(std::fs::read(path).unwrap(), b"existing");
	}
	#[tokio::test]
	async fn commit_and_cancellation_have_a_single_order() {
		let endpoint = Endpoint::builder(iroh::endpoint::presets::Minimal)
			.bind_addr("127.0.0.1:0")
			.unwrap()
			.bind()
			.await
			.unwrap();
		let (commands, _rx) = mpsc::channel(8);
		let (_, snapshots) = watch::channel(Snapshot::default());
		let files = FileTransfers::new(endpoint.clone(), commands, snapshots);
		let tmp = tempfile::tempdir().unwrap();
		for cancel_first in [true, false] {
			let id = random_id();
			let partial = tmp.path().join(format!("{id}.part"));
			let target = tmp.path().join(format!("{id}.zip"));
			let guard = PartialFile::create(partial.clone()).unwrap();
			let (cancel, _) = watch::channel(false);
			files.entries.lock().unwrap().insert(
				id.clone(),
				Entry {
					view: Transfer {
						id: id.clone(),
						peer: "peer".into(),
						sender_name: "Sender".into(),
						name: "data.zip".into(),
						size: 0,
						done: 0,
						incoming: true,
						status: "transferring".into(),
						path: None,
						error: None,
					},
					offer: Offer {
						id: id.clone(),
						name: "data.zip".into(),
						size: 0,
						address: endpoint.addr(),
						capability: random_id(),
					},
					file: None,
					cancel,
					created: std::time::Instant::now(),
				},
			);
			if cancel_first {
				files.cancel(&id).unwrap();
				assert!(files.commit(&id, &partial, &target).is_err());
				drop(guard);
				assert!(!target.exists());
				assert!(!partial.exists());
			} else {
				files.commit(&id, &partial, &target).unwrap();
				assert!(files.cancel(&id).is_err());
				files.close();
				files.finish(&id, Err(anyhow::anyhow!("lost acknowledgement")));
				let view = files.list().into_iter().find(|t| t.id == id).unwrap();
				assert_eq!(view.status, "complete");
				assert_eq!(view.path, Some(target.to_string_lossy().into_owned()));
				assert!(target.exists());
				drop(guard);
				assert!(!partial.exists());
			}
		}
		endpoint.close().await;
	}
	#[tokio::test]
	async fn corrupted_digest_removes_partial_file() {
		let sender = Endpoint::builder(iroh::endpoint::presets::Minimal)
			.alpns(vec![FILE_ALPN.to_vec()])
			.bind_addr("127.0.0.1:0")
			.unwrap()
			.bind()
			.await
			.unwrap();
		let receiver = Endpoint::builder(iroh::endpoint::presets::Minimal)
			.alpns(vec![FILE_ALPN.to_vec()])
			.bind_addr("127.0.0.1:0")
			.unwrap()
			.bind()
			.await
			.unwrap();
		let members = [&sender, &receiver].map(|e| Member {
			id: e.id().to_string(),
			profile: Profile {
				name: "Test".into(),
				accent: [0; 3],
				card: None,
			},
			host: false,
			voice: Some(VoiceState {
				channel: "call".into(),
				epoch: 1,
				muted: true,
				deafened: false,
				screen: false,
				screen_epoch: 0,
			}),
		});
		let (_, snapshots) = watch::channel(Snapshot {
			members: members.to_vec(),
			..Default::default()
		});
		let (commands, _) = mpsc::channel(8);
		let files = FileTransfers::new(receiver.clone(), commands, snapshots);
		let offer = Offer {
			id: random_id(),
			name: "corrupt.zip".into(),
			size: 5,
			address: sender.addr(),
			capability: random_id(),
		};
		let id = offer.id.clone();
		files
			.receive(Envelope {
				sender: sender.id().to_string(),
				sender_name: "Sender".into(),
				signal: Signal::Offer(offer),
			})
			.unwrap();
		let task_sender = sender.clone();
		let corrupt = tokio::spawn(async move {
			let connection = task_sender.accept().await.unwrap().await.unwrap();
			let (mut send, mut recv) = connection.accept_bi().await.unwrap();
			let _: Request = read_control(&mut recv).await.unwrap();
			write_frame(&mut send, &true).await.unwrap();
			send.write_all(b"hello").await.unwrap();
			send.write_all(&[0u8; 32]).await.unwrap();
			send.finish().unwrap();
			let _ = connection.closed().await;
		});
		let tmp = tempfile::tempdir().unwrap();
		files.accept(&id, tmp.path().into()).await.unwrap();
		timeout(Duration::from_secs(5), async {
			loop {
				if files.list()[0].status == "failed" {
					break;
				}
				tokio::time::sleep(Duration::from_millis(10)).await;
			}
		})
		.await
		.unwrap();
		assert!(
			files.list()[0]
				.error
				.as_ref()
				.unwrap()
				.contains("kontrollsumma")
		);
		assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
		files.close();
		receiver.close().await;
		sender.close().await;
		corrupt.await.unwrap();
	}
}
