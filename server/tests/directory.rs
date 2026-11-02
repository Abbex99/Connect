use iroh::SecretKey;
use serein_connect::{directory::Directory, protocol::*};

fn claim(key: &SecretKey, owner: &str, issued_at: u64) -> SignedRegistration {
	SignedRegistration::sign(
		Registration {
			owner: owner.into(),
			slug: "spelrummet".into(),
			name: "Spelrummet".into(),
			address: key.public().into(),
			issued_at,
		},
		key,
	)
	.unwrap()
}

#[test]
fn alias_normalization_rejects_ambiguous_or_unsafe_names() {
	assert_eq!(
		Alias::parse(" @Albin/Spelrummet ").unwrap().to_string(),
		"@albin/spelrummet"
	);
	for input in [
		"../admin",
		"@albin/",
		"@albin/a/b",
		"@åke/server",
		"@a b/server",
	] {
		assert!(Alias::parse(input).is_err(), "accepted {input}");
	}
}

#[test]
fn invitation_roundtrip_and_untrusted_input_limits() {
	let key = SecretKey::from_bytes(&[7; 32]);
	let invite = Invite {
		version: 1,
		address: key.public().into(),
		token: "a".repeat(64),
		alias: "@albin/spelrummet".into(),
		name: "Spelrummet".into(),
	};
	let encoded = invite.encode().unwrap();
	assert!(encoded.starts_with("connect://join/"));
	assert!(Invite::is_link(&encoded));
	assert_eq!(
		Invite::decode(&encoded.replacen("connect://", "serein://", 1))
			.unwrap()
			.address
			.id,
		key.public()
	);
	let decoded = Invite::decode(&encoded).unwrap();
	assert_eq!(decoded.address.id, key.public());
	assert_eq!(decoded.token, "a".repeat(64));
	assert!(Invite::decode("https://evil.invalid/?invite=abc").is_err());
	assert!(Invite::decode(&format!("serein://join/{}", "a".repeat(100_000))).is_err());
	assert!(Invite::decode("serein://join/not-json").is_err());
}

#[test]
fn signatures_bind_names_addresses_and_freshness() {
	let key = SecretKey::from_bytes(&[8; 32]);
	let mut signed = claim(&key, "albin", 1000);
	assert!(signed.verify(1001).is_ok());
	assert!(signed.verify(2000).is_err());
	signed.record.name = "Imposter".into();
	assert!(signed.verify(1001).is_err());
}

#[test]
fn directory_retains_username_ownership_after_restart_and_offline_expiry() {
	let temp = tempfile::tempdir().unwrap();
	let path = temp.path().join("directory.json");
	let owner = SecretKey::from_bytes(&[1; 32]);
	let attacker = SecretKey::from_bytes(&[2; 32]);
	let mut directory = Directory::open(&path).unwrap();
	directory
		.register(claim(&owner, "albin", 1000), 1000)
		.unwrap();
	assert_eq!(
		directory
			.resolve(&Alias::parse("@albin/spelrummet").unwrap(), 1001)
			.unwrap()
			.record
			.name,
		"Spelrummet"
	);
	assert!(
		directory
			.register(claim(&attacker, "albin", 1001), 1001)
			.is_err()
	);
	drop(directory);
	let mut directory = Directory::open(&path).unwrap();
	assert!(
		directory
			.resolve(&Alias::parse("@albin/spelrummet").unwrap(), 2000)
			.is_err()
	);
	assert!(
		directory
			.register(claim(&attacker, "albin", 2000), 2000)
			.is_err()
	);
	directory
		.register(claim(&owner, "albin", 2000), 2000)
		.unwrap();
	assert!(
		directory
			.register(claim(&owner, "albin", 1999), 2000)
			.is_err()
	);
}

#[test]
fn username_lookup_only_returns_live_servers_of_that_owner() {
	let temp = tempfile::tempdir().unwrap();
	let key = SecretKey::from_bytes(&[3; 32]);
	let other = SecretKey::from_bytes(&[4; 32]);
	let mut directory = Directory::open(&temp.path().join("directory.json")).unwrap();
	directory
		.register(claim(&key, "albin", 1000), 1000)
		.unwrap();
	directory
		.register(claim(&other, "mira", 1000), 1000)
		.unwrap();
	let found = directory.active_for_owner("albin", 1001).unwrap();
	assert_eq!(found.len(), 1);
	assert_eq!(found[0].record.owner, "albin");
	assert!(
		directory
			.active_for_owner("albin", 2000)
			.unwrap()
			.is_empty()
	);
}
