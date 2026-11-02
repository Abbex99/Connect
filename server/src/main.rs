#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod app;
mod appearance;

fn main() -> eframe::Result {
	let args: Vec<String> = std::env::args().collect();
	let demo = args.iter().any(|a| a == "--demo");
	let profile = args
		.windows(2)
		.find(|a| a[0] == "--profile")
		.map(|a| a[1].clone())
		.unwrap_or_else(|| "default".into());
	if serein_connect::protocol::validate_slug(&profile).is_err() {
		eprintln!("Invalid local profile name");
		return Ok(());
	}
	let options = eframe::NativeOptions {
		viewport: eframe::egui::ViewportBuilder::default()
			.with_inner_size([1340.0, 860.0])
			.with_min_inner_size([900.0, 620.0])
			.with_decorations(false),
		..Default::default()
	};
	eframe::run_native(
		"Serein Connect",
		options,
		Box::new(move |cc| Ok(Box::new(app::App::new(cc, profile, demo)))),
	)
}
