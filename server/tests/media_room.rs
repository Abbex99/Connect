use serein_connect::{protocol::*, room::Room};
fn profile() -> Profile {
	Profile {
		name: "Test".into(),
		accent: [100, 100, 100],
		card: None,
	}
}
fn command(value: serde_json::Value) -> Command {
	serde_json::from_value(value).expect("voice command must exist")
}
#[test]
fn voice_channels_require_membership_and_isolate_screen_ownership() {
	let mut room = Room::new("Room".into(), "host".into(), "invite", false);
	room.join("host", profile(), "invite").unwrap();
	room.join("guest", profile(), "invite").unwrap();
	let create = command(serde_json::json!({"CreateVoiceChannel":{"name":"voice"}}));
	assert!(room.apply("guest", create.clone(), 1).is_err());
	room.apply("host", create, 1).unwrap();
	assert!(
		room.apply(
			"host",
			command(serde_json::json!({"JoinVoice":{"channel":"allmant","epoch":1}})),
			2
		)
		.is_err()
	);
	for (peer, epoch) in [("host", 1), ("guest", 2)] {
		room.apply(
			peer,
			command(serde_json::json!({"JoinVoice":{"channel":"voice","epoch":epoch}})),
			2,
		)
		.unwrap();
	}
	room.apply(
		"host",
		command(serde_json::json!({"SetScreen":{"epoch":1,"active":true,"stream":33}})),
		3,
	)
	.unwrap();
	assert!(
		room.apply(
			"guest",
			command(serde_json::json!({"SetScreen":{"epoch":2,"active":true,"stream":33}})),
			3
		)
		.is_ok()
	);
	assert!(
		room.apply(
			"host",
			Command::SendMessage {
				channel: "voice".into(),
				text: "wrong kind".into()
			},
			3
		)
		.is_err()
	);
	room.apply(
		"host",
		command(serde_json::json!({"LeaveVoice":{"epoch":1}})),
		4,
	)
	.unwrap();
	room.apply(
		"guest",
		command(serde_json::json!({"SetScreen":{"epoch":2,"active":true,"stream":33}})),
		4,
	)
	.unwrap();
	room.leave("guest");
	assert!(
		!serde_json::to_value(&room.snapshot).unwrap()["members"]
			.as_array()
			.unwrap()
			.iter()
			.any(|m| m["voice"]["screen"] == true)
	);
}
#[test]
fn stale_control_cannot_change_new_voice_session_and_members_do_not_restore() {
	let mut room = Room::new("Room".into(), "host".into(), "invite", false);
	room.join("host", profile(), "invite").unwrap();
	room.apply(
		"host",
		command(serde_json::json!({"CreateVoiceChannel":{"name":"voice"}})),
		1,
	)
	.unwrap();
	for epoch in [11, 12] {
		room.apply(
			"host",
			command(serde_json::json!({"JoinVoice":{"channel":"voice","epoch":epoch}})),
			2,
		)
		.unwrap();
	}
	assert!(
		room.apply(
			"host",
			command(serde_json::json!({"LeaveVoice":{"epoch":11}})),
			3
		)
		.is_err()
	);
	assert!(
		room.apply(
			"host",
			command(serde_json::json!({"SetVoice":{"epoch":11,"muted":false,"deafened":false}})),
			3
		)
		.is_err()
	);
	let temp = tempfile::tempdir().unwrap();
	let path = temp.path().join("room.json");
	room.save(&path).unwrap();
	assert!(
		Room::load(&path, "host".into(), "new", false)
			.unwrap()
			.snapshot
			.members
			.is_empty()
	);
}
#[test]
fn legacy_room_adds_a_voice_channel_without_replacing_text_history() {
	let temp = tempfile::tempdir().unwrap();
	let path = temp.path().join("old.json");
	std::fs::write(&path,serde_json::to_vec(&serde_json::json!({"name":"Old","channels":[{"id":"allmant","name":"allmänt"},{"id":"samtal","name":"samtal"}],"members":[],"messages":[]})).unwrap()).unwrap();
	let room = Room::load(&path, "host".into(), "new", false).unwrap();
	assert!(
		room.snapshot
			.channels
			.iter()
			.any(|c| c.id == "samtal" && c.kind == ChannelKind::Text)
	);
	assert!(
		room.snapshot
			.channels
			.iter()
			.any(|c| c.id == "samtal-2" && c.kind == ChannelKind::Voice)
	);
}
