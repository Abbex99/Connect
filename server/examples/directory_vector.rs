//! Synthetic cross-language protocol fixture. No user keys or data.
use iroh::{EndpointAddr, SecretKey};
use serein_connect::{
	directory_v2,
	protocol::{Alias, Registration, SignedRegistration},
};
fn main() -> anyhow::Result<()> {
	let key = SecretKey::from_bytes(&[42; 32]);
	let server = SecretKey::from_bytes(&[43; 32]);
	let address = EndpointAddr::new(server.public())
		.with_relay_url("https://euc1-1.relay.n0.iroh.link./".parse()?);
	let v1 = SignedRegistration::sign(
		Registration {
			owner: "fixture".into(),
			slug: "server".into(),
			name: "Åäö 🎮 Connect".into(),
			address: address.clone(),
			issued_at: serein_connect::protocol::now(),
		},
		&server,
	)?;
	let delegation =
		directory_v2::Delegation::sign(&Alias::parse("@fixture/server")?, server.public(), &key)?;
	let v2 =
		directory_v2::Registration::sign(delegation, "Åäö 🎮 Connect".into(), address, &server)?;
	let v3 = serein_connect::directory_online::sign(v1.record.clone(), &server)?;
	let password = serein_connect::server_password::PasswordAccess::create("test-password")?;
	println!(
		"{}",
		serde_json::json!({"v1":v1,"v2":v2,"v3":v3,"password":password})
	);
	Ok(())
}
