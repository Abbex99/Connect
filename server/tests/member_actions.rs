use serein_connect::{protocol::*, room::Room};
#[test]
fn only_owner_can_disconnect_a_voice_participant() {
	let mut room = Room::new("Room".into(), "owner".into(), "key", false);
	for id in ["owner", "guest", "other"] {
		room.join(
			id,
			Profile {
				name: id.into(),
				accent: [1, 2, 3],
				card: None,
			},
			"key",
		)
		.unwrap();
		room.apply(
			id,
			Command::JoinVoice {
				channel: "samtal".into(),
				epoch: 1,
			},
			0,
		)
		.unwrap();
	}
	let command: Command =
		serde_json::from_str(r#"{"DisconnectVoice":{"member_id":"guest"}}"#).unwrap();
	assert!(room.apply("other", command.clone(), 0).is_err());
	assert!(
		room.snapshot
			.members
			.iter()
			.find(|m| m.id == "guest")
			.unwrap()
			.voice
			.is_some()
	);
	room.apply("owner", command, 0).unwrap();
	assert!(
		room.snapshot
			.members
			.iter()
			.find(|m| m.id == "guest")
			.unwrap()
			.voice
			.is_none()
	);
	assert!(
		room.snapshot
			.members
			.iter()
			.find(|m| m.id == "other")
			.unwrap()
			.voice
			.is_some()
	);
}
