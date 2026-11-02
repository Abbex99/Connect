use crate::{
	media,
	network::{read_frame_limited, write_frame},
	protocol::*,
	server::Core,
	server_control::{Request, Response},
};
use anyhow::{Result, ensure};
use iroh::endpoint::Connection;
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::timeout};

pub(crate) async fn serve(connection: Connection, core: Arc<Core>) -> Result<()> {
	let result = if connection.alpn() == crate::server_control::ALPN {
		control(&connection, core).await
	} else if connection.alpn() == crate::server::CHAT_ALPN {
		chat(&connection, core).await
	} else {
		Err(anyhow::anyhow!("Fel protokoll."))
	};
	connection.close(0u32.into(), b"session ended");
	result
}
async fn control(connection: &Connection, core: Arc<Core>) -> Result<()> {
	let (mut send, mut recv) = timeout(Duration::from_secs(10), connection.accept_bi()).await??;
	let request: Request = timeout(
		Duration::from_secs(10),
		read_frame_limited(&mut recv, 64 * 1024),
	)
	.await??;
	let peer = connection.remote_id().to_string();
	let result = tokio::task::spawn_blocking(move || core.execute(&peer, request)).await?;
	let response = result.unwrap_or_else(|error| Response::Error(error.to_string()));
	write_frame(&mut send, &response).await?;
	send.finish()?;
	let _ = timeout(Duration::from_secs(2), send.stopped()).await;
	Ok(())
}
struct Membership {
	core: Arc<Core>,
	peer: String,
	generation: u64,
}
impl Drop for Membership {
	fn drop(&mut self) {
		let core = self.core.clone();
		let peer = self.peer.clone();
		let generation = self.generation;
		tokio::task::spawn_blocking(move || core.leave(&peer, generation));
	}
}
async fn chat(connection: &Connection, core: Arc<Core>) -> Result<()> {
	let peer = connection.remote_id().to_string();
	let (mut send, mut recv) = timeout(Duration::from_secs(10), connection.accept_bi()).await??;
	let hello: Hello = timeout(
		Duration::from_secs(10),
		read_frame_limited(&mut recv, 64 * 1024),
	)
	.await??;
	let c = core.clone();
	let p = peer.clone();
	let admission = tokio::task::spawn_blocking(move || {
		let generation = c.admit(&p, hello)?;
		Ok::<_, anyhow::Error>(Membership {
			core: c,
			peer: p,
			generation,
		})
	})
	.await?;
	let _member = match admission {
		Ok(member) => member,
		Err(error) => {
			write_frame(&mut send, &ServerFrame::Error(error.to_string())).await?;
			send.finish()?;
			let _ = timeout(Duration::from_secs(2), send.stopped()).await;
			return Ok(());
		}
	};
	let generation = _member.generation;
	let c = core.clone();
	let p = peer.clone();
	let connection_copy = connection.clone();
	let mut file_rx =
		tokio::task::spawn_blocking(move || c.attach(&p, generation, connection_copy)).await??;
	let mut updates = core.updates.subscribe();
	let initial = updates.borrow_and_update().clone();
	write_frame(&mut send, &ServerFrame::Snapshot(initial)).await?;
	let (errors, mut error_rx) = mpsc::channel(8);
	let reader = async {
		let mut start = tokio::time::Instant::now();
		let mut count = 0;
		loop {
			let command: Command = read_frame_limited(&mut recv, 64 * 1024).await?;
			if start.elapsed() >= Duration::from_secs(1) {
				start = tokio::time::Instant::now();
				count = 0;
			}
			count += 1;
			ensure!(count <= 20, "För många kommandon.");
			let c = core.clone();
			let p = peer.clone();
			if let Err(error) =
				tokio::task::spawn_blocking(move || c.apply(&p, generation, command)).await?
			{
				errors
					.try_send(error.to_string())
					.map_err(|_| anyhow::anyhow!("För många felaktiga kommandon."))?;
			}
		}
		#[allow(unreachable_code)]
		Ok::<(), anyhow::Error>(())
	};
	let writer = async {
		loop {
			let frame = tokio::select! {
			 changed=updates.changed()=>{changed?;let snapshot=updates.borrow_and_update().clone();ensure!(core.current(&peer,generation)&&snapshot.members.iter().any(|m|m.id==peer),"Du har kopplats från servern.");ServerFrame::Snapshot(snapshot)},
			 Some(event)=file_rx.recv()=>ServerFrame::FileSignal(event),
			 Some(error)=error_rx.recv()=>ServerFrame::Error(error),
			};
			write_frame(&mut send, &frame).await?;
		}
		#[allow(unreachable_code)]
		Ok::<(), anyhow::Error>(())
	};
	tokio::select! {result=reader=>result,result=writer=>result,result=media::serve(connection.clone(),core.media.clone(),peer.clone())=>result,result=core.assets.clone().serve_connection(connection.clone())=>result}
}
