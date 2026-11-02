//! Independent server lifecycle and domain orchestration. No desktop or capture dependencies.
use crate::{
	file_transfer, media,
	network::{self, TransportConfig},
	profile_assets::{AssetCache, ProfileAssets},
	protocol::*,
	room::Room,
	server_control::{Request, Response, Status},
	server_store::{AdminAction, AdminResult, Authority, Role, ServerStore},
	storage,
};
use anyhow::{Context, Result, ensure};
use iroh::{Endpoint, SecretKey};
use serde::{Deserialize, Serialize};
use std::{
	collections::HashMap,
	fs::{File, OpenOptions},
	io::Write,
	path::{Path, PathBuf},
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	time::{Duration, Instant},
};
use tokio::{
	sync::{mpsc, watch},
	task::JoinSet,
	time::timeout,
};

pub const CHAT_ALPN: &[u8] = b"connect/server/1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerConfig {
	pub alias: String,
	pub name: String,
	pub public: bool,
	pub directory_url: Option<String>,
	pub panel_bind: String,
	pub panel_origin: String,
}
impl ServerConfig {
	pub fn local(alias: &str, name: &str) -> Result<Self> {
		Alias::parse(alias)?;
		validate_text(name, 96)?;
		Ok(Self {
			alias: alias.into(),
			name: name.into(),
			public: false,
			directory_url: None,
			panel_bind: "127.0.0.1:4748".into(),
			panel_origin: "http://127.0.0.1:4748".into(),
		})
	}
	pub fn load(root: &Path) -> Result<Self> {
		let config: Self = storage::read_json(&root.join("server.json"), 16384)?;
		Alias::parse(&config.alias)?;
		validate_text(&config.name, 96)?;
		Ok(config)
	}
}
pub struct SetupInfo {
	pub server_id: String,
	pub pairing_token: String,
}
pub fn initialize(root: &Path, config: ServerConfig) -> Result<SetupInfo> {
	ensure!(
		!root.exists() || std::fs::read_dir(root)?.next().is_none(),
		"Välj en tom datakatalog för den nya servern."
	);
	ensure!(
		!root.join("server.json").exists()
			&& !root.join("identity.key").exists()
			&& !root.join("server.db").exists(),
		"Datakatalogen innehåller redan en server."
	);
	std::fs::create_dir_all(root)?;
	protect_directory(root)?;
	let key = SecretKey::generate();
	write_secret(&root.join("identity.key"), &key.to_bytes())?;
	let store = ServerStore::open(&root.join("server.db"), &config.name)?;
	let token = store.reset_pairing(3600, now())?;
	storage::write_json(&root.join("server.json"), &config, 16384)?;
	Ok(SetupInfo {
		server_id: key.public().to_string(),
		pairing_token: token,
	})
}
pub(crate) fn protect_directory(root: &Path) -> Result<()> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
	}
	#[cfg(windows)]
	{
		use std::os::windows::process::CommandExt;
		// Grant only the current principal, SYSTEM and Administrators; set before writing secrets.
		let output = std::process::Command::new("whoami.exe")
			.args(["/user", "/fo", "csv", "/nh"])
			.creation_flags(0x08000000)
			.output()?;
		ensure!(
			output.status.success(),
			"Windows kunde inte identifiera serverkontot."
		);
		let text = String::from_utf8(output.stdout)?;
		let sid = text
			.trim()
			.split(',')
			.next_back()
			.context("Serverkontots SID saknas.")?
			.trim_matches('"');
		ensure!(
			sid.starts_with("S-1-")
				&& sid
					.bytes()
					.all(|b| b.is_ascii_digit() || b == b'S' || b == b'-'),
			"Ogiltigt serverkonto."
		);
		let status = std::process::Command::new("icacls.exe")
			.arg(root)
			.args([
				"/inheritance:r",
				"/grant:r",
				&format!("*{sid}:(OI)(CI)F"),
				"*S-1-5-18:(OI)(CI)F",
				"*S-1-5-32-544:(OI)(CI)F",
			])
			.creation_flags(0x08000000)
			.output()?;
		ensure!(
			status.status.success(),
			"Windows kunde inte skydda serverns datakatalog."
		);
	}
	Ok(())
}
pub(crate) fn write_secret(path: &Path, bytes: &[u8]) -> Result<()> {
	let mut options = OpenOptions::new();
	options.write(true).create_new(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}
	let mut file = options.open(path)?;
	file.write_all(bytes)?;
	file.sync_all()?;
	Ok(())
}
pub(crate) fn load_key(root: &Path) -> Result<SecretKey> {
	let bytes = std::fs::read(root.join("identity.key"))?;
	let bytes: [u8; 32] = bytes
		.try_into()
		.map_err(|_| anyhow::anyhow!("Servernyckeln är skadad; återställ en säkerhetskopia."))?;
	Ok(SecretKey::from_bytes(&bytes))
}
pub fn lock_directory(root: &Path) -> Result<File> {
	let file = OpenOptions::new()
		.read(true)
		.write(true)
		.create(true)
		.truncate(false)
		.open(root.join("instance.lock"))?;
	file.try_lock()
		.context("Servern kör redan från denna datakatalog.")?;
	Ok(file)
}

pub struct Core {
	pub store: Arc<ServerStore>,
	pub config: ServerConfig,
	pub root: PathBuf,
	pub server_id: String,
	pub(crate) room: Mutex<Room>,
	pub(crate) updates: watch::Sender<Snapshot>,
	pub(crate) media: Arc<media::Hub>,
	pub(crate) assets: Arc<ProfileAssets>,
	pub(crate) routes: Mutex<HashMap<String, mpsc::Sender<file_transfer::Envelope>>>,
	operation: Mutex<()>,
	generations: Mutex<HashMap<String, u64>>,
	connections: Mutex<HashMap<String, iroh::endpoint::Connection>>,
	directory_status: Mutex<String>,
	sequence: AtomicU64,
	closing: AtomicBool,
	rates: Mutex<HashMap<String, (Instant, u32)>>,
	started: Instant,
	pub(crate) invitation: Mutex<Invite>,
	pub(crate) browser: Mutex<crate::server_admin::BrowserSessions>,
}
impl Core {
	pub(crate) fn first_owner_invite(&self) -> Result<String> {
		let _guard = self.operation.lock().unwrap();
		ensure!(!self.closing.load(Ordering::Acquire), "Servern stängs.");
		ensure!(
			self.store
				.list_members_as(Authority::LocalOperator)?
				.iter()
				.all(|m| m.role == Role::Member || !m.active),
			"Servern har redan en ägare eller administratör. Befintlig åtkomst har inte ändrats."
		);
		let token = self.store.reset_pairing(3600, now())?;
		let mut invite = self.invitation.lock().unwrap().clone();
		invite.token = token;
		invite.name = self.snapshot().name;
		invite.encode()
	}
	pub fn maintain(&self, time: u64) -> Result<()> {
		let _guard = self.operation.lock().unwrap();
		ensure!(!self.closing.load(Ordering::Acquire), "Servern stängs.");
		self.store.maintain(time)?;
		self.refresh()
	}
	pub fn snapshot(&self) -> Snapshot {
		self.updates.borrow().clone()
	}
	pub fn execute(&self, peer: &str, request: Request) -> Result<Response> {
		self.check_rate(peer)?;
		self.execute_as(Authority::Peer(peer), request)
	}
	pub(crate) fn execute_local(&self, request: Request) -> Result<Response> {
		ensure!(
			matches!(request, Request::Status | Request::Admin(_)),
			"Denna åtgärd kräver en Connect-profil."
		);
		self.execute_as(Authority::LocalOperator, request)
	}
	fn execute_as(&self, authority: Authority<'_>, request: Request) -> Result<Response> {
		let peer = match authority {
			Authority::Peer(peer) => peer,
			Authority::LocalOperator => "local-operator",
		};
		let _guard = self
			.operation
			.lock()
			.map_err(|_| anyhow::anyhow!("Servern är upptagen."))?;
		ensure!(!self.closing.load(Ordering::Acquire), "Servern stängs.");
		let result = match request {
			Request::Status => {
				let storage = self.store.settings_as(authority)?;
				let s = self.snapshot();
				let (sent, received) = self.connections.lock().unwrap().values().fold(
					(0u64, 0u64),
					|(sent, received), connection| {
						let stats = connection.stats();
						(
							sent.saturating_add(stats.udp_tx.bytes),
							received.saturating_add(stats.udp_rx.bytes),
						)
					},
				);
				Response::Status(Status {
					version: env!("CARGO_PKG_VERSION").into(),
					directory_status: self.directory_status.lock().unwrap().clone(),
					active_sent_bytes: sent,
					active_received_bytes: received,
					server_id: self.server_id.clone(),
					name: s.name,
					uptime_seconds: self.started.elapsed().as_secs(),
					online: s.members.len(),
					storage,
					channels: s.channels,
					members: self.store.list_members_as(authority)?,
					invitations: self.store.list_invites_as(authority)?,
					audit: self.store.audit_as(authority, None, 100)?,
				})
			}
			Request::History {
				channel,
				before,
				limit,
			} => Response::History(self.store.history(peer, &channel, before, limit)?),
			Request::SendMessage {
				channel,
				text,
				request_id,
			} => {
				let message = self
					.store
					.send_message(peer, &channel, &text, &request_id, now())?;
				self.refresh()?;
				Response::Message(message)
			}
			Request::Admin(action) => {
				let result = self.store.admin_as(authority, action, now())?;
				self.refresh()?;
				if let AdminResult::Invite { token } = result {
					let mut invite = self.invitation.lock().unwrap().clone();
					invite.token = token;
					invite.name = self.snapshot().name;
					Response::Invite(invite.encode()?)
				} else {
					Response::Admin(result)
				}
			}
			Request::ApproveBrowser { code } => {
				ensure!(
					self.store.role(peer)? != Role::Member,
					"Du saknar administratörsbehörighet."
				);
				self.browser.lock().unwrap().approve(&code, peer)?;
				Response::Ok
			}
			Request::RegisterName(publication) => {
				ensure!(
					self.store.role(peer)? == Role::Owner,
					"Endast ägaren kan registrera servernamnet."
				);
				publication.delegation.verify()?;
				ensure!(
					publication.delegation.server.to_string() == self.server_id
						&& format!(
							"@{}/{}",
							publication.delegation.owner, publication.delegation.slug
						) == self.config.alias,
					"Fullmakten gäller en annan server."
				);
				crate::directory::validate_url(&publication.directory)?;
				storage::write_json(&self.root.join("directory-v2.json"), &publication, 16384)?;
				*self.directory_status.lock().unwrap() = "Väntar på namnregistrering.".into();
				Response::Ok
			}
		};
		Ok(result)
	}
	fn check_rate(&self, peer: &str) -> Result<()> {
		let mut rates = self.rates.lock().unwrap();
		rates.retain(|_, (time, _)| time.elapsed() < Duration::from_secs(60));
		ensure!(
			rates.len() < 4096 || rates.contains_key(peer),
			"Servern är upptagen."
		);
		let entry = rates.entry(peer.into()).or_insert((Instant::now(), 0));
		entry.1 += 1;
		ensure!(entry.1 <= 180, "För många serveråtgärder. Vänta en stund.");
		Ok(())
	}
	pub(crate) fn refresh(&self) -> Result<()> {
		let mut next = self.store.snapshot()?;
		let mut room = self.room.lock().unwrap();
		for mut member in room.snapshot.members.clone() {
			if let Ok(role) = self.store.role(&member.id) {
				member.host = role != Role::Member;
				if member.voice.as_ref().is_some_and(|v| {
					!next
						.channels
						.iter()
						.any(|c| c.id == v.channel && c.kind == ChannelKind::Voice)
				}) {
					member.voice = None;
				}
				next.members.push(member);
			}
		}
		ensure!(
			serde_json::to_vec(&next)?.len() <= MAX_SNAPSHOT,
			"Serverns tillstånd är för stort."
		);
		room.snapshot = next.clone();
		self.updates.send_replace(next);
		Ok(())
	}
	pub(crate) fn admit(&self, peer: &str, hello: Hello) -> Result<u64> {
		self.check_rate(peer)?;
		let _guard = self.operation.lock().unwrap();
		ensure!(!self.closing.load(Ordering::Acquire), "Servern stängs.");
		ensure!(hello.version == 1, "Uppdatera Connect för att ansluta.");
		{
			let room = self.room.lock().unwrap();
			ensure!(
				room.snapshot.members.len() < 64,
				"Servern är full (64 online)."
			);
			ensure!(
				!room.snapshot.members.iter().any(|m| m.id == peer),
				"Du är redan ansluten."
			);
		}
		let profile = hello.profile;
		if self.config.public && hello.token.is_empty() {
			self.store.admit_public(peer, profile.clone(), now())?;
		} else {
			self.store.admit(
				peer,
				profile.clone(),
				(!hello.token.is_empty()).then_some(hello.token.as_str()),
				now(),
			)?;
		}
		self.room.lock().unwrap().join(peer, profile, "")?;
		self.refresh()?;
		let generation = self.sequence.fetch_add(1, Ordering::Relaxed);
		self.generations
			.lock()
			.unwrap()
			.insert(peer.into(), generation);
		Ok(generation)
	}
	pub(crate) fn apply(&self, peer: &str, generation: u64, command: Command) -> Result<()> {
		let _guard = self.operation.lock().unwrap();
		ensure!(!self.closing.load(Ordering::Acquire), "Servern stängs.");
		ensure!(self.current(peer, generation), "Anslutningen har ersatts.");
		let role = self.store.role(peer)?;
		match command {
			Command::SendMessage { .. } => {
				anyhow::bail!("Uppdatera Connect för säker meddelandesändning.")
			}
			Command::CreateChannel { name } => {
				self.store.admin(
					peer,
					AdminAction::CreateChannel {
						name,
						kind: ChannelKind::Text,
					},
					now(),
				)?;
				self.refresh()?;
			}
			Command::CreateVoiceChannel { name } => {
				self.store.admin(
					peer,
					AdminAction::CreateChannel {
						name,
						kind: ChannelKind::Voice,
					},
					now(),
				)?;
				self.refresh()?;
			}
			Command::FileSignal(signal) => {
				self.route_file(peer, signal)?;
			}
			Command::UpdateProfile(profile) => {
				self.store.admit(peer, profile.clone(), None, now())?;
				let mut room = self.room.lock().unwrap();
				room.apply_authorized(
					peer,
					Command::UpdateProfile(profile),
					now(),
					role != Role::Member,
				)?;
				self.updates.send_replace(room.snapshot.clone());
			}
			other => {
				let mut room = self.room.lock().unwrap();
				room.apply_authorized(peer, other, now(), role != Role::Member)?;
				self.updates.send_replace(room.snapshot.clone());
			}
		};
		Ok(())
	}
	fn route_file(&self, peer: &str, directed: file_transfer::Directed) -> Result<()> {
		directed.signal.validate(peer)?;
		let room = self.room.lock().unwrap();
		let sender = room
			.snapshot
			.members
			.iter()
			.find(|m| m.id == peer)
			.context("Avsändaren är frånkopplad.")?;
		let receiver = room
			.snapshot
			.members
			.iter()
			.find(|m| m.id == directed.recipient)
			.context("Mottagaren är frånkopplad.")?;
		ensure!(peer != directed.recipient, "Välj en annan mottagare.");
		if matches!(directed.signal, file_transfer::Signal::Offer(_)) {
			ensure!(
				sender
					.voice
					.as_ref()
					.zip(receiver.voice.as_ref())
					.is_some_and(|(a, b)| a.channel == b.channel),
				"Mottagaren måste vara i samma samtal."
			);
		}
		self.routes
			.lock()
			.unwrap()
			.get(&directed.recipient)
			.context("Mottagaren är inte tillgänglig.")?
			.try_send(file_transfer::Envelope {
				sender: peer.into(),
				sender_name: sender.profile.name.clone(),
				signal: directed.signal,
			})
			.map_err(|_| anyhow::anyhow!("Mottagaren är upptagen."))?;
		Ok(())
	}
	pub(crate) fn current(&self, peer: &str, generation: u64) -> bool {
		self.generations.lock().unwrap().get(peer) == Some(&generation)
	}
	pub(crate) fn attach(
		&self,
		peer: &str,
		generation: u64,
		connection: iroh::endpoint::Connection,
	) -> Result<mpsc::Receiver<file_transfer::Envelope>> {
		let _guard = self.operation.lock().unwrap();
		ensure!(
			self.current(peer, generation) && !self.closing.load(Ordering::Acquire),
			"Anslutningen har ersatts."
		);
		self.store.role(peer)?;
		self.connections
			.lock()
			.unwrap()
			.insert(peer.into(), connection.clone());
		self.assets.attach(connection);
		let (tx, rx) = mpsc::channel(16);
		self.routes.lock().unwrap().insert(peer.into(), tx);
		Ok(rx)
	}
	pub(crate) fn leave(&self, peer: &str, generation: u64) {
		let _guard = self.operation.lock().unwrap();
		if !self.current(peer, generation) {
			return;
		}
		self.generations.lock().unwrap().remove(peer);
		self.connections.lock().unwrap().remove(peer);
		self.routes.lock().unwrap().remove(peer);
		self.assets.detach(peer);
		let mut room = self.room.lock().unwrap();
		room.leave(peer);
		self.updates.send_replace(room.snapshot.clone());
	}
}
pub struct RunningServer {
	pub core: Arc<Core>,
	pub invite: Invite,
	endpoint: Endpoint,
	task: tokio::task::JoinHandle<()>,
	lock: Option<File>,
}
impl RunningServer {
	pub fn snapshot(&self) -> Snapshot {
		self.core.snapshot()
	}
	pub async fn close(mut self) {
		self.core.closing.store(true, Ordering::Release);
		self.task.abort();
		let _ = (&mut self.task).await;
		self.endpoint.close().await;
		let core = self.core.clone();
		let _ = tokio::task::spawn_blocking(move || {
			let _guard = core.operation.lock().unwrap();
		})
		.await;
		self.lock.take();
	}
}
impl Drop for RunningServer {
	fn drop(&mut self) {
		self.task.abort();
	}
}
pub async fn start(root: &Path, transport: TransportConfig) -> Result<RunningServer> {
	let lock = lock_directory(root)?;
	let config = ServerConfig::load(root)?;
	let key = load_key(root)?;
	let store = Arc::new(ServerStore::open(&root.join("server.db"), &config.name)?);
	let snapshot = store.snapshot()?;
	let endpoint = network::endpoint_with_alpns(
		key,
		&transport,
		vec![CHAT_ALPN.to_vec(), crate::server_control::ALPN.to_vec()],
	)
	.await?;
	if !transport.offline {
		let _ = timeout(Duration::from_secs(12), endpoint.online()).await;
	}
	let invite = Invite {
		version: 2,
		address: endpoint.addr(),
		alias: config.alias.clone(),
		name: snapshot.name.clone(),
		token: String::new(),
	};
	let (updates, receiver) = watch::channel(snapshot.clone());
	let assets = ProfileAssets::new(
		AssetCache::open(root.join("assets"))?,
		receiver.clone(),
		None,
	);
	let mut room = Room::new(snapshot.name.clone(), "independent-server".into(), "", true);
	room.snapshot = snapshot;
	let core = Arc::new(Core {
		store,
		config,
		root: root.into(),
		server_id: endpoint.id().to_string(),
		room: Mutex::new(room),
		updates,
		media: Arc::new(media::Hub::new(receiver)),
		assets,
		routes: Mutex::new(HashMap::new()),
		operation: Mutex::new(()),
		generations: Mutex::new(HashMap::new()),
		connections: Mutex::new(HashMap::new()),
		directory_status: Mutex::new(
			"Servernamnet har ännu inte registrerats under denna körning.".into(),
		),
		sequence: AtomicU64::new(1),
		closing: AtomicBool::new(false),
		rates: Mutex::new(HashMap::new()),
		started: Instant::now(),
		invitation: Mutex::new(invite.clone()),
		browser: Mutex::new(crate::server_admin::BrowserSessions::default()),
	});
	let listener = endpoint.clone();
	let shared = core.clone();
	let publication_key = load_key(root)?;
	let task = tokio::spawn(async move {
		let mut tasks = JoinSet::new();
		let publish_core = shared.clone();
		let publish_endpoint = listener.clone();
		tasks.spawn(async move {
			let mut interval = tokio::time::interval(Duration::from_secs(60));
			loop {
				interval.tick().await;
				let root = publish_core.root.clone();
				let publication = tokio::task::spawn_blocking(move || {
					storage::read_json::<crate::directory_v2::Publication>(
						&root.join("directory-v2.json"),
						16384,
					)
				})
				.await;
				if let Ok(Ok(publication)) = publication
					&& let Ok(record) = crate::directory_v2::Registration::sign(
						publication.delegation,
						publish_core.snapshot().name,
						publish_endpoint.addr(),
						&publication_key,
					) {
					let result =
						crate::directory_v2::publish(&publication.directory, &record).await;
					*publish_core.directory_status.lock().unwrap() = if result.is_ok() {
						"Servernamnet är registrerat i adressboken.".into()
					} else {
						"Namnregistreringen misslyckades. Kontrollera adressbok och anslutning."
							.into()
					};
				}
				let maintenance = publish_core.clone();
				let _ = tokio::task::spawn_blocking(move || maintenance.maintain(now())).await;
			}
		});
		loop {
			tokio::select! {
			 incoming=listener.accept()=>{let Some(incoming)=incoming else{break};if tasks.len()>=128{incoming.refuse();continue;}let core=shared.clone();tasks.spawn(async move {if let Ok(Ok(connection))=timeout(Duration::from_secs(10),incoming).await {let _=crate::server_transport::serve(connection,core).await;}});},
			 _=tasks.join_next(),if !tasks.is_empty()=>{},
			}
		}
	});
	Ok(RunningServer {
		core,
		invite,
		endpoint,
		task,
		lock: Some(lock),
	})
}

#[cfg(test)]
mod lifecycle_tests {
	use super::*;
	fn hello(name: &str, token: &str) -> Hello {
		Hello {
			version: 1,
			alias: "@test/generations".into(),
			profile: Profile {
				name: name.into(),
				accent: [1, 2, 3],
				card: None,
			},
			token: token.into(),
		}
	}
	#[tokio::test]
	async fn stale_cleanup_cannot_remove_replacement_connection() {
		let root = tempfile::tempdir().unwrap();
		let mut config = ServerConfig::local("@test/generations", "Generations").unwrap();
		config.public = true;
		let setup = initialize(root.path(), config).unwrap();
		let running = start(
			root.path(),
			TransportConfig {
				offline: true,
				..Default::default()
			},
		)
		.await
		.unwrap();
		running
			.core
			.admit("owner", hello("Owner", &setup.pairing_token))
			.unwrap();
		let old = running.core.admit("guest", hello("Guest", "")).unwrap();
		running
			.core
			.execute(
				"owner",
				Request::Admin(AdminAction::RemoveMember {
					peer: "guest".into(),
				}),
			)
			.unwrap();
		let replacement = running.core.admit("guest", hello("Guest", "")).unwrap();
		running.core.leave("guest", old);
		assert!(running.core.current("guest", replacement));
		assert!(running.snapshot().members.iter().any(|m| m.id == "guest"));
		running.core.leave("guest", replacement);
		assert!(!running.snapshot().members.iter().any(|m| m.id == "guest"));
		running.close().await;
	}
}
