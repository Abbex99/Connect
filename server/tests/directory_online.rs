use iroh::SecretKey;
use serein_connect::{
	directory, identity,
	network::{self, TransportConfig},
	protocol::*,
	server::{self, ServerConfig},
	server_control::{Request, Response},
	server_store::AdminAction,
};
use std::time::Duration;

#[tokio::test]
#[ignore = "Publishes a synthetic server to the configured live directory and uses external relays"]
async fn live_directory_resolves_private_server_and_joins_over_relay() {
	let base = std::env::var("CONNECT_DIRECTORY_URL").expect("explicit live directory required");
	let root = tempfile::tempdir().unwrap();
	let name = format!("qa-{}", &SecretKey::generate().public().to_string()[..16]);
	let setup = server::initialize(
		root.path(),
		ServerConfig::local(&format!("@{name}/{name}"), "Connect integration test").unwrap(),
	)
	.unwrap();
	let transport = TransportConfig {
		relay_only: true,
		..Default::default()
	};
	let running = server::start(root.path(), transport.clone()).await.unwrap();
	let id = running.invite.address.id;
	let profile = Profile {
		name: "Synthetic peer".into(),
		accent: [100, 120, 160],
		card: None,
	};
	running
		.core
		.store
		.admit(
			"qa-owner",
			profile.clone(),
			Some(&setup.pairing_token),
			now(),
		)
		.unwrap();
	let response = running
		.core
		.execute(
			"qa-owner",
			Request::Admin(AdminAction::CreateInvite {
				expires_at: now() + 120,
				max_uses: 1,
			}),
		)
		.unwrap();
	let Response::Invite(link) = response else {
		panic!("expected invite")
	};
	let private = Invite::decode(&link).unwrap();
	let mut resolved = None;
	for _ in 0..20 {
		let response = directory::client()
			.unwrap()
			.get(format!("{base}/v3/servers/{id}"))
			.send()
			.await
			.unwrap();
		if response.status().is_success() {
			resolved = Some(response.json::<SignedRegistration>().await.unwrap());
			break;
		}
		tokio::time::sleep(Duration::from_millis(500)).await;
	}
	let signed = resolved.expect("automatic directory publication");
	serein_connect::directory_online::verify(&signed, now()).unwrap();
	assert!(
		signed.verify(now()).is_err(),
		"Location must not be replayable as a name claim"
	);
	assert_eq!(signed.record.address.id, id);
	let mut invite = private;
	invite.address = signed.record.address;
	let guest = identity::server_key(&SecretKey::generate(), id);
	let joined = network::join(invite.clone(), guest, profile, transport)
		.await
		.unwrap();
	assert!(
		joined
			.snapshots
			.borrow()
			.members
			.iter()
			.any(|m| m.profile.name == "Synthetic peer")
	);
	// Web links never expose the private invitation key to the directory request.
	let url =
		url::Url::parse(&serein_connect::directory_online::web_invite(&base, &invite).unwrap())
			.unwrap();
	assert_eq!(url.fragment(), Some(invite.token.as_str()));
	joined.close().await;
	running.close().await;
	println!(
		"Synthetic live server {id}: registration, signed lookup and relay-only private join passed."
	);
}
