use serein_connect::{protocol::*, room::Room};

fn profile(name: &str) -> Profile {
	Profile {
		name: name.into(),
		accent: [176, 158, 211],
		card: None,
	}
}

#[test]
fn different_aliases_never_share_private_history() {
	let root = std::path::Path::new("data");
	let first =
		serein_connect::storage::server_history_path(root, &Alias::parse("@a-b/c").unwrap());
	let second =
		serein_connect::storage::server_history_path(root, &Alias::parse("@a/b-c").unwrap());
	assert_ne!(first, second);
}

#[test]
fn private_room_requires_invite_and_only_host_manages_channels() {
	let mut room = Room::new("My room".into(), "host".into(), "secret", false);
	assert!(room.join("guest", profile("Guest"), "wrong").is_err());
	room.join("host", profile("Host"), "secret").unwrap();
	room.join("guest", profile("Guest"), "secret").unwrap();
	assert!(
		room.apply(
			"guest",
			Command::CreateChannel {
				name: "admin".into()
			},
			1
		)
		.is_err()
	);
	room.apply(
		"host",
		Command::CreateChannel {
			name: "projekt".into(),
		},
		2,
	)
	.unwrap();
	assert_eq!(
		room.snapshot
			.channels
			.iter()
			.filter(|c| c.kind == ChannelKind::Text)
			.count(),
		2
	);
	assert!(
		room.apply(
			"unknown",
			Command::SendMessage {
				channel: "allmant".into(),
				text: "hi".into()
			},
			3
		)
		.is_err()
	);
}

#[test]
fn messages_have_authoritative_identity_order_and_valid_channels() {
	let mut room = Room::new("Room".into(), "host".into(), "secret", true);
	room.join("guest", profile("Albin"), "").unwrap();
	assert!(
		room.apply(
			"guest",
			Command::SendMessage {
				channel: "missing".into(),
				text: "hi".into()
			},
			1
		)
		.is_err()
	);
	assert!(
		room.apply(
			"guest",
			Command::SendMessage {
				channel: "allmant".into(),
				text: "a".repeat(4097)
			},
			1
		)
		.is_err()
	);
	room.apply(
		"guest",
		Command::SendMessage {
			channel: "allmant".into(),
			text: "hej".into(),
		},
		2,
	)
	.unwrap();
	room.apply(
		"guest",
		Command::SendMessage {
			channel: "allmant".into(),
			text: "igen".into(),
		},
		3,
	)
	.unwrap();
	assert_eq!(room.snapshot.messages[0].author_id, "guest");
	assert_eq!(room.snapshot.messages[1].id, 2);
	room.leave("guest");
	assert!(room.snapshot.members.is_empty());
	assert_eq!(room.snapshot.messages.len(), 2);
}

#[test]
fn history_is_bounded_by_wire_size_and_survives_restart_without_online_members() {
	let temp = tempfile::tempdir().unwrap();
	let path = temp.path().join("room.json");
	let mut room = Room::new("Room".into(), "host".into(), "secret", true);
	room.join("host", profile("Albin"), "").unwrap();
	for i in 1..550 {
		room.apply(
			"host",
			Command::SendMessage {
				channel: "allmant".into(),
				text: "x".repeat(4096),
			},
			i,
		)
		.unwrap();
	}
	assert!(serde_json::to_vec(&room.snapshot).unwrap().len() < 60 * 1024);
	assert_eq!(room.snapshot.messages.last().unwrap().id, 549);
	room.save(&path).unwrap();
	let restored = Room::load(&path, "host".into(), "new-secret", false).unwrap();
	assert!(restored.snapshot.members.is_empty());
	assert_eq!(restored.snapshot.messages.last().unwrap().id, 549);
}
