use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::SecretKey;
use ring::{
	aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
	pbkdf2,
};
use serein_connect::{
	identity,
	network::{self, TransportConfig},
	protocol::Profile,
	server::{self, ServerConfig},
	server_password::PasswordAccess,
	storage,
};
use std::num::NonZeroU32;
fn unlock(access: &PasswordAccess, password: &str) -> String {
	let mut key = [0; 32];
	pbkdf2::derive(
		pbkdf2::PBKDF2_HMAC_SHA256,
		NonZeroU32::new(600_000).unwrap(),
		&URL_SAFE_NO_PAD.decode(&access.salt).unwrap(),
		password.as_bytes(),
		&mut key,
	);
	let cipher = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key).unwrap());
	let nonce: [u8; 12] = URL_SAFE_NO_PAD
		.decode(&access.nonce)
		.unwrap()
		.try_into()
		.unwrap();
	let mut ciphertext = URL_SAFE_NO_PAD.decode(&access.ciphertext).unwrap();
	String::from_utf8(
		cipher
			.open_in_place(
				Nonce::assume_unique_for_key(nonce),
				Aad::empty(),
				&mut ciphertext,
			)
			.unwrap()
			.to_vec(),
	)
	.unwrap()
}
#[tokio::test]
async fn password_is_enforced_for_new_and_returning_peers_and_rotates() {
	let root = tempfile::tempdir().unwrap();
	let mut config = ServerConfig::local("@test/password", "Password test").unwrap();
	config.password = Some(PasswordAccess::create("first password").unwrap());
	let token = unlock(config.password.as_ref().unwrap(), "first password");
	let setup = server::initialize(root.path(), config.clone()).unwrap();
	let transport = TransportConfig {
		offline: true,
		..Default::default()
	};
	let running = server::start(root.path(), transport.clone()).await.unwrap();
	let peer = identity::server_key(&SecretKey::generate(), running.invite.address.id);
	let profile = Profile {
		name: "Guest".into(),
		accent: [80, 100, 140],
		card: None,
	};
	let mut invite = running.invite.clone();
	assert!(
		network::join(
			invite.clone(),
			peer.clone(),
			profile.clone(),
			transport.clone()
		)
		.await
		.is_err()
	);
	invite.token = token.clone();
	let joined = network::join(
		invite.clone(),
		peer.clone(),
		profile.clone(),
		transport.clone(),
	)
	.await
	.unwrap();
	running
		.core
		.store
		.admit(
			"qa-owner",
			profile.clone(),
			Some(&setup.pairing_token),
			serein_connect::protocol::now(),
		)
		.unwrap();
	use serein_connect::{
		server_control::{self, Request, Response},
		server_store::{AdminAction, AdminResult, Role},
	};
	running
		.core
		.store
		.admin(
			"qa-owner",
			AdminAction::SetRole {
				peer: peer.public().to_string(),
				role: Role::Admin,
			},
			serein_connect::protocol::now(),
		)
		.unwrap();
	let AdminResult::Invite { token: single_use } = running
		.core
		.store
		.admin(
			"qa-owner",
			AdminAction::CreateInvite {
				expires_at: serein_connect::protocol::now() + 120,
				max_uses: 1,
			},
			serein_connect::protocol::now(),
		)
		.unwrap()
	else {
		panic!("invite")
	};
	joined.close().await;
	running.close().await;
	let running = server::start(root.path(), transport.clone()).await.unwrap();
	invite.address = running.invite.address.clone();
	invite.token = String::new();
	let control = server_control::request(
		&invite,
		peer.clone(),
		transport.clone(),
		Request::History {
			channel: "allmant".into(),
			before: None,
			limit: 10,
		},
	)
	.await;
	assert!(
		matches!(control, Err(_) | Ok(Response::Error(_))),
		"Control transport must not bypass password after restart"
	);
	assert!(
		network::join(
			invite.clone(),
			peer.clone(),
			profile.clone(),
			transport.clone()
		)
		.await
		.is_err(),
		"Known membership must not bypass password"
	);
	invite.token = single_use;
	let joined = network::join(
		invite.clone(),
		peer.clone(),
		profile.clone(),
		transport.clone(),
	)
	.await
	.unwrap();
	assert_eq!(
		running.core.store.role(&peer.public().to_string()).unwrap(),
		Role::Admin,
		"Valid invite rejoin must preserve admin role"
	);
	assert!(matches!(
		server_control::request(
			&invite,
			peer.clone(),
			transport.clone(),
			Request::History {
				channel: "allmant".into(),
				before: None,
				limit: 10
			}
		)
		.await
		.unwrap(),
		Response::History(_)
	));
	joined.close().await;
	running.close().await;
	config.password = Some(PasswordAccess::create("second password").unwrap());
	let next = unlock(config.password.as_ref().unwrap(), "second password");
	storage::write_json(&root.path().join("server.json"), &config, 16384).unwrap();
	let running = server::start(root.path(), transport.clone()).await.unwrap();
	invite.address = running.invite.address.clone();
	invite.token = token;
	assert!(
		network::join(
			invite.clone(),
			peer.clone(),
			profile.clone(),
			transport.clone()
		)
		.await
		.is_err(),
		"Old password grant must stop working after rotation"
	);
	invite.token = next;
	let joined = network::join(invite, peer, profile, transport)
		.await
		.unwrap();
	joined.close().await;
	running.close().await;
}
