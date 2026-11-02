use serein_connect::{
	network::{self, HostOptions, TransportConfig},
	protocol::*,
};
use std::{path::PathBuf, time::Duration};
fn profile(name: &str) -> Profile {
	Profile {
		name: name.into(),
		accent: [1, 2, 3],
		card: None,
	}
}
async fn wait_status(session: &network::Session, id: &str, status: &str) {
	tokio::time::timeout(Duration::from_secs(8), async {
		loop {
			if session
				.files
				.list()
				.iter()
				.any(|t| t.id == id && t.status == status)
			{
				break;
			}
			tokio::time::sleep(Duration::from_millis(20)).await;
		}
	})
	.await
	.unwrap_or_else(|_| panic!("expected {status}, got {:?}", session.files.list()));
}
#[tokio::test]
async fn receiver_only_consent_decline_and_integrity() {
	let tmp = tempfile::tempdir().unwrap();
	let transport = TransportConfig {
		offline: true,
		..Default::default()
	};
	let host = network::host(
		HostOptions {
			alias: Alias::parse("@files/test").unwrap(),
			name: "Files".into(),
			profile: profile("Host"),
			public: false,
			history: tmp.path().join("history"),
			directory_url: None,
		},
		iroh::SecretKey::generate(),
		transport.clone(),
	)
	.await
	.unwrap();
	let a = network::join(
		host.invite.clone().unwrap(),
		iroh::SecretKey::generate(),
		profile("Alice"),
		transport.clone(),
	)
	.await
	.unwrap();
	let b = network::join(
		host.invite.clone().unwrap(),
		iroh::SecretKey::generate(),
		profile("Bob"),
		transport,
	)
	.await
	.unwrap();
	for s in [&host, &a, &b] {
		s.send(Command::JoinVoice {
			channel: "samtal".into(),
			epoch: 1,
		})
		.unwrap();
	}
	tokio::time::sleep(Duration::from_millis(150)).await;
	let path = tmp.path().join("archive.zip");
	let content = vec![42u8; 1024 * 1024 + 7];
	std::fs::write(&path, &content).unwrap();
	let id = a
		.files
		.offer(b.self_id.clone(), path.clone())
		.await
		.unwrap();
	wait_status(&b, &id, "offered").await;
	assert!(
		host.files.list().is_empty(),
		"host must not see guest-to-guest offer"
	);
	assert_eq!(a.files.list()[0].done, 0);
	assert_eq!(b.files.list()[0].sender_name, "Alice");
	b.files.decline(&id).unwrap();
	wait_status(&a, &id, "declined").await;
	let id = a.files.offer(b.self_id.clone(), path).await.unwrap();
	wait_status(&b, &id, "offered").await;
	let dest = tmp.path().join("downloads");
	b.files.accept(&id, dest.clone()).await.unwrap();
	wait_status(&a, &id, "complete").await;
	wait_status(&b, &id, "complete").await;
	let completed = b.files.list().into_iter().find(|t| t.id == id).unwrap();
	let saved = PathBuf::from(completed.path.unwrap());
	assert!(saved.starts_with(dest));
	assert_eq!(std::fs::read(saved).unwrap(), content);
	assert_eq!(
		b.files.list().iter().find(|t| t.id == id).unwrap().done,
		content.len() as u64
	);
	// An existing final path must remain untouched.
	let path2 = tmp.path().join("second.zip");
	std::fs::write(&path2, b"second").unwrap();
	let collision = a.files.offer(b.self_id.clone(), path2).await.unwrap();
	wait_status(&b, &collision, "offered").await;
	let target = tmp
		.path()
		.join("downloads")
		.join(format!("{}-second.zip", &collision[..16]));
	std::fs::write(&target, b"existing").unwrap();
	b.files
		.accept(&collision, tmp.path().join("downloads"))
		.await
		.unwrap();
	wait_status(&b, &collision, "failed").await;
	assert_eq!(std::fs::read(target).unwrap(), b"existing");
	// Cancellation after bytes begin must remove the partial and never publish a final file.
	let large = tmp.path().join("cancel.zip");
	std::fs::File::create(&large)
		.unwrap()
		.set_len(64 * 1024 * 1024)
		.unwrap();
	let cancelled = a
		.files
		.offer(b.self_id.clone(), large.clone())
		.await
		.unwrap();
	wait_status(&b, &cancelled, "offered").await;
	let cancel_dir = tmp.path().join("cancelled");
	b.files
		.accept(&cancelled, cancel_dir.clone())
		.await
		.unwrap();
	tokio::time::timeout(Duration::from_secs(5), async {
		loop {
			if b.files
				.list()
				.iter()
				.any(|t| t.id == cancelled && t.done > 0)
			{
				break;
			}
			tokio::time::sleep(Duration::from_millis(1)).await;
		}
	})
	.await
	.unwrap();
	b.files.cancel(&cancelled).unwrap();
	wait_status(&a, &cancelled, "cancelled").await;
	tokio::time::timeout(Duration::from_secs(5), async {
		loop {
			if std::fs::read_dir(&cancel_dir).unwrap().count() == 0 {
				break;
			}
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	})
	.await
	.unwrap();
	let pending = a.files.offer(b.self_id.clone(), large).await.unwrap();
	wait_status(&b, &pending, "offered").await;
	let closed_dir = tmp.path().join("room-closed");
	b.files.accept(&pending, closed_dir.clone()).await.unwrap();
	tokio::time::timeout(Duration::from_secs(5), async {
		loop {
			if b.files.list().iter().any(|t| t.id == pending && t.done > 0) {
				break;
			}
			tokio::time::sleep(Duration::from_millis(1)).await;
		}
	})
	.await
	.unwrap();
	host.close().await;
	wait_status(&a, &pending, "cancelled").await;
	wait_status(&b, &pending, "cancelled").await;
	tokio::time::timeout(Duration::from_secs(5), async {
		loop {
			if std::fs::read_dir(&closed_dir).unwrap().count() == 0 {
				break;
			}
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	})
	.await
	.unwrap();
	a.close().await;
	b.close().await;
}
#[test]
fn filenames_are_single_safe_components() {
	for name in [
		"../evil.zip",
		"C:\\evil.zip",
		"CON",
		"a:b",
		"a/b",
		".",
		"a\nzip",
		"a.",
	] {
		assert!(
			serein_connect::file_transfer::validate_filename(name).is_err(),
			"{name}"
		);
	}
	assert!(serein_connect::file_transfer::validate_filename("My Archive.zip").is_ok());
}

#[tokio::test]
#[ignore = "External relay file integration; run explicitly with ephemeral synthetic peers"]
async fn external_relay_carries_guest_file_without_direct_ip_transport() {
	tokio::time::timeout(Duration::from_secs(90), async {
		let tmp = tempfile::tempdir().unwrap();
		// All three endpoints disable their IP transports, including the independent
		// FILE_ALPN connection. The room host never carries file payloads itself.
		let transport = TransportConfig {
			relay_only: true,
			..Default::default()
		};
		let host = network::host(
			HostOptions {
				alias: Alias::parse("@serein-test/files-relay").unwrap(),
				name: "Synthetic file relay test".into(),
				profile: profile("Relay host"),
				public: false,
				history: tmp.path().join("relay-history"),
				directory_url: None,
			},
			iroh::SecretKey::generate(),
			transport.clone(),
		)
		.await
		.unwrap();
		let invite = host.invite.clone().unwrap();
		assert!(!invite.address.addrs.is_empty());
		assert!(invite.address.addrs.iter().all(|a| !a.is_ip()));
		let a = network::join(
			invite.clone(),
			iroh::SecretKey::generate(),
			profile("Synthetic sender"),
			transport.clone(),
		)
		.await
		.unwrap();
		let b = network::join(
			invite,
			iroh::SecretKey::generate(),
			profile("Synthetic recipient"),
			transport,
		)
		.await
		.unwrap();
		for session in [&a, &b] {
			session
				.send(Command::JoinVoice {
					channel: "samtal".into(),
					epoch: 1,
				})
				.unwrap();
		}
		tokio::time::timeout(Duration::from_secs(20), async {
			loop {
				if [&a, &b].iter().all(|s| {
					s.snapshots
						.borrow()
						.members
						.iter()
						.filter(|m| m.voice.is_some())
						.count() == 2
				}) {
					break;
				}
				tokio::time::sleep(Duration::from_millis(25)).await;
			}
		})
		.await
		.expect("relay room membership");
		let content: Vec<u8> = (0..(128 * 1024 + 7)).map(|i| (i % 251) as u8).collect();
		let source = tmp.path().join("synthetic-relay.bin");
		std::fs::write(&source, &content).unwrap();
		let id = a.files.offer(b.self_id.clone(), source).await.unwrap();
		wait_status(&b, &id, "offered").await;
		assert_eq!(
			a.files.list()[0].done,
			0,
			"no bytes before recipient consent"
		);
		assert_eq!(b.files.list()[0].sender_name, "Synthetic sender");
		assert!(host.files.list().is_empty(), "host is not the recipient");
		b.files
			.accept(&id, tmp.path().join("received"))
			.await
			.unwrap();
		tokio::time::timeout(Duration::from_secs(30), async {
			loop {
				if [&a, &b].iter().all(|s| {
					s.files
						.list()
						.iter()
						.any(|t| t.id == id && t.status == "complete")
				}) {
					break;
				}
				tokio::time::sleep(Duration::from_millis(25)).await;
			}
		})
		.await
		.unwrap_or_else(|_| {
			panic!(
				"relay transfer sender={:?}, recipient={:?}",
				a.files.list(),
				b.files.list()
			)
		});
		let received = b.files.list().into_iter().find(|t| t.id == id).unwrap();
		assert_eq!(received.done, content.len() as u64);
		assert_eq!(std::fs::read(received.path.unwrap()).unwrap(), content);
		assert!(host.files.list().is_empty());
		a.close().await;
		b.close().await;
		host.close().await;
	})
	.await
	.expect("external relay file test exceeded 90 seconds");
}
