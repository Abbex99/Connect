use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::{EndpointAddr, SecretKey, Signature};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alias {
	pub owner: String,
	pub slug: String,
}
impl Alias {
	pub fn parse(input: &str) -> Result<Self> {
		ensure!(input.len() <= 100, "Namnet är för långt.");
		let value = input.trim().trim_start_matches('@').to_ascii_lowercase();
		let (owner, slug) = value
			.split_once('/')
			.ok_or_else(|| anyhow::anyhow!("Använd @namn/server."))?;
		validate_slug(owner)?;
		validate_slug(slug)?;
		Ok(Self {
			owner: owner.into(),
			slug: slug.into(),
		})
	}
}
pub fn validate_slug(value: &str) -> Result<()> {
	ensure!(
		!value.is_empty()
			&& value.len() <= 32
			&& value
				.bytes()
				.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_'),
		"Använd 1–32 tecken: a–z, 0–9, bindestreck eller understreck."
	);
	Ok(())
}
pub fn validate_text(value: &str, max: usize) -> Result<()> {
	ensure!(
		!value.trim().is_empty()
			&& value.len() <= max
			&& !value.chars().any(|c| c.is_control() && c != '\n'),
		"Texten är tom, för lång eller innehåller otillåtna tecken."
	);
	Ok(())
}
pub fn now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_secs()
}
impl std::fmt::Display for Alias {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "@{}/{}", self.owner, self.slug)
	}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Registration {
	pub owner: String,
	pub slug: String,
	pub name: String,
	pub address: EndpointAddr,
	pub issued_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedRegistration {
	pub record: Registration,
	pub signature: String,
}
impl SignedRegistration {
	pub fn sign(record: Registration, key: &SecretKey) -> Result<Self> {
		ensure!(record.address.id == key.public(), "Fel serveridentitet.");
		let signature = URL_SAFE_NO_PAD.encode(key.sign(&signing_bytes(&record)?).to_bytes());
		Ok(Self { record, signature })
	}
	pub fn verify(&self, now: u64) -> Result<()> {
		validate_slug(&self.record.owner)?;
		validate_slug(&self.record.slug)?;
		validate_text(&self.record.name, 96)?;
		ensure!(
			self.record.issued_at <= now.saturating_add(30)
				&& now.saturating_sub(self.record.issued_at) <= 180,
			"Registreringen har gått ut. Starta om anslutningen."
		);
		ensure!(
			serde_json::to_vec(&self.record.address)?.len() <= 8192 && self.signature.len() <= 128,
			"Registreringen är för stor."
		);
		let signature = Signature::try_from(URL_SAFE_NO_PAD.decode(&self.signature)?.as_slice())?;
		self.record
			.address
			.id
			.verify(&signing_bytes(&self.record)?, &signature)?;
		Ok(())
	}
}
fn signing_bytes(record: &Registration) -> Result<Vec<u8>> {
	let mut bytes = b"serein-directory-registration-v1\0".to_vec();
	bytes.extend(serde_json::to_vec(record)?);
	Ok(bytes)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invite {
	pub version: u8,
	pub address: EndpointAddr,
	pub token: String,
	pub alias: String,
	pub name: String,
}
impl Invite {
	pub fn is_link(value: &str) -> bool {
		let value = value.trim();
		value.starts_with("connect://") || value.starts_with("serein://")
	}
	pub fn encode(&self) -> Result<String> {
		self.validate()?;
		Ok(format!(
			"connect://join/{}",
			URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?)
		))
	}
	pub fn decode(value: &str) -> Result<Self> {
		ensure!(value.len() <= 16 * 1024, "Inbjudan är för lång.");
		let payload = value
			.trim()
			.strip_prefix("connect://join/")
			.or_else(|| value.trim().strip_prefix("serein://join/"))
			.ok_or_else(|| anyhow::anyhow!("Klistra in en Connect-inbjudan."))?;
		let invite: Self = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload)?)?;
		invite.validate()?;
		Ok(invite)
	}
	fn validate(&self) -> Result<()> {
		ensure!(
			matches!(self.version, 1 | 2),
			"Inbjudan använder en annan appversion."
		);
		ensure!(
			self.token.is_empty()
				|| (self.token.len() == 64 && self.token.bytes().all(|b| b.is_ascii_hexdigit())),
			"Ogiltig inbjudningsnyckel."
		);
		Alias::parse(&self.alias)?;
		validate_text(&self.name, 96)?;
		ensure!(
			serde_json::to_vec(&self.address)?.len() <= 8192,
			"Adressen är för stor."
		);
		Ok(())
	}
}

pub const ALPN: &[u8] = b"connect/chat/8";
pub const MAX_FRAME: usize = 5 * 1024 * 1024;
pub const MAX_SNAPSHOT: usize = 4 * 1024 * 1024;
pub const MAX_HISTORY: usize = 48 * 1024;

/// Content-addressed profile media. Bytes travel on separate streams, never snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileAssetRef {
	pub format: String,
	pub hash: String,
	pub size: usize,
}
impl ProfileAssetRef {
	pub fn parse(value: &str) -> Result<Self> {
		ensure!(value.len() <= 112, "Ogiltig mediereferens.");
		let parts: Vec<_> = value.split(':').collect();
		ensure!(
			parts.len() == 4 && parts[0] == "serein-asset",
			"Ogiltig mediereferens."
		);
		ensure!(
			["png", "jpeg", "webp", "gif", "mp4"].contains(&parts[1]),
			"Ogiltigt medieformat."
		);
		ensure!(
			parts[2].len() == 64
				&& parts[2]
					.bytes()
					.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
			"Ogiltig mediehash."
		);
		let size: usize = parts[3].parse()?;
		ensure!(
			size > 0 && size <= crate::profile_media::ASSET_LIMIT && size.to_string() == parts[3],
			"Profilmedia får vara högst 50 MB."
		);
		Ok(Self {
			format: parts[1].into(),
			hash: parts[2].into(),
			size,
		})
	}
	pub fn mime(&self) -> &str {
		match self.format.as_str() {
			"png" => "image/png",
			"jpeg" => "image/jpeg",
			"webp" => "image/webp",
			"gif" => "image/gif",
			"mp4" => "video/mp4",
			_ => unreachable!(),
		}
	}
}
impl std::fmt::Display for ProfileAssetRef {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"serein-asset:{}:{}:{}",
			self.format, self.hash, self.size
		)
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
	pub name: String,
	pub accent: [u8; 3],
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub card: Option<ProfileCard>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileCard {
	pub skin: String,
	pub color: String,
	pub avatar: String,
	pub cover: String,
}
impl Default for ProfileCard {
	fn default() -> Self {
		Self {
			skin: "aurora".into(),
			color: "#b5a9cd".into(),
			avatar: String::new(),
			cover: String::new(),
		}
	}
}
impl Profile {
	pub fn validate(&self) -> Result<()> {
		validate_text(&self.name, 96)?;
		ensure!(
			!self.name.chars().any(char::is_control),
			"Visningsnamnet måste vara en rad."
		);
		if let Some(card) = &self.card {
			card.validate()?;
		}
		Ok(())
	}
}
impl ProfileCard {
	pub fn validate(&self) -> Result<()> {
		ensure!(
			["midnight", "aurora", "glass", "neon", "portrait"].contains(&self.skin.as_str()),
			"Ogiltigt kortutseende."
		);
		ensure!(
			self.color.len() == 7
				&& self.color.starts_with('#')
				&& self.color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit),
			"Ogiltig kortfärg."
		);
		crate::profile_media::validate_media(
			&self.avatar,
			4096,
			crate::profile_media::AVATAR_ANIMATION_LIMIT,
			512,
			Some(10.0),
		)?;
		crate::profile_media::validate_media(
			&self.cover,
			12288,
			crate::profile_media::COVER_ANIMATION_LIMIT,
			512,
			Some(10.0),
		)?;
		Ok(())
	}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Member {
	pub id: String,
	pub profile: Profile,
	pub host: bool,
	#[serde(default)]
	pub voice: Option<VoiceState>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoiceState {
	pub channel: String,
	pub epoch: u32,
	pub muted: bool,
	pub deafened: bool,
	pub screen: bool,
	#[serde(default)]
	pub screen_epoch: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
	#[default]
	Text,
	Voice,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
	pub id: String,
	pub name: String,
	#[serde(default)]
	pub kind: ChannelKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
	pub id: u64,
	pub channel: String,
	pub author_id: String,
	pub author: Profile,
	pub text: String,
	pub sent_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Snapshot {
	pub name: String,
	pub channels: Vec<Channel>,
	pub members: Vec<Member>,
	pub messages: Vec<Message>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Command {
	DisconnectVoice {
		member_id: String,
	},
	FileSignal(crate::file_transfer::Directed),
	SendMessage {
		channel: String,
		text: String,
	},
	CreateChannel {
		name: String,
	},
	CreateVoiceChannel {
		name: String,
	},
	JoinVoice {
		channel: String,
		epoch: u32,
	},
	LeaveVoice {
		epoch: u32,
	},
	SetVoice {
		epoch: u32,
		muted: bool,
		deafened: bool,
	},
	SetScreen {
		epoch: u32,
		active: bool,
		stream: u32,
	},
	UpdateProfile(Profile),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
	pub version: u8,
	pub alias: String,
	pub profile: Profile,
	pub token: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServerFrame {
	FileSignal(crate::file_transfer::Envelope),
	Snapshot(Snapshot),
	Error(String),
}
