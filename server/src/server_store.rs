//! Durable domain for standalone servers. Call blocking operations off the media executor.
//! Peer IDs must come from authenticated transport, never a request body.
use crate::protocol::{Channel, ChannelKind, Message, Profile, Snapshot, validate_text};
use anyhow::{Context, Result, bail, ensure};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
	path::{Path, PathBuf},
	sync::{Mutex, MutexGuard},
	time::Duration,
};

pub const DEFAULT_MAX_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_PAGE: usize = 100;
pub const MAX_AUDIT: usize = 1000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
	Owner,
	Admin,
	Member,
}

impl Role {
	fn text(self) -> &'static str {
		match self {
			Self::Owner => "owner",
			Self::Admin => "admin",
			Self::Member => "member",
		}
	}

	fn parse(s: &str) -> Result<Self> {
		match s {
			"owner" => Ok(Self::Owner),
			"admin" => Ok(Self::Admin),
			"member" => Ok(Self::Member),
			_ => bail!("Ogiltig lagrad roll."),
		}
	}
}

/// Internal authority is never deserialized from a network request.
#[derive(Clone, Copy)]
pub(crate) enum Authority<'a> {
	Peer(&'a str),
	LocalOperator,
}
impl Authority<'_> {
	fn authorize(self, c: &Connection) -> Result<Role> {
		match self {
			Self::Peer(peer) => require_admin(c, peer),
			Self::LocalOperator => Ok(Role::Owner),
		}
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredMember {
	pub peer: String,
	pub profile: Profile,
	pub role: Role,
	pub active: bool,
	pub banned: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryPage {
	pub messages: Vec<Message>,
	pub next_before: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageSettings {
	pub max_bytes: u64,
	pub retention_days: Option<u32>,
	pub used_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEntry {
	pub id: u64,
	pub peer: String,
	pub action: String,
	pub at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredInvite {
	pub id: String,
	pub expires_at: u64,
	pub max_uses: u32,
	pub uses: u32,
	pub revoked: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AdminAction {
	CreateChannel {
		name: String,
		kind: ChannelKind,
	},
	RenameChannel {
		channel: String,
		name: String,
	},
	ReorderChannels {
		channels: Vec<String>,
	},
	DeleteChannel {
		channel: String,
		delete_history: bool,
	},
	DeleteMessage {
		id: u64,
	},
	SetRole {
		peer: String,
		role: Role,
	},
	RemoveMember {
		peer: String,
	},
	BanMember {
		peer: String,
	},
	UnbanMember {
		peer: String,
	},
	CreateInvite {
		expires_at: u64,
		max_uses: u32,
	},
	RevokeInvite {
		id: String,
	},
	RenameServer {
		name: String,
	},
	SetStorage {
		max_bytes: u64,
		retention_days: Option<u32>,
	},
	ApplyRetention,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AdminResult {
	Ok,
	Channel(Channel),
	Invite { token: String },
}

pub struct ServerStore {
	path: PathBuf,
	db: Mutex<Connection>,
}

impl ServerStore {
	pub fn open(path: &Path, name: &str) -> Result<Self> {
		validate_text(name, 96)?;
		let mut c = Connection::open(path).context("Serverdatabasen kunde inte öppnas.")?;
		c.busy_timeout(Duration::from_secs(5))?;
		c.execute_batch(
			"PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
		)?;
		let version: u32 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
		ensure!(version <= 1, "Databasen kräver en nyare serverversion.");
		if version == 0 {
			let tx = c.transaction()?;
			tx.execute_batch("CREATE TABLE settings(id INTEGER PRIMARY KEY CHECK(id=1),name TEXT NOT NULL,max_bytes INTEGER NOT NULL,retention_days INTEGER,setup_hash TEXT,setup_expiry INTEGER NOT NULL DEFAULT 0);
CREATE TABLE members(peer TEXT PRIMARY KEY,profile TEXT NOT NULL,role TEXT NOT NULL CHECK(role IN ('owner','admin','member')),active INTEGER NOT NULL,banned INTEGER NOT NULL);
CREATE TABLE channels(id TEXT PRIMARY KEY,name TEXT NOT NULL,kind TEXT NOT NULL,position INTEGER NOT NULL,deleted INTEGER NOT NULL DEFAULT 0);
CREATE TABLE invites(hash TEXT PRIMARY KEY,expires INTEGER NOT NULL,max_uses INTEGER NOT NULL,uses INTEGER NOT NULL DEFAULT 0,revoked INTEGER NOT NULL DEFAULT 0);
CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT,channel TEXT NOT NULL REFERENCES channels(id),author_id TEXT NOT NULL,profile TEXT NOT NULL,text TEXT NOT NULL,sent_at INTEGER NOT NULL);
CREATE INDEX message_channel_id ON messages(channel,id);
CREATE TABLE requests(peer TEXT NOT NULL,request_id TEXT NOT NULL,message_id INTEGER NOT NULL,channel TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(peer,request_id));
CREATE TABLE audit(id INTEGER PRIMARY KEY AUTOINCREMENT,peer TEXT NOT NULL,action TEXT NOT NULL,at INTEGER NOT NULL);
PRAGMA user_version=1;")?;
			tx.execute(
				"INSERT INTO settings(id,name,max_bytes) VALUES(1,?1,?2)",
				params![name, DEFAULT_MAX_BYTES as i64],
			)?;
			tx.execute("INSERT INTO channels(id,name,kind,position) VALUES('allmant','Allmänt','text',0),('samtal','Samtal','voice',1)",[])?;
			tx.commit()?;
		}

		let s = Self {
			path: path.to_path_buf(),
			db: Mutex::new(c),
		};
		s.read(|c| {
			let check: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
			ensure!(check == "ok", "Serverdatabasen är skadad.");
			Ok(())
		})?;
		Ok(s)
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
		self.db
			.lock()
			.map_err(|_| anyhow::anyhow!("Serverdatabasens lås är skadat."))
	}

	fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
		{
			let c = self.lock()?;
			f(&c)
		}
	}

	fn write<T>(&self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
		let mut c = self.lock()?;
		let tx = c
			.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
			.context("Serverdatabasen är upptagen.")?;
		let result = f(&tx)?;
		check_budget(&tx)?;
		tx.commit()
			.context("Ändringen kunde inte sparas beständigt. Kontrollera ledigt diskutrymme.")?;
		Ok(result)
	}

	/// Explicit local operator recovery; revokes all old elevated roles atomically.
	pub fn reset_pairing(&self, ttl_secs: u64, now: u64) -> Result<String> {
		ensure!(
			(1..=3600).contains(&ttl_secs),
			"Parkoppling får gälla i högst en timme."
		);
		let token = secret();
		self.write(|tx| {
			tx.execute(
				"UPDATE members SET role='member' WHERE role IN ('owner','admin')",
				[],
			)?;
			tx.execute(
				"UPDATE settings SET setup_hash=?1,setup_expiry=?2 WHERE id=1",
				params![
					hash(&token),
					timestamp(now.checked_add(ttl_secs).context("Ogiltig tid.")?)?
				],
			)?;
			audit(tx, "local", "reset_pairing", now)?;
			Ok(token)
		})
	}

	pub fn pair(&self, peer: &str, token: &str, now: u64) -> Result<Role> {
		validate_peer(peer)?;
		self.write(|tx| pair_tx(tx, peer, token, now))
	}

	pub fn role(&self, peer: &str) -> Result<Role> {
		self.read(|c| role(c, peer))
	}

	pub fn admit(
		&self,
		peer: &str,
		profile: Profile,
		token: Option<&str>,
		now: u64,
	) -> Result<Role> {
		validate_peer(peer)?;
		profile.validate()?;
		let p = serde_json::to_string(&profile)?;
		ensure!(p.len() <= 32768, "Profilen är för stor.");
		self.write(|tx| {
            let state: Option<(bool, bool)> = tx.query_row(
                "SELECT active,banned FROM members WHERE peer=?1", [peer],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ).optional()?;
            ensure!(!state.is_some_and(|s| s.1), "Du är blockerad från servern.");
            let setup_token = token.unwrap_or("");
            let setup: Option<String> = tx.query_row("SELECT setup_hash FROM settings WHERE id=1", [], |r| r.get(0))?;
            if setup.as_deref() == Some(hash(setup_token).as_str()) {
                pair_tx(tx, peer, setup_token, now)?;
                tx.execute("UPDATE members SET profile=?1 WHERE peer=?2", params![p, peer])?;
                return Ok(Role::Owner);
            }
            if state.is_some_and(|s| s.0) {
                tx.execute("UPDATE members SET profile=?1 WHERE peer=?2", params![p, peer])?;
                return role(tx, peer);
            }
            let token = token.context("En giltig inbjudan behövs.")?;
            validate_token(token)?;
            let h = hash(token);
            let changed = tx.execute(
                "UPDATE invites SET uses=uses+1 WHERE hash=?1 AND revoked=0 AND expires>?2 AND uses<max_uses",
                params![h, timestamp(now)?],
            )?;
            ensure!(changed == 1, "Inbjudan är ogiltig, förbrukad eller har gått ut.");
            ensure_capacity(tx, peer)?;
            tx.execute(
                "INSERT INTO members(peer,profile,role,active,banned) VALUES(?1,?2,'member',1,0) ON CONFLICT(peer) DO UPDATE SET profile=excluded.profile,role='member',active=1",
                params![peer, p],
            )?;
            audit(tx, peer, "admit", now)?;
            Ok(Role::Member)
        })
	}
	/// Only call when transport configuration explicitly allows public admission.
	pub fn admit_public(&self, peer: &str, profile: Profile, now: u64) -> Result<Role> {
		validate_peer(peer)?;
		profile.validate()?;
		let p = serde_json::to_string(&profile)?;
		ensure!(p.len() <= 32768, "Profilen är för stor.");

		self.write(|tx| {
            let banned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM members WHERE peer=?1 AND banned=1)", [peer], |r| r.get(0))?;
            ensure!(!banned, "Du är blockerad från servern.");
            ensure_capacity(tx, peer)?;
            tx.execute("INSERT INTO members(peer,profile,role,active,banned) VALUES(?1,?2,'member',1,0) ON CONFLICT(peer) DO UPDATE SET profile=excluded.profile,active=1", params![peer, p])?;
            audit(tx, peer, "admit_public", now)?;
            role(tx, peer)
        })
	}
	/// Apply only the operator-selected retention policy. Does not trim history by default.
	pub fn maintain(&self, now: u64) -> Result<()> {
		self.write(|tx| {
			let days: Option<u32> =
				tx.query_row("SELECT retention_days FROM settings", [], |r| r.get(0))?;
			if let Some(days) = days {
				let cutoff = now.saturating_sub(u64::from(days) * 86400);
				let count = tx.execute(
					"DELETE FROM messages WHERE sent_at<?1",
					[timestamp(cutoff)?],
				)?;
				if count > 0 {
					audit(tx, "local", "scheduled_retention", now)?;
				}
			}

			Ok(())
		})
	}

	pub fn snapshot(&self) -> Result<Snapshot> {
		self.read(|c| {
            let messages = query_messages(c,
                "SELECT m.id,m.channel,m.author_id,m.profile,m.text,m.sent_at FROM messages m JOIN channels c ON c.id=m.channel WHERE c.deleted=0 ORDER BY m.id DESC LIMIT 100", [],
            )?.into_iter().rev().collect();
            Ok(Snapshot {
                name: c.query_row("SELECT name FROM settings", [], |r| r.get(0))?,
                channels: channels(c)?,
                members: vec![],
                messages,
            })
        })
	}

	/// Explicit operator import into a server which has never received messages.
	pub fn import_legacy(&self, snapshot: &Snapshot) -> Result<()> {
		ensure!(
			snapshot.channels.len() <= 64
				&& !snapshot.channels.is_empty()
				&& snapshot.messages.len() <= 500,
			"Ogiltig äldre serverexport."
		);
		ensure!(
			serde_json::to_vec(snapshot)?.len() <= crate::protocol::MAX_FRAME,
			"Exporten är för stor."
		);
		let mut ids = std::collections::HashSet::new();
		for channel in &snapshot.channels {
			crate::protocol::validate_slug(&channel.id)?;
			validate_text(&channel.name, 96)?;
			ensure!(ids.insert(channel.id.as_str()), "Dubbla kanalidentiteter.");
		}
		self.write(|tx| {
			let used: i64 = tx.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?;
			let imported: bool = tx.query_row(
				"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='legacy_import')",
				[],
				|r| r.get(0),
			)?;
			ensure!(
				used == 0 && !imported,
				"Import kräver en tom server som inte redan har importerats."
			);
			tx.execute("DELETE FROM requests", [])?;
			tx.execute("DELETE FROM channels", [])?;
			for (position, c) in snapshot.channels.iter().enumerate() {
				tx.execute(
					"INSERT INTO channels(id,name,kind,position) VALUES(?1,?2,?3,?4)",
					params![
						c.id,
						c.name,
						if c.kind == ChannelKind::Voice {
							"voice"
						} else {
							"text"
						},
						position as i64
					],
				)?;
			}
			for m in &snapshot.messages {
				ensure!(m.id > 0, "Ogiltigt meddelande-ID.");
				validate_text(&m.text, 4096)?;
				validate_peer(&m.author_id)?;
				let mut author = m.author.clone();
				author.card = None;
				author.validate()?;
				ensure!(
					snapshot
						.channels
						.iter()
						.any(|c| c.id == m.channel && c.kind == ChannelKind::Text),
					"Meddelandet saknar textkanal."
				);
				tx.execute(
					"INSERT INTO messages(id,channel,author_id,profile,text,sent_at) VALUES(?1,?2,?3,?4,?5,?6)",
					params![
						timestamp(m.id)?,
						m.channel,
						m.author_id,
						serde_json::to_string(&author)?,
						m.text,
						timestamp(m.sent_at)?
					],
				)?;
			}
			tx.execute_batch(
				"CREATE TABLE legacy_import(completed INTEGER NOT NULL); INSERT INTO legacy_import VALUES(1);",
			)?;
			audit(tx, "local", "import_legacy", crate::protocol::now())?;
			Ok(())
		})
	}
	pub fn history(
		&self,
		peer: &str,
		channel: &str,
		before: Option<u64>,
		limit: usize,
	) -> Result<HistoryPage> {
		ensure!(
			(1..=MAX_PAGE).contains(&limit),
			"Historiksidan får innehålla 1–100 meddelanden."
		);

		self.read(|c| {
            role(c, peer)?;
            channel_exists(c, channel)?;
            let kind: String = c.query_row("SELECT kind FROM channels WHERE id=?1", [channel], |r| r.get(0))?;
            ensure!(kind == "text", "Historik kräver en textkanal.");
            let mut messages = query_messages(c,
                "SELECT id,channel,author_id,profile,text,sent_at FROM messages WHERE channel=?1 AND id<?2 ORDER BY id DESC LIMIT ?3",
                params![channel, before.map(timestamp).transpose()?.unwrap_or(i64::MAX), (limit + 1) as i64],
            )?;
            let more = messages.len() > limit;
            messages.truncate(limit);
            let next_before = if more { messages.last().map(|m| m.id) } else { None };
            messages.reverse();
            Ok(HistoryPage { messages, next_before })
        })
	}
	pub fn send_message(
		&self,
		peer: &str,
		channel: &str,
		text: &str,
		request_id: &str,
		now: u64,
	) -> Result<Message> {
		validate_text(text, 4096)?;
		validate_text(request_id, 128)?;
		self.write(|tx| {
			role(tx, peer)?;
			channel_exists(tx, channel)?;
			let digest = hash(text);
			let old: Option<(i64, String, String)> = tx
				.query_row(
					"SELECT message_id,channel,digest FROM requests WHERE peer=?1 AND request_id=?2",
					params![peer, request_id],
					|r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
				)
				.optional()?;
			if let Some((id, ch, d)) = old {
				ensure!(
					ch == channel && d == digest,
					"Samma meddelande-ID har redan använts med annat innehåll."
				);
				return query_messages(
					tx,
					"SELECT id,channel,author_id,profile,text,sent_at FROM messages WHERE id=?1",
					[id],
				)?
				.pop()
				.context("Meddelandet har raderats och får inte återskapas.");
			}

			let profile: String =
				tx.query_row("SELECT profile FROM members WHERE peer=?1", [peer], |r| {
					r.get(0)
				})?;
			let mut author: Profile = serde_json::from_str(&profile)?;
			author.card = None;
			let profile = serde_json::to_string(&author)?;
			let kind: String =
				tx.query_row("SELECT kind FROM channels WHERE id=?1", [channel], |r| {
					r.get(0)
				})?;
			ensure!(kind == "text", "Meddelanden kräver en textkanal.");
			tx.execute(
				"INSERT INTO messages(channel,author_id,profile,text,sent_at) VALUES(?1,?2,?3,?4,?5)",
				params![channel, peer, profile, text, timestamp(now)?],
			)?;
			let id = tx.last_insert_rowid() as u64;
			tx.execute(
				"INSERT INTO requests(peer,request_id,message_id,channel,digest) VALUES(?1,?2,?3,?4,?5)",
				params![peer, request_id, id as i64, channel, digest],
			)?;
			Ok(Message {
				id,
				channel: channel.into(),
				author_id: peer.into(),
				author: serde_json::from_str(&profile)?,
				text: text.into(),
				sent_at: now,
			})
		})
	}

	pub fn list_members(&self, peer: &str) -> Result<Vec<StoredMember>> {
		self.list_members_as(Authority::Peer(peer))
	}
	pub(crate) fn list_members_as(&self, authority: Authority<'_>) -> Result<Vec<StoredMember>> {
		self.read(|c| {
			authority.authorize(c)?;
			let mut q =
				c.prepare("SELECT peer,profile,role,active,banned FROM members ORDER BY peer")?;
			q.query_map([], |r| {
				Ok((
					r.get::<_, String>(0)?,
					r.get::<_, String>(1)?,
					r.get::<_, String>(2)?,
					r.get::<_, bool>(3)?,
					r.get::<_, bool>(4)?,
				))
			})?
			.map(|row| {
				let (p, profile, r, active, banned) = row?;
				let mut profile: Profile = serde_json::from_str(&profile)?;
				profile.card = None;
				Ok(StoredMember {
					peer: p,
					profile,
					role: Role::parse(&r)?,
					active,
					banned,
				})
			})
			.collect()
		})
	}

	pub fn settings(&self, peer: &str) -> Result<StorageSettings> {
		self.settings_as(Authority::Peer(peer))
	}
	pub(crate) fn settings_as(&self, authority: Authority<'_>) -> Result<StorageSettings> {
		self.read(|c| {
			authority.authorize(c)?;
			let (max_bytes, retention_days): (i64, Option<u32>) =
				c.query_row("SELECT max_bytes,retention_days FROM settings", [], |r| {
					Ok((r.get(0)?, r.get(1)?))
				})?;
			Ok(StorageSettings {
				max_bytes: max_bytes as u64,
				retention_days,
				used_bytes: used_bytes(c)?,
			})
		})
	}

	pub fn list_invites(&self, peer: &str) -> Result<Vec<StoredInvite>> {
		self.list_invites_as(Authority::Peer(peer))
	}
	pub(crate) fn list_invites_as(&self, authority: Authority<'_>) -> Result<Vec<StoredInvite>> {
		self.read(|c| {
			authority.authorize(c)?;
			let mut q = c.prepare(
				"SELECT hash,expires,max_uses,uses,revoked FROM invites ORDER BY expires DESC LIMIT 1000",
			)?;
			Ok(q.query_map([], |r| {
				Ok(StoredInvite {
					id: r.get(0)?,
					expires_at: r.get::<_, i64>(1)? as u64,
					max_uses: r.get(2)?,
					uses: r.get(3)?,
					revoked: r.get(4)?,
				})
			})?
			.collect::<rusqlite::Result<_>>()?)
		})
	}

	pub fn audit(&self, peer: &str, before: Option<u64>, limit: usize) -> Result<Vec<AuditEntry>> {
		self.audit_as(Authority::Peer(peer), before, limit)
	}
	pub(crate) fn audit_as(
		&self,
		authority: Authority<'_>,
		before: Option<u64>,
		limit: usize,
	) -> Result<Vec<AuditEntry>> {
		ensure!(
			(1..=MAX_PAGE).contains(&limit),
			"Loggsidan får innehålla 1–100 poster."
		);
		self.read(|c| {
			authority.authorize(c)?;
			let mut q = c.prepare(
				"SELECT id,peer,action,at FROM audit WHERE id<?1 ORDER BY id DESC LIMIT ?2",
			)?;
			Ok(q.query_map(
				params![
					before.map(timestamp).transpose()?.unwrap_or(i64::MAX),
					limit as i64
				],
				|r| {
					Ok(AuditEntry {
						id: r.get::<_, i64>(0)? as u64,
						peer: r.get(1)?,
						action: r.get(2)?,
						at: r.get::<_, i64>(3)? as u64,
					})
				},
			)?
			.collect::<rusqlite::Result<_>>()?)
		})
	}

	/// Produces a transactionally consistent SQLite database. Destination must not exist.
	pub fn export_database(&self, destination: &Path) -> Result<()> {
		std::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(destination)
			.context("Säkerhetskopians mål finns redan eller kan inte skapas.")?;
		let c = self.lock()?;
		let mut target =
			Connection::open_with_flags(destination, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
		let backup = rusqlite::backup::Backup::new(&c, &mut target)?;
		backup
			.run_to_completion(128, Duration::from_millis(5), None)
			.context("Databaskopian kunde inte sparas.")?;
		Ok(())
	}

	pub fn admin(&self, peer: &str, action: AdminAction, now: u64) -> Result<AdminResult> {
		self.admin_as(Authority::Peer(peer), action, now)
	}
	pub(crate) fn admin_as(
		&self,
		authority: Authority<'_>,
		action: AdminAction,
		now: u64,
	) -> Result<AdminResult> {
		self.write(|tx| {
            let actor = authority.authorize(tx)?;
            let peer = match authority { Authority::Peer(peer) => peer, Authority::LocalOperator => "local-operator" };
            let is_ban = matches!(&action, AdminAction::BanMember { .. });
            let label;
            let mut result = AdminResult::Ok;
            match action {
                AdminAction::CreateChannel { name, kind } => {
                    validate_text(&name, 96)?;
                    let count: i64 = tx.query_row("SELECT count(*) FROM channels WHERE deleted=0", [], |r| r.get(0))?;
                    ensure!(count < 64, "Servern får ha högst 64 kanaler.");
                    let id = secret();
                    let kind_text = if kind == ChannelKind::Text { "text" } else { "voice" };
                    tx.execute(
                        "INSERT INTO channels(id,name,kind,position) VALUES(?1,?2,?3,(SELECT coalesce(max(position),-1)+1 FROM channels))",
                        params![id, name, kind_text],
                    )?;
                    result = AdminResult::Channel(Channel { id, name, kind });
                    label = "create_channel";
                }
                AdminAction::RenameChannel { channel, name } => {
                    validate_text(&name, 96)?;
                    channel_exists(tx, &channel)?;
                    tx.execute("UPDATE channels SET name=?1 WHERE id=?2", params![name, channel])?;
                    label = "rename_channel";
                }
                AdminAction::ReorderChannels { channels: ids } => {
                    let existing = channels(tx)?;
                    ensure!(
                        ids.len() == existing.len()
                            && ids.iter().collect::<std::collections::HashSet<_>>().len() == ids.len()
                            && existing.iter().all(|c| ids.contains(&c.id)),
                        "Kanalordningen måste innehålla varje aktiv kanal en gång."
                    );
                    for (i, id) in ids.iter().enumerate() {
                        tx.execute("UPDATE channels SET position=?1 WHERE id=?2", params![i as i64, id])?;
                    }
                    label = "reorder_channels";
                }
                AdminAction::DeleteChannel { channel, delete_history } => {
                    channel_exists(tx, &channel)?;
                    if delete_history {
                        tx.execute("DELETE FROM messages WHERE channel=?1", [&channel])?;
                    }
                    tx.execute("UPDATE channels SET deleted=1 WHERE id=?1", [channel])?;
                    label = if delete_history { "delete_channel_and_history" } else { "archive_channel_history" };
                }
                AdminAction::DeleteMessage { id } => {
                    ensure!(tx.execute("DELETE FROM messages WHERE id=?1", [timestamp(id)?])? == 1, "Meddelandet finns inte.");
                    label = "delete_message";
                }
                AdminAction::SetRole { peer: target, role: new_role } => {
                    ensure!(actor == Role::Owner, "Endast ägaren får ändra roller.");
                    ensure!(new_role != Role::Owner, "Ägarskap ändras bara genom lokal återställning.");
                    let target_role = role(tx, &target)?;
                    ensure!(target_role != Role::Owner, "Ägaren får inte degraderas.");
                    tx.execute("UPDATE members SET role=?1 WHERE peer=?2", params![new_role.text(), target])?;
                    label = "set_role";
                }
                AdminAction::RemoveMember { peer: target } | AdminAction::BanMember { peer: target } => {
                    validate_peer(&target)?;
                    let target_role = tx.query_row("SELECT role FROM members WHERE peer=?1", [&target], |r| r.get::<_, String>(0)).optional()?;
                    ensure!(target_role.as_deref() != Some("owner"), "Ägaren får inte tas bort eller blockeras.");
                    ensure!(actor == Role::Owner || target_role.as_deref() != Some("admin"), "En administratör får inte ta bort andra administratörer.");
                    if target_role.is_none() {
                        ensure!(is_ban, "Medlemmen finns inte.");
                        ensure_capacity(tx, &target)?;
                        let p = serde_json::to_string(&Profile { name: "Blockerad".into(), accent: [0, 0, 0], card: None })?;
                        tx.execute("INSERT INTO members(peer,profile,role,active,banned) VALUES(?1,?2,'member',0,1)", params![target, p])?;
                    } else {
                        tx.execute("UPDATE members SET active=0,role='member',banned=CASE WHEN ?1 THEN 1 ELSE banned END WHERE peer=?2", params![is_ban, target])?;
                    }
                    label = if is_ban { "ban_member" } else { "remove_member" };
                }
                AdminAction::UnbanMember { peer: target } => {
                    tx.execute("UPDATE members SET banned=0 WHERE peer=?1", [target])?;
                    label = "unban_member";
                }
                AdminAction::CreateInvite { expires_at, max_uses } => {
                    ensure!(expires_at > now && expires_at.saturating_sub(now) <= 365 * 86400 && max_uses > 0 && max_uses <= 10000, "Ogiltig giltighet eller användningsgräns.");
                    tx.execute("DELETE FROM invites WHERE revoked=1 OR expires<=?1 OR uses>=max_uses", [timestamp(now)?])?;
                    let count: i64 = tx.query_row("SELECT count(*) FROM invites", [], |r| r.get(0))?;
                    ensure!(count < 1000, "Högst 1000 inbjudningar får sparas. Återkalla gamla inbjudningar först.");
                    let token = secret();
                    tx.execute("INSERT INTO invites(hash,expires,max_uses) VALUES(?1,?2,?3)", params![hash(&token), timestamp(expires_at)?, max_uses])?;
                    result = AdminResult::Invite { token };
                    label = "create_invite";
                }
                AdminAction::RevokeInvite { id } => {
                    ensure!(tx.execute("UPDATE invites SET revoked=1 WHERE hash=?1", [id])? == 1, "Inbjudan finns inte.");
                    label = "revoke_invite";
                }
                AdminAction::RenameServer { name } => {
                    validate_text(&name, 96)?;
                    tx.execute("UPDATE settings SET name=?1", [name])?;
                    label = "rename_server";
                }
                AdminAction::SetStorage { max_bytes, retention_days } => {
                    ensure!(actor == Role::Owner, "Endast ägaren får ändra lagringsgränser.");
                    ensure!(max_bytes >= used_bytes(tx)? && max_bytes <= 16 * 1024 * 1024 * 1024, "Lagringsgränsen måste rymma databasen och vara högst 16 GiB.");
                    ensure!(retention_days.is_none_or(|d| (1..=36500).contains(&d)), "Ogiltig bevarandetid.");
                    tx.execute("UPDATE settings SET max_bytes=?1,retention_days=?2", params![max_bytes as i64, retention_days])?;
                    label = "set_storage";
                }
                AdminAction::ApplyRetention => {
                    let days: Option<u32> = tx.query_row("SELECT retention_days FROM settings", [], |r| r.get(0))?;
                    let days = days.context("Ingen bevarandetid är vald.")?;
                    let cutoff = now.saturating_sub(u64::from(days) * 86400);
                    tx.execute("DELETE FROM messages WHERE sent_at<?1", [timestamp(cutoff)?])?;
                    label = "apply_retention";
                }
            }
            audit(tx, peer, label, now)?;
            Ok(result)
        })
	}
}

fn timestamp(n: u64) -> Result<i64> {
	i64::try_from(n).context("Talet är för stort.")
}

fn validate_peer(peer: &str) -> Result<()> {
	ensure!(
		!peer.is_empty()
			&& peer.len() <= 128
			&& peer
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
		"Ogiltig deltagaridentitet."
	);
	Ok(())
}

fn validate_token(token: &str) -> Result<()> {
	ensure!(
		token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
		"Ogiltig inbjudningsnyckel."
	);
	Ok(())
}

fn secret() -> String {
	let mut bytes = [0u8; 32];
	rand::rng().fill_bytes(&mut bytes);
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash(s: &str) -> String {
	Sha256::digest(s.as_bytes())
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect()
}

fn role(c: &Connection, peer: &str) -> Result<Role> {
	let r: Option<String> = c
		.query_row(
			"SELECT role FROM members WHERE peer=?1 AND active=1 AND banned=0",
			[peer],
			|r| r.get(0),
		)
		.optional()?;
	Role::parse(&r.context("Du är inte en aktiv medlem på servern.")?)
}

fn require_admin(c: &Connection, peer: &str) -> Result<Role> {
	let r = role(c, peer)?;
	ensure!(r != Role::Member, "Du saknar administratörsbehörighet.");
	Ok(r)
}

fn ensure_capacity(c: &Connection, peer: &str) -> Result<()> {
	let exists: bool = c.query_row(
		"SELECT EXISTS(SELECT 1 FROM members WHERE peer=?1)",
		[peer],
		|r| r.get(0),
	)?;
	let count: i64 = c.query_row("SELECT count(*) FROM members", [], |r| r.get(0))?;
	ensure!(
		exists || count < 10000,
		"Serverns medlemsregister är fullt (10000)."
	);
	Ok(())
}

fn pair_tx(tx: &Transaction<'_>, peer: &str, token: &str, now: u64) -> Result<Role> {
	validate_token(token)?;
	let banned: bool = tx.query_row(
		"SELECT EXISTS(SELECT 1 FROM members WHERE peer=?1 AND banned=1)",
		[peer],
		|r| r.get(0),
	)?;
	ensure!(!banned, "Du är blockerad från servern.");
	let n = tx.execute(
		"UPDATE settings SET setup_hash=NULL,setup_expiry=0 WHERE setup_hash=?1 AND setup_expiry>?2",
		params![hash(token), timestamp(now)?],
	)?;
	ensure!(
		n == 1,
		"Parkopplingen är ogiltig, förbrukad eller har gått ut."
	);
	ensure_capacity(tx, peer)?;
	let p = serde_json::to_string(&Profile {
		name: "Ägare".into(),
		accent: [1, 2, 3],
		card: None,
	})?;
	tx.execute("INSERT INTO members(peer,profile,role,active,banned) VALUES(?1,?2,'owner',1,0) ON CONFLICT(peer) DO UPDATE SET role='owner',active=1",params![peer,p])?;
	audit(tx, peer, "pair_owner", now)?;
	Ok(Role::Owner)
}

fn channels(c: &Connection) -> Result<Vec<Channel>> {
	let mut q =
		c.prepare("SELECT id,name,kind FROM channels WHERE deleted=0 ORDER BY position,id")?;
	Ok(q.query_map([], |r| {
		let k: String = r.get(2)?;
		Ok(Channel {
			id: r.get(0)?,
			name: r.get(1)?,
			kind: if k == "voice" {
				ChannelKind::Voice
			} else {
				ChannelKind::Text
			},
		})
	})?
	.collect::<rusqlite::Result<_>>()?)
}

fn channel_exists(c: &Connection, ch: &str) -> Result<()> {
	let exists: bool = c.query_row(
		"SELECT EXISTS(SELECT 1 FROM channels WHERE id=?1 AND deleted=0)",
		[ch],
		|r| r.get(0),
	)?;
	ensure!(exists, "Kanalen finns inte.");
	Ok(())
}

fn query_messages(c: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<Vec<Message>> {
	let mut q = c.prepare(sql)?;
	q.query_map(p, |r| {
		Ok((
			r.get::<_, i64>(0)? as u64,
			r.get::<_, String>(1)?,
			r.get::<_, String>(2)?,
			r.get::<_, String>(3)?,
			r.get::<_, String>(4)?,
			r.get::<_, i64>(5)? as u64,
		))
	})?
	.map(|row| {
		let (id, channel, author_id, profile, text, sent_at) = row?;
		let mut author: Profile = serde_json::from_str(&profile)?;
		author.card = None;
		Ok(Message {
			id,
			channel,
			author_id,
			author,
			text,
			sent_at,
		})
	})
	.collect()
}

fn used_bytes(c: &Connection) -> Result<u64> {
	let pages: i64 = c.query_row("PRAGMA page_count", [], |r| r.get(0))?;
	let size: i64 = c.query_row("PRAGMA page_size", [], |r| r.get(0))?;
	Ok((pages * size) as u64)
}

fn check_budget(c: &Connection) -> Result<()> {
	let max: i64 = c.query_row("SELECT max_bytes FROM settings", [], |r| r.get(0))?;
	ensure!(
		used_bytes(c)? <= max as u64,
		"Serverns lagringsbudget är slut. Ändringen sparades inte."
	);
	Ok(())
}

fn audit(c: &Connection, peer: &str, action: &str, now: u64) -> Result<()> {
	c.execute(
		"INSERT INTO audit(peer,action,at) VALUES(?1,?2,?3)",
		params![peer, action, timestamp(now)?],
	)?;
	c.execute(
		"DELETE FROM audit WHERE id NOT IN (SELECT id FROM audit ORDER BY id DESC LIMIT 1000)",
		[],
	)?;
	Ok(())
}
