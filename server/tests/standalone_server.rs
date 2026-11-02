use iroh::SecretKey;
use serein_connect::{
	identity,
	network::{self, TransportConfig},
	protocol::*,
	server::{self, ServerConfig},
	server_control::{self, Request, Response},
};

fn profile(name: &str) -> Profile {
	Profile {
		name: name.into(),
		accent: [120, 130, 160],
		card: None,
	}
}

#[tokio::test]
async fn automatic_retention_removes_messages_from_live_snapshots() {
	use serein_connect::server_store::AdminAction;
	let root = tempfile::tempdir().unwrap();
	let setup = server::initialize(
		root.path(),
		ServerConfig::local("@test/retention", "Retention").unwrap(),
	)
	.unwrap();
	let server = server::start(
		root.path(),
		TransportConfig {
			offline: true,
			..Default::default()
		},
	)
	.await
	.unwrap();
	server
		.core
		.store
		.admit("owner", profile("Owner"), Some(&setup.pairing_token), now())
		.unwrap();
	server
		.core
		.execute(
			"owner",
			Request::SendMessage {
				channel: "allmant".into(),
				text: "old".into(),
				request_id: "old".into(),
			},
		)
		.unwrap();
	server
		.core
		.execute(
			"owner",
			Request::Admin(AdminAction::SetStorage {
				max_bytes: 1024 * 1024,
				retention_days: Some(1),
			}),
		)
		.unwrap();
	assert_eq!(server.snapshot().messages.len(), 1);
	server.core.maintain(now() + 2 * 86400).unwrap();
	assert!(server.snapshot().messages.is_empty());
	server.close().await;
}

#[tokio::test]
async fn dedicated_server_outlives_owner_and_restores_committed_history() {
	let root = tempfile::tempdir().unwrap();
	let mut config = ServerConfig::local("@studio/lounge", "Studion").unwrap();
	config.public = true;
	let setup = server::initialize(root.path(), config).unwrap();
	let transport = TransportConfig {
		offline: true,
		..Default::default()
	};
	let server = server::start(root.path(), transport.clone()).await.unwrap();
	let root_key = SecretKey::generate();
	let owner = identity::server_key(&root_key, server.invite.address.id);
	let mut invite = server.invite.clone();
	invite.token = setup.pairing_token;
	let owner_session = network::join(
		invite.clone(),
		owner.clone(),
		profile("Ägare"),
		transport.clone(),
	)
	.await
	.unwrap();
	assert_eq!(owner_session.snapshots.borrow().members.len(), 1);
	assert!(owner_session.snapshots.borrow().members[0].host);
	let result = server_control::request(
		&invite,
		owner.clone(),
		transport.clone(),
		Request::SendMessage {
			channel: "allmant".into(),
			text: "Beständig text".into(),
			request_id: "request-1".into(),
		},
	)
	.await
	.unwrap();
	assert!(matches!(result, Response::Message(_)));
	let guest = identity::server_key(&SecretKey::generate(), server.invite.address.id);
	let guest_session = network::join(
		server.invite.clone(),
		guest.clone(),
		profile("Vän"),
		transport.clone(),
	)
	.await
	.unwrap();
	owner_session.close().await;
	let page = server_control::request(
		&server.invite,
		guest.clone(),
		transport.clone(),
		Request::History {
			channel: "allmant".into(),
			before: None,
			limit: 50,
		},
	)
	.await
	.unwrap();
	match page {
		Response::History(page) => assert_eq!(page.messages[0].text, "Beständig text"),
		_ => panic!("history expected"),
	}
	guest_session.close().await;
	let server_id = server.invite.address.id;
	server.close().await;
	let server = server::start(root.path(), transport.clone()).await.unwrap();
	assert_eq!(server.invite.address.id, server_id);
	assert!(server.snapshot().members.is_empty());
	let mut rejoin = server.invite.clone();
	rejoin.token.clear();
	let owner_session = network::join(
		rejoin.clone(),
		owner.clone(),
		profile("Nytt namn"),
		transport.clone(),
	)
	.await
	.unwrap();
	assert!(owner_session.snapshots.borrow().members[0].host);
	let result = server_control::request(
		&rejoin,
		owner,
		transport,
		Request::SendMessage {
			channel: "allmant".into(),
			text: "Beständig text".into(),
			request_id: "request-1".into(),
		},
	)
	.await
	.unwrap();
	assert!(matches!(result, Response::Message(_)));
	assert_eq!(server.snapshot().messages.len(), 1);
	owner_session.close().await;
	server.close().await;
}
