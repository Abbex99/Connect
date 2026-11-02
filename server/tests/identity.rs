#[cfg(target_os = "windows")]
#[test]
#[ignore = "Uses Windows Credential Manager with a new random synthetic test entry"]
fn identity_and_invitation_survive_reload_in_windows_credential_storage() {
	use serein_connect::identity;
	let profile = format!("test-{}", iroh::SecretKey::generate().public());
	struct Cleanup(String);
	impl Drop for Cleanup {
		fn drop(&mut self) {
			if let Ok(entry) = keyring::Entry::new("Serein Connect identity", &self.0) {
				let _ = entry.delete_credential();
			}
			if let Ok(entry) = keyring::Entry::new(
				"Serein Connect invitation",
				&format!("{}:@test/room", self.0),
			) {
				let _ = entry.delete_credential();
			}
		}
	}
	let _cleanup = Cleanup(profile.clone());
	let first = identity::load_or_create(&profile).unwrap();
	let second = identity::load_or_create(&profile).unwrap();
	assert_eq!(first.public(), second.public());
	identity::save_invite(&profile, "@test/room", "synthetic invite payload").unwrap();
	assert_eq!(
		identity::load_invite(&profile, "@test/room").unwrap(),
		"synthetic invite payload"
	);
}
