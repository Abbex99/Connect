use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::{
	io::{Read, Write},
	path::Path,
};

pub fn server_history_path(root: &Path, alias: &crate::protocol::Alias) -> std::path::PathBuf {
	root.join("servers")
		.join(&alias.owner)
		.join(format!("{}.json", alias.slug))
}

pub fn read_json<T: DeserializeOwned>(path: &Path, max_bytes: u64) -> Result<T> {
	let file =
		std::fs::File::open(path).with_context(|| format!("Kunde inte läsa {}", path.display()))?;
	ensure!(
		file.metadata()?.len() <= max_bytes,
		"Den sparade filen är för stor."
	);
	let mut bytes = Vec::new();
	file.take(max_bytes + 1).read_to_end(&mut bytes)?;
	ensure!(
		bytes.len() as u64 <= max_bytes,
		"Den sparade filen är för stor."
	);
	Ok(serde_json::from_slice(&bytes)?)
}

/// Callers serialize access to each path; rename replaces the prior complete file.
pub fn write_json<T: Serialize>(path: &Path, value: &T, max_bytes: usize) -> Result<()> {
	let bytes = serde_json::to_vec(value)?;
	ensure!(
		bytes.len() <= max_bytes,
		"Det sparade innehållet är för stort."
	);
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent)?;
	}
	let pending = path.with_extension("pending");
	let result = (|| -> Result<()> {
		let mut file = std::fs::File::create(&pending)?;
		file.write_all(&bytes)?;
		file.sync_all()?;
		drop(file);
		std::fs::rename(&pending, path)?;
		Ok(())
	})();
	if result.is_err() {
		let _ = std::fs::remove_file(&pending);
	}
	result.with_context(|| format!("Kunde inte spara {}", path.display()))
}
