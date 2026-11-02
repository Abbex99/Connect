use serein_connect::{media::*, protocol::*, room::Room};
#[test]
fn voice_opus_packets_are_bounded_and_variable_length() {
	for size in [1, 160, 320, 1000] {
		assert!(
			Frame {
				kind: Kind::Audio,
				epoch: 1,
				sequence: 1,
				data: vec![0; size]
			}
			.validate()
			.is_ok()
		);
	}
	for size in [0, 1001] {
		assert!(
			Frame {
				kind: Kind::Audio,
				epoch: 1,
				sequence: 1,
				data: vec![0; size]
			}
			.validate()
			.is_err()
		);
	}
}
#[test]
fn buffered_media_cannot_be_relabelled_for_a_new_listener_session() {
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
	room.apply(
		"a",
		Command::SetVoice {
			epoch: 1,
			muted: false,
			deafened: false,
		},
		0,
	)
	.unwrap();
	let (tx, rx) = tokio::sync::watch::channel(room.snapshot.clone());
	let hub = Hub::new(rx);
	let mut events = hub.audio.subscribe();
	hub.publish(
		"a",
		Frame {
			kind: Kind::Audio,
			epoch: 1,
			sequence: 1,
			data: vec![1; 640],
		},
	)
	.unwrap();
	let event = events.try_recv().unwrap();
	room.apply(
		"b",
		Command::JoinVoice {
			channel: "samtal".into(),
			epoch: 3,
		},
		0,
	)
	.unwrap();
	tx.send_replace(room.snapshot);
	assert!(
		hub.delivery("b", &event).is_none(),
		"queued audio must retain its original receiver epoch"
	);
}
#[test]
fn screen_restart_rejects_old_frames_and_old_stop_commands() {
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
	let (tx, rx) = tokio::sync::watch::channel(room.snapshot.clone());
	let hub = Hub::new(rx);
	let mut events = hub.video.subscribe();
	let old = Frame {
		kind: Kind::Video,
		epoch: 11,
		sequence: 1,
		data: vec![0, 0, 0, 0x9d, 1, 0x2a, 0x80, 0x07, 0x38, 0x04],
	};
	hub.publish("a", old.clone()).unwrap();
	let event = events.try_recv().unwrap();
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
	assert!(
		room.apply(
			"a",
			Command::SetScreen {
				epoch: 1,
				active: false,
				stream: 11
			},
			0
		)
		.is_err()
	);
	tx.send_replace(room.snapshot);
	assert!(hub.delivery("b", &event).is_none());
	assert!(hub.publish("a", old).is_err());
}
#[test]
fn bounded_wire_roundtrip_rejects_bad_lengths_dimensions_and_identity_leaks() {
	let frame = Frame {
		kind: Kind::Audio,
		epoch: 1,
		sequence: 1,
		data: vec![1; 640],
	};
	assert_eq!(
		Frame::decode(&frame.encode().unwrap()).unwrap().data,
		frame.data
	);
	assert!(Frame::decode(&[0; 8]).is_err());
	assert!(
		Frame {
			data: vec![0; 1001],
			..frame.clone()
		}
		.validate()
		.is_err()
	);
	let mut video = Frame {
		kind: Kind::Video,
		epoch: 1,
		sequence: 1,
		data: vec![0, 0, 0, 0x9d, 1, 0x2a, 0x80, 0x07, 0x38, 0x04],
	};
	assert!(video.validate().is_ok());
	video.data[7] = 0x10;
	assert!(video.validate().is_err());
	let mut room = Room::new("Room".into(), "a".into(), "key", false);
	let profile = Profile {
		name: "Test".into(),
		accent: [1, 2, 3],
		card: None,
	};
	for (id, epoch) in [("a", 1), ("b", 2), ("c", 3)] {
		room.join(id, profile.clone(), "key").unwrap();
		room.apply(
			id,
			Command::JoinVoice {
				channel: "samtal".into(),
				epoch,
			},
			0,
		)
		.unwrap();
		room.apply(
			id,
			Command::SetVoice {
				epoch,
				muted: false,
				deafened: false,
			},
			0,
		)
		.unwrap();
	}
	let (tx, rx) = tokio::sync::watch::channel(room.snapshot.clone());
	let hub = Hub::new(rx);
	let mut events = hub.audio.subscribe();
	assert!(hub.publish("outsider", frame.clone()).is_err());
	hub.publish("a", frame.clone()).unwrap();
	let event = events.try_recv().unwrap();
	assert!(hub.delivery("a", &event).is_none());
	let delivery = hub.delivery("b", &event).unwrap();
	assert_eq!(
		Delivery::decode(&delivery.encode().unwrap()).unwrap().epoch,
		2
	);
	assert!(
		hub.publish("a", frame).is_err(),
		"replayed packets must be rejected"
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
	room.apply("a", Command::LeaveVoice { epoch: 1 }, 0)
		.unwrap();
	tx.send_replace(room.snapshot);
	assert!(
		hub.delivery("c", &event).is_none(),
		"queued audio must not outlive sender membership"
	);
}
