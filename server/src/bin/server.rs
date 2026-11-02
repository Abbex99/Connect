use anyhow::{Context, Result, ensure};
use serein_connect::{
	network::TransportConfig,
	protocol::now,
	server::{self, ServerConfig},
	server_admin,
	server_store::ServerStore,
};
use std::{path::PathBuf, sync::Arc};

fn value(args: &[String], flag: &str) -> Option<String> {
	args.windows(2)
		.find(|pair| pair[0] == flag)
		.map(|pair| pair[1].clone())
}
#[tokio::main]
async fn main() -> Result<()> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let command = args.first().map(String::as_str).unwrap_or("help");
	if command == "help" || args.iter().any(|a| a == "--help") {
		println!(
			"Connect Server\n\nmanage --data <mapp> [--open] [--offline] (lokal installationsguide och panel)\nmanage --data <mapp> --listen <intern adress:port> --origin <https://paneladress> --key-file <privat nyckelfil>\ninit --data <ny tom mapp> --alias @namn/rum --name <namn> [--public]\nrun --data <mapp> [--pair] [--offline] [--relay <https-adress>]\nrecovery-key --out <ny nyckelfil>\nbackup --data <mapp> --out <ny backupfil> --recipient <age-publik-nyckel>\nrestore --backup <fil> --data <ny mapp> --key-file <nyckelfil>\n\n--pair återställer administratörsbehörigheten och visar en engångslänk. Använd endast vid installation eller uttrycklig återställning.\nServerpanelen konfigureras i server.json; fjärråtkomst kräver HTTPS-proxy."
		);
		return Ok(());
	}
	let root =
		PathBuf::from(value(&args, "--data").unwrap_or_else(|| "connect-server-data".into()));
	match command {
		"manage" => {
			let offline = args.iter().any(|a| a == "--offline");
			if let Some(origin) = value(&args, "--origin") {
				ensure!(
					!args.iter().any(|a| a == "--open"),
					"Fjärrpanelen öppnas i din egen webbläsare."
				);
				let bind =
					value(&args, "--listen").context("Ange --listen för proxyanslutningen.")?;
				let key = PathBuf::from(
					value(&args, "--key-file")
						.context("Ange --key-file för administrationsnyckeln.")?,
				);
				serein_connect::server_manager::run_remote(root, bind, origin, key, offline)
					.await?;
			} else {
				ensure!(
					value(&args, "--listen").is_none() && value(&args, "--key-file").is_none(),
					"Ange även --origin med panelens HTTPS-adress."
				);
				serein_connect::server_manager::run(
					root,
					args.iter().any(|a| a == "--open"),
					offline,
				)
				.await?;
			}
		}
		"init" => {
			let alias = value(&args, "--alias").context("Ange --alias @namn/rum.")?;
			let name = value(&args, "--name").unwrap_or_else(|| "Min Connect-server".into());
			let mut config = ServerConfig::local(&alias, &name)?;
			config.public = args.iter().any(|a| a == "--public");
			let setup = server::initialize(&root, config)?;
			println!(
				"Servern skapades: {}\nStarta med run --data \"{}\" --pair och klistra in engångslänken i Connect → Anslut.",
				setup.server_id,
				root.display()
			);
		}
		"run" => {
			let token = if args.iter().any(|a| a == "--pair") {
				let _lock = server::lock_directory(&root)?;
				let c = ServerConfig::load(&root)?;
				Some(
					ServerStore::open(&root.join("server.db"), &c.name)?
						.reset_pairing(3600, now())?,
				)
			} else {
				None
			};
			let config = ServerConfig::load(&root)?;
			let bind: std::net::SocketAddr =
				config.panel_bind.parse().context("Ogiltig panel_bind.")?;
			ensure!(
				bind.ip().is_loopback() || config.panel_origin.starts_with("https://"),
				"Publik panel kräver HTTPS-origin och konfigurerad proxy."
			);
			let running = server::start(
				&root,
				TransportConfig {
					offline: args.iter().any(|a| a == "--offline"),
					relay_url: value(&args, "--relay"),
					relay_only: false,
				},
			)
			.await?;
			let web = Arc::new(server_admin::WebState::new(
				running.core.clone(),
				&config.panel_origin,
			)?);
			let listener = tokio::net::TcpListener::bind(bind)
				.await
				.context("Serverpanelen kunde inte starta på vald adress.")?;
			println!(
				"Connect Server körs. Panel: {}\nDatakatalog: {}",
				config.panel_origin,
				root.display()
			);
			if let Some(token) = token {
				let mut invite = running.invite.clone();
				invite.token = token;
				println!(
					"\nHEMLIG ENGÅNGSLÄNK FÖR ÄGAREN (gäller en timme):\n{}\n",
					invite.encode()?
				);
			} else {
				println!(
					"Serveradress (ger inte administratörsrättigheter):\n{}",
					running.invite.encode()?
				);
			}
			let app = server_admin::router(web);
			let result = server_admin::serve(listener, app, async {
				let _ = tokio::signal::ctrl_c().await;
			})
			.await;
			running.close().await;
			result?;
		}
		"recovery-key" => {
			let out = PathBuf::from(
				value(&args, "--out").context("Ange --out för den nya hemliga nyckelfilen.")?,
			);
			let (private, public) = serein_connect::server_backup::generate_recovery_key();
			serein_connect::server_backup::save_recovery_key(&out, &private)?;
			println!(
				"Återställningsnyckeln sparades. Förvara filen separat från säkerhetskopian.\nPublik mottagarnyckel för backup: {public}"
			);
		}
		"backup" => {
			let out = PathBuf::from(value(&args, "--out").context("Ange --out.")?);
			let recipient =
				value(&args, "--recipient").context("Ange den publika --recipient-nyckeln.")?;
			serein_connect::server_backup::export(&root, &out, &recipient)?;
			println!("Säkerhetskopian är sparad: {}", out.display());
		}
		"import-legacy" => {
			let path = PathBuf::from(
				value(&args, "--file")
					.context("Ange --file med den signerade exporten från Connect.")?,
			);
			serein_connect::server_migration::import(&root, &path)?;
			println!("Historiken importerades. Originalet är kvar.");
		}
		"restore" => {
			let backup = PathBuf::from(value(&args, "--backup").context("Ange --backup.")?);
			let keyfile = PathBuf::from(value(&args, "--key-file").context("Ange --key-file.")?);
			ensure!(
				std::fs::metadata(&keyfile)?.len() < 4096,
				"Fel återställningsnyckelfil."
			);
			let key = std::fs::read_to_string(keyfile)?;
			serein_connect::server_backup::restore(&backup, &root, key.trim())?;
			println!("Servern återställdes. Stäng den gamla servern innan denna startas.");
		}
		_ => anyhow::bail!("Okänt kommando. Använd --help."),
	};
	Ok(())
}
