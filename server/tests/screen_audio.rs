use serein_connect::{media::*, protocol::*, room::Room};
#[test]
fn screen_sound_is_independent_of_microphone_and_bound_to_capture_and_listener() {
	let mut room = Room::new("Room".into(), "a".into(), "key", false);
	for (id, epoch) in [("a", 1), ("b", 2)] {
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
				epoch,
			},
			0,
		)
		.unwrap();
	}
	let sound: Frame = serde_json::from_value(
		serde_json::json!({"kind":"screen_audio","epoch":11,"sequence":1,"data":"+P/+/g=="}),
	)
	.expect("screen audio must have its own media kind");
	let (tx, rx) = tokio::sync::watch::channel(room.snapshot.clone());
	let hub = Hub::new(rx);
	let mut audio = hub.screen_audio.subscribe();
	assert!(hub.publish("a", sound.clone()).is_err());
	room.apply(
		"a",
		Command::SetScreen {
			epoch: 1,
			active: true,
			stream: 11,
		},
		0,
	)
	.unwrap();
	tx.send_replace(room.snapshot.clone());
	hub.publish("a", sound.clone()).unwrap();
	let event = audio.try_recv().unwrap();
	assert!(
		hub.delivery("b", &event).is_some(),
		"muted microphone must not mute screen sound"
	);
	room.apply(
		"b",
		Command::SetVoice {
			epoch: 2,
			muted: true,
			deafened: true,
		},
		0,
	)
	.unwrap();
	tx.send_replace(room.snapshot.clone());
	assert!(hub.delivery("b", &event).is_none());
	room.apply(
		"a",
		Command::SetScreen {
			epoch: 1,
			active: false,
			stream: 11,
		},
		0,
	)
	.unwrap();
	tx.send_replace(room.snapshot.clone());
	assert!(hub.delivery("b", &event).is_none());
	assert!(
		hub.publish(
			"a",
			Frame {
				sequence: 2,
				..sound.clone()
			}
		)
		.is_err()
	);
	room.apply(
		"a",
		Command::SetScreen {
			epoch: 1,
			active: true,
			stream: 12,
		},
		0,
	)
	.unwrap();
	tx.send_replace(room.snapshot);
	assert!(
		hub.publish(
			"a",
			Frame {
				sequence: 3,
				..sound.clone()
			}
		)
		.is_err()
	);
	let mut oversized = sound;
	oversized.data = vec![0; 1001];
	assert!(oversized.validate().is_err());
}
