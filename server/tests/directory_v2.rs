use iroh::{EndpointAddr, SecretKey};
use serein_connect::{
	directory::Directory,
	directory_v2::{Delegation, Registration},
	protocol::{Alias, now},
};
#[test]
fn separate_server_key_requires_namespace_authority_and_rejects_replays() {
	let root = tempfile::tempdir().unwrap();
	let mut directory = Directory::open(&root.path().join("names.json")).unwrap();
	let owner = SecretKey::generate();
	let server = SecretKey::generate();
	let alias = Alias::parse("@alice/lounge").unwrap();
	let d = Delegation::sign(&alias, server.public(), &owner).unwrap();
	let registration = Registration::sign(
		d,
		"Lounge".into(),
		EndpointAddr::new(server.public()),
		&server,
	)
	.unwrap();
	directory.register_v2(registration.clone(), now()).unwrap();
	assert!(directory.register_v2(registration, now()).is_err());
	let attacker = SecretKey::generate();
	let forged = Registration::sign(
		Delegation::sign(&alias, attacker.public(), &attacker).unwrap(),
		"Fake".into(),
		EndpointAddr::new(attacker.public()),
		&attacker,
	)
	.unwrap();
	assert!(directory.register_v2(forged, now()).is_err());
	let saved = directory.resolve_v2(&alias, now()).unwrap();
	assert_eq!(saved.invite().address.id, server.public());
	assert_eq!(saved.invite().version, 2);
	assert_ne!(owner.public(), server.public());
}

#[tokio::test]
async fn chunked_directory_response_is_rejected_before_eof() {
	use tokio::io::{AsyncReadExt, AsyncWriteExt};
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let (done, wait) = tokio::sync::oneshot::channel();
	let task = tokio::spawn(async move {
		let (mut stream, _) = listener.accept().await.unwrap();
		let mut request = [0u8; 2048];
		let _ = stream.read(&mut request).await;
		stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n").await.unwrap();
		for _ in 0..5 {
			if stream.write_all(b"2000\r\n").await.is_err() {
				break;
			}
			if stream.write_all(&vec![b'x'; 8192]).await.is_err() {
				break;
			}
			if stream.write_all(b"\r\n").await.is_err() {
				break;
			}
		}
		let _ = wait.await;
	});
	let result =
		serein_connect::directory_v2::lookup(&format!("http://{address}"), "@test/room").await;
	let _ = done.send(());
	task.await.unwrap();
	assert!(result.unwrap_err().to_string().contains("för stort"));
}
