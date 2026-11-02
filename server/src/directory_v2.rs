//! Namespace ownership delegates a single name to a distinct server identity.
use crate::{directory, protocol::*};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::{EndpointAddr, PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delegation {
	pub owner: String,
	pub slug: String,
	pub authority: PublicKey,
	pub server: PublicKey,
	pub issued_at: u64,
	pub signature: String,
}
impl Delegation {
	fn bytes(&self) -> Result<Vec<u8>> {
		let mut b = b"connect/server-delegation/1\0".to_vec();
		b.extend(serde_json::to_vec(&(
			&self.owner,
			&self.slug,
			self.authority,
			self.server,
			self.issued_at,
		))?);
		Ok(b)
	}
	pub fn sign(alias: &Alias, server: PublicKey, key: &SecretKey) -> Result<Self> {
		let mut d = Self {
			owner: alias.owner.clone(),
			slug: alias.slug.clone(),
			authority: key.public(),
			server,
			issued_at: now(),
			signature: String::new(),
		};
		d.signature = URL_SAFE_NO_PAD.encode(key.sign(&d.bytes()?).to_bytes());
		Ok(d)
	}
	pub fn verify(&self) -> Result<()> {
		validate_slug(&self.owner)?;
		validate_slug(&self.slug)?;
		ensure!(
			self.signature.len() <= 128 && self.issued_at <= now() + 30,
			"Ogiltig namnfullmakt."
		);
		self.authority.verify(
			&self.bytes()?,
			&Signature::try_from(URL_SAFE_NO_PAD.decode(&self.signature)?.as_slice())?,
		)?;
		Ok(())
	}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Registration {
	pub delegation: Delegation,
	pub name: String,
	pub address: EndpointAddr,
	pub issued_at: u64,
	pub signature: String,
}
impl Registration {
	fn bytes(&self) -> Result<Vec<u8>> {
		let mut b = b"connect/server-registration/2\0".to_vec();
		b.extend(serde_json::to_vec(&(
			&self.delegation,
			&self.name,
			&self.address,
			self.issued_at,
		))?);
		Ok(b)
	}
	pub fn sign(
		delegation: Delegation,
		name: String,
		address: EndpointAddr,
		key: &SecretKey,
	) -> Result<Self> {
		let mut r = Self {
			delegation,
			name,
			address,
			issued_at: now(),
			signature: String::new(),
		};
		r.signature = URL_SAFE_NO_PAD.encode(key.sign(&r.bytes()?).to_bytes());
		r.verify(now())?;
		Ok(r)
	}
	pub fn verify(&self, time: u64) -> Result<()> {
		self.delegation.verify()?;
		validate_text(&self.name, 96)?;
		ensure!(
			self.address.id == self.delegation.server
				&& self.signature.len() <= 128
				&& serde_json::to_vec(&self.address)?.len() <= 8192,
			"Fel serveridentitet."
		);
		ensure!(
			self.issued_at <= time + 30 && time.saturating_sub(self.issued_at) <= 180,
			"Serveradressen har gått ut."
		);
		self.address.id.verify(
			&self.bytes()?,
			&Signature::try_from(URL_SAFE_NO_PAD.decode(&self.signature)?.as_slice())?,
		)?;
		Ok(())
	}
	pub fn invite(&self) -> Invite {
		Invite {
			version: 2,
			alias: format!("@{}/{}", self.delegation.owner, self.delegation.slug),
			name: self.name.clone(),
			address: self.address.clone(),
			token: String::new(),
		}
	}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Publication {
	pub directory: String,
	pub delegation: Delegation,
}
pub async fn publish(base: &str, record: &Registration) -> Result<()> {
	let mut url = directory::validate_url(base)?;
	url.set_path("/v2/register");
	let response = directory::client()?.post(url).json(record).send().await?;
	ensure!(
		response.status().is_success(),
		"Adressboken nekade servernamnet."
	);
	Ok(())
}
pub async fn transition(
	base: &str,
	alias: &str,
	previous: PublicKey,
	next: PublicKey,
) -> Result<()> {
	let alias = Alias::parse(alias)?;
	let mut url = directory::validate_url(base)?;
	url.set_path(&format!("/v2/servers/{}/{}", alias.owner, alias.slug));
	let mut response = directory::client()?.get(url).send().await?;
	ensure!(
		response.status().is_success(),
		"Serverflytten kunde inte verifieras."
	);
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await? {
		ensure!(
			bytes.len() + chunk.len() <= 16384,
			"Adressbokssvaret är för stort."
		);
		bytes.extend_from_slice(&chunk);
	}
	let record: Registration = serde_json::from_slice(&bytes)?;
	record.verify(now())?;
	ensure!(
		record.delegation.authority == previous
			&& record.delegation.server == next
			&& record.delegation.owner == alias.owner
			&& record.delegation.slug == alias.slug,
		"Serverflytten är inte godkänd av den tidigare ägaren."
	);
	Ok(())
}
pub async fn lookup(base: &str, target: &str) -> Result<Invite> {
	let alias = Alias::parse(target)?;
	let mut url = directory::validate_url(base)?;
	url.set_path(&format!("/v2/servers/{}/{}", alias.owner, alias.slug));
	let mut response = directory::client()?.get(url).send().await?;
	if response.status() == reqwest::StatusCode::NOT_FOUND {
		let old = directory::lookup_name(base, target).await?;
		return Ok(Invite {
			version: 1,
			alias: alias.to_string(),
			name: old.record.name,
			address: old.record.address,
			token: String::new(),
		});
	}
	ensure!(
		response.status().is_success(),
		"Servern finns inte eller adressboken är otillgänglig."
	);
	ensure!(
		response.content_length().is_none_or(|len| len <= 16384),
		"Adressbokssvaret är för stort."
	);
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await? {
		ensure!(
			bytes.len() + chunk.len() <= 16384,
			"Adressbokssvaret är för stort."
		);
		bytes.extend_from_slice(&chunk);
	}
	let record: Registration = serde_json::from_slice(&bytes)?;
	record.verify(now())?;
	ensure!(
		record.delegation.owner == alias.owner && record.delegation.slug == alias.slug,
		"Adressboken svarade för en annan server."
	);
	Ok(record.invite())
}
