use base64::Engine;
use iroh::SecretKey;
use serein_connect::{network::*, protocol::*};
use std::time::Duration;

async fn until(session: &mut Session, predicate: impl Fn(&Snapshot) -> bool) {
	tokio::time::timeout(Duration::from_secs(15), async {
		loop {
			if predicate(&session.snapshots.borrow()) {
				break;
			}
			session.snapshots.changed().await.unwrap();
		}
	})
	.await
	.expect("expected room update");
}

#[tokio::test]
async fn invite_cannot_admit_to_a_different_room_with_the_same_endpoint_key() {
	let temp = tempfile::tempdir().unwrap();
	let transport = TransportConfig {
		offline: true,
		..Default::default()
	};
	let host_session = host(
		HostOptions {
			alias: Alias::parse("@albin/two").unwrap(),
			name: "Second room".into(),
			profile: Profile {
				name: "Host".into(),
				accent: [0; 3],
				card: None,
			},
			public: true,
			history: temp.path().join("second.json"),
			directory_url: None,
		},
		SecretKey::generate(),
		transport.clone(),
	)
	.await
	.unwrap();
	let mut old_room = host_session.invite.clone().unwrap();
	old_room.alias = "@albin/one".into();
	old_room.token.clear();
	let result = join(
		old_room,
		SecretKey::generate(),
		Profile {
			name: "Guest".into(),
			accent: [0; 3],
			card: None,
		},
		transport,
	)
	.await;
	assert!(result.is_err(), "an invitation for /one entered /two");
	host_session.close().await;
}

#[tokio::test]
#[ignore = "External relay integration; run explicitly with synthetic peers"]
async fn external_relay_carries_real_chat_without_direct_ip_transport() {
	let temp = tempfile::tempdir().unwrap();
	let transport = TransportConfig {
		relay_only: true,
		..Default::default()
	};
	let mut host_session = host(
		HostOptions {
			alias: Alias::parse("@serein-test/relay").unwrap(),
			name: "Synthetic relay test".into(),
			profile: Profile {
				name: "Test host".into(),
				accent: [0; 3],
				card: None,
			},
			public: false,
			history: temp.path().join("relay.json"),
			directory_url: None,
		},
		SecretKey::generate(),
		transport.clone(),
	)
	.await
	.unwrap();
	let invite = host_session.invite.clone().unwrap();
	assert!(invite.address.addrs.iter().all(|a| !a.is_ip()));
	let guest = join(
		invite,
		SecretKey::generate(),
		Profile {
			name: "Test guest".into(),
			accent: [0; 3],
			card: None,
		},
		transport,
	)
	.await
	.unwrap();
	guest
		.send(Command::SendMessage {
			channel: "allmant".into(),
			text: "Synthetic relay verification".into(),
		})
		.unwrap();
	until(&mut host_session, |s| {
		s.messages
			.iter()
			.any(|m| m.text == "Synthetic relay verification")
	})
	.await;
	guest.close().await;
	host_session.close().await;
}

#[tokio::test]
async fn real_peers_authenticate_exchange_messages_and_leave() {
	let temp = tempfile::tempdir().unwrap();
	let transport = TransportConfig {
		offline: true,
		..Default::default()
	};
	let mut host_session = host(
		HostOptions {
			alias: Alias::parse("@albin/test").unwrap(),
			name: "Test".into(),
			profile: Profile {
				name: "Albin".into(),
				accent: [1, 2, 3],
				card: None,
			},
			public: false,
			history: temp.path().join("room.json"),
			directory_url: None,
		},
		SecretKey::generate(),
		transport.clone(),
	)
	.await
	.unwrap();
	let invite = host_session.invite.clone().unwrap();
	let mut bad = invite.clone();
	bad.token = "b".repeat(64);
	assert!(
		join(
			bad,
			SecretKey::generate(),
			Profile {
				name: "Denied".into(),
				accent: [0; 3],
				card: None,
			},
			transport.clone()
		)
		.await
		.is_err()
	);
	let mut guest = join(
		invite.clone(),
		SecretKey::generate(),
		Profile {
			name: "Guest".into(),
			accent: [5; 3],
			card: None,
		},
		transport.clone(),
	)
	.await
	.unwrap();
	let mut second = join(
		invite,
		SecretKey::generate(),
		Profile {
			name: "Second".into(),
			accent: [6; 3],
			card: None,
		},
		transport,
	)
	.await
	.unwrap();
	guest
		.send(Command::UpdateProfile(Profile {
			name: "Guest".into(),
			accent: [5; 3],
			card: Some(ProfileCard {
				skin: "neon".into(),
				color: "#e59e7c".into(),
				avatar: format!(
					"data:image/gif;base64,{}",
					base64::engine::general_purpose::STANDARD
						.encode(include_bytes!("../web/tests/fixtures/profile-loop.gif"))
				),
				cover: format!(
					"data:video/mp4;base64,{}",
					base64::engine::general_purpose::STANDARD
						.encode(include_bytes!("../web/tests/fixtures/profile-loop.mp4"))
				),
			}),
		}))
		.unwrap();
	until(&mut second, |s| {
		s.members.iter().any(|m| {
			m.id == guest.self_id
				&& m.profile.card.as_ref().is_some_and(|c| {
					c.skin == "neon"
						&& c.color == "#e59e7c"
						&& c.avatar.starts_with("data:image/gif;base64,")
						&& c.cover.starts_with("data:video/mp4;base64,")
				})
		})
	})
	.await;
	guest
		.send(Command::SendMessage {
			channel: "allmant".into(),
			text: "Real peer message".into(),
		})
		.unwrap();
	until(&mut second, |s| {
		s.messages.iter().any(|m| m.text == "Real peer message")
	})
	.await;
	host_session
		.send(Command::CreateChannel {
			name: "projekt".into(),
		})
		.unwrap();
	until(&mut guest, |s| s.channels.iter().any(|c| c.id == "projekt")).await;
	until(&mut host_session, |s| s.members.len() == 3).await;
	guest.close().await;
	until(&mut host_session, |s| s.members.len() == 2).await;
	second.close().await;
	host_session.close().await;
	let room = serein_connect::room::Room::load(
		&temp.path().join("room.json"),
		"owner".into(),
		"new",
		false,
	)
	.unwrap();
	assert_eq!(
		room.snapshot.messages.last().unwrap().text,
		"Real peer message"
	);
}
