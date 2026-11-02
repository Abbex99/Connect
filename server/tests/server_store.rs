use serein_connect::{
	protocol::{ChannelKind, Profile},
	server_store::{AdminAction, AdminResult, Role, ServerStore},
};
#[test]
fn history_does_not_duplicate_profile_media() {
	let (_dir, store) = setup();
	let mut p = profile("Owner");
	p.card = Some(serein_connect::protocol::ProfileCard::default());
	store.admit("owner", p, None, 103).unwrap();
	let message = store
		.send_message("owner", "allmant", "hello", "card", 104)
		.unwrap();
	assert!(message.author.card.is_none());
	assert!(
		store
			.history("owner", "allmant", None, 100)
			.unwrap()
			.messages[0]
			.author
			.card
			.is_none()
	);
}
fn profile(name: &str) -> Profile {
	Profile {
		name: name.into(),
		accent: [1, 2, 3],
		card: None,
	}
}
fn setup() -> (tempfile::TempDir, ServerStore) {
	let dir = tempfile::tempdir().unwrap();
	let store = ServerStore::open(&dir.path().join("server.db"), "Test").unwrap();
	let token = store.reset_pairing(60, 100).unwrap();
	store.pair("owner", &token, 101).unwrap();
	store.admit("owner", profile("Owner"), None, 102).unwrap();
	(dir, store)
}
fn invite(s: &ServerStore) -> String {
	match s
		.admin(
			"owner",
			AdminAction::CreateInvite {
				expires_at: 10000,
				max_uses: 20,
			},
			103,
		)
		.unwrap()
	{
		AdminResult::Invite { token } => token,
		_ => panic!(),
	}
}
#[test]
fn setup_one_use_expiry_and_owner_independent() {
	let d = tempfile::tempdir().unwrap();
	let s = ServerStore::open(&d.path().join("s.db"), "Test").unwrap();
	let t = s.reset_pairing(10, 100).unwrap();
	assert!(s.pair("attacker", &t, 110).is_err());
	let t = s.reset_pairing(10, 110).unwrap();
	assert_eq!(s.pair("owner", &t, 111).unwrap(), Role::Owner);
	assert!(s.pair("other", &t, 112).is_err());
	assert!(s.role("endpoint").is_err());
	assert!(s.snapshot().unwrap().members.is_empty());
}
#[test]
fn roster_and_history_survive_reopen_and_paginate() {
	let (d, s) = setup();
	let token = invite(&s);
	s.admit("member", profile("Member"), Some(&token), 104)
		.unwrap();
	for i in 0..550 {
		s.send_message(
			"member",
			"allmant",
			&format!("message {i}"),
			&i.to_string(),
			105 + i,
		)
		.unwrap();
	}
	drop(s);
	let s = ServerStore::open(&d.path().join("server.db"), "Ignored").unwrap();
	assert_eq!(s.role("member").unwrap(), Role::Member);
	assert_eq!(s.role("owner").unwrap(), Role::Owner);
	assert!(s.snapshot().unwrap().members.is_empty());
	assert!(s.snapshot().unwrap().messages.len() <= 100);
	let mut before = None;
	let mut ids = std::collections::HashSet::new();
	loop {
		let p = s.history("member", "allmant", before, 100).unwrap();
		assert!(p.messages.len() <= 100);
		for m in p.messages {
			assert!(ids.insert(m.id));
		}
		before = p.next_before;
		if before.is_none() {
			break;
		}
	}
	assert_eq!(ids.len(), 550);
}
#[test]
fn sender_dedup_and_revocation() {
	let (_d, s) = setup();
	let token = invite(&s);
	s.admit("member", profile("Member"), Some(&token), 104)
		.unwrap();
	let a = s
		.send_message("owner", "allmant", "hello", "same", 105)
		.unwrap();
	assert_eq!(
		a.id,
		s.send_message("owner", "allmant", "hello", "same", 106)
			.unwrap()
			.id
	);
	assert!(
		s.send_message("owner", "allmant", "changed", "same", 106)
			.is_err()
	);
	let b = s
		.send_message("member", "allmant", "hello", "same", 106)
		.unwrap();
	assert_ne!(a.id, b.id);
	assert!(
		s.admin(
			"member",
			AdminAction::CreateChannel {
				name: "bad".into(),
				kind: ChannelKind::Text
			},
			107
		)
		.is_err()
	);
	s.admin(
		"owner",
		AdminAction::RemoveMember {
			peer: "member".into(),
		},
		108,
	)
	.unwrap();
	assert!(s.history("member", "allmant", None, 20).is_err());
	assert!(s.admit("member", profile("Member"), None, 109).is_err());
	assert_eq!(
		s.history("owner", "allmant", None, 20)
			.unwrap()
			.messages
			.len(),
		2
	);
}
#[test]
fn low_disk_quota_rejects_before_acknowledgment() {
	let (_d, s) = setup();
	s.admin(
		"owner",
		AdminAction::SetStorage {
			max_bytes: 65536,
			retention_days: None,
		},
		104,
	)
	.unwrap();
	let body = "x".repeat(4096);
	let mut accepted = 0;
	for i in 0..1000 {
		if s.send_message("owner", "allmant", &body, &i.to_string(), 105)
			.is_err()
		{
			break;
		}
		accepted += 1;
	}
	assert!(accepted < 1000);
	assert!(accepted > 0);
	let mut before = None;
	let mut count = 0;
	loop {
		let p = s.history("owner", "allmant", before, 100).unwrap();
		count += p.messages.len();
		before = p.next_before;
		if before.is_none() {
			break;
		}
	}
	assert_eq!(accepted, count);
}
#[test]
fn invite_limits_expiry_and_revocation() {
	let (_d, s) = setup();
	let make = |expiry, uses| match s
		.admin(
			"owner",
			AdminAction::CreateInvite {
				expires_at: expiry,
				max_uses: uses,
			},
			104,
		)
		.unwrap()
	{
		AdminResult::Invite { token } => token,
		_ => panic!(),
	};
	let t = make(110, 1);
	assert!(s.admit("late", profile("Late"), Some(&t), 110).is_err());
	let t = make(1000, 1);
	s.admit("first", profile("First"), Some(&t), 105).unwrap();
	assert!(s.admit("second", profile("Second"), Some(&t), 106).is_err());
	let t = make(1000, 3);
	let id = s
		.list_invites("owner")
		.unwrap()
		.into_iter()
		.find(|i| i.max_uses == 3)
		.unwrap()
		.id;
	s.admin("owner", AdminAction::RevokeInvite { id }, 106)
		.unwrap();
	assert!(s.admit("second", profile("Second"), Some(&t), 107).is_err());
}
#[test]
fn deleted_message_retry_does_not_resurrect_and_bans_persist() {
	let (d, s) = setup();
	let t = invite(&s);
	s.admit("member", profile("Member"), Some(&t), 104).unwrap();
	let m = s
		.send_message("member", "allmant", "secret", "req", 105)
		.unwrap();
	s.admin("owner", AdminAction::DeleteMessage { id: m.id }, 106)
		.unwrap();
	assert!(
		s.send_message("member", "allmant", "secret", "req", 107)
			.is_err()
	);
	assert!(
		s.history("owner", "allmant", None, 100)
			.unwrap()
			.messages
			.is_empty()
	);
	s.admin(
		"owner",
		AdminAction::BanMember {
			peer: "member".into(),
		},
		108,
	)
	.unwrap();
	drop(s);
	let s = ServerStore::open(&d.path().join("server.db"), "Test").unwrap();
	assert!(s.admit("member", profile("Member"), Some(&t), 109).is_err());
	assert!(s.role("member").is_err());
	assert!(
		s.admin(
			"owner",
			AdminAction::RemoveMember {
				peer: "owner".into()
			},
			110
		)
		.is_err()
	);
	assert!(
		s.admin(
			"owner",
			AdminAction::SetRole {
				peer: "owner".into(),
				role: Role::Member
			},
			110
		)
		.is_err()
	);
}
#[test]
fn setup_admit_atomic_backup_and_channel_administration() {
	let d = tempfile::tempdir().unwrap();
	let s = ServerStore::open(&d.path().join("s.db"), "Test").unwrap();
	let t = s.reset_pairing(60, 100).unwrap();
	assert_eq!(
		s.admit("owner", profile("Owner"), Some(&t), 101).unwrap(),
		Role::Owner
	);
	assert!(s.admit("evil", profile("Evil"), Some(&t), 102).is_err());
	let ch = match s
		.admin(
			"owner",
			AdminAction::CreateChannel {
				name: "Other".into(),
				kind: ChannelKind::Text,
			},
			103,
		)
		.unwrap()
	{
		AdminResult::Channel(ch) => ch,
		_ => panic!(),
	};
	s.send_message("owner", &ch.id, "old", "req", 104).unwrap();
	s.admin(
		"owner",
		AdminAction::RenameChannel {
			channel: ch.id.clone(),
			name: "Renamed".into(),
		},
		105,
	)
	.unwrap();
	s.admin(
		"owner",
		AdminAction::ReorderChannels {
			channels: vec![ch.id.clone(), "allmant".into(), "samtal".into()],
		},
		106,
	)
	.unwrap();
	assert_eq!(s.snapshot().unwrap().channels[0].name, "Renamed");
	s.admin(
		"owner",
		AdminAction::DeleteChannel {
			channel: ch.id.clone(),
			delete_history: false,
		},
		107,
	)
	.unwrap();
	assert!(s.history("owner", &ch.id, None, 20).is_err());
	assert!(s.snapshot().unwrap().messages.is_empty());
	let target = d.path().join("backup.db");
	s.export_database(&target).unwrap();
	let backup = ServerStore::open(&target, "Ignored").unwrap();
	assert_eq!(backup.role("owner").unwrap(), Role::Owner);
	assert!(s.export_database(&target).is_err());
}

#[test]
fn public_admission_preserves_ban_and_existing_member_can_pair() {
	let (_d, s) = setup();
	s.admit_public("public", profile("Public"), 105).unwrap();
	s.admin(
		"owner",
		AdminAction::BanMember {
			peer: "public".into(),
		},
		106,
	)
	.unwrap();
	assert!(s.admit_public("public", profile("Public"), 107).is_err());
	s.admit_public("next", profile("Next"), 108).unwrap();
	let t = s.reset_pairing(60, 109).unwrap();
	assert_eq!(
		s.admit("next", profile("Next"), Some(&t), 110).unwrap(),
		Role::Owner
	);
	assert_eq!(s.role("owner").unwrap(), Role::Member);
}

#[test]
fn voice_history_is_rejected_and_retention_requires_explicit_policy() {
	let (_d, s) = setup();
	assert!(s.history("owner", "samtal", None, 20).is_err());
	assert!(s.send_message("owner", "samtal", "bad", "v", 105).is_err());
	s.send_message("owner", "allmant", "old", "old", 105)
		.unwrap();
	s.maintain(200000).unwrap();
	assert_eq!(
		s.history("owner", "allmant", None, 20)
			.unwrap()
			.messages
			.len(),
		1
	);
	s.admin(
		"owner",
		AdminAction::SetStorage {
			max_bytes: 1024 * 1024,
			retention_days: Some(1),
		},
		200001,
	)
	.unwrap();
	s.maintain(200002).unwrap();
	assert!(
		s.history("owner", "allmant", None, 20)
			.unwrap()
			.messages
			.is_empty()
	);
	assert!(
		s.send_message("owner", "allmant", "old", "old", 200003)
			.is_err()
	);
}
#[test]
fn expired_invites_reclaim_capacity_and_audit_is_bounded() {
	let (_d, s) = setup();
	for _ in 0..1000 {
		s.admin(
			"owner",
			AdminAction::CreateInvite {
				expires_at: 105,
				max_uses: 1,
			},
			104,
		)
		.unwrap();
	}
	s.admin(
		"owner",
		AdminAction::CreateInvite {
			expires_at: 110,
			max_uses: 1,
		},
		106,
	)
	.unwrap();
	assert_eq!(s.list_invites("owner").unwrap().len(), 1);
	let mut before = None;
	let mut total = 0;
	loop {
		let page = s.audit("owner", before, 100).unwrap();
		if page.is_empty() {
			break;
		}
		total += page.len();
		before = page.last().map(|a| a.id);
	}
	assert_eq!(total, 1000);
}
#[test]
fn full_quota_can_be_increased_without_losing_acknowledged_data() {
	let (_d, s) = setup();
	s.admin(
		"owner",
		AdminAction::SetStorage {
			max_bytes: 65536,
			retention_days: None,
		},
		104,
	)
	.unwrap();
	let body = "x".repeat(4096);
	let mut i = 0;
	while s
		.send_message("owner", "allmant", &body, &i.to_string(), 105)
		.is_ok()
	{
		i += 1;
	}
	s.admin(
		"owner",
		AdminAction::SetStorage {
			max_bytes: 1024 * 1024,
			retention_days: None,
		},
		106,
	)
	.unwrap();
	s.send_message("owner", "allmant", &body, &i.to_string(), 107)
		.unwrap();
	assert_eq!(
		s.history("owner", "allmant", None, 100)
			.unwrap()
			.messages
			.len(),
		i + 1
	);
}
#[test]
fn concurrent_pairing_has_exactly_one_owner_and_revoked_admin_loses_all_access() {
	let (_dir, store) = setup();
	let token = invite(&store);
	store
		.admit("admin", profile("Admin"), Some(&token), 104)
		.unwrap();
	store
		.admin(
			"owner",
			AdminAction::SetRole {
				peer: "admin".into(),
				role: Role::Admin,
			},
			105,
		)
		.unwrap();
	assert!(store.settings("admin").is_ok());
	store
		.admin(
			"owner",
			AdminAction::SetRole {
				peer: "admin".into(),
				role: Role::Member,
			},
			106,
		)
		.unwrap();
	assert!(store.settings("admin").is_err());
	assert!(store.list_members("admin").is_err());
	assert!(store.list_invites("admin").is_err());
	assert!(store.audit("admin", None, 100).is_err());
	let token = store.reset_pairing(60, 107).unwrap();
	let store = std::sync::Arc::new(store);
	let handles: Vec<_> = (0..8)
		.map(|i| {
			let store = store.clone();
			let token = token.clone();
			std::thread::spawn(move || store.pair(&format!("peer{i}"), &token, 108).is_ok())
		})
		.collect();
	assert_eq!(
		handles
			.into_iter()
			.map(|h| h.join().unwrap())
			.filter(|ok| *ok)
			.count(),
		1
	);
	assert_eq!(store.snapshot().unwrap().members.len(), 0);
}
#[test]
fn removing_banned_member_does_not_implicitly_unban() {
	let (_dir, store) = setup();
	store
		.admit_public("blocked", profile("Blocked"), 104)
		.unwrap();
	store
		.admin(
			"owner",
			AdminAction::BanMember {
				peer: "blocked".into(),
			},
			105,
		)
		.unwrap();
	store
		.admin(
			"owner",
			AdminAction::RemoveMember {
				peer: "blocked".into(),
			},
			106,
		)
		.unwrap();
	assert!(
		store
			.admit_public("blocked", profile("Blocked"), 107)
			.is_err()
	);
	store
		.admin(
			"owner",
			AdminAction::UnbanMember {
				peer: "blocked".into(),
			},
			108,
		)
		.unwrap();
	assert_eq!(
		store
			.admit_public("blocked", profile("Blocked"), 109)
			.unwrap(),
		Role::Member
	);
}
