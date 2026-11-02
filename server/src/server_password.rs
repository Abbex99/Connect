//! Password-protected admission without changing the preserved client protocol.
//! The browser unlocks an admission key; the server checks its hash on every join.
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
	aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
	pbkdf2,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::num::NonZeroU32;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasswordAccess {
	pub token_hash: String,
	pub salt: String,
	pub nonce: String,
	pub ciphertext: String,
}
impl PasswordAccess {
	pub fn create(password: &str) -> Result<Self> {
		ensure!(
			!password.is_empty() && password.len() <= 256,
			"Lösenordet måste vara 1–256 byte."
		);
		let random: [u8; 32] = rand::random();
		let token = format!("{:x}", Sha256::digest(random));
		let salt: [u8; 16] = rand::random();
		let nonce: [u8; 12] = rand::random();
		let mut key = [0; 32];
		pbkdf2::derive(
			pbkdf2::PBKDF2_HMAC_SHA256,
			NonZeroU32::new(600_000).unwrap(),
			&salt,
			password.as_bytes(),
			&mut key,
		);
		let cipher = LessSafeKey::new(
			UnboundKey::new(&AES_256_GCM, &key)
				.map_err(|_| anyhow::anyhow!("Kryptering misslyckades."))?,
		);
		let mut ciphertext = token.as_bytes().to_vec();
		cipher
			.seal_in_place_append_tag(
				Nonce::assume_unique_for_key(nonce),
				Aad::empty(),
				&mut ciphertext,
			)
			.map_err(|_| anyhow::anyhow!("Kryptering misslyckades."))?;
		Ok(Self {
			token_hash: format!("{:x}", Sha256::digest(token.as_bytes())),
			salt: URL_SAFE_NO_PAD.encode(salt),
			nonce: URL_SAFE_NO_PAD.encode(nonce),
			ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
		})
	}
	pub fn accepts(&self, token: &str) -> bool {
		token.len() == 64 && format!("{:x}", Sha256::digest(token.as_bytes())) == self.token_hash
	}
	pub fn validate(&self) -> Result<()> {
		ensure!(
			self.token_hash.len() == 64 && self.token_hash.bytes().all(|b| b.is_ascii_hexdigit()),
			"Ogiltig lösenordsinställning."
		);
		ensure!(
			URL_SAFE_NO_PAD.decode(&self.salt)?.len() == 16
				&& URL_SAFE_NO_PAD.decode(&self.nonce)?.len() == 12
				&& URL_SAFE_NO_PAD.decode(&self.ciphertext)?.len() == 80,
			"Ogiltig lösenordsinställning."
		);
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn password_unlocks_only_matching_admission_key() {
		let stored = PasswordAccess::create("spelkväll").unwrap();
		stored.validate().unwrap();
		let mut key = [0; 32];
		pbkdf2::derive(
			pbkdf2::PBKDF2_HMAC_SHA256,
			NonZeroU32::new(600_000).unwrap(),
			&URL_SAFE_NO_PAD.decode(&stored.salt).unwrap(),
			"spelkväll".as_bytes(),
			&mut key,
		);
		let cipher = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key).unwrap());
		let mut data = URL_SAFE_NO_PAD.decode(&stored.ciphertext).unwrap();
		let nonce: [u8; 12] = URL_SAFE_NO_PAD
			.decode(&stored.nonce)
			.unwrap()
			.try_into()
			.unwrap();
		let token = cipher
			.open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data)
			.unwrap();
		assert!(stored.accepts(std::str::from_utf8(token).unwrap()));
		assert!(!stored.accepts("spelkväll"));
		assert!(
			!PasswordAccess::create("annat")
				.unwrap()
				.accepts(std::str::from_utf8(token).unwrap())
		);
	}
}
