//! Authenticated control operations. The transport determines the caller identity.
use crate::{
	network::{self, TransportConfig},
	protocol::*,
	server_store::*,
};
use anyhow::{Result, ensure};
use iroh::{Endpoint, SecretKey};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const ALPN: &[u8] = b"connect/control/1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
	Status,
	History {
		channel: String,
		before: Option<u64>,
		limit: usize,
	},
	SendMessage {
		channel: String,
		text: String,
		request_id: String,
	},
	Admin(AdminAction),
	ApproveBrowser {
		code: String,
	},
	RegisterName(crate::directory_v2::Publication),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
	pub version: String,
	pub directory_status: String,
	#[serde(default)]
	pub invitation_base: String,
	#[serde(default)]
	pub server_link: String,
	#[serde(default)]
	pub access_mode: String,
	pub active_sent_bytes: u64,
	pub active_received_bytes: u64,
	pub server_id: String,
	pub name: String,
	pub uptime_seconds: u64,
	pub online: usize,
	pub storage: StorageSettings,
	pub channels: Vec<Channel>,
	pub members: Vec<StoredMember>,
	pub invitations: Vec<StoredInvite>,
	pub audit: Vec<AuditEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
	Status(Status),
	History(HistoryPage),
	Message(Message),
	Admin(AdminResult),
	Invite(String),
	Ok,
	Error(String),
}

#[derive(Clone)]
pub struct Client {
	pub(crate) endpoint: Endpoint,
	pub invite: Invite,
}
impl Client {
	pub async fn request(&self, request: Request) -> Result<Response> {
		let result = tokio::time::timeout(Duration::from_secs(15), async {
			let connection = self
				.endpoint
				.connect(self.invite.address.clone(), ALPN)
				.await?;
			let result = async {
				let (mut send, mut recv) = connection.open_bi().await?;
				network::write_frame(&mut send, &request).await?;
				let response: Response = network::read_frame(&mut recv).await?;
				if let Response::Error(error) = response {
					anyhow::bail!(error);
				}
				Ok::<_, anyhow::Error>(response)
			}
			.await;
			connection.close(0u32.into(), b"control completed");
			result
		})
		.await;
		result.map_err(|_| anyhow::anyhow!("Servern svarade inte i tid."))?
	}
}
pub async fn request(
	invite: &Invite,
	key: SecretKey,
	transport: TransportConfig,
	request: Request,
) -> Result<Response> {
	ensure!(
		invite.version == 2,
		"Serverpanelen kräver en fristående Connect-server."
	);
	let endpoint = network::endpoint_with_alpns(key, &transport, vec![]).await?;
	let client = Client {
		endpoint: endpoint.clone(),
		invite: invite.clone(),
	};
	let result = client.request(request).await;
	endpoint.close().await;
	result
}
