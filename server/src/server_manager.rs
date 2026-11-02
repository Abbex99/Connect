//! Local server setup and administration, separate from network member identities.
use crate::{
	network::TransportConfig,
	server::{self, RunningServer, ServerConfig},
	server_admin, server_control,
};
use anyhow::{Context, Result, ensure};
use axum::{
	Json, Router,
	extract::{DefaultBodyLimit, Request, State},
	http::{HeaderMap, HeaderValue, Method, StatusCode},
	middleware::{self, Next},
	response::{Html, IntoResponse, Response},
	routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
	collections::HashMap,
	path::PathBuf,
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};

fn random() -> String {
	iroh::SecretKey::generate()
		.to_bytes()
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect()
}
struct Session {
	csrf: String,
	created: Instant,
}
struct Manager {
	root: PathBuf,
	host: String,
	origin: String,
	secure: bool,
	failed_logins: Mutex<(Instant, u32)>,
	key: String,
	offline: bool,
	sessions: Mutex<HashMap<String, Session>>,
	running: tokio::sync::Mutex<Option<RunningServer>>,
	operation: Arc<tokio::sync::Mutex<()>>,
	jobs: Mutex<HashMap<String, Option<std::result::Result<Value, String>>>>,
	startup_error: Mutex<Option<String>>,
}
impl Manager {
	fn new(root: PathBuf, host: String, key: String, offline: bool) -> Self {
		Self {
			root,
			origin: format!("http://{host}"),
			secure: false,
			failed_logins: Mutex::new((Instant::now(), 0)),
			host,
			key,
			offline,
			sessions: Mutex::new(HashMap::new()),
			running: tokio::sync::Mutex::new(None),
			operation: Arc::new(tokio::sync::Mutex::new(())),
			jobs: Mutex::new(HashMap::new()),
			startup_error: Mutex::new(None),
		}
	}
	fn remote(root: PathBuf, origin: &str, key: String, offline: bool) -> Result<Self> {
		let parsed = url::Url::parse(origin)?;
		ensure!(
			parsed.scheme() == "https"
				&& parsed.host_str().is_some()
				&& parsed.username().is_empty()
				&& parsed.password().is_none()
				&& parsed.path() == "/"
				&& parsed.query().is_none()
				&& parsed.fragment().is_none(),
			"Fjärrpanelen kräver en exakt HTTPS-adress utan sökväg eller inloggningsuppgifter."
		);
		ensure!(
			key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
			"Administrationsnyckeln ska innehålla 64 hexadecimala tecken."
		);
		let host = parsed[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
		let mut state = Self::new(root, host, key, offline);
		state.origin = parsed.origin().ascii_serialization();
		state.secure = true;
		Ok(state)
	}
	async fn start(&self) -> Result<()> {
		let mut running = self.running.lock().await;
		if running.is_none() {
			*running = Some(
				server::start(
					&self.root,
					TransportConfig {
						offline: self.offline,
						..Default::default()
					},
				)
				.await?,
			);
		}
		*self.startup_error.lock().unwrap() = None;
		Ok(())
	}
	async fn stop(&self) {
		if let Some(running) = self.running.lock().await.take() {
			running.close().await;
		}
	}
	fn cookie_name(&self) -> String {
		format!(
			"connect_operator_{}",
			self.host.replace(['.', ':', '[', ']'], "_")
		)
	}
	fn token<'a>(&self, headers: &'a HeaderMap) -> Option<&'a str> {
		let prefix = format!("{}=", self.cookie_name());
		headers
			.get("cookie")
			.and_then(|s| s.to_str().ok())
			.unwrap_or("")
			.split(';')
			.find_map(|s| s.trim().strip_prefix(&prefix))
	}
	fn authenticate(&self, headers: &HeaderMap) -> Result<()> {
		let token = self.token(headers);
		let csrf = headers
			.get("x-connect-csrf")
			.and_then(|s| s.to_str().ok())
			.unwrap_or("");
		let mut sessions = self.sessions.lock().unwrap();
		sessions.retain(|_, s| s.created.elapsed() < Duration::from_secs(43200));
		ensure!(
			token
				.and_then(|t| sessions.get(t))
				.is_some_and(|s| s.csrf == csrf),
			"Öppna panelen via startfilen eller ange administrationsnyckeln igen."
		);
		Ok(())
	}
	async fn status(&self) -> Result<Value> {
		let config = if self.root.join("server.json").exists() {
			Some(ServerConfig::load(&self.root)?)
		} else {
			None
		};
		let running = self.running.lock().await;
		Ok(
			json!({"configured":config.is_some(),"running":running.is_some(),"config":config,"data":self.root,"error":*self.startup_error.lock().unwrap()}),
		)
	}
}
type HttpError = (StatusCode, String);
fn bad(e: impl std::fmt::Display) -> HttpError {
	(StatusCode::BAD_REQUEST, e.to_string())
}
fn auth(state: &Manager, headers: &HeaderMap) -> Result<(), HttpError> {
	state
		.authenticate(headers)
		.map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))
}
fn router(state: Arc<Manager>) -> Router {
	Router::new()
		.route(
			"/",
			get(|| async { Html(include_str!("../server-ui/manager.html")) }),
		)
		.route(
			"/manager.js",
			get(|| async {
				(
					[("content-type", "text/javascript")],
					include_str!("../server-ui/manager.js"),
				)
			}),
		)
		.route(
			"/app.js",
			get(|| async {
				(
					[("content-type", "text/javascript")],
					include_str!("../server-ui/app.js"),
				)
			}),
		)
		.route(
			"/style.css",
			get(|| async {
				(
					[("content-type", "text/css")],
					include_str!("../server-ui/style.css"),
				)
			}),
		)
		.route("/local/login", post(login))
		.route("/local/action", post(action))
		.route("/local/control", post(control))
		.layer(DefaultBodyLimit::max(64 * 1024))
		.layer(middleware::from_fn_with_state(state.clone(), guard))
		.with_state(state)
}
async fn guard(State(state): State<Arc<Manager>>, request: Request, next: Next) -> Response {
	let h = request.headers();
	let mut response = if h.get("host").and_then(|v| v.to_str().ok()) != Some(state.host.as_str())
		|| (*request.method() != Method::GET
			&& h.get("origin").and_then(|v| v.to_str().ok()) != Some(state.origin.as_str()))
	{
		(StatusCode::FORBIDDEN, "Otillåtet ursprung.").into_response()
	} else {
		next.run(request).await
	};
	for (k, v) in [
		("cache-control", "no-store"),
		("x-content-type-options", "nosniff"),
		("referrer-policy", "no-referrer"),
		("x-frame-options", "DENY"),
		(
			"content-security-policy",
			"default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
		),
	] {
		response
			.headers_mut()
			.insert(k, HeaderValue::from_static(v));
	}
	response
}
#[derive(Deserialize)]
struct Login {
	key: String,
}
async fn login(
	State(state): State<Arc<Manager>>,
	headers: HeaderMap,
	Json(login): Json<Login>,
) -> Result<Response, HttpError> {
	{
		let mut failed = state.failed_logins.lock().unwrap();
		if failed.0.elapsed() >= Duration::from_secs(60) {
			*failed = (Instant::now(), 0);
		}
		if failed.1 >= 20 {
			return Err((
				StatusCode::TOO_MANY_REQUESTS,
				"För många försök. Vänta en minut.".into(),
			));
		}
		if login.key != state.key {
			failed.1 += 1;
		}
	}
	if login.key != state.key {
		return Err((
			StatusCode::UNAUTHORIZED,
			"Fel administrationsnyckel. Använd nyckeln från serverinstallationen.".into(),
		));
	}
	let mut sessions = state.sessions.lock().unwrap();
	sessions.retain(|_, s| s.created.elapsed() < Duration::from_secs(43200));
	if let Some(session) = state.token(&headers).and_then(|t| sessions.get(t)) {
		return Ok(Json(json!({"csrf":session.csrf})).into_response());
	}
	if sessions.len() >= 128 {
		return Err((
			StatusCode::TOO_MANY_REQUESTS,
			"För många öppna paneler.".into(),
		));
	}
	let token = random();
	let csrf = random();
	sessions.insert(
		token.clone(),
		Session {
			csrf: csrf.clone(),
			created: Instant::now(),
		},
	);
	let mut response = Json(json!({"csrf":csrf})).into_response();
	response.headers_mut().insert(
		"set-cookie",
		HeaderValue::from_str(&format!(
			"{}={token}; Path=/local; HttpOnly; SameSite=Strict; Max-Age=43200{}",
			state.cookie_name(),
			if state.secure { "; Secure" } else { "" }
		))
		.unwrap(),
	);
	Ok(response)
}
async fn control(
	State(state): State<Arc<Manager>>,
	headers: HeaderMap,
	Json(request): Json<server_control::Request>,
) -> Result<Json<server_control::Response>, HttpError> {
	auth(&state, &headers)?;
	let _operation = state
		.operation
		.try_lock()
		.map_err(|_| bad("Servern arbetar. Vänta tills åtgärden är klar."))?;
	let core = state
		.running
		.lock()
		.await
		.as_ref()
		.ok_or_else(|| bad("Starta servern först."))?
		.core
		.clone();
	let result = tokio::task::spawn_blocking(move || core.execute_local(request))
		.await
		.map_err(bad)?
		.map_err(bad)?;
	Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
	Status,
	Job { id: String },
	Setup { name: String, alias: String },
	Start,
	Stop,
	Configure { public: bool },
	RecoveryKey,
	OwnerInvite,
	Backup { recipient: String },
	Restore { backup: PathBuf, key_file: PathBuf },
}
async fn action(
	State(state): State<Arc<Manager>>,
	headers: HeaderMap,
	Json(action): Json<Action>,
) -> Result<Json<Value>, HttpError> {
	auth(&state, &headers)?;
	if let Action::Job { id } = action {
		let jobs = state.jobs.lock().unwrap();
		return match jobs.get(&id) {
			Some(None) => Ok(Json(json!({"job":id}))),
			Some(Some(Ok(result))) => Ok(Json(result.clone())),
			Some(Some(Err(error))) => Err(bad(error)),
			None => Err(bad("Åtgärden finns inte längre. Uppdatera panelen.")),
		};
	}
	if matches!(action, Action::Status) {
		return Ok(Json(state.status().await.map_err(bad)?));
	}
	if matches!(action, Action::RecoveryKey) {
		let (private, public) = crate::server_backup::generate_recovery_key();
		return Ok(Json(json!({"private":private,"public":public})));
	}
	let guard = state
		.operation
		.clone()
		.try_lock_owned()
		.map_err(|_| bad("Servern arbetar. Vänta tills åtgärden är klar."))?;
	let id = random();
	{
		let mut jobs = state.jobs.lock().unwrap();
		if jobs.len() >= 16 {
			jobs.retain(|_, v| v.is_none());
		}
		jobs.insert(id.clone(), None);
	}
	let job_id = id.clone();
	tokio::spawn(async move {
		let _guard = guard;
		let result = perform_action(state.clone(), action)
			.await
			.map(|Json(v)| v)
			.map_err(|(_, e)| e);
		state.jobs.lock().unwrap().insert(job_id, Some(result));
	});
	Ok(Json(json!({"job":id})))
}
async fn perform_action(state: Arc<Manager>, action: Action) -> Result<Json<Value>, HttpError> {
	match action {
		Action::Status | Action::Job { .. } => {}
		Action::Setup { name, alias } => {
			let root = state.root.clone();
			tokio::task::spawn_blocking(move || {
				server::initialize(&root, ServerConfig::local(alias.trim(), name.trim())?)
			})
			.await
			.map_err(bad)?
			.map_err(bad)?;
			if let Err(e) = state.start().await {
				*state.startup_error.lock().unwrap() = Some(e.to_string());
			}
		}
		Action::Start => {
			state.start().await.map_err(bad)?;
		}
		Action::Stop => state.stop().await,
		Action::Configure { public } => {
			if state.running.lock().await.is_some() {
				return Err(bad("Stoppa servern innan du ändrar åtkomsten."));
			}
			let root = state.root.clone();
			tokio::task::spawn_blocking(move || -> Result<()> {
				let _lock = server::lock_directory(&root)?;
				let mut config = ServerConfig::load(&root)?;
				config.public = public;
				crate::storage::write_json(&root.join("server.json"), &config, 16384)
			})
			.await
			.map_err(bad)?
			.map_err(bad)?;
		}
		Action::RecoveryKey => {
			let (private, public) = crate::server_backup::generate_recovery_key();
			return Ok(Json(json!({"private":private,"public":public})));
		}
		Action::OwnerInvite => {
			let core = state
				.running
				.lock()
				.await
				.as_ref()
				.ok_or_else(|| bad("Starta servern först."))?
				.core
				.clone();
			let invite = tokio::task::spawn_blocking(move || core.first_owner_invite())
				.await
				.map_err(bad)?
				.map_err(bad)?;
			return Ok(Json(json!({"invite":invite})));
		}
		Action::Backup { recipient } => {
			let root = state.root.clone();
			let path = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
				let backups = root.with_extension("backups");
				std::fs::create_dir_all(&backups)?;
				server::protect_directory(&backups)?;
				let path = backups.join(format!(
					"Connect-{}-{}.age",
					crate::protocol::now(),
					&random()[..8]
				));
				crate::server_backup::export(&root, &path, &recipient)?;
				Ok(path)
			})
			.await
			.map_err(bad)?
			.map_err(bad)?;
			return Ok(Json(json!({"path":path})));
		}
		Action::Restore { backup, key_file } => {
			if state.running.lock().await.is_some() {
				return Err(bad("Stoppa servern först."));
			}
			let root = state.root.clone();
			tokio::task::spawn_blocking(move || -> Result<()> {
				ensure!(
					std::fs::metadata(&key_file)?.len() < 4096,
					"Ogiltig nyckelfil."
				);
				let key = std::fs::read_to_string(key_file)?;
				crate::server_backup::restore(&backup, &root, key.trim())
			})
			.await
			.map_err(bad)?
			.map_err(bad)?;
			state.start().await.map_err(bad)?;
		}
	}
	Ok(Json(state.status().await.map_err(bad)?))
}

#[derive(Serialize, Deserialize)]
struct Launch {
	host: String,
	key: String,
}
fn open_browser(launch: &Launch) -> Result<()> {
	let address: std::net::SocketAddr = launch.host.parse()?;
	ensure!(
		address.ip().is_loopback(),
		"Paneladressen måste vara lokal."
	);
	ensure!(
		launch.key.len() == 64 && launch.key.bytes().all(|b| b.is_ascii_hexdigit()),
		"Ogiltig lokal panelnyckel."
	);
	let url = format!("http://{}/#{}", launch.host, launch.key);
	#[cfg(windows)]
	{
		use std::os::windows::process::CommandExt;
		let status = std::process::Command::new("rundll32.exe")
			.args(["url.dll,FileProtocolHandler", &url])
			.creation_flags(0x08000000)
			.status()?;
		ensure!(status.success(), "Webbläsaren kunde inte öppnas.");
	}
	#[cfg(not(windows))]
	{
		std::process::Command::new(if cfg!(target_os = "macos") {
			"open"
		} else {
			"xdg-open"
		})
		.arg(url)
		.spawn()?;
	}
	Ok(())
}
/// Local GUI entry point. The launch file is protected by the operator's OS account.
pub async fn run(root: PathBuf, open: bool, offline: bool) -> Result<()> {
	run_with(root, open, offline, None).await
}
/// The external HTTPS proxy must preserve Host; never publish this backend HTTP port.
pub async fn run_remote(
	root: PathBuf,
	bind: String,
	origin: String,
	key_file: PathBuf,
	offline: bool,
) -> Result<()> {
	let metadata = std::fs::symlink_metadata(&key_file)?;
	ensure!(
		metadata.is_file() && metadata.len() <= 128,
		"Ogiltig nyckelfil."
	);
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		ensure!(
			metadata.permissions().mode() & 0o077 == 0,
			"Nyckelfilen måste vara privat (chmod 600)."
		);
	}
	let key = std::fs::read_to_string(key_file)?.trim().to_owned();
	let state = Manager::remote(std::path::absolute(&root)?, &origin, key, offline)?;
	let bind: std::net::SocketAddr = bind.parse().context("Ogiltig lyssningsadress.")?;
	run_with(root, false, offline, Some((bind, state))).await
}
async fn shutdown_signal() {
	#[cfg(unix)]
	{
		if let Ok(mut signal) =
			tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
		{
			tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=signal.recv()=>{} }
			return;
		}
	}
	let _ = tokio::signal::ctrl_c().await;
}
async fn run_with(
	root: PathBuf,
	open: bool,
	offline: bool,
	remote: Option<(std::net::SocketAddr, Manager)>,
) -> Result<()> {
	let root = std::path::absolute(root)?;
	let panel = root.with_extension("panel");
	std::fs::create_dir_all(&panel)?;
	server::protect_directory(&panel)?;
	let launch_path = panel.join("launch.json");
	let lock = match server::lock_directory(&panel) {
		Ok(lock) => lock,
		Err(e) => {
			if open {
				let launch: Launch = crate::storage::read_json(&launch_path, 4096)
					.context("Panelen startar redan. Försök igen om några sekunder.")?;
				open_browser(&launch)?;
				return Ok(());
			}
			return Err(e);
		}
	};
	let (listener, state, launch) = if let Some((bind, state)) = remote {
		let listener = tokio::net::TcpListener::bind(bind)
			.await
			.context("Panelporten kunde inte öppnas.")?;
		(listener, Arc::new(state), None)
	} else {
		let listener = match tokio::net::TcpListener::bind("127.0.0.1:4749").await {
			Ok(listener) => listener,
			Err(_) => tokio::net::TcpListener::bind("127.0.0.1:0").await?,
		};
		let launch = Launch {
			host: listener.local_addr()?.to_string(),
			key: random(),
		};
		crate::storage::write_json(&launch_path, &launch, 4096)?;
		let state = Arc::new(Manager::new(
			root,
			launch.host.clone(),
			launch.key.clone(),
			offline,
		));
		(listener, state, Some(launch))
	};
	if state.root.join("server.json").exists()
		&& let Err(e) = state.start().await
	{
		*state.startup_error.lock().unwrap() = Some(e.to_string());
	}
	println!(
		"Connect Server · panel {}\nServern fortsätter köra när webbläsaren stängs. Avsluta med Ctrl+C.",
		state.origin
	);
	if open && let Some(launch) = launch {
		open_browser(&launch)?;
	}
	let result = server_admin::serve(listener, router(state.clone()), shutdown_signal()).await;
	let _guard = state.operation.lock().await;
	state.stop().await;
	let _ = std::fs::remove_file(launch_path);
	drop(lock);
	result?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use axum::{
		body::{Body, to_bytes},
		http::Request,
	};
	use tower::ServiceExt;

	#[tokio::test]
	async fn remote_manager_requires_https_exact_origin_and_secure_session() {
		let tmp = tempfile::tempdir().unwrap();
		let key = "a".repeat(64);
		for origin in [
			"http://192.168.1.20:4750",
			"https://user:pass@server.test",
			"https://server.test/path",
			"https://server.test/?key=x",
		] {
			assert!(Manager::remote(tmp.path().into(), origin, key.clone(), true).is_err());
		}
		let state = Arc::new(
			Manager::remote(
				tmp.path().into(),
				"https://192.168.1.20:4750",
				key.clone(),
				true,
			)
			.unwrap(),
		);
		let app = router(state);
		let send = |host: &str, origin: &str, key: &str| {
			Request::builder()
				.method("POST")
				.uri("/local/login")
				.header("host", host)
				.header("origin", origin)
				.header("content-type", "application/json")
				.body(Body::from(json!({"key":key}).to_string()))
				.unwrap()
		};
		assert_eq!(
			app.clone()
				.oneshot(send("192.168.1.20:4750", "https://evil.test", &key))
				.await
				.unwrap()
				.status(),
			StatusCode::FORBIDDEN
		);
		assert_eq!(
			app.clone()
				.oneshot(send("evil.test", "https://192.168.1.20:4750", &key))
				.await
				.unwrap()
				.status(),
			StatusCode::FORBIDDEN
		);
		let response = app
			.clone()
			.oneshot(send("192.168.1.20:4750", "https://192.168.1.20:4750", &key))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		assert!(
			response.headers()["set-cookie"]
				.to_str()
				.unwrap()
				.contains("; Secure")
		);
		for _ in 0..20 {
			assert_eq!(
				app.clone()
					.oneshot(send(
						"192.168.1.20:4750",
						"https://192.168.1.20:4750",
						"wrong"
					))
					.await
					.unwrap()
					.status(),
				StatusCode::UNAUTHORIZED
			);
		}
		assert_eq!(
			app.oneshot(send(
				"192.168.1.20:4750",
				"https://192.168.1.20:4750",
				"wrong"
			))
			.await
			.unwrap()
			.status(),
			StatusCode::TOO_MANY_REQUESTS
		);
	}

	async fn raw_call(
		app: &Router,
		path: &str,
		value: serde_json::Value,
		cookie: &str,
		csrf: &str,
		origin: &str,
	) -> (StatusCode, HeaderMap, serde_json::Value) {
		let response = app
			.clone()
			.oneshot(
				Request::builder()
					.method("POST")
					.uri(path)
					.header("host", "127.0.0.1:4749")
					.header("origin", origin)
					.header("content-type", "application/json")
					.header("cookie", cookie)
					.header("x-connect-csrf", csrf)
					.body(Body::from(value.to_string()))
					.unwrap(),
			)
			.await
			.unwrap();
		let status = response.status();
		let headers = response.headers().clone();
		let body = to_bytes(response.into_body(), 1048576).await.unwrap();
		(
			status,
			headers,
			serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
		)
	}

	async fn call(
		app: &Router,
		path: &str,
		value: Value,
		cookie: &str,
		csrf: &str,
		origin: &str,
	) -> (StatusCode, HeaderMap, Value) {
		let mut result = raw_call(app, path, value, cookie, csrf, origin).await;
		for _ in 0..1000 {
			let Some(id) = result.2.get("job").and_then(Value::as_str) else {
				return result;
			};
			tokio::time::sleep(Duration::from_millis(10)).await;
			result = raw_call(
				app,
				"/local/action",
				json!({"action":"job","id":id}),
				cookie,
				csrf,
				origin,
			)
			.await;
		}
		panic!("Job did not finish");
	}

	#[tokio::test]
	async fn local_setup_auth_and_restart_preserve_members() {
		let tmp = tempfile::tempdir().unwrap();
		let state = Arc::new(Manager::new(
			tmp.path().join("data"),
			"127.0.0.1:4749".into(),
			"secret".into(),
			true,
		));
		let app = router(state.clone());
		let origin = "http://127.0.0.1:4749";
		assert_eq!(
			call(
				&app,
				"/local/action",
				serde_json::json!({"action":"status"}),
				"",
				"",
				origin
			)
			.await
			.0,
			StatusCode::UNAUTHORIZED
		);
		assert_eq!(
			call(
				&app,
				"/local/login",
				serde_json::json!({"key":"secret"}),
				"",
				"",
				"http://evil.test"
			)
			.await
			.0,
			StatusCode::FORBIDDEN
		);
		assert_eq!(
			call(
				&app,
				"/local/login",
				serde_json::json!({"key":"wrong"}),
				"",
				"",
				origin
			)
			.await
			.0,
			StatusCode::UNAUTHORIZED
		);
		let (status, headers, login) = call(
			&app,
			"/local/login",
			serde_json::json!({"key":"secret"}),
			"",
			"",
			origin,
		)
		.await;
		assert_eq!(status, StatusCode::OK);
		let cookie = headers["set-cookie"]
			.to_str()
			.unwrap()
			.split(';')
			.next()
			.unwrap();
		let csrf = login["csrf"].as_str().unwrap();
		let reopened = call(
			&app,
			"/local/login",
			json!({"key":"secret"}),
			cookie,
			"",
			origin,
		)
		.await;
		assert_eq!(reopened.2["csrf"], csrf);
		assert!(!reopened.1.contains_key("set-cookie"));
		let other = Manager::new(
			tmp.path().join("other"),
			"127.0.0.1:4750".into(),
			"other".into(),
			true,
		);
		assert_ne!(state.cookie_name(), other.cookie_name());

		assert_eq!(
			call(
				&app,
				"/local/action",
				serde_json::json!({"action":"setup","name":"Test","alias":"@local/test"}),
				cookie,
				"wrong",
				origin
			)
			.await
			.0,
			StatusCode::UNAUTHORIZED
		);
		assert_eq!(
			call(
				&app,
				"/local/action",
				serde_json::json!({"action":"setup","name":"Test","alias":"@local/test"}),
				cookie,
				csrf,
				origin
			)
			.await
			.0,
			StatusCode::OK
		);
		// The response can disappear while the independently owned job retains serialization.
		let blocked = state.running.lock().await;
		let queued = raw_call(
			&app,
			"/local/action",
			json!({"action":"start"}),
			cookie,
			csrf,
			origin,
		)
		.await;
		let job = queued.2["job"].as_str().unwrap().to_owned();
		drop(queued);
		assert_eq!(
			raw_call(
				&app,
				"/local/action",
				json!({"action":"stop"}),
				cookie,
				csrf,
				origin
			)
			.await
			.0,
			StatusCode::BAD_REQUEST
		);
		drop(blocked);
		assert_eq!(
			call(
				&app,
				"/local/action",
				json!({"action":"job","id":job}),
				cookie,
				csrf,
				origin
			)
			.await
			.0,
			StatusCode::OK
		);
		let core = state.running.lock().await.as_ref().unwrap().core.clone();
		let owner_link = core.first_owner_invite().unwrap();
		let token = crate::protocol::Invite::decode(&owner_link).unwrap().token;
		core.store
			.admit(
				"owner",
				crate::protocol::Profile {
					name: "Original owner".into(),
					accent: [1, 2, 3],
					card: None,
				},
				Some(&token),
				crate::protocol::now(),
			)
			.unwrap();
		assert!(core.first_owner_invite().is_err());
		let id = core.server_id.clone();
		assert!(
			core.execute("local-operator", crate::server_control::Request::Status)
				.is_err()
		);
		let result = call(
			&app,
			"/local/control",
			serde_json::json!({"Admin":{"CreateChannel":{"name":"Test channel","kind":"text"}}}),
			cookie,
			csrf,
			origin,
		)
		.await;
		assert_eq!(result.0, StatusCode::OK);
		assert_eq!(
			core.store.role("owner").unwrap(),
			crate::server_store::Role::Owner
		);
		assert_eq!(
			call(
				&app,
				"/local/action",
				serde_json::json!({"action":"setup","name":"Overwrite","alias":"@bad/test"}),
				cookie,
				csrf,
				origin
			)
			.await
			.0,
			StatusCode::BAD_REQUEST
		);
		assert_eq!(
			call(
				&app,
				"/local/action",
				serde_json::json!({"action":"stop"}),
				cookie,
				csrf,
				origin
			)
			.await
			.0,
			StatusCode::OK
		);
		assert_eq!(
			call(
				&app,
				"/local/action",
				serde_json::json!({"action":"start"}),
				cookie,
				csrf,
				origin
			)
			.await
			.0,
			StatusCode::OK
		);
		let guard = state.running.lock().await;
		let restarted = guard.as_ref().unwrap();
		assert_eq!(restarted.core.server_id, id);
		assert_eq!(
			restarted.core.store.role("owner").unwrap(),
			crate::server_store::Role::Owner
		);
		assert!(
			restarted
				.snapshot()
				.channels
				.iter()
				.any(|c| c.name == "Test channel")
		);
		drop(guard);
		state.stop().await;
	}
}
