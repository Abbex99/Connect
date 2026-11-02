use axum::{
	body::Body,
	http::{Request, StatusCode},
};
use serein_connect::{network::TransportConfig, server, server_admin};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn admin_http_rejects_foreign_hosts_and_unauthenticated_mutations() {
	let root = tempfile::tempdir().unwrap();
	server::initialize(
		root.path(),
		server::ServerConfig::local("@test/admin", "Admin").unwrap(),
	)
	.unwrap();
	let running = server::start(
		root.path(),
		TransportConfig {
			offline: true,
			..Default::default()
		},
	)
	.await
	.unwrap();
	let state = Arc::new(
		server_admin::WebState::new(running.core.clone(), "http://127.0.0.1:4748").unwrap(),
	);
	let app = server_admin::router(state);
	for (host, origin) in [
		("evil.test", "http://evil.test"),
		("127.0.0.1:4748", "http://evil.test"),
	] {
		let response = app
			.clone()
			.oneshot(
				Request::builder()
					.method("POST")
					.uri("/api/request")
					.header("host", host)
					.header("origin", origin)
					.header("content-type", "application/json")
					.body(Body::from("\"Status\""))
					.unwrap(),
			)
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::FORBIDDEN);
	}
	let response = app
		.oneshot(
			Request::builder()
				.method("POST")
				.uri("/api/request")
				.header("host", "127.0.0.1:4748")
				.header("origin", "http://127.0.0.1:4748")
				.header("content-type", "application/json")
				.body(Body::from("\"Status\""))
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
	running.close().await;
}

#[tokio::test]
async fn browser_requires_approval_and_rechecks_authority_after_recovery() {
	use axum::body::to_bytes;
	use serein_connect::{
		protocol::{Profile, now},
		server_control::Request as Control,
	};
	let root = tempfile::tempdir().unwrap();
	let setup = server::initialize(
		root.path(),
		server::ServerConfig::local("@test/panel", "Panel").unwrap(),
	)
	.unwrap();
	let running = server::start(
		root.path(),
		TransportConfig {
			offline: true,
			..Default::default()
		},
	)
	.await
	.unwrap();
	running
		.core
		.store
		.admit(
			"owner",
			Profile {
				name: "Owner".into(),
				accent: [1, 2, 3],
				card: None,
			},
			Some(&setup.pairing_token),
			now(),
		)
		.unwrap();
	let app = server_admin::router(Arc::new(
		server_admin::WebState::new(running.core.clone(), "http://127.0.0.1:4748").unwrap(),
	));
	let req = |path: &str, cookie: &str, csrf: &str, body: &str| {
		Request::builder()
			.method("POST")
			.uri(path)
			.header("host", "127.0.0.1:4748")
			.header("origin", "http://127.0.0.1:4748")
			.header("cookie", cookie)
			.header("x-connect-csrf", csrf)
			.header("content-type", "application/json")
			.body(Body::from(body.to_owned()))
			.unwrap()
	};
	let response = app
		.clone()
		.oneshot(req("/api/pair/start", "", "", ""))
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::OK);
	let pending = response.headers()["set-cookie"]
		.to_str()
		.unwrap()
		.split(';')
		.next()
		.unwrap()
		.to_owned();
	let value: serde_json::Value =
		serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
	let csrf = value["csrf"].as_str().unwrap();
	assert_eq!(
		app.clone()
			.oneshot(req("/api/pair/finish", &pending, csrf, ""))
			.await
			.unwrap()
			.status(),
		StatusCode::ACCEPTED
	);
	running
		.core
		.execute(
			"owner",
			Control::ApproveBrowser {
				code: value["code"].as_str().unwrap().into(),
			},
		)
		.unwrap();
	let response = app
		.clone()
		.oneshot(req("/api/pair/finish", &pending, csrf, ""))
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::OK);
	let cookie = response.headers()["set-cookie"]
		.to_str()
		.unwrap()
		.split(';')
		.next()
		.unwrap()
		.to_owned();
	let value: serde_json::Value =
		serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
	let csrf = value["csrf"].as_str().unwrap();
	assert_eq!(
		app.clone()
			.oneshot(req("/api/request", &cookie, csrf, "\"Status\""))
			.await
			.unwrap()
			.status(),
		StatusCode::OK
	);
	running.core.store.reset_pairing(60, now()).unwrap();
	assert_ne!(
		app.oneshot(req("/api/request", &cookie, csrf, "\"Status\""))
			.await
			.unwrap()
			.status(),
		StatusCode::OK
	);
	running.close().await;
}

#[tokio::test]
async fn stalled_http_body_is_closed_without_blocking_another_request() {
	use std::time::Duration;
	use tokio::io::{AsyncReadExt, AsyncWriteExt};
	let root = tempfile::tempdir().unwrap();
	server::initialize(
		root.path(),
		server::ServerConfig::local("@test/slow", "Slow").unwrap(),
	)
	.unwrap();
	let running = server::start(
		root.path(),
		TransportConfig {
			offline: true,
			..Default::default()
		},
	)
	.await
	.unwrap();
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	let origin = format!("http://{address}");
	let app = server_admin::router(Arc::new(
		server_admin::WebState::new(running.core.clone(), &origin).unwrap(),
	));
	let (stop, stopped) = tokio::sync::oneshot::channel();
	let task = tokio::spawn(server_admin::serve(listener, app, async {
		let _ = stopped.await;
	}));
	let mut slow = tokio::net::TcpStream::connect(address).await.unwrap();
	slow.write_all(format!("POST /api/request HTTP/1.1\r\nHost: {address}\r\nOrigin: {origin}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{").as_bytes()).await.unwrap();
	let mut healthy = tokio::net::TcpStream::connect(address).await.unwrap();
	healthy
		.write_all(
			format!("GET / HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n").as_bytes(),
		)
		.await
		.unwrap();
	let mut response = Vec::new();
	tokio::time::timeout(Duration::from_secs(5), healthy.read_to_end(&mut response))
		.await
		.unwrap()
		.unwrap();
	assert!(response.starts_with(b"HTTP/1.1 200"));
	let mut body = Vec::new();
	let _ = tokio::time::timeout(Duration::from_secs(25), slow.read_to_end(&mut body))
		.await
		.expect("stalled connection must close");
	stop.send(()).unwrap();
	task.await.unwrap().unwrap();
	running.close().await;
}
