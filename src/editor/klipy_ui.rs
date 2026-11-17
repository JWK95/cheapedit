//! The KLIPY GIF search window and the Features window (where the key goes).

use super::*;
use crate::klipy;

const PARTNER_PANEL: &str = "https://partner.klipy.com";

/// Background work for the search window that isn't the search itself.
#[derive(Default)]
pub(super) struct KlipyUi {
    pub search: klipy::Search,
    pub query: String,
    /// When the search box last changed (searches wait for a short pause).
    pub typed: Option<Instant>,
    /// Thumbnails being downloaded.
    pub fetching: Arc<Mutex<std::collections::HashSet<String>>>,
    /// The GIF being downloaded, then what happened.
    pub downloading: Option<String>,
    pub done: Arc<Mutex<Option<Result<PathBuf, String>>>>,
    pub note: Option<String>,
    pub searched_once: bool,
}

impl Editor {
    fn klipy_key(&self) -> Option<String> {
        let k = self.settings.klipy_key.trim();
        (!k.is_empty()).then(|| k.to_string())
    }

    pub(super) fn open_klipy(&mut self) {
        if self.klipy_key().is_none() {
            // No key yet: show how to get one.
            self.show_features = true;
            return;
        }
        self.show_klipy = true;
    }

    /// Fetch a result's small preview into the temp folder, once.
    fn ensure_thumb(&self, g: &klipy::Gif) -> Option<PathBuf> {
        let path = klipy::thumb_path(g);
        if path.is_file() {
            return Some(path);
        }
        let mut busy = self.klipy.fetching.lock().unwrap();
        if busy.insert(g.id.clone()) {
            let (url, id, fetching, ctx) = (g.thumb_url.clone(), g.id.clone(), self.klipy.fetching.clone(), self.egui_ctx.clone());
            std::thread::spawn(move || {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = klipy::download(&url, &path);
                fetching.lock().unwrap().remove(&id);
                ctx.request_repaint();
            });
        }
        None
    }

    /// Download the full GIF into the Assets folder, then add it as an overlay.
    fn take_gif(&mut self, g: &klipy::Gif) {
        let Some(dir) = self.settings.assets_dir.clone() else {
            self.klipy.note = Some("Choose an Assets folder first (in Overlays): that's where GIFs are saved.".into());
            return;
        };
        if self.klipy.downloading.is_some() {
            return;
        }
        let q = self.klipy.search.state.lock().unwrap().query.clone();
        let file = dir.join(klipy::file_name(&q, &g.title, &g.id, &g.file_ext));
        if file.is_file() {
            // Already downloaded earlier: use it.
            *self.klipy.done.lock().unwrap() = Some(Ok(file));
            return;
        }
        self.klipy.downloading = Some(g.id.clone());
        self.klipy.note = None;
        let (url, done, ctx, log) = (g.file_url.clone(), self.klipy.done.clone(), self.egui_ctx.clone(), self.log.clone());
        std::thread::spawn(move || {
            let res = klipy::download(&url, &file).map(|_| file.clone());
            match &res {
                Ok(f) => log.info(format!("KLIPY GIF saved to {}", f.display())),
                Err(e) => log.error(format!("KLIPY download: {e}")),
            }
            *done.lock().unwrap() = Some(res);
            ctx.request_repaint();
        });
    }

    /// A finished download becomes an overlay at the playhead.
    pub(super) fn poll_klipy(&mut self) {
        let done = self.klipy.done.lock().unwrap().take();
        if let Some(res) = done {
            self.klipy.downloading = None;
            match res {
                Ok(path) => {
                    self.rescan_assets();
                    if self.overlays_blocked_reason().is_some() {
                        self.klipy.note = Some(format!("Saved {} to your Assets folder.", path.file_name().unwrap_or_default().to_string_lossy()));
                    } else {
                        self.add_media_overlay(path);
                        self.klipy.note = Some("Added at the playhead.".into());
                    }
                }
                Err(e) => self.klipy.note = Some(e),
            }
        }
        // Search once typing pauses.
        if let Some(t) = self.klipy.typed {
            if t.elapsed() >= Duration::from_millis(450) {
                self.klipy.typed = None;
                if let Some(key) = self.klipy_key() {
                    self.klipy.search.run(&key, &self.klipy.query.clone(), false, &self.egui_ctx);
                }
            } else {
                self.egui_ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }

    pub(super) fn klipy_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_klipy;
        if open && !self.klipy.searched_once {
            // Trending, to start with.
            self.klipy.searched_once = true;
            if let Some(key) = self.klipy_key() {
                self.klipy.search.run(&key, "", false, ctx);
            }
        }
        let mut take: Option<klipy::Gif> = None;
        let mut more = false;
        egui::Window::new("Search KLIPY").open(&mut open).default_size([520.0, 560.0]).resizable(true).collapsible(false).show(ctx, |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.klipy.query).hint_text("Search KLIPY").desired_width(f32::INFINITY));
            if r.changed() {
                self.klipy.typed = Some(Instant::now());
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.klipy.typed = Some(Instant::now() - Duration::from_secs(1));
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Powered by KLIPY").small().strong().color(theme::MUTED));
                let s = self.klipy.search.state.lock().unwrap();
                let what = if s.query.trim().is_empty() { "Trending".to_string() } else { format!("Results for \u{201c}{}\u{201d}", s.query.trim()) };
                ui.label(RichText::new(what).small().color(theme::MUTED));
                if s.busy {
                    ui.spinner();
                }
            });
            if let Some(n) = &self.klipy.note {
                theme::hint(ui, n.clone());
            }
            if self.klipy.downloading.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Downloading…").small());
                });
            }
            if self.settings.assets_dir.is_none() {
                theme::warn(ui, "Choose an Assets folder in Overlays first: GIFs you pick are saved there.");
            }
            let (results, error, has_next, busy) = {
                let s = self.klipy.search.state.lock().unwrap();
                (s.results.clone(), s.error.clone(), s.has_next, s.busy)
            };
            if let Some(e) = error {
                ui.label(RichText::new(e).color(theme::DANGER));
            }
            if results.is_empty() && !busy && self.klipy.search.state.lock().unwrap().error.is_none() && self.klipy.searched_once {
                theme::hint(ui, "Nothing found. Try other words.");
            }
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let cell = 118.0;
                let cols = ((ui.available_width() + 6.0) / (cell + 6.0)).floor().max(1.0) as usize;
                for row in results.chunks(cols) {
                    ui.horizontal(|ui| {
                        for g in row {
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(cell, cell), Sense::click());
                            ui.painter().rect_filled(rect, 4.0, theme::CARD);
                            let anim = self.ensure_thumb(g).and_then(|p| self.previews.get(&p, false, &self.ffmpeg, ctx));
                            match anim {
                                Some(a) => {
                                    let t = ctx.input(|i| i.time);
                                    let tex = a.frame_at(t, false);
                                    let sz = tex.size_vec2();
                                    let fit = (rect.width() / sz.x).min(rect.height() / sz.y);
                                    let r = egui::Rect::from_center_size(rect.center(), sz * fit);
                                    ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                                    if a.frames.len() > 1 {
                                        ctx.request_repaint_after(Duration::from_millis(50));
                                    }
                                }
                                None => {
                                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(16.0), theme::MUTED);
                                }
                            }
                            if self.klipy.downloading.as_deref() == Some(g.id.as_str()) {
                                ui.painter().rect_filled(rect, 4.0, Color32::from_black_alpha(140));
                            }
                            if resp.hovered() {
                                ui.painter().rect_stroke(rect, 4.0, Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            let tip = if g.title.is_empty() { "Click to add it to your clip".to_string() } else { format!("{}\nClick to add it to your clip", g.title) };
                            if resp.on_hover_text(tip).clicked() {
                                take = Some(g.clone());
                            }
                        }
                    });
                }
                if has_next {
                    ui.add_space(6.0);
                    if ui.add_enabled(!busy, egui::Button::new("More")).clicked() {
                        more = true;
                    }
                }
            });
        });
        self.show_klipy = open;
        if let Some(g) = take {
            self.take_gif(&g);
        }
        if more {
            if let Some(key) = self.klipy_key() {
                self.klipy.search.run(&key, "", true, ctx);
            }
        }
    }

    pub(super) fn features_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_features;
        egui::Window::new("Features").open(&mut open).default_width(460.0).resizable(false).collapsible(false).show(ctx, |ui| {
            theme::section(ui, "KLIPY GIF search");
            theme::hint(ui, "Search millions of GIFs from inside cheapedit and drop them on your clip. It uses your own free KLIPY key, so nobody else can use up your searches.");
            ui.horizontal(|ui| {
                ui.label("API key");
                let w = ui.available_width() - 60.0;
                ui.add(egui::TextEdit::singleline(&mut self.settings.klipy_key).password(!self.klipy_key_visible).desired_width(w).hint_text("Paste your key here"));
                if ui.small_button(if self.klipy_key_visible { "Hide" } else { "Show" }).clicked() {
                    self.klipy_key_visible = !self.klipy_key_visible;
                }
            });
            if self.klipy_key().is_some() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Key saved.").small().color(theme::OK));
                    if ui.small_button("Open the search").clicked() {
                        self.show_klipy = true;
                    }
                });
            }
            ui.add_space(4.0);
            egui::CollapsingHeader::new("How to get a free key").default_open(self.klipy_key().is_none()).show(ui, |ui| {
                ui.label("1. Go to the KLIPY Partner Panel and make a free account:");
                ui.horizontal(|ui| {
                    ui.hyperlink_to(PARTNER_PANEL, PARTNER_PANEL);
                    if ui.small_button("Copy").clicked() {
                        ui.ctx().copy_text(PARTNER_PANEL.into());
                    }
                });
                ui.label("2. Open API Keys and create a platform (call it cheapedit). KLIPY shows your key.");
                ui.label("3. Copy the key and paste it in the box above. That's it.");
                ui.add_space(4.0);
                theme::hint(ui, "Why your own key: a key built into cheapedit could be copied out of the app, and everyone would share one limit. Yours is just yours.");
                theme::hint(ui, "New keys are in Testing mode: 100 searches an hour. KLIPY can lift that if you ask for Production access in the Partner Panel.");
                theme::hint(ui, "The key is kept in cheapedit's settings file on this PC. Only your searches and the key are sent to KLIPY.");
                theme::warn(ui, "Heads-up: KLIPY's API terms say apps shouldn't store their GIFs. cheapedit saves the GIFs you pick into your Assets folder, so if KLIPY objects they could switch your key off. Ask developers@klipy.com if you want to be sure.");
            });
            ui.separator();
            theme::section(ui, "Tracking");
            theme::hint(ui, "Tracked zoom zones follow something on screen. This is where the work runs.");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.settings.tracker_gpu, false, "CPU").on_hover_text("Works on every PC. Usually fast enough.");
                ui.selectable_value(&mut self.settings.tracker_gpu, true, "Graphics card (GPU)").on_hover_text("Uses DirectML. Can be faster on long zones; if it doesn't work on your PC, cheapedit uses the CPU instead.");
            });
            if let Some(n) = &self.tracker_last {
                theme::hint(ui, n.clone());
            }
        });
        self.show_features = open;
    }
}
