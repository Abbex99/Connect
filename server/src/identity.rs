use anyhow::{Context, Result};

/// Dedicated servers keep member identity stable across alias changes.
pub fn server_key(root: &SecretKey, server: iroh::PublicKey) -> SecretKey {
	use sha2::{Digest, Sha256};
	let mut hash = Sha256::new();
	hash.update(b"connect/member-server-key/v1\0");
	hash.update(root.to_bytes());
	hash.update(server.as_bytes());
	SecretKey::from_bytes(&hash.finalize().into())
}
use iroh::SecretKey;

pub fn load_or_create(profile: &str) -> Result<SecretKey> {
	let entry = keyring::Entry::new("Serein Connect identity", profile)?;
	match entry.get_password() {
		Ok(value) => value
			.parse()
			.context("Den sparade identiteten kunde inte läsas. Den har inte ersatts."),
		Err(keyring::Error::NoEntry) => {
			let key = SecretKey::generate();
			let value: String = key.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
			entry
				.set_password(&value)
				.context("Windows kunde inte spara identiteten säkert.")?;
			Ok(key)
		}
		Err(error) => Err(error).context("Windows autentiseringslagring kunde inte öppnas."),
	}
}

pub fn save_invite(profile: &str, alias: &str, invite: &str) -> Result<()> {
	keyring::Entry::new("Serein Connect invitation", &format!("{profile}:{alias}"))?
		.set_password(invite)?;
	Ok(())
}
pub fn load_invite(profile: &str, alias: &str) -> Result<String> {
	Ok(
		keyring::Entry::new("Serein Connect invitation", &format!("{profile}:{alias}"))?
			.get_password()?,
	)
}

/// Stable, domain-separated guest identity keeps endpoints in different rooms independent.
pub fn room_key(root: &SecretKey, alias: &crate::protocol::Alias) -> SecretKey {
	use sha2::{Digest, Sha256};
	let mut hash = Sha256::new();
	hash.update(b"serein-connect/guest-room-key/v1\0");
	hash.update(root.to_bytes());
	hash.update(alias.to_string().as_bytes());
	SecretKey::from_bytes(&hash.finalize().into())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn guest_room_keys_are_stable_and_domain_separated() {
		let root = SecretKey::generate();
		let first = crate::protocol::Alias::parse("@test/first").unwrap();
		let second = crate::protocol::Alias::parse("@test/second").unwrap();
		assert_eq!(
			room_key(&root, &first).public(),
			room_key(&root, &first).public()
		);
		assert_ne!(room_key(&root, &first).public(), root.public());
		assert_ne!(
			room_key(&root, &first).public(),
			room_key(&root, &second).public()
		);
		assert_ne!(
			room_key(&root, &first).public(),
			room_key(&SecretKey::generate(), &first).public()
		);
	}
}
