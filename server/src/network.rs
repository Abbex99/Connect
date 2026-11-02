use crate::file_transfer::{self, FileTransfers};
use crate::media::{self, Frame as MediaFrame};
use crate::profile_assets::{AssetCache, ProfileAssets};
use crate::protocol::*;
use crate::{directory, room::Room};
use anyhow::{Context, Result, ensure};
use iroh::{
	Endpoint, RelayMap, RelayMode, SecretKey,
	endpoint::{Connection, RecvStream, SendStream, presets},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{
	path::PathBuf,
	sync::{Arc, Mutex},
	time::Duration,
};
use tokio::sync::{mpsc, watch};
use tokio::{task::JoinSet, time::timeout};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TransportConfig {
	pub relay_url: Option<String>,
	pub offline: bool,
	pub relay_only: bool,
}
pub struct HostOptions {
	pub alias: Alias,
	pub name: String,
	pub profile: Profile,
	pub public: bool,
	pub history: PathBuf,
	pub directory_url: Option<String>,
}
pub struct Session {
	pub dedicated: Option<crate::server_control::Client>,
	pub assets: Arc<ProfileAssets>,
	pub files: Arc<FileTransfers>,
	pub media: media::MediaInbox,
	audio_tx: mpsc::Sender<MediaFrame>,
	screen_audio_tx: mpsc::Sender<MediaFrame>,
	video_tx: mpsc::Sender<MediaFrame>,
	pub snapshots: watch::Receiver<Snapshot>,
	pub notices: mpsc::Receiver<String>,
	pub invite: Option<Invite>,
	pub self_id: String,
	commands: mpsc::Sender<Command>,
	endpoint: Endpoint,
	task: tokio::task::JoinHandle<()>,
	runtime: tokio::runtime::Handle,
}
impl Session {
	pub fn is_admin(&self) -> bool {
		self.snapshots
			.borrow()
			.members
			.iter()
			.any(|member| member.id == self.self_id && member.host)
	}
	pub fn send_media(&self, frame: MediaFrame) -> Result<()> {
		frame.validate()?;
		let tx = if frame.kind == media::Kind::Audio {
			&self.audio_tx
		} else if frame.kind.is_audio() {
			&self.screen_audio_tx
		} else {
			&self.video_tx
		};
		tx.try_send(frame)
			.map_err(|_| anyhow::anyhow!("Mediaanslutningen är upptagen eller stängd."))
	}
	pub fn send(&self, command: Command) -> Result<()> {
		self.commands
			.try_send(command)
			.map_err(|_| anyhow::anyhow!("Anslutningen är upptagen eller stängd. Försök igen."))
	}
	pub async fn close(self) {
		self.files.close();
		self.endpoint.close().await;
	}
	pub fn is_closed(&self) -> bool {
		self.commands.is_closed()
	}
}
impl Drop for Session {
	fn drop(&mut self) {
		self.files.close();
		self.task.abort();
		let endpoint = self.endpoint.clone();
		self.runtime.spawn(async move {
			endpoint.close().await;
		});
	}
}

async fn endpoint(key: SecretKey, transport: &TransportConfig) -> Result<Endpoint> {
	endpoint_with_alpns(
		key,
		transport,
		vec![ALPN.to_vec(), file_transfer::FILE_ALPN.to_vec()],
	)
	.await
}

pub(crate) async fn endpoint_with_alpns(
	key: SecretKey,
	transport: &TransportConfig,
	alpns: Vec<Vec<u8>>,
) -> Result<Endpoint> {
	let mut builder = Endpoint::builder(presets::Minimal)
		.secret_key(key)
		.alpns(alpns);
	if transport.offline {
		builder = builder.clear_ip_transports().bind_addr("127.0.0.1:0")?;
	} else {
		let mode = if let Some(url) = &transport.relay_url {
			ensure!(
				url::Url::parse(url)?.scheme() == "https",
				"Reläet måste använda HTTPS."
			);
			RelayMode::Custom(RelayMap::try_from_iter([url.as_str()])?)
		} else {
			RelayMode::Default
		};
		builder = builder.relay_mode(mode);
		if transport.relay_only {
			builder = builder.clear_ip_transports();
		}
	}
	Ok(builder.bind().await?)
}

pub async fn write_frame<T: Serialize>(send: &mut SendStream, value: &T) -> Result<()> {
	let bytes = serde_json::to_vec(value)?;
	ensure!(bytes.len() <= MAX_FRAME, "Nätverkspaketet är för stort.");
	timeout(Duration::from_secs(10), async {
		send.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
		send.write_all(&bytes).await?;
		Ok::<_, anyhow::Error>(())
	})
	.await
	.context("Mottagaren svarar för långsamt.")??;
	Ok(())
}
pub async fn read_frame<T: serde::de::DeserializeOwned>(recv: &mut RecvStream) -> Result<T> {
	read_frame_limited(recv, MAX_FRAME).await
}
pub(crate) async fn read_frame_limited<T: serde::de::DeserializeOwned>(
	recv: &mut RecvStream,
	limit: usize,
) -> Result<T> {
	let mut length = [0u8; 4];
	recv.read_exact(&mut length).await?;
	let length = u32::from_be_bytes(length) as usize;
	ensure!(length > 0 && length <= limit, "Ogiltig paketstorlek.");
	let mut bytes = vec![0; length];
	timeout(Duration::from_secs(10), recv.read_exact(&mut bytes))
		.await
		.context("Ofullständigt nätverkspaket.")??;
	Ok(serde_json::from_slice(&bytes)?)
}

struct SharedRoom {
	assets: Arc<ProfileAssets>,
	file_routes: Mutex<HashMap<String, mpsc::Sender<file_transfer::Envelope>>>,
	media: Arc<media::Hub>,
	room: Mutex<Room>,
	updates: watch::Sender<Snapshot>,
	history: PathBuf,
	alias: String,
}
impl SharedRoom {
	fn apply(&self, peer: &str, command: Command) -> Result<()> {
		if let Command::FileSignal(directed) = command {
			return self.route_file(peer, directed);
		}
		let durable = matches!(
			&command,
			Command::SendMessage { .. }
				| Command::CreateChannel { .. }
				| Command::CreateVoiceChannel { .. }
				| Command::UpdateProfile(_)
		);
		let mut room = self
			.room
			.lock()
			.map_err(|_| anyhow::anyhow!("Servern kunde inte uppdateras."))?;
		let mut next = room.clone();
		next.apply(peer, command, now())?;
		if durable {
			next.save(&self.history)?;
		}
		self.updates.send_replace(next.snapshot.clone());
		*room = next;
		Ok(())
	}
	fn route_file(&self, peer: &str, directed: file_transfer::Directed) -> Result<()> {
		directed.signal.validate(peer)?;
		let room = self
			.room
			.lock()
			.map_err(|_| anyhow::anyhow!("Servern är upptagen."))?;
		let sender = room
			.snapshot
			.members
			.iter()
			.find(|m| m.id == peer)
			.context("Avsändaren är inte ansluten.")?;
		let recipient = room
			.snapshot
			.members
			.iter()
			.find(|m| m.id == directed.recipient)
			.context("Mottagaren är inte ansluten.")?;
		ensure!(peer != directed.recipient, "Välj en annan mottagare.");
		if matches!(directed.signal, file_transfer::Signal::Offer(_)) {
			ensure!(
				sender
					.voice
					.as_ref()
					.zip(recipient.voice.as_ref())
					.is_some_and(|(a, b)| a.channel == b.channel),
				"Mottagaren måste vara i ditt samtal."
			);
		}
		let event = file_transfer::Envelope {
			sender: peer.into(),
			sender_name: sender.profile.name.clone(),
			signal: directed.signal,
		};
		self.file_routes
			.lock()
			.map_err(|_| anyhow::anyhow!("Servern är upptagen."))?
			.get(&directed.recipient)
			.context("Mottagaren är inte tillgänglig.")?
			.try_send(event)
			.map_err(|_| anyhow::anyhow!("Mottagaren är upptagen."))?;
		Ok(())
	}
	fn leave(&self, peer: &str) {
		if let Ok(mut routes) = self.file_routes.lock() {
			routes.remove(peer);
		}
		if let Ok(mut room) = self.room.lock() {
			room.leave(peer);
			self.updates.send_replace(room.snapshot.clone());
		}
	}
}

pub async fn host(
	options: HostOptions,
	key: SecretKey,
	transport: TransportConfig,
) -> Result<Session> {
	host_with_assets(options, key, transport, AssetCache::temporary()?).await
}

pub async fn host_with_assets(
	options: HostOptions,
	key: SecretKey,
	transport: TransportConfig,
	cache: Arc<AssetCache>,
) -> Result<Session> {
	validate_text(&options.name, 96)?;
	validate_text(&options.profile.name, 96)?;
	validate_slug(&options.alias.owner)?;
	validate_slug(&options.alias.slug)?;
	let self_id = key.public().to_string();
	// Each server start creates a new capability: old invitations cannot silently regain access.
	let token = SecretKey::generate()
		.to_bytes()
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect::<String>();
	let mut room = if options.history.exists() {
		Room::load(&options.history, self_id.clone(), &token, options.public)?
	} else {
		Room::new(
			options.name.clone(),
			self_id.clone(),
			&token,
			options.public,
		)
	};
	room.snapshot.name = options.name.clone();
	room.join(&self_id, options.profile.clone(), &token)?;
	room.save(&options.history)?;
	let endpoint = endpoint(key.clone(), &transport).await?;
	let (notice_tx, notices) = mpsc::channel(16);
	if !transport.offline
		&& timeout(Duration::from_secs(12), endpoint.online())
			.await
			.is_err()
	{
		let _ = notice_tx.try_send(
			"Reläet kunde inte nås. Servern är startad, men internetanslutning är inte bekräftad."
				.into(),
		);
	}
	let invite = Invite {
		version: 1,
		address: endpoint.addr(),
		token,
		alias: options.alias.to_string(),
		name: options.name.clone(),
	};
	let (updates, snapshots) = watch::channel(room.snapshot.clone());
	let assets = ProfileAssets::new(cache, snapshots.clone(), None);
	let media_hub = Arc::new(media::Hub::new(snapshots.clone()));
	let (file_tx, mut file_rx) = mpsc::channel(16);
	let shared = Arc::new(SharedRoom {
		assets: assets.clone(),
		file_routes: Mutex::new(HashMap::from([(self_id.clone(), file_tx)])),
		media: media_hub.clone(),
		room: Mutex::new(room),
		updates,
		history: options.history,
		alias: options.alias.to_string(),
	});
	let (commands, mut command_rx) = mpsc::channel(32);
	let files = FileTransfers::new(endpoint.clone(), commands.clone(), snapshots.clone());
	let task_files = files.clone();
	let (audio_tx, mut audio_rx) = mpsc::channel::<MediaFrame>(8);
	let (screen_audio_tx, mut screen_audio_rx) = mpsc::channel::<MediaFrame>(8);
	let (video_tx, mut video_rx) = mpsc::channel::<MediaFrame>(2);
	let (media_tx, media) = media::channel();
	let task_endpoint = endpoint.clone();
	let host_id = self_id.clone();
	let task = tokio::spawn(async move {
		let mut tasks = JoinSet::new();
		tasks.spawn(task_files.clone().maintain());
		let local_hub = media_hub.clone();
		let local_peer = host_id.clone();
		tasks.spawn(async move {
			media::local(local_hub, local_peer, media_tx).await;
		});
		if let Some(base) = options.directory_url {
			let directory_endpoint = task_endpoint.clone();
			let notices = notice_tx.clone();
			tasks.spawn(async move {
                let mut heartbeat = tokio::time::interval(Duration::from_secs(60));
                loop {
                    heartbeat.tick().await;
                    let record = Registration { owner: options.alias.owner.clone(), slug: options.alias.slug.clone(), name: options.name.clone(), address: directory_endpoint.addr(), issued_at: now() };
                    let result = match SignedRegistration::sign(record, &key) { Ok(record) => directory::publish(&base, &record).await, Err(error) => Err(error) };
                    let text = match result {
                        Ok(()) => "Servernamnet är registrerat i adressboken.".into(),
                        Err(error) => format!("Namnanslutning är inte tillgänglig: {error}. Inbjudningslänken fungerar oberoende av adressboken."),
                    };
                    let _ = notices.try_send(text);
                }
            });
		}
		loop {
			tokio::select! {
				Some(event)=file_rx.recv()=>{let _=task_files.receive(event);}
				Some(frame)=audio_rx.recv()=>{let _=media_hub.publish(&host_id,frame);}
				Some(frame)=screen_audio_rx.recv()=>{let _=media_hub.publish(&host_id,frame);}
				Some(frame)=video_rx.recv()=>{let _=media_hub.publish(&host_id,frame);}
				incoming = task_endpoint.accept() => {
					let Some(incoming) = incoming else { break; };
					if tasks.len() >= 65 { incoming.refuse(); continue; }
					let room = shared.clone();
					let files=task_files.clone();
					tasks.spawn(async move {
						if let Ok(Ok(connection)) = timeout(Duration::from_secs(10), incoming).await {
							if connection.alpn()==file_transfer::FILE_ALPN {let _=files.serve(connection).await;}else{let _ = serve_guest(connection, room).await;}
						}
					});
				}
				command = command_rx.recv() => {
					let Some(command) = command else { break; };
					let room = shared.clone(); let peer = host_id.clone();
					let result = tokio::task::spawn_blocking(move || room.apply(&peer, command)).await;
					match result {
						Ok(Ok(())) => (),
						Ok(Err(error)) => { let _ = notice_tx.try_send(error.to_string()); },
						Err(_) => { let _ = notice_tx.try_send("Servern kunde inte spara ändringen.".into()); },
					}
				}
				_ = tasks.join_next(), if !tasks.is_empty() => {}
			}
		}
	});
	Ok(Session {
		dedicated: None,
		assets,
		files,
		media,
		audio_tx,
		screen_audio_tx,
		video_tx,
		snapshots,
		notices,
		invite: Some(invite),
		self_id,
		commands,
		endpoint,
		task,
		runtime: tokio::runtime::Handle::current(),
	})
}

async fn serve_guest(connection: Connection, shared: Arc<SharedRoom>) -> Result<()> {
	let peer = connection.remote_id().to_string();
	let (mut send, mut recv) = timeout(Duration::from_secs(10), connection.accept_bi()).await??;
	let hello: Hello = timeout(Duration::from_secs(10), read_frame(&mut recv)).await??;
	ensure!(
		hello.version == 1 && hello.token.len() <= 64,
		"Ogiltig anslutningsbegäran."
	);
	let admitted = {
		let mut room = shared
			.room
			.lock()
			.map_err(|_| anyhow::anyhow!("Servern är upptagen."))?;
		let result = if hello.alias != shared.alias {
			Err(anyhow::anyhow!(
				"Inbjudan gäller ett annat rum. Be om en ny inbjudan."
			))
		} else {
			room.join(&peer, hello.profile, &hello.token)
		};
		if result.is_ok() {
			shared.updates.send_replace(room.snapshot.clone());
		}
		result
	};
	if let Err(error) = admitted {
		write_frame(&mut send, &ServerFrame::Error(error.to_string())).await?;
		send.finish()?;
		let _ = timeout(Duration::from_secs(2), send.stopped()).await;
		return Ok(());
	}
	// Always remove the authenticated peer, including initial write failure and cancellation.
	struct Membership {
		shared: Arc<SharedRoom>,
		peer: String,
	}
	impl Drop for Membership {
		fn drop(&mut self) {
			self.shared.assets.detach(&self.peer);
			self.shared.leave(&self.peer);
		}
	}
	let _membership = Membership {
		shared: shared.clone(),
		peer: peer.clone(),
	};
	let (file_tx, mut file_rx) = mpsc::channel(16);
	shared
		.file_routes
		.lock()
		.map_err(|_| anyhow::anyhow!("Servern är upptagen."))?
		.insert(peer.clone(), file_tx);
	shared.assets.attach(connection.clone());
	let mut updates = shared.updates.subscribe();
	let first = updates.borrow_and_update().clone();
	write_frame(&mut send, &ServerFrame::Snapshot(first)).await?;
	let (errors, mut error_rx) = mpsc::channel(8);
	let reader = async {
		let mut window = tokio::time::Instant::now();
		let mut count = 0;
		loop {
			let command: Command = read_frame(&mut recv).await?;
			if window.elapsed() >= Duration::from_secs(1) {
				window = tokio::time::Instant::now();
				count = 0;
			}
			count += 1;
			ensure!(count <= 20, "För många meddelanden.");
			let room = shared.clone();
			let peer = peer.clone();
			if let Err(error) =
				tokio::task::spawn_blocking(move || room.apply(&peer, command)).await?
			{
				errors
					.try_send(error.to_string())
					.map_err(|_| anyhow::anyhow!("För många ogiltiga kommandon."))?;
			}
		}
		#[allow(unreachable_code)]
		Ok::<(), anyhow::Error>(())
	};
	let writer = async {
		loop {
			let frame = tokio::select! {
				biased;
				changed = updates.changed() => { changed?; ServerFrame::Snapshot(updates.borrow_and_update().clone()) }
				Some(event)=file_rx.recv()=>ServerFrame::FileSignal(event),
				error = error_rx.recv() => { let Some(error) = error else { break; }; ServerFrame::Error(error) }
			};
			write_frame(&mut send, &frame).await?;
		}
		Ok::<(), anyhow::Error>(())
	};
	let result = tokio::select! { result = reader => result, result = writer => result, result=media::serve(connection.clone(),shared.clone().media.clone(),peer.clone())=>result, result=shared.assets.clone().serve_connection(connection.clone())=>result };
	connection.close(0u32.into(), b"session ended");
	result
}

pub async fn join(
	invite: Invite,
	key: SecretKey,
	profile: Profile,
	transport: TransportConfig,
) -> Result<Session> {
	join_with_assets(invite, key, profile, transport, AssetCache::temporary()?).await
}

pub async fn join_with_assets(
	invite: Invite,
	key: SecretKey,
	profile: Profile,
	transport: TransportConfig,
	cache: Arc<AssetCache>,
) -> Result<Session> {
	// Apply the same validation to locally constructed alias targets and pasted invitations.
	invite.encode()?;
	validate_text(&profile.name, 96)?;
	let endpoint = endpoint(key, &transport).await?;
	let dedicated = (invite.version == 2).then(|| crate::server_control::Client {
		endpoint: endpoint.clone(),
		invite: invite.clone(),
	});
	// A joining guest also needs its own advertised relay address for direct
	// peer file connections, even when the room connection can already dial out.
	let relay_ready = transport.offline
		|| timeout(Duration::from_secs(12), endpoint.online())
			.await
			.is_ok();
	let connection = match timeout(
		Duration::from_secs(20),
		endpoint.connect(
			invite.address.clone(),
			if invite.version == 2 {
				crate::server::CHAT_ALPN
			} else {
				ALPN
			},
		),
	)
	.await
	{
		Ok(Ok(connection)) => connection,
		_ => {
			endpoint.close().await;
			anyhow::bail!(
				"Servern kunde inte nås. Kontrollera att värdens dator och server är igång."
			);
		}
	};
	let attempt = async {
		let (mut send, mut recv) = connection.open_bi().await?;
		write_frame(
			&mut send,
			&Hello {
				version: 1,
				alias: Alias::parse(&invite.alias)?.to_string(),
				profile,
				token: invite.token,
			},
		)
		.await?;
		let frame: ServerFrame = timeout(Duration::from_secs(10), read_frame(&mut recv)).await??;
		match frame {
			ServerFrame::Snapshot(snapshot) => Ok((send, recv, snapshot)),
			ServerFrame::Error(error) => Err(anyhow::anyhow!(error)),
			ServerFrame::FileSignal(_) => Err(anyhow::anyhow!("Ogiltig start på anslutningen.")),
		}
	}
	.await;
	let (mut send, mut recv, snapshot) = match attempt {
		Ok(value) => value,
		Err(error) => {
			endpoint.close().await;
			return Err(error);
		}
	};
	let (updates, snapshots) = watch::channel(snapshot);
	let assets = ProfileAssets::new(
		cache,
		snapshots.clone(),
		Some(connection.remote_id().to_string()),
	);
	assets.attach(connection.clone());
	let task_assets = assets.clone();
	let (notice_tx, notices) = mpsc::channel(16);
	if !relay_ready {
		let _ = notice_tx.try_send("Ansluten till rummet, men din reläadress är inte klar. Filöverföring över internet kan behöva ett nytt försök.".into());
	}
	let (commands, mut command_rx) = mpsc::channel(32);
	let files = FileTransfers::new(endpoint.clone(), commands.clone(), snapshots.clone());
	let task_files = files.clone();
	let (audio_tx, audio_rx) = mpsc::channel(8);
	let (screen_audio_tx, screen_audio_rx) = mpsc::channel(8);
	let (video_tx, video_rx) = mpsc::channel(2);
	let (media_tx, media) = media::channel();
	let file_endpoint = endpoint.clone();
	let task = tokio::spawn(async move {
		let mut file_tasks = JoinSet::new();
		file_tasks.spawn(task_files.clone().maintain());
		let accept_files = async {
			loop {
				tokio::select! {incoming=file_endpoint.accept()=>{let Some(incoming)=incoming else {break};if file_tasks.len()>=10{incoming.refuse();continue}let files=task_files.clone();file_tasks.spawn(async move {if let Ok(Ok(connection))=timeout(Duration::from_secs(10),incoming).await && connection.alpn()==file_transfer::FILE_ALPN {let _=files.serve(connection).await;}});},_=file_tasks.join_next(),if !file_tasks.is_empty()=>{}}
			}
			Ok::<(), anyhow::Error>(())
		};
		let reader = async {
			loop {
				match read_frame::<ServerFrame>(&mut recv).await? {
					ServerFrame::FileSignal(event) => {
						let _ = task_files.receive(event);
					}
					ServerFrame::Snapshot(snapshot) => {
						updates.send_replace(snapshot);
					}
					ServerFrame::Error(error) => {
						let _ = notice_tx.try_send(error);
					}
				}
			}
			#[allow(unreachable_code)]
			Ok::<(), anyhow::Error>(())
		};
		let writer = async {
			while let Some(command) = command_rx.recv().await {
				write_frame(&mut send, &command).await?;
			}
			Ok::<(), anyhow::Error>(())
		};
		let result = tokio::select! { result = reader => result, result = writer => result,result=media::client(connection.clone(),audio_rx,screen_audio_rx,video_rx,media_tx)=>result,result=accept_files=>result, result=task_assets.serve_connection(connection.clone())=>result };
		task_files.close();
		let text = match result {
			Ok(()) => "Du har lämnat servern.".into(),
			Err(_) => {
				"Anslutningen till servern stängdes. Anslut igen när värden är online.".into()
			}
		};
		let _ = notice_tx.try_send(text);
		connection.close(0u32.into(), b"session ended");
	});
	Ok(Session {
		dedicated,
		assets,
		files,
		media,
		audio_tx,
		screen_audio_tx,
		video_tx,
		snapshots,
		notices,
		invite: None,
		self_id: endpoint.id().to_string(),
		commands,
		endpoint,
		task,
		runtime: tokio::runtime::Handle::current(),
	})
}

#[cfg(test)]
mod media_queue_tests {
	use super::*;
	#[tokio::test]
	async fn session_screen_audio_overload_leaves_outbound_voice_capacity() {
		let root = tempfile::tempdir().unwrap();
		let session = host(
			HostOptions {
				alias: Alias::parse("@test/audio").unwrap(),
				name: "Synthetic".into(),
				profile: Profile {
					name: "peer".into(),
					accent: [1, 2, 3],
					card: None,
				},
				public: false,
				history: root.path().join("room.json"),
				directory_url: None,
			},
			SecretKey::generate(),
			TransportConfig {
				offline: true,
				..Default::default()
			},
		)
		.await
		.unwrap();
		// No yield: the real session writer cannot drain until this producer's
		// burst has finished, exactly the overload boundary being exercised.
		let mut rejected = 0;
		for sequence in 0..256 {
			if session
				.send_media(MediaFrame {
					kind: media::Kind::ScreenAudio,
					epoch: 1,
					sequence,
					data: vec![1],
				})
				.is_err()
			{
				rejected += 1;
			}
		}
		assert!(rejected > 0);
		assert!(
			session
				.send_media(MediaFrame {
					kind: media::Kind::Audio,
					epoch: 1,
					sequence: 1,
					data: vec![1]
				})
				.is_ok()
		);
		session.close().await;
	}
}
