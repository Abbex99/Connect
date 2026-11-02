use base64::Engine;
use serein_connect::{protocol::*, room::Room};
#[test]
fn custom_cards_sync_but_invalid_or_remote_images_never_enter_the_room() {
	let value = serde_json::json!({"name":"Ada","accent":[100,120,200],"card":{"skin":"aurora","color":"#aabbcc","avatar":"","cover":""}});
	let profile: Profile = serde_json::from_value(value.clone()).unwrap();
	let mut room = Room::new("Room".into(), "host".into(), "key", false);
	room.join("host", profile, "key").unwrap();
	assert_eq!(
		serde_json::to_value(&room.snapshot.members[0].profile).unwrap()["card"]["skin"],
		"aurora"
	);
	let mut invalid = value;
	invalid["card"]["avatar"] = serde_json::json!("https://tracker.invalid/avatar.png");
	assert!(
		room.apply(
			"host",
			Command::UpdateProfile(serde_json::from_value(invalid).unwrap()),
			1
		)
		.is_err()
	);
	assert_eq!(
		serde_json::to_value(&room.snapshot.members[0].profile).unwrap()["card"]["avatar"],
		""
	);
	room.apply(
		"host",
		Command::SendMessage {
			channel: "allmant".into(),
			text: "Hej".into(),
		},
		2,
	)
	.unwrap();
	assert!(
		serde_json::to_value(&room.snapshot.messages[0].author)
			.unwrap()
			.get("card")
			.is_none(),
		"do not duplicate profile images into each history message"
	);
}

#[test]
fn card_images_are_bounded_and_profiles_fit_without_using_the_history_budget() {
	let mut profile:Profile=serde_json::from_value(serde_json::json!({"name":"Image","accent":[1,2,3],"card":{"skin":"portrait","color":"#112233","avatar":"","cover":""}})).unwrap();
	let mut bytes = std::io::Cursor::new(Vec::new());
	image::DynamicImage::new_rgba8(2, 2)
		.write_to(&mut bytes, image::ImageFormat::Png)
		.unwrap();
	let image = bytes.into_inner();
	let mut padded = image.clone();
	padded.resize((4096 - 22) / 4 * 3, 0);
	profile.card.as_mut().unwrap().avatar = format!(
		"data:image/png;base64,{}",
		base64::engine::general_purpose::STANDARD.encode(padded)
	);
	let mut cover = image;
	cover.resize((12288 - 22) / 4 * 3, 0);
	profile.card.as_mut().unwrap().cover = format!(
		"data:image/png;base64,{}",
		base64::engine::general_purpose::STANDARD.encode(cover)
	);
	assert!(profile.validate().is_ok());
	let mut room = Room::new("Room".into(), "0".into(), "key", false);
	for n in 0..64 {
		room.join(&n.to_string(), profile.clone(), "key").unwrap();
	}
	room.apply(
		"0",
		Command::SendMessage {
			channel: "allmant".into(),
			text: "still works".into(),
		},
		1,
	)
	.unwrap();
	assert_eq!(room.snapshot.members.len(), 64);
	assert_eq!(room.snapshot.messages.len(), 1);
	let card = profile.card.as_mut().unwrap();
	card.avatar = "data:image/png;base64,".to_owned() + &"A".repeat(4096);
	assert!(profile.validate().is_err());
	profile.card.as_mut().unwrap().avatar = "data:image/png;base64,AAAA".into();
	assert!(profile.validate().is_err());
}

#[test]
fn profile_gif_is_validated_without_flattening_animation() {
	let gif = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
	let mut profile: Profile = serde_json::from_value(
		serde_json::json!({"name":"GIF","accent":[1,2,3],"card":{"avatar":gif,"cover":gif}}),
	)
	.unwrap();
	assert!(profile.validate().is_ok());
	profile.card.as_mut().unwrap().avatar = "data:image/gif;base64,AAAA".into();
	assert!(profile.validate().is_err());
}

#[test]
fn animated_profiles_preserve_all_64_members_and_room_history() {
	let gif = include_bytes!("../web/tests/fixtures/profile-loop.gif");
	let uri = |limit| {
		let mut data = gif.to_vec();
		data.resize((limit - 22) / 4 * 3, 0);
		format!(
			"data:image/gif;base64,{}",
			base64::engine::general_purpose::STANDARD.encode(data)
		)
	};
	let profile = Profile {
		name: "Animated".into(),
		accent: [1, 2, 3],
		card: Some(ProfileCard {
			avatar: uri(serein_connect::profile_media::AVATAR_ANIMATION_LIMIT),
			cover: uri(serein_connect::profile_media::COVER_ANIMATION_LIMIT),
			..Default::default()
		}),
	};
	profile.validate().unwrap();
	let mut room = Room::new("Room".into(), "0".into(), "key", false);
	for id in 0..64 {
		room.join(&id.to_string(), profile.clone(), "key").unwrap();
	}
	for time in 0..11 {
		room.apply(
			"0",
			Command::SendMessage {
				channel: "allmant".into(),
				text: "x".repeat(4000),
			},
			time,
		)
		.unwrap();
	}
	assert_eq!(room.snapshot.members.len(), 64);
	assert_eq!(room.snapshot.messages.len(), 11);
	assert!(serde_json::to_vec(&room.snapshot).unwrap().len() < MAX_SNAPSHOT);
	assert!(
		serde_json::to_vec(&ServerFrame::Snapshot(room.snapshot))
			.unwrap()
			.len() < MAX_FRAME
	);
}
#[test]
fn mp4_profile_video_requires_a_real_bounded_video_track() {
	let bytes = include_bytes!("../web/tests/fixtures/profile-loop.mp4");
	let video = format!(
		"data:video/mp4;base64,{}",
		base64::engine::general_purpose::STANDARD.encode(bytes)
	);
	let mut card = ProfileCard {
		avatar: video.clone(),
		cover: video,
		..Default::default()
	};
	card.validate().unwrap();
	for invalid in [
		"data:video/mp4;base64,AAAA".into(),
		"data:image/gif;base64,".to_owned()
			+ &base64::engine::general_purpose::STANDARD.encode(bytes),
		"data:image/svg+xml;base64,PHN2Zy8+".into(),
		"https://example.com/video.mp4".into(),
		"data:video/mp4;base64,".to_owned() + &"A".repeat(16384),
	] {
		card.avatar = invalid;
		assert!(card.validate().is_err());
	}
}

#[test]
fn profile_video_dimensions_duration_and_truncated_boxes_are_rejected() {
	let original = include_bytes!("../web/tests/fixtures/profile-loop.mp4");
	let validate = |bytes: &[u8]| {
		let uri = format!(
			"data:video/mp4;base64,{}",
			base64::engine::general_purpose::STANDARD.encode(bytes)
		);
		serein_connect::profile_media::validate_media(&uri, 4096, 16384, 512, Some(10.0))
	};
	let mut oversized = original.to_vec();
	let track = oversized
		.windows(4)
		.position(|part| part == b"tkhd")
		.unwrap();
	let length = u32::from_be_bytes(oversized[track - 4..track].try_into().unwrap()) as usize;
	let width = track - 4 + length - 8;
	oversized[width..width + 4].copy_from_slice(&(513u32 << 16).to_be_bytes());
	assert!(validate(&oversized).is_err());
	let mut long = original.to_vec();
	let movie = long.windows(4).position(|part| part == b"mvhd").unwrap();
	assert_eq!(long[movie + 4], 1);
	let scale = u32::from_be_bytes(long[movie + 24..movie + 28].try_into().unwrap());
	long[movie + 28..movie + 36].copy_from_slice(&(u64::from(scale) * 11).to_be_bytes());
	assert!(validate(&long).is_err());
	assert!(validate(&original[..original.len() - 4]).is_err());
}
