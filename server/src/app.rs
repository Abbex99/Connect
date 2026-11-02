use crate::appearance::{self, Appearance, Mode};
use eframe::egui::{self, Align, Color32, Frame, Layout, RichText, Stroke};
use serde::{Deserialize, Serialize};
use serein_connect::{
	directory, identity,
	network::{self, HostOptions, Session, TransportConfig},
	protocol::*,
	storage,
};
use std::{path::PathBuf, sync::mpsc, time::Duration};

#[derive(Clone, Serialize, Deserialize)]
struct Bookmark {
	alias: String,
	name: String,
	owned: bool,
	public: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
	profile: Profile,
	username: String,
	bio: String,
	appearance: Appearance,
	directory: String,
	relay: String,
	bookmarks: Vec<Bookmark>,
}
impl Default for Preferences {
	fn default() -> Self {
		Self {
			profile: Profile {
				name: "Du".into(),
				accent: [184, 168, 218],
			},
			username: "".into(),
			bio: "".into(),
			appearance: Appearance::default(),
			directory: "http://127.0.0.1:4747".into(),
			relay: String::new(),
			bookmarks: Vec::new(),
		}
	}
}

enum Action {
	Host(Bookmark),
	Join(String),
}
struct Connected {
	session: Session,
	bookmark: Bookmark,
}

pub struct App {
	prefs: Preferences,
	data_dir: PathBuf,
	profile_key: String,
	runtime: tokio::runtime::Runtime,
	session: Option<Session>,
	pending: Option<mpsc::Receiver<Result<Connected, String>>>,
	pending_task: Option<tokio::task::JoinHandle<()>>,
	snapshot: Snapshot,
	active_alias: String,
	channel: String,
	draft: String,
	notice: String,
	home: bool,
	demo: bool,
	settings: bool,
	settings_tab: usize,
	show_members: bool,
	create_open: bool,
	join_open: bool,
	invite_open: bool,
	channel_open: bool,
	create_name: String,
	create_slug: String,
	create_public: bool,
	join_value: String,
	channel_name: String,
	wallpaper: Option<egui::TextureHandle>,
	wallpaper_job: Option<mpsc::Receiver<Result<egui::ColorImage, String>>>,
}

impl App {
	pub fn new(cc: &eframe::CreationContext<'_>, profile_key: String, demo: bool) -> Self {
		appearance::install_fonts(&cc.egui_ctx);
		let data_dir = std::env::var_os("LOCALAPPDATA")
			.map(PathBuf::from)
			.unwrap_or_else(|| PathBuf::from("data"))
			.join("SereinConnect")
			.join(&profile_key);
		let path = data_dir.join("preferences.json");
		let (mut prefs, notice) = if path.exists() && !demo {
			match storage::read_json::<Preferences>(&path, 128 * 1024) {
				Ok(prefs) => (prefs, String::new()),
				Err(error) => (
					Preferences::default(),
					format!("Inställningarna kunde inte läsas: {error}"),
				),
			}
		} else {
			(Preferences::default(), String::new())
		};
		prefs.bookmarks.truncate(32);
		if demo {
			prefs.appearance.mode = Mode::Dark;
			prefs.username = "albin".into();
			prefs.profile.name = "Albin".into();
		}
		let mut app = Self {
			prefs,
			data_dir,
			profile_key,
			runtime: tokio::runtime::Builder::new_multi_thread()
				.enable_all()
				.build()
				.expect("Tokio runtime"),
			session: None,
			pending: None,
			pending_task: None,
			snapshot: Snapshot::default(),
			active_alias: String::new(),
			channel: "allmant".into(),
			draft: String::new(),
			notice,
			home: !demo,
			demo,
			settings: false,
			settings_tab: 0,
			show_members: false,
			create_open: false,
			join_open: false,
			invite_open: false,
			channel_open: false,
			create_name: "Mitt rum".into(),
			create_slug: "mitt-rum".into(),
			create_public: false,
			join_value: String::new(),
			channel_name: String::new(),
			wallpaper: None,
			wallpaper_job: None,
		};
		if demo {
			app.snapshot = demo_snapshot();
			app.active_alias = "@albin/studion".into();
		}
		if !app.prefs.appearance.wallpaper.is_empty() {
			app.load_wallpaper();
		}
		app
	}

	fn save(&mut self) {
		if self.demo {
			return;
		}
		if let Err(error) = storage::write_json(
			&self.data_dir.join("preferences.json"),
			&self.prefs,
			128 * 1024,
		) {
			self.notice = error.to_string();
		}
	}
	fn load_wallpaper(&mut self) {
		if self.prefs.appearance.wallpaper.is_empty() {
			self.wallpaper = None;
			return;
		}
		let path = self.prefs.appearance.wallpaper.clone();
		let blur = self.prefs.appearance.blur;
		let (sender, receiver) = mpsc::channel();
		self.wallpaper_job = Some(receiver);
		self.runtime.spawn_blocking(move || {
			let _ = sender.send(appearance::load_wallpaper(&path, blur).map_err(|e| e.to_string()));
		});
	}
	fn poll(&mut self, ctx: &egui::Context) {
		if let Some(receiver) = &self.pending {
			match receiver.try_recv() {
				Ok(result) => {
					self.pending = None;
					self.pending_task = None;
					match result {
						Ok(connected) => {
							self.active_alias = connected.bookmark.alias.clone();
							self.snapshot = connected.session.snapshots.borrow().clone();
							self.session = Some(connected.session);
							self.home = false;
							self.demo = false;
							self.channel = "allmant".into();
							self.prefs
								.bookmarks
								.retain(|b| b.alias != connected.bookmark.alias);
							self.prefs.bookmarks.insert(0, connected.bookmark);
							self.prefs.bookmarks.truncate(32);
							self.save();
							self.notice = "Ansluten. Meddelanden lagras på värdens dator.".into();
						}
						Err(error) => {
							self.notice = error;
							self.home = true;
						}
					}
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					self.pending = None;
					self.notice = "Anslutningen avbröts. Försök igen.".into();
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}
		if let Some(receiver) = &self.wallpaper_job {
			if let Ok(result) = receiver.try_recv() {
				self.wallpaper_job = None;
				match result {
					Ok(pixels) => {
						self.wallpaper = Some(ctx.load_texture(
							"wallpaper",
							pixels,
							egui::TextureOptions::LINEAR,
						));
					}
					Err(error) => self.notice = format!("Bakgrunden kunde inte öppnas: {error}"),
				}
			}
		}
		if let Some(session) = &mut self.session {
			if session.snapshots.has_changed().unwrap_or(false) {
				self.snapshot = session.snapshots.borrow_and_update().clone();
			}
			while let Ok(notice) = session.notices.try_recv() {
				self.notice = notice;
			}
		}
		if self.session.as_ref().is_some_and(Session::is_closed) {
			self.session = None;
			self.snapshot.members.clear();
			self.notice = "Servern är frånkopplad. Anslut igen när värden är online.".into();
		}
	}

	fn start(&mut self, action: Action) {
		if self.pending.is_some() {
			return;
		}
		if let Err(error) = validate_text(&self.prefs.profile.name, 96) {
			self.notice = error.to_string();
			return;
		}
		let prefs = self.prefs.clone();
		let root = self.data_dir.clone();
		let profile_key = self.profile_key.clone();
		let old = self.session.take();
		let (sender, receiver) = mpsc::channel();
		self.pending = Some(receiver);
		self.notice.clear();
		self.snapshot = Snapshot::default();
		self.demo = false;
		self.pending_task = Some(self.runtime.spawn(async move {
			if let Some(old) = old {
				old.close().await;
			}
			let result = async {
				let identity_profile = profile_key.clone();
				let key = tokio::task::spawn_blocking(move || {
					identity::load_or_create(&identity_profile)
				})
				.await??;
				let transport = TransportConfig {
					relay_url: if prefs.relay.trim().is_empty() {
						None
					} else {
						Some(prefs.relay.trim().into())
					},
					..Default::default()
				};
				match action {
					Action::Host(bookmark) => {
						let alias = Alias::parse(&bookmark.alias)?;
						let history = storage::server_history_path(&root, &alias);
						let session = network::host(
							HostOptions {
								alias,
								name: bookmark.name.clone(),
								profile: prefs.profile,
								public: bookmark.public,
								history,
								directory_url: Some(prefs.directory),
							},
							key,
							transport,
						)
						.await?;
						Ok::<_, anyhow::Error>(Connected { session, bookmark })
					}
					Action::Join(value) => {
						let invite = if value.trim().starts_with("serein://") {
							Invite::decode(&value)?
						} else {
							let alias = Alias::parse(&value).ok();
							match alias
								.as_ref()
								.and_then(|a| {
									identity::load_invite(&profile_key, &a.to_string()).ok()
								})
								.and_then(|s| Invite::decode(&s).ok())
							{
								Some(invite) => invite,
								None => {
									let record =
										directory::lookup_name(&prefs.directory, &value).await?;
									let alias =
										format!("@{}/{}", record.record.owner, record.record.slug);
									Invite {
										version: 1,
										address: record.record.address,
										token: String::new(),
										alias,
										name: record.record.name,
									}
								}
							}
						};
						let encoded = invite.encode()?;
						let bookmark = Bookmark {
							alias: invite.alias.clone(),
							name: invite.name.clone(),
							owned: false,
							public: invite.token.is_empty(),
						};
						let mut session =
							network::join(invite, key, prefs.profile, transport).await?;
						if let Err(error) =
							identity::save_invite(&profile_key, &bookmark.alias, &encoded)
						{
							// Keep the working connection; show that rejoining will require the invitation.
							let _ = &mut session;
							eprintln!("Invitation was not saved in credential storage: {error}");
						}
						Ok(Connected { session, bookmark })
					}
				}
			}
			.await;
			let _ = sender.send(result.map_err(|e| e.to_string()));
		}));
	}

	fn send(&mut self, command: Command) {
		if let Some(session) = &self.session {
			if let Err(error) = session.send(command) {
				self.notice = error.to_string();
			}
		} else {
			self.notice = "Starta eller anslut till en server först.".into();
		}
	}
	fn leave(&mut self) {
		if let Some(task) = self.pending_task.take() {
			task.abort();
		}
		self.pending = None;
		if let Some(session) = self.session.take() {
			self.runtime.spawn(async move {
				session.close().await;
			});
		}
		self.snapshot = Snapshot::default();
		self.active_alias.clear();
		self.home = true;
	}
	fn host_selected(&mut self) {
		let alias = format!(
			"@{}/{}",
			self.prefs.username.trim().to_ascii_lowercase(),
			self.create_slug.trim().to_ascii_lowercase()
		);
		if let Err(error) = Alias::parse(&alias).and_then(|_| validate_text(&self.create_name, 96))
		{
			self.notice = error.to_string();
			return;
		}
		self.save();
		self.create_open = false;
		self.start(Action::Host(Bookmark {
			alias,
			name: self.create_name.trim().into(),
			owned: true,
			public: self.create_public,
		}));
	}

	fn background(&self, ui: &egui::Ui) {
		let rect = ui.max_rect();
		let dark = ui.visuals().dark_mode;
		ui.painter().rect_filled(
			rect,
			0,
			if dark {
				Color32::from_rgb(24, 23, 29)
			} else {
				Color32::from_rgb(234, 229, 242)
			},
		);
		if let Some(texture) = &self.wallpaper {
			let image_ratio = texture.size_vec2().x / texture.size_vec2().y;
			let screen_ratio = rect.width() / rect.height();
			let uv = if image_ratio > screen_ratio {
				let half = screen_ratio / image_ratio * 0.5;
				egui::Rect::from_min_max(egui::pos2(0.5 - half, 0.0), egui::pos2(0.5 + half, 1.0))
			} else {
				let half = image_ratio / screen_ratio * 0.5;
				egui::Rect::from_min_max(egui::pos2(0.0, 0.5 - half), egui::pos2(1.0, 0.5 + half))
			};
			ui.painter().image(
				texture.id(),
				rect,
				uv,
				Color32::from_white_alpha((self.prefs.appearance.visibility * 255.0) as u8),
			);
		} else {
			let center = egui::pos2(
				rect.right() - rect.width() * 0.27,
				rect.top() + rect.height() * 0.47,
			);
			let radius = rect.height() * 0.32;
			for step in (0..32).rev() {
				let spread = step as f32 * 2.5;
				let alpha =
					((1.0 - step as f32 / 32.0) * self.prefs.appearance.visibility * 7.0) as u8;
				let accent = self.prefs.appearance.accent;
				let color = Color32::from_rgba_unmultiplied(accent[0], accent[1], accent[2], alpha);
				ui.painter()
					.circle_stroke(center, radius + spread, Stroke::new(9.0, color));
			}
		}
	}

	fn sidebar(&mut self, ui: &mut egui::Ui) {
		ui.add_space(12.0);
		ui.horizontal(|ui| {
			appearance::orb(ui, 28.0, self.prefs.appearance.accent_color());
			ui.label(RichText::new("Serein").size(21.0).strong());
		});
		ui.add_space(20.0);
		if ui
			.add_sized(
				[ui.available_width(), 36.0],
				egui::Button::new("Hem").selected(self.home),
			)
			.clicked()
		{
			self.home = true;
		}
		if ui
			.add_sized(
				[ui.available_width(), 34.0],
				egui::Button::new("+   Nytt rum").frame(false),
			)
			.clicked()
		{
			self.create_open = true;
		}
		ui.add_space(14.0);
		ui.horizontal(|ui| {
			appearance::eyebrow(ui, "DINA SERVRAR");
			ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
				if ui
					.small_button("+")
					.on_hover_text("Skapa en server")
					.clicked()
				{
					self.create_open = true;
				}
			});
		});
		for bookmark in self.prefs.bookmarks.clone() {
			let selected = !self.home && self.active_alias == bookmark.alias;
			let label = bookmark.name.clone();
			if ui
				.add_sized(
					[ui.available_width(), 38.0],
					egui::Button::new(label).selected(selected),
				)
				.clicked()
			{
				if self.active_alias == bookmark.alias && self.session.is_some() {
					self.home = false;
				} else if bookmark.owned {
					self.start(Action::Host(bookmark));
				} else {
					self.start(Action::Join(bookmark.alias));
				}
			}
		}
		if self.demo {
			ui.add_sized(
				[ui.available_width(), 48.0],
				egui::Button::new("Studion").selected(true),
			);
		}
		if self.prefs.bookmarks.is_empty() && !self.demo {
			ui.label(RichText::new("Dina rum samlas här.").small().weak());
		}
		ui.add_space(8.0);
		if ui
			.add_sized(
				[ui.available_width(), 34.0],
				egui::Button::new("+   Anslut till server").frame(false),
			)
			.clicked()
		{
			self.join_open = true;
		}
		if !self.home && !self.snapshot.channels.is_empty() {
			ui.add_space(25.0);
			ui.horizontal(|ui| {
				appearance::eyebrow(ui, "TEXTKANALER");
				if self.session.as_ref().is_some_and(|s| s.invite.is_some())
					&& ui.small_button("+").clicked()
				{
					self.channel_open = true;
				}
			});
			for channel in self.snapshot.channels.clone() {
				if ui
					.add_sized(
						[ui.available_width(), 34.0],
						egui::Button::new(&channel.name).selected(self.channel == channel.id),
					)
					.clicked()
				{
					self.channel = channel.id;
				}
			}
			ui.add_space(20.0);
			appearance::eyebrow(ui, "SAMTAL & DELNING");
			ui.add_enabled(
				false,
				egui::Button::new("Röst och skärmdelning").frame(false),
			)
			.on_hover_text("Byggs i nästa steg.");
		}
		ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
			Frame::new()
				.fill(ui.visuals().faint_bg_color)
				.stroke(ui.visuals().widgets.noninteractive.bg_stroke)
				.corner_radius(12)
				.inner_margin(12)
				.show(ui, |ui| {
					ui.set_width(ui.available_width());
					ui.horizontal(|ui| {
						appearance::avatar(
							ui,
							&self.prefs.profile.name,
							self.prefs.profile.accent,
							34.0,
						);
						ui.vertical(|ui| {
							ui.strong(&self.prefs.profile.name);
							ui.label(
								RichText::new(if self.session.is_some() {
									"●  Ansluten"
								} else {
									"○  Inte ansluten"
								})
								.size(11.0)
								.weak(),
							);
						});
						if ui
							.button("···")
							.on_hover_text("Profil och inställningar")
							.clicked()
						{
							self.settings = true;
						}
					});
				});
			ui.add_space(10.0);
			if self.session.is_some() && ui.button("Lämna / stoppa server").clicked() {
				self.leave();
			}
			ui.label(RichText::new("DIN DATOR. DITT UTRYMME.").size(10.0).weak());
		});
	}

	fn home_view(&mut self, ui: &mut egui::Ui) {
		ui.add_space((ui.available_height() * 0.13).max(25.0));
		ui.vertical_centered(|ui| {
			appearance::orb(ui, 70.0, self.prefs.appearance.accent_color());
			ui.add_space(22.0);
			ui.label(RichText::new("Ett eget rum för er.").size(34.0).strong());
			ui.add_space(10.0);
			ui.label(
				RichText::new("Samla människorna. Låt samtalen ta plats.")
					.size(17.0)
					.weak(),
			);
			ui.add_space(30.0);
			ui.horizontal(|ui| {
				let width = 460.0_f32.min(ui.available_width());
				ui.add_space(((ui.available_width() - width) * 0.5).max(0.0));
				if ui
					.add_sized(
						[220.0, 48.0],
						egui::Button::new(
							RichText::new("+   Skapa en server")
								.color(Color32::from_rgb(35, 29, 43)),
						)
						.fill(self.prefs.appearance.accent_color())
						.stroke(Stroke::NONE),
					)
					.clicked()
				{
					self.create_open = true;
				}
				if ui
					.add_sized(
						[220.0, 48.0],
						egui::Button::new("Anslut med namn eller länk"),
					)
					.clicked()
				{
					self.join_open = true;
				}
			});
			ui.add_space(38.0);
			ui.label(
				RichText::new("Servern bor på din dator. Vännerna kommer in via en inbjudan.")
					.weak(),
			);
			ui.label(RichText::new("Ingen IP-adress. Inga portar att skriva in.").weak());
			ui.add_space(35.0);
			Frame::new()
				.fill(ui.visuals().faint_bg_color)
				.corner_radius(12)
				.inner_margin(18)
				.show(ui, |ui| {
					ui.set_max_width(470.0);
					ui.label(
						RichText::new("FÖRSTA BYGGET")
							.size(11.0)
							.color(self.prefs.appearance.accent_color())
							.strong(),
					);
					ui.label("Egen server, inbjudningar, textkanaler och teman.");
					ui.label(
						RichText::new("Röst, skärmdelning och fjärrstyrning kommer i nästa steg.")
							.small()
							.weak(),
					);
				});
		});
	}

	fn chat_view(&mut self, ui: &mut egui::Ui) {
		let connected = self.session.is_some();
		egui::Panel::bottom("composer")
			.resizable(false)
			.frame(Frame::new().inner_margin(egui::Margin::symmetric(26, 18)))
			.show(ui, |ui| {
				Frame::new()
					.fill(self.prefs.appearance.panel(ui.visuals().dark_mode))
					.stroke(ui.visuals().widgets.noninteractive.bg_stroke)
					.corner_radius(14)
					.inner_margin(16)
					.show(ui, |ui| {
						let response = ui.add_sized(
							[ui.available_width(), 52.0],
							egui::TextEdit::multiline(&mut self.draft)
								.frame(Frame::new())
								.hint_text("Skriv ett meddelande…")
								.desired_rows(2)
								.char_limit(4096)
								.return_key(egui::KeyboardShortcut::new(
									egui::Modifiers::SHIFT,
									egui::Key::Enter,
								)),
						);
						let enter = response.has_focus()
							&& ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
						ui.horizontal(|ui| {
							ui.label(RichText::new("Shift + Enter för ny rad").size(11.0).weak());
							ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
								let clicked = ui
									.add_enabled(
										connected && !self.draft.trim().is_empty(),
										egui::Button::new("Skicka  ↑"),
									)
									.clicked();
								if (clicked || enter) && connected && !self.draft.trim().is_empty()
								{
									let text = self.draft.trim_end_matches('\n').to_owned();
									if let Err(error) = validate_text(&text, 4096) {
										self.notice = error.to_string();
									} else if let Some(session) = &self.session {
										match session.send(Command::SendMessage {
											channel: self.channel.clone(),
											text,
										}) {
											Ok(()) => self.draft.clear(),
											Err(error) => self.notice = error.to_string(),
										}
									}
									response.request_focus();
								}
							});
						});
					});
			});
		egui::ScrollArea::vertical()
			.id_salt(("messages", &self.channel))
			.stick_to_bottom(true)
			.auto_shrink([false, false])
			.show(ui, |ui| {
				ui.add_space(25.0);
				if self
					.snapshot
					.messages
					.iter()
					.all(|m| m.channel != self.channel)
				{
					ui.horizontal(|ui| {
						ui.add_space(26.0);
						ui.vertical(|ui| {
							ui.label(RichText::new("Här börjar ert samtal.").size(25.0).strong());
							ui.label(
								RichText::new(format!("Välkommen till {}.", self.snapshot.name))
									.weak(),
							);
						});
					});
				}
				ui.add_space(20.0);
				for message in self
					.snapshot
					.messages
					.iter()
					.filter(|m| m.channel == self.channel)
				{
					let own = self
						.session
						.as_ref()
						.is_some_and(|s| s.self_id == message.author_id)
						|| (self.demo && message.author_id == "a");
					ui.vertical_centered(|ui| {
						ui.label(
							RichText::new(format!(
								"{:02}:{:02} UTC",
								(message.sent_at / 3600) % 24,
								(message.sent_at / 60) % 60
							))
							.size(10.0)
							.weak(),
						);
					});
					ui.add_space(8.0);
					let width = ui.available_width();
					let card_width = (width * 0.76).clamp(220.0, 740.0);
					ui.horizontal_top(|ui| {
						let text_width = ui
							.painter()
							.layout(
								message.text.clone(),
								egui::FontId::proportional(15.0),
								ui.visuals().text_color(),
								card_width - 40.0,
							)
							.size()
							.x;
						ui.add_space(if own {
							(width - text_width - 74.0).max(26.0)
						} else {
							26.0
						});
						if !own {
							appearance::avatar(
								ui,
								&message.author.name,
								message.author.accent,
								28.0,
							);
						}
						ui.vertical(|ui| {
							ui.set_max_width(card_width);
							if !own {
								ui.label(RichText::new(&message.author.name).size(12.0).weak());
							}
							Frame::new()
								.fill(self.prefs.appearance.panel(ui.visuals().dark_mode))
								.stroke(ui.visuals().widgets.noninteractive.bg_stroke)
								.corner_radius(13)
								.inner_margin(egui::Margin::symmetric(17, 14))
								.show(ui, |ui| {
									ui.set_max_width(card_width - 40.0);
									ui.add(egui::Label::new(&message.text).wrap());
								});
						});
					});
					ui.add_space(25.0);
				}
				if self
					.snapshot
					.messages
					.iter()
					.all(|m| m.channel != self.channel)
				{
					ui.horizontal(|ui| {
						ui.add_space(26.0);
						ui.label(RichText::new("Säg hej. Det här rummet är ert.").weak());
					});
				}
				ui.add_space(15.0);
			});
	}

	fn dialogs(&mut self, ctx: &egui::Context) {
		let mut open = self.create_open;
		egui::Window::new("Skapa ett eget rum")
			.open(&mut open)
			.collapsible(false)
			.resizable(false)
			.default_width(420.0)
			.show(ctx, |ui| {
				ui.label("Servern körs på din dator medan appen är öppen.");
				ui.add_space(12.0);
				ui.label("Ditt användarnamn");
				ui.add(
					egui::TextEdit::singleline(&mut self.prefs.username)
						.hint_text("albin")
						.char_limit(32),
				);
				ui.label("Serverns namn");
				ui.add(egui::TextEdit::singleline(&mut self.create_name).char_limit(80));
				ui.label("Kortnamn i adressen");
				ui.add(
					egui::TextEdit::singleline(&mut self.create_slug)
						.hint_text("spelrummet")
						.char_limit(32),
				);
				ui.label(
					RichText::new(format!("@{}/{}", self.prefs.username, self.create_slug))
						.color(self.prefs.appearance.accent_color()),
				);
				ui.checkbox(
					&mut self.create_public,
					"Tillåt anslutning via namn utan inbjudan",
				);
				if !self.create_public {
					ui.label(
						RichText::new("Privat rum – gäster behöver din inbjudningslänk.")
							.small()
							.weak(),
					);
				}
				ui.add_space(8.0);
				if ui
					.add_enabled(
						self.pending.is_none(),
						egui::Button::new("Starta server på min dator"),
					)
					.clicked()
				{
					self.host_selected();
				}
			});
		self.create_open &= open;
		let mut open = self.join_open;
		egui::Window::new("Anslut till ett rum")
			.open(&mut open)
			.collapsible(false)
			.resizable(false)
			.default_width(460.0)
			.show(ctx, |ui| {
				ui.label("Klistra in en inbjudan eller skriv ett användarnamn.");
				ui.add(
					egui::TextEdit::multiline(&mut self.join_value)
						.desired_rows(3)
						.desired_width(f32::INFINITY)
						.char_limit(16 * 1024)
						.hint_text("@albin, @albin/spelrummet eller serein://join/…"),
				);
				ui.label(
					RichText::new("Värdens dator behöver vara på och servern startad.")
						.small()
						.weak(),
				);
				if ui
					.add_enabled(
						self.pending.is_none() && !self.join_value.trim().is_empty(),
						egui::Button::new("Anslut"),
					)
					.clicked()
				{
					self.join_open = false;
					self.start(Action::Join(self.join_value.trim().into()));
				}
			});
		self.join_open &= open;
		let mut open = self.invite_open;
		egui::Window::new("Bjud in till ditt rum")
			.open(&mut open)
			.collapsible(false)
			.default_width(470.0)
			.show(ctx, |ui| {
				ui.label(&self.active_alias);
				ui.label("Länken ger tillgång till servern. Dela den med dem du vill bjuda in.");
				if let Some(invite) = self.session.as_ref().and_then(|s| s.invite.as_ref()) {
					if let Ok(mut link) = invite.encode() {
						ui.add(
							egui::TextEdit::multiline(&mut link)
								.desired_rows(4)
								.interactive(false)
								.desired_width(f32::INFINITY),
						);
						if ui.button("Kopiera inbjudningslänk").clicked() {
							ctx.copy_text(link);
							self.notice =
								"Inbjudan kopierad. Vännen klistrar in den under Anslut.".into();
						}
					}
				}
				ui.label(
					RichText::new("En ny inbjudningsnyckel skapas varje gång servern startas.")
						.small()
						.weak(),
				);
			});
		self.invite_open &= open;
		let mut open = self.channel_open;
		egui::Window::new("Ny textkanal")
			.open(&mut open)
			.collapsible(false)
			.show(ctx, |ui| {
				ui.label("Kanalnamn (a–z, 0–9, bindestreck)");
				ui.text_edit_singleline(&mut self.channel_name);
				if ui.button("Skapa kanal").clicked() {
					self.send(Command::CreateChannel {
						name: self.channel_name.clone(),
					});
					self.channel_open = false;
				}
			});
		self.channel_open &= open;
		self.settings_window(ctx);
	}

	fn settings_window(&mut self, ctx: &egui::Context) {
		let mut open = self.settings;
		egui::Window::new("Ditt Serein").open(&mut open).collapsible(false).resizable(false).default_width(520.0).show(ctx, |ui| {
            ui.horizontal(|ui| { ui.selectable_value(&mut self.settings_tab, 0, "Profil"); ui.selectable_value(&mut self.settings_tab, 1, "Utseende"); ui.selectable_value(&mut self.settings_tab, 2, "Anslutningar"); });
            ui.separator();
            match self.settings_tab {
                0 => {
                    ui.label("Visningsnamn"); ui.add(egui::TextEdit::singleline(&mut self.prefs.profile.name).char_limit(48));
                    ui.label("Användarnamn för dina servrar"); ui.add(egui::TextEdit::singleline(&mut self.prefs.username).char_limit(32));
                    ui.label("Om mig · sparas lokalt i denna version"); ui.add(egui::TextEdit::multiline(&mut self.prefs.bio).desired_rows(3).char_limit(160));
                    ui.horizontal(|ui| { ui.label("Profilfärg"); ui.color_edit_button_srgb(&mut self.prefs.profile.accent); });
                    ui.add_space(12.0);
                    Frame::new().fill(ui.visuals().faint_bg_color).stroke(ui.visuals().widgets.noninteractive.bg_stroke).corner_radius(14).inner_margin(20).show(ui, |ui| {
                        ui.horizontal(|ui| { appearance::avatar(ui, &self.prefs.profile.name, self.prefs.profile.accent, 48.0); ui.vertical(|ui| { ui.heading(&self.prefs.profile.name); ui.label(RichText::new(format!("@{}", self.prefs.username)).weak()); }); });
                        if !self.prefs.bio.is_empty() { ui.label(&self.prefs.bio); }
                    });
                }
                1 => {
                    ui.label("Färgläge"); ui.horizontal(|ui| { ui.selectable_value(&mut self.prefs.appearance.mode, Mode::System, "Följ Windows"); ui.selectable_value(&mut self.prefs.appearance.mode, Mode::Dark, "Mörkt"); ui.selectable_value(&mut self.prefs.appearance.mode, Mode::Light, "Ljust"); });
                    ui.add_space(8.0); ui.label("Accent"); ui.horizontal(|ui| {
                        for color in [[184,168,218], [136,187,178], [213,173,145], [151,181,222], [214,156,179]] {
                            if ui.add(egui::Button::new("   ").fill(Color32::from_rgb(color[0],color[1],color[2])).min_size(egui::vec2(34.0,30.0))).clicked() { self.prefs.appearance.accent = color; }
                        } ui.color_edit_button_srgb(&mut self.prefs.appearance.accent);
                    });
                    ui.add_space(10.0); ui.label("Glas och bakgrund");
                    ui.add(egui::Slider::new(&mut self.prefs.appearance.opacity, 0.35..=1.0).text("Panelernas täthet"));
                    ui.add(egui::Slider::new(&mut self.prefs.appearance.visibility, 0.0..=1.0).text("Bakgrundens synlighet"));
                    let blur = ui.add(egui::Slider::new(&mut self.prefs.appearance.blur, 0.0..=40.0).text("Bakgrundens oskärpa"));
                    if blur.drag_stopped() { self.load_wallpaper(); }
                    ui.horizontal(|ui| {
                        if ui.button("Välj bakgrundsbild…").clicked() {
                            if let Some(path) = rfd::FileDialog::new().add_filter("Bilder", &["png", "jpg", "jpeg", "webp"]).pick_file() { self.prefs.appearance.wallpaper = path.to_string_lossy().into(); self.load_wallpaper(); }
                        }
                        if ui.button("Återställ bakgrund").clicked() { self.prefs.appearance.wallpaper.clear(); self.wallpaper = None; }
                    });
                    ui.label(RichText::new("Glaset visar bakgrunden inuti appen. Välj en bild för justerbar oskärpa.").small().weak());
                }
                _ => {
                    ui.label("Adressbok"); ui.add(egui::TextEdit::singleline(&mut self.prefs.directory).desired_width(f32::INFINITY));
                    ui.label(RichText::new("Standardadressen är lokal för utveckling. För namnanslutning mellan hushåll ska alla använda samma publicerade HTTPS-adressbok.").small().weak());
                    ui.add_space(12.0); ui.label("Eget relä (valfritt)"); ui.add(egui::TextEdit::singleline(&mut self.prefs.relay).hint_text("Tomt = Irohs externa standardrelä").desired_width(f32::INFINITY));
                    ui.label(RichText::new("Byt till er egen HTTPS-reläadress senare. Ingen extern namnuppslagning krävs; vår adressbok och inbjudan innehåller adressen.").small().weak());
                    ui.label(RichText::new("Ändringar används nästa gång du ansluter eller startar servern.").small().weak());
                }
            }
            ui.add_space(14.0);
            if ui.button("Spara inställningar").clicked() {
                if let Err(error) = validate_text(&self.prefs.profile.name, 96).and_then(|_| directory::validate_url(&self.prefs.directory).map(|_| ())) { self.notice = error.to_string(); }
                else { self.save(); if self.session.is_some() { self.send(Command::UpdateProfile(self.prefs.profile.clone())); } self.settings = false; }
            }
        });
		self.settings &= open;
	}
}

impl eframe::App for App {
	fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
		self.poll(&ctx);
		self.prefs.appearance.apply(&ctx);
		self.background(ui);
		ctx.request_repaint_after(Duration::from_millis(
			if self.session.is_some() || self.pending.is_some() || self.wallpaper_job.is_some() {
				100
			} else {
				500
			},
		));
		let panel = self.prefs.appearance.panel(ui.visuals().dark_mode);
		egui::Panel::top("window-controls")
			.exact_size(34.0)
			.resizable(false)
			.frame(
				Frame::new()
					.fill(panel)
					.inner_margin(egui::Margin::symmetric(10, 4)),
			)
			.show(ui, |ui| {
				ui.horizontal(|ui| {
					ui.label(RichText::new("Serein Connect").size(11.0).weak());
					let drag = ui.allocate_response(
						egui::vec2((ui.available_width() - 110.0).max(10.0), 24.0),
						egui::Sense::click_and_drag(),
					);
					if drag.drag_started() {
						ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
					}
					if drag.double_clicked() {
						ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(
							!ctx.input(|i| i.viewport().maximized.unwrap_or(false)),
						));
					}
					appearance::window_controls(ui, &ctx);
				});
			});
		egui::Panel::left("sidebar")
			.exact_size(250.0)
			.resizable(false)
			.frame(Frame::new().fill(panel).inner_margin(16))
			.show(ui, |ui| self.sidebar(ui));
		egui::Panel::top("header")
			.resizable(false)
			.exact_size(58.0)
			.frame(
				Frame::new()
					.fill(panel)
					.inner_margin(egui::Margin::symmetric(24, 12)),
			)
			.show(ui, |ui| {
				ui.horizontal(|ui| {
					ui.strong(if self.home {
						"Hem"
					} else {
						&self.snapshot.name
					});
					if !self.home {
						ui.label(RichText::new(format!("  /  {}", self.channel)).weak());
					}
					ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
						if ui.button("Utseende").clicked() {
							self.settings = true;
							self.settings_tab = 1;
						}
						if !self.home
							&& ui
								.selectable_label(self.show_members, "Medlemmar")
								.clicked()
						{
							self.show_members = !self.show_members;
						}
						if self.session.as_ref().is_some_and(|s| s.invite.is_some())
							&& ui.button("Bjud in").clicked()
						{
							self.invite_open = true;
						}
						if self.demo {
							ui.label(
								RichText::new("FÖRHANDSVISNING · EXEMPELDATA")
									.size(10.0)
									.color(self.prefs.appearance.accent_color()),
							);
						} else if self.session.is_some() {
							ui.label(
								RichText::new("●  Ansluten")
									.size(12.0)
									.color(Color32::from_rgb(138, 190, 164)),
							);
						}
					});
				});
			});
		if !self.notice.is_empty() || self.pending.is_some() {
			egui::Panel::top("notice")
				.resizable(false)
				.frame(
					Frame::new()
						.fill(panel)
						.inner_margin(egui::Margin::symmetric(24, 10)),
				)
				.show(ui, |ui| {
					ui.horizontal_wrapped(|ui| {
						if self.pending.is_some() {
							ui.spinner();
							ui.label("Ansluter…");
							if ui.small_button("Avbryt").clicked() {
								self.leave();
							}
						} else {
							ui.label(RichText::new(&self.notice).size(12.0));
							if ui.small_button("×").clicked() {
								self.notice.clear();
							}
						}
					});
				});
		}
		if !self.home && self.show_members && ui.available_width() > 720.0 {
			egui::Panel::right("members")
				.exact_size(215.0)
				.resizable(false)
				.frame(Frame::new().fill(panel).inner_margin(18))
				.show(ui, |ui| {
					ui.add_space(12.0);
					appearance::eyebrow(
						ui,
						&format!("HÄR JUST NU — {}", self.snapshot.members.len()),
					);
					ui.add_space(10.0);
					for member in &self.snapshot.members {
						ui.horizontal(|ui| {
							appearance::avatar(
								ui,
								&member.profile.name,
								member.profile.accent,
								30.0,
							);
							ui.vertical(|ui| {
								ui.label(&member.profile.name);
								ui.label(
									RichText::new(if member.host {
										"Serverägare"
									} else {
										"Online"
									})
									.size(11.0)
									.weak(),
								);
							});
						});
						ui.add_space(6.0);
					}
					ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
						ui.label(
							RichText::new("Ett litet rum.\nPlats för stora samtal.")
								.size(12.0)
								.weak(),
						);
					});
				});
		}
		egui::CentralPanel::default()
			.frame(Frame::new())
			.show(ui, |ui| {
				if self.home {
					self.home_view(ui);
				} else {
					self.chat_view(ui);
				}
			});
		self.dialogs(&ctx);
	}
	fn on_exit(&mut self) {
		self.save();
		self.leave();
	}
}

fn demo_snapshot() -> Snapshot {
	let albin = Profile {
		name: "Albin".into(),
		accent: [184, 168, 218],
	};
	let mira = Profile {
		name: "Mira".into(),
		accent: [136, 187, 178],
	};
	let sam = Profile {
		name: "Sam".into(),
		accent: [213, 173, 145],
	};
	Snapshot { name: "Studion".into(), channels: vec![Channel { id: "allmant".into(), name: "allmänt".into(), kind: ChannelKind::Text }, Channel { id: "ideer".into(), name: "idéer".into(), kind: ChannelKind::Text }, Channel { id: "projekt".into(), name: "projekt".into(), kind: ChannelKind::Text }],
        members: vec![Member { id: "a".into(), profile: albin.clone(), host: true, voice: None }, Member { id: "m".into(), profile: mira.clone(), host: false, voice: None }, Member { id: "s".into(), profile: sam.clone(), host: false, voice: None }],
        messages: vec![Message { id: 1, channel: "allmant".into(), author_id: "a".into(), author: albin, text: "Välkomna till vårt eget rum.\nHär kan vi samla idéer, bygga saker och bara hänga.".into(), sent_at: 14*3600+32*60 },
            Message { id: 2, channel: "allmant".into(), author_id: "m".into(), author: mira, text: "Gillar lugnet här. Och att vi kan göra det till vårt eget.".into(), sent_at: 14*3600+34*60 },
            Message { id: 3, channel: "allmant".into(), author_id: "s".into(), author: sam, text: "Ska vi lägga skisserna i #projekt? Jag har några idéer till nästa steg.".into(), sent_at: 14*3600+35*60 }] }
}
