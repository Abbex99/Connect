use crate::protocol::*;
use crate::storage;
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::path::Path;
#[derive(Clone)]
pub struct Room {
	pub snapshot: Snapshot,
	owner: String,
	token_hash: [u8; 32],
	public: bool,
	next_id: u64,
}
impl Room {
	pub fn new(name: String, owner: String, token: &str, public: bool) -> Self {
		Self {
			snapshot: Snapshot {
				name,
				channels: vec![
					Channel {
						id: "allmant".into(),
						name: "allmänt".into(),
						kind: ChannelKind::Text,
					},
					Channel {
						id: "samtal".into(),
						name: "Samtal".into(),
						kind: ChannelKind::Voice,
					},
				],
				..Default::default()
			},
			owner,
			token_hash: Sha256::digest(token.as_bytes()).into(),
			public,
			next_id: 1,
		}
	}
	pub fn join(&mut self, peer: &str, profile: Profile, token: &str) -> Result<()> {
		profile.validate()?;
		let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
		let difference = hash
			.iter()
			.zip(self.token_hash)
			.fold(0u8, |difference, (a, b)| difference | (a ^ b));
		ensure!(
			self.public || difference == 0,
			"Servern är privat. Be värden om en giltig inbjudan."
		);
		ensure!(
			self.snapshot.members.len() < 64,
			"Servern är full (64 medlemmar)."
		);
		ensure!(
			!self.snapshot.members.iter().any(|m| m.id == peer),
			"Den här identiteten är redan ansluten."
		);
		self.snapshot.members.push(Member {
			id: peer.into(),
			profile,
			host: peer == self.owner,
			voice: None,
		});
		self.trim()?;
		Ok(())
	}
	pub fn apply(&mut self, peer: &str, command: Command, now: u64) -> Result<()> {
		self.apply_authorized(peer, command, now, peer == self.owner)
	}
	pub(crate) fn apply_authorized(
		&mut self,
		peer: &str,
		command: Command,
		now: u64,
		admin: bool,
	) -> Result<()> {
		let member = self
			.snapshot
			.members
			.iter()
			.find(|m| m.id == peer)
			.ok_or_else(|| anyhow::anyhow!("Du är inte ansluten."))?
			.clone();
		let voice_channel = matches!(&command, Command::CreateVoiceChannel { .. });
		match command {
			Command::DisconnectVoice { member_id } => {
				ensure!(admin, "Bara servervärden kan koppla från en deltagare.");
				ensure!(
					member_id != self.owner,
					"Använd Lämna samtalet för din egen anslutning."
				);
				let target = self
					.snapshot
					.members
					.iter_mut()
					.find(|m| m.id == member_id)
					.ok_or_else(|| anyhow::anyhow!("Deltagaren har lämnat rummet."))?;
				target.voice = None;
			}
			Command::FileSignal(_) => {
				anyhow::bail!("Filmeddelanden måste gå via den privata anslutningen.")
			}
			Command::SendMessage { channel, text } => {
				ensure!(
					self.snapshot
						.channels
						.iter()
						.any(|c| c.id == channel && c.kind == ChannelKind::Text),
					"Välj en textkanal för meddelandet."
				);
				validate_text(&text, 4096)?;
				let mut author = member.profile;
				author.card = None;
				self.snapshot.messages.push(Message {
					id: self.next_id,
					channel,
					author_id: peer.into(),
					author,
					text: text.trim().into(),
					sent_at: now,
				});
				self.next_id += 1;
			}
			Command::CreateChannel { name } | Command::CreateVoiceChannel { name } => {
				ensure!(admin, "Bara serverägaren kan skapa kanaler.");
				let id = name.trim().to_ascii_lowercase();
				validate_slug(&id)?;
				ensure!(self.snapshot.channels.len() < 64, "Maximalt 64 kanaler.");
				ensure!(
					!self.snapshot.channels.iter().any(|c| c.id == id),
					"Kanalen finns redan."
				);
				self.snapshot.channels.push(Channel {
					id: id.clone(),
					name: id,
					kind: if voice_channel {
						ChannelKind::Voice
					} else {
						ChannelKind::Text
					},
				});
			}
			Command::JoinVoice { channel, epoch } => {
				ensure!(epoch != 0, "Ogiltig samtalsidentitet.");
				ensure!(
					self.snapshot
						.channels
						.iter()
						.any(|c| c.id == channel && c.kind == ChannelKind::Voice),
					"Röstkanalen finns inte."
				);
				ensure!(
					self.snapshot
						.members
						.iter()
						.filter(|m| m.id != peer
							&& m.voice.as_ref().is_some_and(|v| v.channel == channel))
						.count() < 8,
					"Röstkanalen är full (8 deltagare)."
				);
				self.snapshot
					.members
					.iter_mut()
					.find(|m| m.id == peer)
					.unwrap()
					.voice = Some(VoiceState {
					channel,
					epoch,
					muted: true,
					deafened: false,
					screen: false,
					screen_epoch: 0,
				});
			}
			Command::LeaveVoice { epoch } => {
				ensure!(
					member.voice.as_ref().is_some_and(|v| v.epoch == epoch),
					"Samtalet har redan ändrats."
				);
				self.snapshot
					.members
					.iter_mut()
					.find(|m| m.id == peer)
					.unwrap()
					.voice = None;
			}
			Command::SetVoice {
				epoch,
				muted,
				deafened,
			} => {
				let state = self
					.snapshot
					.members
					.iter_mut()
					.find(|m| m.id == peer)
					.unwrap()
					.voice
					.as_mut()
					.ok_or_else(|| anyhow::anyhow!("Gå med i en röstkanal först."))?;
				ensure!(state.epoch == epoch, "Samtalet har redan ändrats.");
				state.muted = muted || deafened;
				state.deafened = deafened;
			}
			Command::SetScreen {
				epoch,
				active,
				stream,
			} => {
				let current = member
					.voice
					.as_ref()
					.ok_or_else(|| anyhow::anyhow!("Gå med i en röstkanal först."))?;
				ensure!(current.epoch == epoch, "Samtalet har redan ändrats.");
				ensure!(
					stream != 0 && (active || current.screen_epoch == stream),
					"Skärmdelningen har redan ändrats."
				);
				self.snapshot
					.members
					.iter_mut()
					.find(|m| m.id == peer)
					.unwrap()
					.voice
					.as_mut()
					.unwrap()
					.screen = active;
				if active {
					self.snapshot
						.members
						.iter_mut()
						.find(|m| m.id == peer)
						.unwrap()
						.voice
						.as_mut()
						.unwrap()
						.screen_epoch = stream;
				}
			}
			Command::UpdateProfile(profile) => {
				profile.validate()?;
				if let Some(member) = self.snapshot.members.iter_mut().find(|m| m.id == peer) {
					member.profile = profile;
				}
			}
		}
		self.trim()
	}
	fn trim(&mut self) -> Result<()> {
		while self.snapshot.messages.len() > 500
			|| serde_json::to_vec(&self.snapshot.messages)?.len() > MAX_HISTORY
		{
			ensure!(
				!self.snapshot.messages.is_empty(),
				"Serverns medlemslista är för stor."
			);
			self.snapshot.messages.remove(0);
		}
		ensure!(
			serde_json::to_vec(&self.snapshot)?.len() <= MAX_SNAPSHOT,
			"Serverns medlemslista är för stor."
		);
		Ok(())
	}
	pub fn leave(&mut self, peer: &str) {
		self.snapshot.members.retain(|m| m.id != peer);
	}
	pub fn save(&self, path: &Path) -> Result<()> {
		let mut durable = self.snapshot.clone();
		durable.members.clear();
		storage::write_json(path, &durable, MAX_FRAME)
	}
	pub fn load(path: &Path, owner: String, token: &str, public: bool) -> Result<Self> {
		let mut snapshot: Snapshot = storage::read_json(path, MAX_FRAME as u64)?;
		validate_text(&snapshot.name, 96)?;
		ensure!(
			!snapshot.channels.is_empty()
				&& snapshot.channels.len() <= 64
				&& snapshot.messages.len() <= 500,
			"Ogiltig sparad server."
		);
		for channel in &snapshot.channels {
			validate_slug(&channel.id)?;
			validate_text(&channel.name, 96)?;
		}
		for message in &snapshot.messages {
			validate_text(&message.text, 4096)?;
			validate_text(&message.author.name, 96)?;
		}
		snapshot.members.clear();
		if !snapshot
			.channels
			.iter()
			.any(|c| c.kind == ChannelKind::Voice)
			&& snapshot.channels.len() < 64
		{
			let mut id = "samtal".to_string();
			let mut suffix = 2;
			while snapshot.channels.iter().any(|c| c.id == id) {
				id = format!("samtal-{suffix}");
				suffix += 1;
			}
			snapshot.channels.push(Channel {
				id,
				name: "Samtal".into(),
				kind: ChannelKind::Voice,
			});
		}
		let next_id = snapshot
			.messages
			.iter()
			.map(|m| m.id)
			.max()
			.unwrap_or(0)
			.checked_add(1)
			.ok_or_else(|| anyhow::anyhow!("Ogiltiga meddelandenummer."))?;
		let mut room = Self::new(snapshot.name.clone(), owner, token, public);
		room.snapshot = snapshot;
		room.next_id = next_id;
		room.trim()?;
		Ok(room)
	}
}
