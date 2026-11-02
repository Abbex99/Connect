//! Stable web invitations. Server identity and invitation capability stay separate.
use crate::{directory, protocol::*};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::{EndpointAddr, SecretKey};
pub const DEFAULT_URL: &str = "https://mailhantering.se";

fn signing_bytes(record: &Registration) -> Result<Vec<u8>> {
	let mut bytes = b"connect/server-location/3\0".to_vec();
	bytes.extend(serde_json::to_vec(record)?);
	Ok(bytes)
}
pub fn sign(record: Registration, key: &SecretKey) -> Result<SignedRegistration> {
	ensure!(record.address.id == key.public(), "Fel serveridentitet.");
	let signature = URL_SAFE_NO_PAD.encode(key.sign(&signing_bytes(&record)?).to_bytes());
	let result = SignedRegistration { record, signature };
	verify(&result, now())?;
	Ok(result)
}
pub fn verify(value: &SignedRegistration, time: u64) -> Result<()> {
	let r = &value.record;
	validate_slug(&r.owner)?;
	validate_slug(&r.slug)?;
	validate_text(&r.name, 96)?;
	ensure!(
		r.issued_at <= time.saturating_add(30) && time.saturating_sub(r.issued_at) <= 180,
		"Serveradressen har gått ut."
	);
	ensure!(
		serde_json::to_vec(&r.address)?.len() <= 8192 && value.signature.len() <= 128,
		"Registreringen är för stor."
	);
	r.address.id.verify(
		&signing_bytes(r)?,
		&iroh::Signature::try_from(URL_SAFE_NO_PAD.decode(&value.signature)?.as_slice())?,
	)?;
	Ok(())
}

pub fn base(configured: Option<&str>) -> Result<String> {
	let value = std::env::var("CONNECT_DIRECTORY_URL")
		.unwrap_or_else(|_| configured.unwrap_or(DEFAULT_URL).to_owned());
	if !value.is_empty() {
		directory::validate_url(&value)?;
	}
	Ok(value)
}

pub fn web_invite(base: &str, invite: &Invite) -> Result<String> {
	// Validate the legacy capability before wrapping it in a web link.
	invite.encode()?;
	let mut url = directory::validate_url(base)?;
	url.set_path(&format!("/server/{}", invite.address.id));
	url.set_fragment(if invite.token.is_empty() {
		None
	} else {
		Some(&invite.token)
	});
	Ok(url.into())
}

pub async fn publish(
	base: &str,
	alias: &str,
	name: String,
	address: EndpointAddr,
	key: &SecretKey,
	config: &crate::server::ServerConfig,
) -> Result<()> {
	let alias = Alias::parse(alias)?;
	let record = sign(
		Registration {
			owner: alias.owner,
			slug: alias.slug,
			name,
			address,
			issued_at: now(),
		},
		key,
	)?;
	let mut url = directory::validate_url(base)?;
	url.set_path("/v3/register");
	let mode = if config.public {
		"open"
	} else if config.password.is_some() {
		"password"
	} else {
		"invite"
	};
	let (salt, nonce, ciphertext) = config
		.password
		.as_ref()
		.map(|p| (p.salt.as_str(), p.nonce.as_str(), p.ciphertext.as_str()))
		.unwrap_or(("", "", ""));
	let server = key.public().to_string();
	let issued = record.record.issued_at;
	let mut access_bytes = b"connect/server-access/1\0".to_vec();
	access_bytes.extend(serde_json::to_vec(&(
		&server, issued, mode, salt, nonce, ciphertext,
	))?);
	let signature = URL_SAFE_NO_PAD.encode(key.sign(&access_bytes).to_bytes());
	let mut payload = serde_json::to_value(&record)?;
	payload["access"] = serde_json::json!({"server":server,"issued_at":issued,"mode":mode,"salt":salt,"nonce":nonce,"ciphertext":ciphertext,"signature":signature});
	let response = directory::client()?.post(url).json(&payload).send().await?;
	ensure!(
		response.status() != reqwest::StatusCode::CONFLICT,
		"Länknamnet är upptaget. Stoppa servern och välj ett annat länknamn under Drift och säkerhetskopiering."
	);
	ensure!(
		response.status().is_success(),
		"Adressboken kunde inte uppdatera serverlänken ({}).",
		response.status()
	);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn web_link_keeps_capability_in_fragment_and_identity_stable() {
		let key = SecretKey::from_bytes(&[11; 32]);
		let mut invite = Invite {
			version: 2,
			address: EndpointAddr::new(key.public()),
			token: "ab".repeat(32),
			alias: "@host/room".into(),
			name: "Room".into(),
		};
		let link = web_invite("https://mailhantering.se", &invite).unwrap();
		let mut url = url::Url::parse(&link).unwrap();
		assert_eq!(url.fragment(), Some(invite.token.as_str()));
		assert!(url.query().is_none());
		url.set_fragment(None);
		assert!(!url.as_str().contains(&invite.token));
		invite.address = invite
			.address
			.with_relay_url("https://relay.example/".parse().unwrap());
		assert_eq!(
			link,
			web_invite("https://mailhantering.se", &invite).unwrap()
		);
		assert!(web_invite("http://unsafe.example", &invite).is_err());
	}
}
