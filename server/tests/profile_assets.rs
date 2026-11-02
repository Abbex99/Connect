use serein_connect::{
	network::{self, HostOptions, TransportConfig},
	profile_assets::AssetCache,
	profile_media::ASSET_LIMIT,
	protocol::*,
};
use std::time::Duration;
fn profile(name: &str, asset: &str) -> Profile {
	Profile {
		name: name.into(),
		accent: [1, 2, 3],
		card: Some(ProfileCard {
			avatar: asset.into(),
			..Default::default()
		}),
	}
}
fn gif(size: usize) -> Vec<u8> {
	let mut data = include_bytes!("../web/tests/fixtures/profile-loop.gif").to_vec();
	data.resize(size.max(data.len()), 0);
	data
}
#[test]
fn original_fifty_mb_mp4_background_survives_disk_restart() {
	let tmp = tempfile::tempdir().unwrap();
	let cache = AssetCache::open(tmp.path()).unwrap();
	let mut original = include_bytes!("../web/tests/fixtures/profile-loop.mp4").to_vec();
	let padding = ASSET_LIMIT - original.len();
	original.extend_from_slice(&(padding as u32).to_be_bytes());
	original.extend_from_slice(b"free");
	original.resize(ASSET_LIMIT, 0);
	let reference = cache.put("video/mp4", &original).unwrap();
	assert!(reference.starts_with("serein-asset:mp4:"));
	drop(cache);
	let cache = AssetCache::open(tmp.path()).unwrap();
	assert_eq!(cache.get(&reference).unwrap().unwrap(), original);
	original.push(0);
	assert!(cache.put("video/mp4", &original).is_err());
}

#[test]
fn mp4_background_accepts_thirty_seconds_and_4k_but_enforces_media_bounds() {
	use serein_connect::profile_media::validate_asset;
	let mut original = include_bytes!("../web/tests/fixtures/profile-loop.mp4").to_vec();
	let movie = original
		.windows(4)
		.position(|part| part == b"mvhd")
		.unwrap();
	assert_eq!(original[movie + 4], 1);
	let scale = u32::from_be_bytes(original[movie + 24..movie + 28].try_into().unwrap());
	original[movie + 28..movie + 36].copy_from_slice(&(u64::from(scale) * 30).to_be_bytes());
	let track = original
		.windows(4)
		.position(|part| part == b"tkhd")
		.unwrap();
	let length = u32::from_be_bytes(original[track - 4..track].try_into().unwrap()) as usize;
	let width = track - 4 + length - 8;
	original[width..width + 4].copy_from_slice(&(3840u32 << 16).to_be_bytes());
	original[width + 4..width + 8].copy_from_slice(&(2160u32 << 16).to_be_bytes());
	validate_asset("video/mp4", &original).unwrap();
	for seconds in [0, 301] {
		let mut invalid = original.clone();
		invalid[movie + 28..movie + 36]
			.copy_from_slice(&(u64::from(scale) * seconds).to_be_bytes());
		assert!(validate_asset("video/mp4", &invalid).is_err());
	}
	let mut huge = original.clone();
	huge[width + 4..width + 8].copy_from_slice(&(3840u32 << 16).to_be_bytes());
	assert!(validate_asset("video/mp4", &huge).is_err());
	let mut missing_duration = original.clone();
	missing_duration[movie..movie + 4].copy_from_slice(b"free");
	assert!(validate_asset("video/mp4", &missing_duration).is_err());
	assert!(validate_asset("video/mp4", &original[..original.len() - 4]).is_err());
	assert!(validate_asset("video/mp4", b"not an MP4").is_err());
}
#[test]
fn fifty_megabyte_profile_reference_is_tiny_and_validated() {
	let reference = format!("serein-asset:gif:{}:{}", "a".repeat(64), ASSET_LIMIT);
	let mut card = ProfileCard {
		avatar: reference,
		..Default::default()
	};
	card.validate().unwrap();
	assert!(serde_json::to_vec(&card).unwrap().len() < 256);
	for invalid in [
		format!("serein-asset:gif:{}:{}", "a".repeat(64), ASSET_LIMIT + 1),
		format!("serein-asset:gif:{}:0", "a".repeat(64)),
		format!("serein-asset:gif:{}:001", "a".repeat(64)),
		format!("serein-asset:svg:{}:10", "a".repeat(64)),
		"serein-asset:gif:../../private:10".into(),
		"https://tracker.invalid/a.gif".into(),
	] {
		card.avatar = invalid;
		assert!(card.validate().is_err());
	}
}
#[test]
fn original_fifty_mb_gif_survives_disk_restart_and_bounds_are_enforced() {
	let tmp = tempfile::tempdir().unwrap();
	let cache = AssetCache::open(tmp.path()).unwrap();
	let original = gif(ASSET_LIMIT);
	let reference = cache.put("image/gif", &original).unwrap();
	assert_eq!(cache.get(&reference).unwrap().unwrap(), original);
	drop(cache);
	let cache = AssetCache::open(tmp.path()).unwrap();
	assert_eq!(cache.get(&reference).unwrap().unwrap(), original);
	assert!(cache.put("image/gif", &gif(ASSET_LIMIT + 1)).is_err());
	assert!(cache.put("image/png", &original).is_err());
	let mut huge = gif(0);
	huge[6..8].copy_from_slice(&2049u16.to_le_bytes());
	assert!(cache.put("image/gif", &huge).is_err());
	let mut broken = gif(0);
	broken.truncate(20);
	assert!(cache.put("image/gif", &broken).is_err());
	let parsed = ProfileAssetRef::parse(&reference).unwrap();
	std::fs::write(
		tmp.path().join(format!("{}.gif", parsed.hash)),
		vec![1; original.len()],
	)
	.unwrap();
	assert!(cache.get(&reference).is_err());
}
#[test]
fn animation_frame_budget_and_disk_cache_budget_are_bounded() {
	let tmp = tempfile::tempdir().unwrap();
	let cache = AssetCache::open(tmp.path()).unwrap();
	let mut excessive = Vec::new();
	{
		let mut encoder = image::codecs::gif::GifEncoder::new(&mut excessive);
		encoder
			.encode_frames((0..601).map(|_| image::Frame::new(image::RgbaImage::new(1, 1))))
			.unwrap();
	}
	assert!(cache.put("image/gif", &excessive).is_err());
	let pinned = cache.put("image/gif", &gif(1000)).unwrap();
	cache.pin(std::slice::from_ref(&pinned)).unwrap();
	// Sparse old entries exercise the disk quota without hashing hundreds of MB.
	for n in 1..13 {
		std::fs::File::create(tmp.path().join(format!("{n:064x}.gif")))
			.unwrap()
			.set_len(ASSET_LIMIT as u64)
			.unwrap();
	}
	cache.put("image/gif", &gif(2000)).unwrap();
	let total: u64 = std::fs::read_dir(tmp.path())
		.unwrap()
		.map(|entry| entry.unwrap().metadata().unwrap().len())
		.sum();
	assert!(total <= 512 * 1024 * 1024);
	assert!(cache.get(&pinned).unwrap().is_some());
}
#[test]
fn reopening_full_cache_does_not_evict_saved_asset_before_pinning() {
	let tmp = tempfile::tempdir().unwrap();
	let cache = AssetCache::open(tmp.path()).unwrap();
	let mut first = String::new();
	for n in 0..128 {
		let mut bytes = gif(1000);
		*bytes.last_mut().unwrap() = n;
		let reference = cache.put("image/gif", &bytes).unwrap();
		if n == 0 {
			first = reference;
		}
	}
	drop(cache);
	let reopened = AssetCache::open(tmp.path()).unwrap();
	reopened.pin(std::slice::from_ref(&first)).unwrap();
	assert!(reopened.get(&first).unwrap().is_some());
	assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 128);
}
#[tokio::test]
async fn corrupted_peer_stream_is_rejected_before_caching() {
	use tokio::io::{AsyncReadExt, AsyncWriteExt};
	let endpoint = || async {
		iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
			.alpns(vec![b"profile-test".to_vec()])
			.clear_ip_transports()
			.bind_addr("127.0.0.1:0")
			.unwrap()
			.bind()
			.await
			.unwrap()
	};
	let server = endpoint().await;
	let client = endpoint().await;
	let cache = AssetCache::temporary().unwrap();
	let source = AssetCache::temporary().unwrap();
	let bytes = gif(20000);
	let reference = source.put("image/gif", &bytes).unwrap();
	let snapshot = Snapshot {
		members: vec![Member {
			id: server.id().to_string(),
			profile: profile("Source", &reference),
			host: true,
			voice: None,
		}],
		..Default::default()
	};
	let (_sender, receiver) = tokio::sync::watch::channel(snapshot);
	let assets = serein_connect::profile_assets::ProfileAssets::new(
		cache.clone(),
		receiver,
		Some(server.id().to_string()),
	);
	let (connection, accepted) =
		tokio::join!(client.connect(server.addr(), b"profile-test"), async {
			server.accept().await.unwrap().await.unwrap()
		});
	assets.attach(connection.unwrap());
	let adversary = tokio::spawn(async move {
		let (mut send, mut recv) = accepted.accept_bi().await.unwrap();
		let length = recv.read_u16().await.unwrap();
		let mut request = vec![0; length as usize];
		recv.read_exact(&mut request).await.unwrap();
		let mut wrong = bytes;
		wrong[20] ^= 1;
		send.write_u32(wrong.len() as u32).await.unwrap();
		send.write_all(&wrong).await.unwrap();
		send.finish().unwrap();
		let _ = send.stopped().await;
	});
	assert!(
		assets
			.resolve(&reference)
			.await
			.unwrap_err()
			.to_string()
			.contains("kontrollsumma")
	);
	assert!(cache.get(&reference).unwrap().is_none());
	adversary.await.unwrap();
	client.close().await;
	server.close().await;
}
#[tokio::test]
async fn separate_stream_transfers_large_gif_via_host_once_without_snapshot_bytes() {
	let root = tempfile::tempdir().unwrap();
	let host_cache = AssetCache::open(root.path().join("host")).unwrap();
	let alice_cache = AssetCache::open(root.path().join("alice")).unwrap();
	let bob_cache = AssetCache::open(root.path().join("bob")).unwrap();
	let original = gif(1024 * 1024 + 23);
	let reference = alice_cache.put("image/gif", &original).unwrap();
	let unpublished = alice_cache.put("image/gif", &gif(90_000)).unwrap();
	let transport = TransportConfig {
		offline: true,
		..Default::default()
	};
	let host = network::host_with_assets(
		HostOptions {
			alias: Alias::parse("@assets/test").unwrap(),
			name: "Assets".into(),
			profile: profile("Host", ""),
			public: false,
			history: root.path().join("history"),
			directory_url: None,
		},
		iroh::SecretKey::generate(),
		transport.clone(),
		host_cache,
	)
	.await
	.unwrap();
	let alice = network::join_with_assets(
		host.invite.clone().unwrap(),
		iroh::SecretKey::generate(),
		profile("Alice", &reference),
		transport.clone(),
		alice_cache,
	)
	.await
	.unwrap();
	let bob = network::join_with_assets(
		host.invite.clone().unwrap(),
		iroh::SecretKey::generate(),
		profile("Bob", ""),
		transport,
		bob_cache.clone(),
	)
	.await
	.unwrap();
	tokio::time::timeout(Duration::from_secs(5), async {
		while bob.snapshots.borrow().members.len() != 3
			|| alice.snapshots.borrow().members.len() != 3
		{
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
	})
	.await
	.unwrap();
	assert!(serde_json::to_vec(&*bob.snapshots.borrow()).unwrap().len() < 4096);
	assert!(bob.assets.resolve(&unpublished).await.is_err());
	assert!(bob_cache.get(&reference).unwrap().is_none());
	let (one, two) = tokio::join!(
		bob.assets.resolve(&reference),
		bob.assets.resolve(&reference)
	);
	assert_eq!(one.unwrap(), original);
	assert_eq!(two.unwrap(), original);
	assert_eq!(bob_cache.get(&reference).unwrap().unwrap(), original);
	// The second request still works with no source connection: it is served from checked disk bytes.
	bob.assets.detach(&host.self_id);
	assert_eq!(bob.assets.resolve(&reference).await.unwrap(), original);
	bob.close().await;
	alice.close().await;
	host.close().await;
}

#[tokio::test]
#[ignore = "External relay profile integration; run explicitly with ephemeral synthetic peers"]
async fn external_relay_carries_profile_asset_without_direct_ip_transport() {
	tokio::time::timeout(Duration::from_secs(90), async {
		let root = tempfile::tempdir().unwrap();
		let host_cache = AssetCache::open(root.path().join("host")).unwrap();
		let alice_cache = AssetCache::open(root.path().join("alice")).unwrap();
		let bob_cache = AssetCache::open(root.path().join("bob")).unwrap();
		let mut original = Vec::new();
		{
			let mut encoder = image::codecs::gif::GifEncoder::new(&mut original);
			encoder
				.set_repeat(image::codecs::gif::Repeat::Infinite)
				.unwrap();
			encoder
				.encode_frames(
					[[30, 80, 140, 255], [180, 90, 20, 255]]
						.into_iter()
						.map(|color| {
							image::Frame::new(image::RgbaImage::from_pixel(
								16,
								16,
								image::Rgba(color),
							))
						}),
				)
				.unwrap();
		}
		original.resize(128 * 1024, 0);
		let reference = alice_cache.put("image/gif", &original).unwrap();
		// Every endpoint disables all direct IP transports. Only fresh test identities
		// and generated pixels leave this machine through the external relay.
		let transport = TransportConfig {
			relay_only: true,
			..Default::default()
		};
		let host = network::host_with_assets(
			HostOptions {
				alias: Alias::parse("@connect-test/profile-relay").unwrap(),
				name: "Synthetic profile relay test".into(),
				profile: profile("Relay host", ""),
				public: false,
				history: root.path().join("history"),
				directory_url: None,
			},
			iroh::SecretKey::generate(),
			transport.clone(),
			host_cache.clone(),
		)
		.await
		.unwrap();
		assert_eq!(ALPN, b"connect/chat/8");
		let invite = host.invite.clone().unwrap();
		assert!(!invite.address.addrs.is_empty());
		assert!(invite.address.addrs.iter().all(|address| !address.is_ip()));
		let encoded = invite.encode().unwrap();
		assert!(encoded.starts_with("connect://join/"));
		let alice = network::join_with_assets(
			Invite::decode(&encoded).unwrap(),
			iroh::SecretKey::generate(),
			profile("Synthetic Alice", &reference),
			transport.clone(),
			alice_cache,
		)
		.await
		.unwrap();
		let bob = network::join_with_assets(
			Invite::decode(&encoded).unwrap(),
			iroh::SecretKey::generate(),
			profile("Synthetic Bob", ""),
			transport,
			bob_cache.clone(),
		)
		.await
		.unwrap();
		tokio::time::timeout(Duration::from_secs(20), async {
			while [&host, &alice, &bob]
				.iter()
				.any(|peer| peer.snapshots.borrow().members.len() != 3)
			{
				tokio::time::sleep(Duration::from_millis(25)).await;
			}
		})
		.await
		.expect("relay profile membership");
		for peer in [&host, &alice, &bob] {
			let snapshot = peer.snapshots.borrow();
			assert!(serde_json::to_vec(&*snapshot).unwrap().len() < 4096);
			assert!(snapshot.members.iter().any(|member| {
				member
					.profile
					.card
					.as_ref()
					.is_some_and(|card| card.avatar == reference)
			}));
		}
		assert!(host_cache.get(&reference).unwrap().is_none());
		assert!(bob_cache.get(&reference).unwrap().is_none());
		let received = bob.assets.resolve(&reference).await.unwrap();
		assert_eq!(received, original);
		assert_eq!(host_cache.get(&reference).unwrap().unwrap(), original);
		assert_eq!(bob_cache.get(&reference).unwrap().unwrap(), original);
		// A second lookup succeeds from checked cached bytes with the route detached.
		bob.assets.detach(&host.self_id);
		assert_eq!(bob.assets.resolve(&reference).await.unwrap(), original);
		bob.close().await;
		alice.close().await;
		host.close().await;
	})
	.await
	.expect("external relay profile transfer within 90 seconds");
}
