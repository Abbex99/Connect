//! Browser administration uses one-use approval and HttpOnly cookies, never URL credentials.
use crate::{
	server::Core,
	server_control::{Request as ControlRequest, Response as ControlResponse},
	server_store::Role,
};
use anyhow::{Result, ensure};
use axum::{
	Json, Router,
	extract::{DefaultBodyLimit, Request, State},
	http::{HeaderMap, HeaderValue, StatusCode},
	middleware::{self, Next},
	response::{Html, IntoResponse, Response},
	routing::{get, post},
};

use std::{
	collections::HashMap,
	sync::Arc,
	time::{Duration, Instant},
};

fn random() -> String {
	iroh::SecretKey::generate()
		.to_bytes()
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect()
}
struct Pending {
	code: String,
	csrf: String,
	owner: Option<String>,
	created: Instant,
}
struct Session {
	peer: String,
	csrf: String,
	created: Instant,
}
#[derive(Default)]
pub struct BrowserSessions {
	pending: HashMap<String, Pending>,
	sessions: HashMap<String, Session>,
}
impl BrowserSessions {
	fn prune(&mut self) {
		self.pending
			.retain(|_, p| p.created.elapsed() < Duration::from_secs(120));
		self.sessions
			.retain(|_, s| s.created.elapsed() < Duration::from_secs(900));
	}
	fn begin(&mut self) -> Result<(String, String, String)> {
		self.prune();
		ensure!(self.pending.len() < 128, "För många parkopplingsförsök.");
		let token = random();
		let code = random()[..12].to_uppercase();
		let csrf = random();
		self.pending.insert(
			token.clone(),
			Pending {
				code: code.clone(),
				csrf: csrf.clone(),
				owner: None,
				created: Instant::now(),
			},
		);
		Ok((token, code, csrf))
	}
	pub(crate) fn approve(&mut self, code: &str, peer: &str) -> Result<()> {
		self.prune();
		let pending = self
			.pending
			.values_mut()
			.find(|p| p.code == code.trim().to_ascii_uppercase())
			.ok_or_else(|| anyhow::anyhow!("Parkopplingskoden finns inte eller har gått ut."))?;
		ensure!(pending.owner.is_none(), "Koden har redan godkänts.");
		pending.owner = Some(peer.into());
		Ok(())
	}
	fn finish(&mut self, token: &str, csrf: &str) -> Result<Option<(String, String)>> {
		self.prune();
		ensure!(self.sessions.len() < 128, "För många panelsessioner.");
		let pending = self
			.pending
			.get(token)
			.ok_or_else(|| anyhow::anyhow!("Parkopplingen har gått ut."))?;
		ensure!(pending.csrf == csrf, "Ogiltig begäran.");
		let Some(peer) = pending.owner.clone() else {
			return Ok(None);
		};
		let session = random();
		let csrf = random();
		self.pending.remove(token);
		self.sessions.insert(
			session.clone(),
			Session {
				peer,
				csrf: csrf.clone(),
				created: Instant::now(),
			},
		);
		Ok(Some((session, csrf)))
	}
	fn authenticated(&mut self, token: &str, csrf: &str) -> Result<String> {
		self.prune();
		let session = self
			.sessions
			.get(token)
			.ok_or_else(|| anyhow::anyhow!("Godkänn panelen från Connect först."))?;
		ensure!(session.csrf == csrf, "Ogiltig begäran.");
		Ok(session.peer.clone())
	}
}
pub struct WebState {
	core: Arc<Core>,
	origin: String,
	host: String,
	secure: bool,
}
impl WebState {
	pub fn new(core: Arc<Core>, origin: &str) -> Result<Self> {
		let parsed = url::Url::parse(origin)?;
		let host = parsed
			.host_str()
			.ok_or_else(|| anyhow::anyhow!("Paneladressen saknar värd."))?;
		let local = host == "127.0.0.1" || host == "localhost" || host == "[::1]";
		ensure!(
			parsed.scheme() == "https" || (parsed.scheme() == "http" && local),
			"Fjärrpanelen måste använda HTTPS."
		);
		ensure!(
			parsed.username().is_empty()
				&& parsed.password().is_none()
				&& parsed.path() == "/"
				&& parsed.query().is_none()
				&& parsed.fragment().is_none(),
			"Ange panelens ursprung utan sökväg eller hemligheter."
		);
		let authority = match parsed.port() {
			Some(port) => format!("{host}:{port}"),
			None => host.into(),
		};
		Ok(Self {
			core,
			origin: origin.trim_end_matches('/').into(),
			host: authority,
			secure: parsed.scheme() == "https",
		})
	}
}
type HttpError = (StatusCode, String);
fn bad(error: impl std::fmt::Display) -> HttpError {
	(StatusCode::BAD_REQUEST, error.to_string())
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
	headers
		.get("cookie")?
		.to_str()
		.ok()?
		.split(';')
		.find_map(|part| {
			part.trim()
				.strip_prefix(&format!("{name}="))
				.map(str::to_owned)
		})
}
fn csrf(headers: &HeaderMap) -> &str {
	headers
		.get("x-connect-csrf")
		.and_then(|v| v.to_str().ok())
		.unwrap_or("")
}
fn set_cookie(response: &mut Response, name: &str, value: &str, secure: bool, age: u32) {
	let header = format!(
		"{name}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={age}{}",
		if secure { "; Secure" } else { "" }
	);
	response
		.headers_mut()
		.append("set-cookie", HeaderValue::from_str(&header).unwrap());
}
pub fn router(state: Arc<WebState>) -> Router {
	Router::new()
		.route(
			"/",
			get(|| async { Html(include_str!("../server-ui/index.html")) }),
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
		.route("/api/pair/start", post(begin))
		.route("/api/pair/finish", post(finish))
		.route("/api/request", post(request))
		.route("/api/logout", post(logout))
		.layer(DefaultBodyLimit::max(64 * 1024))
		.layer(middleware::from_fn_with_state(state.clone(), guard))
		.with_state(state)
}

/// Bound accepted sockets, headers and complete requests, including unauthenticated bodies.
pub async fn serve(
	listener: tokio::net::TcpListener,
	app: Router,
	shutdown: impl std::future::Future<Output = ()>,
) -> std::io::Result<()> {
	let mut tasks = tokio::task::JoinSet::new();
	tokio::pin!(shutdown);
	loop {
		tokio::select! {
		 biased;
		 _=&mut shutdown=>break,
		 _=tasks.join_next(),if !tasks.is_empty()=>{},
		 accepted=listener.accept()=>{let(stream,_)=accepted?;if tasks.len()>=64{drop(stream);continue;}let app=app.clone();tasks.spawn(async move{
		  let io=hyper_util::rt::TokioIo::new(stream);
		  let mut builder=hyper::server::conn::http1::Builder::new();
		  builder.timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(Duration::from_secs(5)).keep_alive(false).max_buf_size(32*1024);
		  let connection=builder.serve_connection(io,hyper_util::service::TowerToHyperService::new(app));
		  let _=tokio::time::timeout(Duration::from_secs(15),connection).await;
		 });}
		}
	}
	tasks.abort_all();
	while tasks.join_next().await.is_some() {}
	Ok(())
}
async fn guard(State(state): State<Arc<WebState>>, request: Request, next: Next) -> Response {
	let headers = request.headers();
	let host = headers
		.get("host")
		.and_then(|v| v.to_str().ok())
		.unwrap_or("");
	let mut response = if host != state.host
		|| (*request.method() != axum::http::Method::GET
			&& headers.get("origin").and_then(|v| v.to_str().ok()) != Some(state.origin.as_str()))
	{
		(StatusCode::FORBIDDEN, "Otillåtet ursprung.").into_response()
	} else {
		next.run(request).await
	};
	for (key, value) in [
		("cache-control", "no-store"),
		("x-content-type-options", "nosniff"),
		("referrer-policy", "no-referrer"),
		("x-frame-options", "DENY"),
		(
			"content-security-policy",
			"default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
		),
	] {
		response
			.headers_mut()
			.insert(key, HeaderValue::from_static(value));
	}
	response
}
async fn begin(State(state): State<Arc<WebState>>) -> Result<Response, HttpError> {
	let (token, code, csrf) = state.core.browser.lock().unwrap().begin().map_err(bad)?;
	let mut response = Json(serde_json::json!({"code":code,"csrf":csrf})).into_response();
	set_cookie(&mut response, "connect_pending", &token, state.secure, 120);
	Ok(response)
}
async fn finish(
	State(state): State<Arc<WebState>>,
	headers: HeaderMap,
) -> Result<Response, HttpError> {
	let token = cookie(&headers, "connect_pending").ok_or((
		StatusCode::UNAUTHORIZED,
		"Starta parkopplingen igen.".into(),
	))?;
	let result = state
		.core
		.browser
		.lock()
		.unwrap()
		.finish(&token, csrf(&headers))
		.map_err(bad)?;
	let Some((session, csrf)) = result else {
		return Ok(StatusCode::ACCEPTED.into_response());
	};
	let mut response = Json(serde_json::json!({"csrf":csrf})).into_response();
	set_cookie(
		&mut response,
		"connect_session",
		&session,
		state.secure,
		900,
	);
	set_cookie(&mut response, "connect_pending", "", state.secure, 0);
	Ok(response)
}
async fn request(
	State(state): State<Arc<WebState>>,
	headers: HeaderMap,
	Json(request): Json<ControlRequest>,
) -> Result<Json<ControlResponse>, HttpError> {
	let token = cookie(&headers, "connect_session").ok_or((
		StatusCode::UNAUTHORIZED,
		"Godkänn panelen från Connect först.".into(),
	))?;
	let peer = state
		.core
		.browser
		.lock()
		.unwrap()
		.authenticated(&token, csrf(&headers))
		.map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
	let core = state.core.clone();
	let response = tokio::task::spawn_blocking(move || {
		ensure!(
			core.store.role(&peer)? != Role::Member,
			"Administratörsbehörigheten har återkallats."
		);
		core.execute(&peer, request)
	})
	.await
	.map_err(bad)?
	.map_err(bad)?;
	Ok(Json(response))
}
async fn logout(
	State(state): State<Arc<WebState>>,
	headers: HeaderMap,
) -> Result<Response, HttpError> {
	if let Some(token) = cookie(&headers, "connect_session") {
		let mut sessions = state.core.browser.lock().unwrap();
		sessions
			.authenticated(&token, csrf(&headers))
			.map_err(bad)?;
		sessions.sessions.remove(&token);
	}
	let mut response = StatusCode::NO_CONTENT.into_response();
	set_cookie(&mut response, "connect_session", "", state.secure, 0);
	Ok(response)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn approvals_are_single_use_and_sessions_expire() {
		let mut b = BrowserSessions::default();
		let (token, code, csrf) = b.begin().unwrap();
		assert!(b.finish(&token, "bad").is_err());
		assert!(b.finish(&token, &csrf).unwrap().is_none());
		b.approve(&code, "owner").unwrap();
		assert!(b.approve(&code, "other").is_err());
		let (s, c) = b.finish(&token, &csrf).unwrap().unwrap();
		assert!(b.finish(&token, &csrf).is_err());
		assert_eq!(b.authenticated(&s, &c).unwrap(), "owner");
		b.sessions.get_mut(&s).unwrap().created = Instant::now() - Duration::from_secs(901);
		assert!(b.authenticated(&s, &c).is_err());
	}
}
