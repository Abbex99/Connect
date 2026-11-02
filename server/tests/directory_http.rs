use iroh::SecretKey;
use serein_connect::{
	directory::{self, Directory},
	protocol::*,
};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn signed_name_publication_and_username_resolution_work_over_http() {
	let temp = tempfile::tempdir().unwrap();
	let state = Arc::new(Mutex::new(
		Directory::open(&temp.path().join("directory.json")).unwrap(),
	));
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let base = format!("http://{}", listener.local_addr().unwrap());
	let server = tokio::spawn(async move {
		axum::serve(listener, directory::router(state))
			.await
			.unwrap();
	});
	let key = SecretKey::generate();
	let mut registration = SignedRegistration::sign(
		Registration {
			owner: "albin".into(),
			slug: "studion".into(),
			name: "Studion".into(),
			address: key.public().into(),
			issued_at: now(),
		},
		&key,
	)
	.unwrap();
	directory::publish(&base, &registration).await.unwrap();
	let found = directory::lookup_name(&base, "@albin").await.unwrap();
	assert_eq!(found.record.address.id, key.public());
	assert_eq!(found.record.slug, "studion");
	assert!(directory::lookup_name(&base, "@missing").await.is_err());
	registration.record.name = "Forged name".into();
	assert!(directory::publish(&base, &registration).await.is_err());
	let unchanged = directory::lookup_name(&base, "@albin/studion")
		.await
		.unwrap();
	assert_eq!(unchanged.record.name, "Studion");
	let oversized = directory::client()
		.unwrap()
		.post(format!("{base}/v1/register"))
		.header("Content-Type", "application/json")
		.body(" ".repeat(20 * 1024))
		.send()
		.await
		.unwrap();
	assert_eq!(oversized.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
	server.abort();
}
