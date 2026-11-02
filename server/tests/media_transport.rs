use serein_connect::{
	media::{Frame, Kind},
	network::{self, HostOptions, Session, TransportConfig},
	protocol::*,
};
use std::time::Duration;
fn profile() -> Profile {
	Profile {
		name: "Synthetic".into(),
		accent: [100, 100, 100],
		card: None,
	}
}
async fn voice(session: &mut Session, channel: &str, epoch: u32) {
	session
		.send(Command::JoinVoice {
			channel: channel.into(),
			epoch,
		})
		.unwrap();
	tokio::time::timeout(Duration::from_secs(3), async {
		loop {
			if session.snapshots.borrow().members.iter().any(|m| {
				m.id == session.self_id && m.voice.as_ref().is_some_and(|v| v.epoch == epoch)
			}) {
				break;
			}
			session.snapshots.changed().await.unwrap();
		}
	})
	.await
	.unwrap();
	session
		.send(Command::SetVoice {
			epoch,
			muted: false,
			deafened: false,
		})
		.unwrap();
	tokio::time::timeout(Duration::from_secs(3), async {
		loop {
			if session
				.snapshots
				.borrow()
				.members
				.iter()
				.any(|m| m.id == session.self_id && m.voice.as_ref().is_some_and(|v| !v.muted))
			{
				break;
			}
			session.snapshots.changed().await.unwrap();
		}
	})
	.await
	.unwrap();
}
#[tokio::test]
async fn real_audio_and_video_relay_isolate_channels_and_stop_on_mute() {
	run(false).await;
}
#[tokio::test]
#[ignore = "Explicit external-relay media check with synthetic samples only"]
async fn external_relay_carries_audio_and_1080p_screen_packets() {
	run(true).await;
}
async fn run(relay: bool) {
	let temp = tempfile::tempdir().unwrap();
	let transport = TransportConfig {
		offline: !relay,
		relay_only: relay,
		..Default::default()
	};
	let mut host = network::host(
		HostOptions {
			alias: Alias::parse("@media/test").unwrap(),
			name: "Media".into(),
			profile: profile(),
			public: false,
			history: temp.path().join("history.json"),
			directory_url: None,
		},
		iroh::SecretKey::generate(),
		transport.clone(),
	)
	.await
	.unwrap();
	let invite = host.invite.clone().unwrap();
	let mut guest = network::join(
		invite.clone(),
		iroh::SecretKey::generate(),
		profile(),
		transport.clone(),
	)
	.await
	.unwrap();
	let mut outside = network::join(invite, iroh::SecretKey::generate(), profile(), transport)
		.await
		.unwrap();
	voice(&mut host, "samtal", 10).await;
	voice(&mut guest, "samtal", 20).await;
	guest
		.send_media(Frame {
			kind: Kind::Audio,
			epoch: 20,
			sequence: 1,
			data: vec![12; 640],
		})
		.unwrap();
	let heard = tokio::time::timeout(Duration::from_secs(3), host.media.recv())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(heard.delivery.sender, guest.self_id);
	assert_eq!(heard.delivery.epoch, 10);
	assert_eq!(heard.delivery.frame.data, vec![12; 640]);
	assert!(
		tokio::time::timeout(Duration::from_millis(150), outside.media.recv())
			.await
			.is_err()
	);
	guest
		.send(Command::SetScreen {
			epoch: 20,
			active: true,
			stream: 33,
		})
		.unwrap();
	tokio::time::timeout(Duration::from_secs(3), async {
		loop {
			if guest
				.snapshots
				.borrow()
				.members
				.iter()
				.any(|m| m.id == guest.self_id && m.voice.as_ref().is_some_and(|v| v.screen))
			{
				break;
			}
			guest.snapshots.changed().await.unwrap();
		}
	})
	.await
	.unwrap();
	// Synthetic VP8 keyframe header announces 1920x1080; tests transport, not codec decoding.
	let mut video = vec![0, 0, 0, 0x9d, 0x01, 0x2a, 0x80, 0x07, 0x38, 0x04];
	video.resize(40_000, 0);
	guest
		.send_media(Frame {
			kind: Kind::Video,
			epoch: 33,
			sequence: 1,
			data: video.clone(),
		})
		.unwrap();
	let viewed = tokio::time::timeout(Duration::from_secs(3), host.media.recv())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(viewed.delivery.frame.data, video);
	assert!(
		tokio::time::timeout(Duration::from_millis(150), outside.media.recv())
			.await
			.is_err()
	);
	guest
		.send(Command::SetVoice {
			epoch: 20,
			muted: true,
			deafened: false,
		})
		.unwrap();
	tokio::time::timeout(Duration::from_secs(3), async {
		loop {
			if guest
				.snapshots
				.borrow()
				.members
				.iter()
				.any(|m| m.id == guest.self_id && m.voice.as_ref().is_some_and(|v| v.muted))
			{
				break;
			}
			guest.snapshots.changed().await.unwrap();
		}
	})
	.await
	.unwrap();
	guest
		.send_media(Frame {
			kind: Kind::Audio,
			epoch: 20,
			sequence: 2,
			data: vec![12; 640],
		})
		.unwrap();
	assert!(
		tokio::time::timeout(Duration::from_millis(180), host.media.recv())
			.await
			.is_err()
	);
	// Screen audio remains available with the microphone muted.
	guest
		.send_media(Frame {
			kind: Kind::ScreenAudio,
			epoch: 33,
			sequence: 1,
			data: vec![0xf8, 0xff, 0xfe],
		})
		.unwrap();
	let sound = tokio::time::timeout(Duration::from_secs(3), host.media.recv())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(sound.delivery.frame.kind, Kind::ScreenAudio);
	assert_eq!(sound.delivery.frame.data, vec![0xf8, 0xff, 0xfe]);
	// OBS already-encoded video follows independent streams; AAC follows audio
	// datagrams and still works with the microphone muted.
	let h264 = vec![
		1, 1, 0, 0, 0, 42, 66, 192, 11, 0, 0, 0, 1, 103, 66, 192, 11, 140, 104, 71, 38, 224, 33,
		160, 32, 240, 136, 70, 160, 0, 0, 0, 1, 104, 206, 60, 128, 0, 0, 0, 1, 101, 184, 0, 4, 9,
		255, 255, 248, 122, 40, 0, 8, 47, 249, 57, 57, 58, 235, 174, 186, 235, 174, 188,
	];
	for (kind, data) in [
		(Kind::VideoH264, h264),
		(
			Kind::ScreenAac,
			vec![1, 0, 0, 0, 42, 0x11, 0x90, 0x21, 0x10],
		),
	] {
		guest
			.send_media(Frame {
				kind,
				epoch: 33,
				sequence: 1,
				data: data.clone(),
			})
			.unwrap();
		let received = tokio::time::timeout(Duration::from_secs(3), host.media.recv())
			.await
			.unwrap()
			.unwrap();
		assert_eq!(received.delivery.frame.kind, kind);
		assert_eq!(received.delivery.frame.data, data);
		assert!(
			tokio::time::timeout(Duration::from_millis(100), outside.media.recv())
				.await
				.is_err()
		);
	}
	// Sharing is bidirectional: the host may also share while the guest is presenting.
	host.send(Command::SetScreen {
		epoch: 10,
		active: true,
		stream: 44,
	})
	.unwrap();
	tokio::time::timeout(Duration::from_secs(3), async {
		loop {
			if host
				.snapshots
				.borrow()
				.members
				.iter()
				.any(|m| m.id == host.self_id && m.voice.as_ref().is_some_and(|v| v.screen))
			{
				break;
			}
			host.snapshots.changed().await.unwrap();
		}
	})
	.await
	.unwrap();
	host.send_media(Frame {
		kind: Kind::Video,
		epoch: 44,
		sequence: 1,
		data: vec![0, 0, 0, 0x9d, 1, 0x2a, 0x80, 0x07, 0x38, 0x04],
	})
	.unwrap();
	let reverse = tokio::time::timeout(Duration::from_secs(3), guest.media.recv())
		.await
		.unwrap()
		.unwrap();
	assert_eq!(reverse.delivery.sender, host.self_id);
	assert_eq!(reverse.delivery.frame.epoch, 44);
	guest.close().await;
	outside.close().await;
	host.close().await;
}
