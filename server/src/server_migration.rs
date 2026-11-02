//! An explicit, signed export; never includes the old user's private key.
use crate::{protocol::*, server_store::ServerStore, storage};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Serialize, Deserialize)]
pub struct Migration {
	pub old_server: PublicKey,
	pub target_server: PublicKey,
	pub snapshot: Snapshot,
	pub signature: String,
}
impl Migration {
	fn bytes(&self) -> Result<Vec<u8>> {
		let mut bytes = b"connect/legacy-server-migration/1\0".to_vec();
		bytes.extend(serde_json::to_vec(&(
			self.old_server,
			self.target_server,
			&self.snapshot,
		))?);
		ensure!(bytes.len() <= MAX_FRAME, "Exporten är för stor.");
		Ok(bytes)
	}
	pub fn sign(snapshot: Snapshot, target_server: PublicKey, key: &SecretKey) -> Result<Self> {
		let mut migration = Self {
			old_server: key.public(),
			target_server,
			snapshot,
			signature: String::new(),
		};
		migration.snapshot.members.clear();
		migration.signature = URL_SAFE_NO_PAD.encode(key.sign(&migration.bytes()?).to_bytes());
		Ok(migration)
	}
	pub fn verify(&self) -> Result<()> {
		ensure!(self.signature.len() <= 128, "Ogiltig migrationssignatur.");
		self.old_server.verify(
			&self.bytes()?,
			&Signature::try_from(URL_SAFE_NO_PAD.decode(&self.signature)?.as_slice())?,
		)?;
		Ok(())
	}
}
pub fn import(root: &Path, file: &Path) -> Result<()> {
	let _lock = crate::server::lock_directory(root)?;
	let migration: Migration = storage::read_json(file, (MAX_FRAME + 1024) as u64)?;
	migration.verify()?;
	ensure!(
		crate::server::load_key(root)?.public() == migration.target_server,
		"Exporten är avsedd för en annan server."
	);
	let config = crate::server::ServerConfig::load(root)?;
	ServerStore::open(&root.join("server.db"), &config.name)?.import_legacy(&migration.snapshot)?;
	Ok(())
}
