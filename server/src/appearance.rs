use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText, Stroke};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Mode {
	#[default]
	System,
	Dark,
	Light,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
	pub mode: Mode,
	pub accent: [u8; 3],
	pub opacity: f32,
	pub wallpaper: String,
	pub blur: f32,
	pub visibility: f32,
}

impl Default for Appearance {
	fn default() -> Self {
		Self {
			mode: Mode::System,
			accent: [184, 168, 218],
			opacity: 0.86,
			wallpaper: String::new(),
			blur: 16.0,
			visibility: 0.55,
		}
	}
}

impl Appearance {
	pub fn apply(&self, ctx: &egui::Context) {
		ctx.set_theme(match self.mode {
			Mode::System => egui::ThemePreference::System,
			Mode::Dark => egui::ThemePreference::Dark,
			Mode::Light => egui::ThemePreference::Light,
		});
		let theme = ctx.theme();
		let dark = theme == egui::Theme::Dark;
		let mut style = (*ctx.style_of(theme)).clone();
		style.spacing.item_spacing = egui::vec2(10.0, 10.0);
		style.spacing.button_padding = egui::vec2(12.0, 8.0);
		style.spacing.interact_size.y = 32.0;
		style.visuals.override_text_color = Some(if dark {
			Color32::from_rgb(221, 220, 225)
		} else {
			Color32::from_rgb(41, 40, 47)
		});
		style.visuals.panel_fill = self.panel(dark);
		style.visuals.window_fill = if dark {
			Color32::from_rgb(32, 31, 37)
		} else {
			Color32::from_rgb(250, 249, 253)
		};
		style.visuals.extreme_bg_color = if dark {
			Color32::from_rgb(23, 22, 28)
		} else {
			Color32::from_rgb(235, 233, 242)
		};
		style.visuals.faint_bg_color = if dark {
			Color32::from_white_alpha(5)
		} else {
			Color32::from_black_alpha(4)
		};
		style.visuals.selection.bg_fill =
			self.accent_color()
				.gamma_multiply(if dark { 0.28 } else { 0.35 });
		style.visuals.selection.stroke = Stroke::new(1.0, self.accent_color());
		style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(
			1.0,
			if dark {
				Color32::from_white_alpha(18)
			} else {
				Color32::from_black_alpha(20)
			},
		);
		for widget in [
			&mut style.visuals.widgets.inactive,
			&mut style.visuals.widgets.hovered,
			&mut style.visuals.widgets.active,
		] {
			widget.corner_radius = egui::CornerRadius::same(10);
		}
		style.visuals.widgets.inactive.bg_fill = if dark {
			Color32::from_rgb(43, 42, 49)
		} else {
			Color32::from_rgb(231, 227, 240)
		};
		style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
		style.visuals.widgets.hovered.bg_fill = if dark {
			Color32::from_rgb(57, 53, 66)
		} else {
			Color32::from_rgb(222, 215, 236)
		};
		style.visuals.window_corner_radius = egui::CornerRadius::same(16);
		style
			.text_styles
			.insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
		style
			.text_styles
			.insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
		style
			.text_styles
			.insert(egui::TextStyle::Heading, egui::FontId::proportional(23.0));
		ctx.set_style_of(theme, style);
	}

	pub fn accent_color(&self) -> Color32 {
		Color32::from_rgb(self.accent[0], self.accent[1], self.accent[2])
	}
	pub fn panel(&self, dark: bool) -> Color32 {
		let alpha = (self.opacity.clamp(0.35, 1.0) * 255.0) as u8;
		if dark {
			Color32::from_rgba_unmultiplied(27, 26, 32, alpha)
		} else {
			Color32::from_rgba_unmultiplied(248, 246, 252, alpha)
		}
	}
}

pub fn install_fonts(ctx: &egui::Context) {
	let mut fonts = FontDefinitions::default();
	fonts.font_data.insert(
		"Inter".into(),
		FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf")).into(),
	);
	fonts
		.families
		.entry(FontFamily::Proportional)
		.or_default()
		.insert(0, "Inter".into());
	ctx.set_fonts(fonts);
}

pub fn avatar(ui: &mut egui::Ui, name: &str, color: [u8; 3], size: f32) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
	let accent = Color32::from_rgb(color[0], color[1], color[2]);
	ui.painter()
		.circle_filled(rect.center(), size / 2.0, accent.gamma_multiply(0.25));
	ui.painter().circle_stroke(
		rect.center(),
		size / 2.0,
		Stroke::new(1.2, accent.gamma_multiply(0.65)),
	);
	ui.painter().text(
		rect.center(),
		egui::Align2::CENTER_CENTER,
		name.chars()
			.next()
			.unwrap_or('S')
			.to_uppercase()
			.to_string(),
		egui::FontId::proportional(size * 0.43),
		accent,
	);
	response
}

pub fn orb(ui: &mut egui::Ui, size: f32, color: Color32) {
	let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
	let center = rect.center();
	ui.painter()
		.circle_stroke(center, size * 0.39, Stroke::new(size * 0.055, color));
	ui.painter().circle_filled(center, size * 0.13, color);
	ui.painter().circle_filled(
		center + egui::vec2(size * 0.27, -size * 0.28),
		size * 0.095,
		color,
	);
}

pub fn eyebrow(ui: &mut egui::Ui, text: &str) {
	ui.label(RichText::new(text).size(11.0).weak().strong());
}

pub fn window_controls(ui: &mut egui::Ui, ctx: &egui::Context) {
	for kind in 0..3 {
		let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 24.0), egui::Sense::click());
		if response.hovered() {
			ui.painter()
				.rect_filled(rect, 5, ui.visuals().widgets.hovered.bg_fill);
		}
		let center = rect.center();
		let color = ui.visuals().text_color();
		let stroke = Stroke::new(1.1, color);
		match kind {
			0 => {
				ui.painter().line_segment(
					[center - egui::vec2(4.0, 0.0), center + egui::vec2(4.0, 0.0)],
					stroke,
				);
			}
			1 => {
				ui.painter().rect_stroke(
					egui::Rect::from_center_size(center, egui::vec2(8.0, 8.0)),
					0,
					stroke,
					egui::StrokeKind::Inside,
				);
			}
			_ => {
				ui.painter().line_segment(
					[center - egui::vec2(4.0, 4.0), center + egui::vec2(4.0, 4.0)],
					stroke,
				);
				ui.painter().line_segment(
					[
						center + egui::vec2(-4.0, 4.0),
						center + egui::vec2(4.0, -4.0),
					],
					stroke,
				);
			}
		}
		if response.clicked() {
			ctx.send_viewport_cmd(match kind {
				0 => egui::ViewportCommand::Minimized(true),
				1 => egui::ViewportCommand::Maximized(
					!ctx.input(|i| i.viewport().maximized.unwrap_or(false)),
				),
				_ => egui::ViewportCommand::Close,
			});
		}
	}
}

/// Decode user-selected wallpaper with file/dimension limits before pixel allocation.
pub fn load_wallpaper(path: &str, blur: f32) -> anyhow::Result<egui::ColorImage> {
	use anyhow::ensure;
	use image::ImageReader;
	let file = std::fs::File::open(path)?;
	ensure!(
		file.metadata()?.len() <= 16 * 1024 * 1024,
		"Välj en bild under 16 MB."
	);
	let mut reader = ImageReader::new(std::io::BufReader::new(file)).with_guessed_format()?;
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(8192);
	limits.max_image_height = Some(8192);
	limits.max_alloc = Some(256 * 1024 * 1024);
	reader.limits(limits);
	let decoded = reader.decode()?.thumbnail(1600, 1000);
	let pixels = decoded.blur(blur.clamp(0.0, 40.0)).to_rgba8();
	Ok(egui::ColorImage::from_rgba_unmultiplied(
		[pixels.width() as usize, pixels.height() as usize],
		&pixels,
	))
}
