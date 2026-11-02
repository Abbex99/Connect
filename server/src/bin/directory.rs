use serein_connect::directory::{Directory, router};
use std::{
	path::PathBuf,
	sync::{Arc, Mutex},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let bind = std::env::var("SEREIN_DIRECTORY_BIND").unwrap_or_else(|_| "127.0.0.1:4747".into());
	let path = PathBuf::from(
		std::env::var("SEREIN_DIRECTORY_DATA").unwrap_or_else(|_| "data/directory.json".into()),
	);
	let directory = Arc::new(Mutex::new(Directory::open(&path)?));
	let listener = tokio::net::TcpListener::bind(&bind).await?;
	println!("Serein directory listening on {}", listener.local_addr()?);
	axum::serve(listener, router(directory))
		.with_graceful_shutdown(async {
			let _ = tokio::signal::ctrl_c().await;
		})
		.await?;
	Ok(())
}
