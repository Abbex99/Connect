use crate::{
	protocol::{Alias, SignedRegistration, now},
	storage,
};
use anyhow::{Result, ensure};
use axum::{
	Json, Router,
	extract::{DefaultBodyLimit, Path as RoutePath, State},
	http::StatusCode,
	routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
	sync::{Arc, Mutex},
};

const MAX_STORE: usize = 32 * 1024 * 1024;
#[derive(Clone, Default, Serialize, Deserialize)]
struct Records {
	#[serde(default)]
	v2: BTreeMap<String, crate::directory_v2::Registration>,
	owners: BTreeMap<String, String>,
	servers: BTreeMap<String, SignedRegistration>,
}
pub struct Directory {
	path: PathBuf,
	records: Records,
}
impl Directory {
	pub fn active_for_owner(&self, owner: &str, now: u64) -> Result<Vec<SignedRegistration>> {
		crate::protocol::validate_slug(owner)?;
		Ok(self
			.records
			.servers
			.values()
			.filter(|r| r.record.owner == owner && r.verify(now).is_ok())
			.take(32)
			.cloned()
			.collect())
	}
	pub fn open(path: &Path) -> Result<Self> {
		let records = if path.exists() {
			storage::read_json(path, MAX_STORE as u64)?
		} else {
			Records::default()
		};
		Ok(Self {
			path: path.into(),
			records,
		})
	}
	pub fn register(&mut self, record: SignedRegistration, now: u64) -> Result<()> {
		record.verify(now)?;
		let owner = &record.record.owner;
		let public_key = record.record.address.id.to_string();
		if let Some(existing) = self.records.owners.get(owner) {
			ensure!(
				existing == &public_key,
				"Användarnamnet tillhör en annan identitet."
			);
		}
		let alias = format!("@{}/{}", owner, record.record.slug);
		ensure!(
			!self.records.v2.contains_key(&alias),
			"Namnet tillhör nu en fristående server. Uppdatera Connect."
		);
		if let Some(existing) = self.records.servers.get(&alias) {
			ensure!(
				record.record.issued_at > existing.record.issued_at,
				"Registreringen är äldre än den sparade."
			);
		}
		ensure!(
			self.records.servers.len() < 10_000 || self.records.servers.contains_key(&alias),
			"Adressboken är full."
		);
		let mut next = self.records.clone();
		next.owners.insert(owner.clone(), public_key);
		next.servers.insert(alias, record);
		storage::write_json(&self.path, &next, MAX_STORE)?;
		self.records = next;
		Ok(())
	}
	pub fn resolve(&self, alias: &Alias, now: u64) -> Result<SignedRegistration> {
		let record = self
			.records
			.servers
			.get(&alias.to_string())
			.ok_or_else(|| anyhow::anyhow!("Servernamnet finns inte i adressboken."))?;
		record.verify(now).map_err(|_| {
			anyhow::anyhow!("Servern är offline eller har inte förnyat sin adress.")
		})?;
		Ok(record.clone())
	}
	pub fn register_v2(
		&mut self,
		record: crate::directory_v2::Registration,
		time: u64,
	) -> Result<()> {
		record.verify(time)?;
		let d = &record.delegation;
		let alias = format!("@{}/{}", d.owner, d.slug);
		if let Some(owner) = self.records.owners.get(&d.owner) {
			ensure!(
				owner == &d.authority.to_string(),
				"Namnet tillhör en annan identitet."
			);
		}
		if let Some(old) = self.records.v2.get(&alias) {
			ensure!(
				record.issued_at > old.issued_at,
				"Registreringen är äldre än den sparade."
			);
			ensure!(
				old.delegation.server == d.server || d.issued_at > old.delegation.issued_at,
				"Serverfullmakten har ersatts."
			);
		}
		ensure!(
			self.records.v2.len() < 10000 || self.records.v2.contains_key(&alias),
			"Adressboken är full."
		);
		let mut next = self.records.clone();
		next.owners.insert(d.owner.clone(), d.authority.to_string());
		next.servers.remove(&alias);
		next.v2.insert(alias, record);
		storage::write_json(&self.path, &next, MAX_STORE)?;
		self.records = next;
		Ok(())
	}
	pub fn resolve_v2(
		&self,
		alias: &Alias,
		time: u64,
	) -> Result<crate::directory_v2::Registration> {
		let record = self
			.records
			.v2
			.get(&alias.to_string())
			.ok_or_else(|| anyhow::anyhow!("Servernamnet finns inte."))?;
		record.verify(time)?;
		Ok(record.clone())
	}
}

type Shared = Arc<Mutex<Directory>>;
pub fn router(directory: Shared) -> Router {
	Router::new()
		.route("/health", get(|| async { "serein-directory-v1" }))
		.route("/v1/register", post(register))
		.route("/v2/register", post(register_v2))
		.route("/v2/servers/{owner}/{slug}", get(resolve_v2))
		.route("/v1/servers/{owner}/{slug}", get(resolve))
		.route("/v1/users/{owner}", get(resolve_owner))
		.layer(DefaultBodyLimit::max(16 * 1024))
		.with_state(directory)
}
async fn register_v2(
	State(directory): State<Shared>,
	Json(record): Json<crate::directory_v2::Registration>,
) -> Result<StatusCode, (StatusCode, String)> {
	tokio::task::spawn_blocking(move || {
		directory
			.lock()
			.map_err(|_| anyhow::anyhow!("Adressboken är upptagen."))?
			.register_v2(record, now())
	})
	.await
	.map_err(|_| {
		(
			StatusCode::INTERNAL_SERVER_ERROR,
			"Adressboken kunde inte svara.".into(),
		)
	})?
	.map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
	Ok(StatusCode::NO_CONTENT)
}
async fn resolve_v2(
	State(directory): State<Shared>,
	RoutePath((owner, slug)): RoutePath<(String, String)>,
) -> Result<Json<crate::directory_v2::Registration>, (StatusCode, String)> {
	let alias = Alias::parse(&format!("@{owner}/{slug}"))
		.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
	directory
		.lock()
		.map_err(|_| {
			(
				StatusCode::INTERNAL_SERVER_ERROR,
				"Adressboken är upptagen.".into(),
			)
		})?
		.resolve_v2(&alias, now())
		.map(Json)
		.map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}
async fn resolve_owner(
	State(directory): State<Shared>,
	RoutePath(owner): RoutePath<String>,
) -> Result<Json<Vec<SignedRegistration>>, (StatusCode, String)> {
	let records = directory
		.lock()
		.map_err(|_| {
			(
				StatusCode::INTERNAL_SERVER_ERROR,
				"Adressboken är tillfälligt otillgänglig.".into(),
			)
		})?
		.active_for_owner(&owner, now())
		.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
	Ok(Json(records))
}
async fn register(
	State(directory): State<Shared>,
	Json(record): Json<SignedRegistration>,
) -> Result<StatusCode, (StatusCode, String)> {
	tokio::task::spawn_blocking(move || {
		directory
			.lock()
			.map_err(|_| {
				(
					StatusCode::INTERNAL_SERVER_ERROR,
					"Adressboken är tillfälligt otillgänglig.".into(),
				)
			})?
			.register(record, now())
			.map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
		Ok(StatusCode::NO_CONTENT)
	})
	.await
	.map_err(|_| {
		(
			StatusCode::INTERNAL_SERVER_ERROR,
			"Adressboken kunde inte svara.".into(),
		)
	})?
}
async fn resolve(
	State(directory): State<Shared>,
	RoutePath((owner, slug)): RoutePath<(String, String)>,
) -> Result<Json<SignedRegistration>, (StatusCode, String)> {
	let alias = Alias::parse(&format!("@{owner}/{slug}"))
		.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
	let record = directory
		.lock()
		.map_err(|_| {
			(
				StatusCode::INTERNAL_SERVER_ERROR,
				"Adressboken är tillfälligt otillgänglig.".into(),
			)
		})?
		.resolve(&alias, now())
		.map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
	Ok(Json(record))
}

pub fn client() -> Result<reqwest::Client> {
	let _ = rustls::crypto::ring::default_provider().install_default();
	Ok(reqwest::Client::builder()
		.timeout(std::time::Duration::from_secs(8))
		.redirect(reqwest::redirect::Policy::none())
		.build()?)
}
pub fn validate_url(value: &str) -> Result<url::Url> {
	let url = url::Url::parse(value)?;
	ensure!(
		url.username().is_empty()
			&& url.password().is_none()
			&& url.query().is_none()
			&& url.fragment().is_none(),
		"Använd en ren adress till adressboken."
	);
	let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
	ensure!(
		url.scheme() == "https" || (url.scheme() == "http" && local),
		"Adressboken måste använda HTTPS (HTTP tillåts bara lokalt)."
	);
	Ok(url)
}
pub async fn publish(base: &str, record: &SignedRegistration) -> Result<()> {
	let url = validate_url(base)?.join("/v1/register")?;
	let response = client()?.post(url).json(record).send().await?;
	ensure!(
		response.status().is_success(),
		"Adressboken avvisade registreringen ({}). Kontrollera att namnet är ditt.",
		response.status()
	);
	Ok(())
}
pub async fn lookup(base: &str, alias: &Alias) -> Result<SignedRegistration> {
	let url = validate_url(base)?.join(&format!("/v1/servers/{}/{}", alias.owner, alias.slug))?;
	let mut response = client()?.get(url).send().await?;
	ensure!(
		response.status().is_success(),
		"Servern finns inte, är offline eller adressboken är otillgänglig."
	);
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await? {
		ensure!(
			bytes.len() + chunk.len() <= 16 * 1024,
			"Adressbokens svar är för stort."
		);
		bytes.extend_from_slice(&chunk);
	}
	let record: SignedRegistration = serde_json::from_slice(&bytes)?;
	record.verify(now())?;
	ensure!(
		record.record.owner == alias.owner && record.record.slug == alias.slug,
		"Adressboken returnerade fel server."
	);
	Ok(record)
}

pub async fn lookup_name(base: &str, input: &str) -> Result<SignedRegistration> {
	if input.contains('/') {
		return lookup(base, &Alias::parse(input)?).await;
	}
	let owner = input.trim().trim_start_matches('@').to_ascii_lowercase();
	crate::protocol::validate_slug(&owner)?;
	let url = validate_url(base)?.join(&format!("/v1/users/{owner}"))?;
	let mut response = client()?.get(url).send().await?;
	ensure!(
		response.status().is_success(),
		"Användaren eller adressboken kunde inte nås."
	);
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await? {
		ensure!(
			bytes.len() + chunk.len() <= 512 * 1024,
			"Adressbokens svar är för stort."
		);
		bytes.extend_from_slice(&chunk);
	}
	let records: Vec<SignedRegistration> = serde_json::from_slice(&bytes)?;
	ensure!(!records.is_empty(), "Användaren har ingen server online.");
	ensure!(
		records.len() <= 32,
		"Adressboken returnerade för många servrar."
	);
	for record in &records {
		record.verify(now())?;
		ensure!(
			record.record.owner == owner,
			"Adressboken returnerade fel användare."
		);
	}
	if records.len() > 1 {
		anyhow::bail!(
			"Flera rum är online. Välj ett genom att ange {}",
			records
				.iter()
				.map(|r| format!("@{}/{}", owner, r.record.slug))
				.collect::<Vec<_>>()
				.join(" eller ")
		);
	}
	Ok(records.into_iter().next().expect("non-empty checked above"))
}
