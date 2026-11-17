//! The Overlays panel, overlays drawn over the video for the live preview,
//! dragging them into place, and their timeline bands.

use super::*;
use crate::overlay::{self, Kind, Overlay, Pos};

pub(super) const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
pub(super) const OVERLAY_VIDEO_EXTS: &[&str] = &["mp4", "mkv", "mov", "webm", "avi", "m4v", "wmv", "flv", "ts", "mts", "m2ts", "mpg", "mpeg", "3gp", "ogv"];

/// What a drag on the video is doing to the selected overlay.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum OvDrag {
    /// Pointer offset from the overlay's top-left (frame fractions).
    Move((f64, f64)),
    /// Resizing; the opposite corner stays here.
    Resize((f64, f64)),
}

pub(super) fn overlay_color(i: usize) -> Color32 {
    zone_color(i + 3)
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn with_alpha(c: Color32, a: f64) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * c.a() as f64) as u8)
}

/// A small button with a drawn up or down triangle. (The ▲/▼ glyphs
/// aren't in egui's built-in fonts and would show as empty boxes.)
fn arrow_button(ui: &mut egui::Ui, up: bool, enabled: bool) -> egui::Response {
    let r = ui.add_enabled(enabled, egui::Button::new("    ").small());
    let c = r.rect.center();
    let col = if enabled { theme::TEXT } else { theme::MUTED.gamma_multiply(0.6) };
    let (w, h) = (4.5, 3.5);
    let pts = if up {
        vec![egui::pos2(c.x - w, c.y + h), egui::pos2(c.x + w, c.y + h), egui::pos2(c.x, c.y - h)]
    } else {
        vec![egui::pos2(c.x - w, c.y - h), egui::pos2(c.x + w, c.y - h), egui::pos2(c.x, c.y + h)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
    r
}

fn color_edit(ui: &mut egui::Ui, c: &mut [u8; 3]) {
    let mut col = rgb(*c);
    if ui.color_edit_button_srgba(&mut col).changed() {
        *c = [col.r(), col.g(), col.b()];
    }
}

impl Editor {
    /// Overlays work when the export re-encodes a picture (like zoom).
    pub(super) fn overlays_blocked_reason(&self) -> Option<&'static str> {
        let o = self.effective_opts();
        match &self.media {
            None => Some("Open a video first."),
            Some(m) if m.video.is_none() => Some("This file has no picture to put things on."),
            _ if o.mode == CutMode::Fast => Some("Switch to Precise to add overlays. Fast mode copies the video untouched."),
            _ if o.av == AvMode::AudioOnly => Some("Overlays need video, and you've chosen Audio only."),
            _ => None,
        }
    }

    /// The video's height / width.
    fn frame_aspect(&self) -> f64 {
        self.media.as_ref().and_then(|m| m.video.as_ref()).map_or(9.0 / 16.0, |v| v.height as f64 / v.width.max(1) as f64)
    }

    pub(super) fn font_list(&mut self) -> &[crate::fonts::Font] {
        if self.fonts.is_none() {
            self.fonts = Some(crate::fonts::installed());
        }
        self.fonts.as_deref().unwrap_or(&[])
    }

    /// Where a new overlay goes: from the playhead (inside the trim) for `len` seconds.
    fn new_span(&self, len: f64) -> Option<(f64, f64)> {
        let (ts, te) = self.trim()?;
        let mut s = self.position().clamp(ts, te);
        let e = (s + len).min(te);
        if e - s < 0.5 {
            s = (e - len).max(ts);
        }
        Some((s, e))
    }

    fn push_overlay(&mut self, o: Overlay) {
        if self.overlays.len() >= overlay::MAX_OVERLAYS {
            return;
        }
        self.overlays.push(o);
        self.select_overlay(self.overlays.len() - 1, false);
    }

    pub(super) fn select_overlay(&mut self, i: usize, end_pos: bool) {
        self.sel_overlay = Some(i);
        self.edit_end_pos = end_pos;
        self.sel_zone = None;
        self.sel_speed = None;
        self.drawing = false;
    }

    pub(super) fn delete_selected_overlay(&mut self) {
        if let Some(i) = self.sel_overlay.take() {
            if i < self.overlays.len() {
                self.overlays.remove(i);
            }
        }
        self.ov_drag = None;
    }

    pub(super) fn new_text_overlay(&mut self) {
        if self.overlays_blocked_reason().is_some() {
            return;
        }
        let Some((s, e)) = self.new_span(3.0) else { return };
        let mut o = Overlay::text(s, e);
        let fonts = self.font_list();
        if let Some(f) = fonts.iter().find(|f| f.name == "Arial Bold").or_else(|| fonts.iter().find(|f| f.name == "Arial")).or(fonts.first()) {
            o.text.font_name = f.name.clone();
            o.text.font_file = Some(f.file.clone());
        }
        self.push_overlay(o);
    }

    /// Add a picture, GIF or video file as an overlay at the playhead.
    pub(super) fn add_media_overlay(&mut self, path: PathBuf) {
        if self.overlays_blocked_reason().is_some() {
            return;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let fa = self.frame_aspect();
        let made = if IMAGE_EXTS.contains(&ext.as_str()) {
            let kind = match ext.as_str() {
                "gif" => Kind::Animated,
                "webp" if crate::previews::webp_is_animated(&path) => Kind::Animated,
                _ => Kind::Image,
            };
            match crate::previews::image_size(&path) {
                Some((w, h)) => self.new_span(3.0).map(|(s, e)| Overlay::media(kind, path.clone(), h as f64 / w.max(1) as f64, fa, false, s, e)),
                None => {
                    self.open_error = Some(format!("Couldn't read {} as a picture.", path.display()));
                    None
                }
            }
        } else {
            match probe::probe(&self.ffprobe, &path) {
                Ok(m) => match &m.video {
                    Some(v) => self.new_span(m.duration.clamp(0.5, 10.0)).map(|(s, e)| Overlay::media(Kind::Video, path.clone(), v.height as f64 / v.width.max(1) as f64, fa, m.audio.is_some(), s, e)),
                    None => {
                        self.open_error = Some(format!("{} has no picture, so it can't be an overlay.", path.display()));
                        None
                    }
                },
                Err(e) => {
                    self.open_error = Some(e);
                    None
                }
            }
        };
        if let Some(o) = made {
            self.log.info(format!("Overlay added: {}", path.display()));
            self.push_overlay(o);
        }
    }

    fn pick_overlay_file(&mut self, video: bool) {
        let mut d = rfd::FileDialog::new().set_title(if video { "Choose a video to overlay" } else { "Choose a picture or GIF" });
        d = if video { d.add_filter("Videos", OVERLAY_VIDEO_EXTS) } else { d.add_filter("Pictures and GIFs", IMAGE_EXTS) };
        if let Some(dir) = self.settings.assets_dir.as_ref().or(self.settings.last_open_dir.as_ref()) {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_file() {
            self.add_media_overlay(p);
        }
    }

    /// The files in the Assets folder (pictures, GIFs, videos), newest first.
    fn assets(&mut self) -> &[PathBuf] {
        let dir = self.settings.assets_dir.clone();
        if self.assets_for != dir || self.assets_scanned.elapsed() > Duration::from_secs(10) {
            let mut list: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
            if let Some(d) = &dir {
                for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                    let p = e.path();
                    let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                    if p.is_file() && (IMAGE_EXTS.contains(&ext.as_str()) || OVERLAY_VIDEO_EXTS.contains(&ext.as_str())) {
                        let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                        list.push((p, t));
                    }
                }
            }
            list.sort_by(|a, b| b.1.cmp(&a.1));
            self.assets = list.into_iter().map(|x| x.0).collect();
            self.assets_for = dir;
            self.assets_scanned = Instant::now();
        }
        &self.assets
    }

    pub(super) fn rescan_assets(&mut self) {
        self.assets_scanned = Instant::now() - Duration::from_secs(60);
    }

    fn choose_assets_folder(&mut self) {
        let mut d = rfd::FileDialog::new().set_title("Choose your Assets folder (GIFs, pictures, videos to put on clips)");
        if let Some(dir) = self.settings.assets_dir.as_ref().or(self.settings.last_open_dir.as_ref()) {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_folder() {
            self.settings.assets_dir = Some(p);
            self.rescan_assets();
        }
    }

    // ------------------------------------------------------------------ panel

    pub(super) fn overlays_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::section(ui, "Overlays");
            if !self.overlays.is_empty() {
                ui.label(RichText::new(format!("{} / {}", self.overlays.len(), overlay::MAX_OVERLAYS)).small().color(theme::MUTED));
            }
        });
        let blocked = self.overlays_blocked_reason();
        let pos = self.position();
        let trim = self.trim();
        let fin_map = self.time_map();
        let fin_s0 = trim.map_or(0.0, |t| t.0);
        let sel = self.sel_overlay;
        let edit_end = self.edit_end_pos;
        let full = self.overlays.len() >= overlay::MAX_OVERLAYS;
        let fonts: Vec<crate::fonts::Font> = if self.overlays.iter().any(|o| o.kind == Kind::Text) { self.font_list().to_vec() } else { Vec::new() };
        enum Act {
            Select(usize, bool),
            SelectQuiet(usize),
            Delete(usize),
            Move(usize, isize),
            StartPan(usize),
            AddText,
            AddFile(bool),
            AddAsset(PathBuf),
            Klipy,
        }
        let mut act: Option<Act> = None;
        ui.add_enabled_ui(blocked.is_none(), |ui| {
            if self.overlays.is_empty() {
                theme::hint(ui, "Put pictures, GIFs, videos or text on top of your clip. Drag them on the video to place them.");
            }
            let count = self.overlays.len();
            for (i, o) in self.overlays.iter_mut().enumerate() {
                let col = overlay_color(i);
                let selected = sel == Some(i);
                let frame = theme::card().stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { col } else { theme::BORDER }));
                let r = frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 5.0, col);
                        if ui.selectable_label(selected, RichText::new(format!("{} {}", o.kind.label(), i + 1)).strong().color(col)).on_hover_text("Select and jump to its start").clicked() {
                            act = Some(Act::Select(i, false));
                        }
                        ui.add(egui::Label::new(RichText::new(o.name()).small().color(theme::MUTED)).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Delete").clicked() {
                                act = Some(Act::Delete(i));
                            }
                            if arrow_button(ui, true, i + 1 < count).on_hover_text("Bring forward (on top of the next overlay)").clicked() {
                                act = Some(Act::Move(i, 1));
                            }
                            if arrow_button(ui, false, i > 0).on_hover_text("Send back (under the previous overlay)").clicked() {
                                act = Some(Act::Move(i, -1));
                            }
                        });
                    });
                    // Times
                    ui.columns(2, |cols| {
                        for (c, (label, text)) in [("Start", &mut o.start_text), ("End", &mut o.end_text)].into_iter().enumerate() {
                            let ui = &mut cols[c];
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(label).small().color(theme::MUTED));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("Use playhead").clicked() {
                                        *text = timecode::format_compact(pos);
                                    }
                                });
                            });
                            ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                            time_feedback(ui, text);
                            if let Some(f) = final_text(&fin_map, fin_s0, text) {
                                theme::hint(ui, f);
                            }
                        }
                    });
                    // Position
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if ui.add(egui::Button::selectable(selected && !(edit_end && o.pan), "Start position")).on_hover_text("Show where it starts, to drag it into place").clicked() {
                            act = Some(Act::Select(i, false));
                        }
                        if ui
                            .add_enabled(o.pan, egui::Button::selectable(selected && edit_end && o.pan, "End position"))
                            .on_hover_text("Show where it ends up, to drag it there")
                            .on_disabled_hover_text("No position change is on")
                            .clicked()
                        {
                            act = Some(Act::Select(i, true));
                        }
                        if ui.selectable_label(!o.pan, "No position change").on_hover_text("On: it stays put. Off: it glides from the start position to an end position.").clicked() {
                            if o.pan {
                                o.pan = false;
                            } else {
                                act = Some(Act::StartPan(i));
                            }
                        }
                    });
                    if o.kind != Kind::Text {
                        ui.horizontal(|ui| {
                            ui.label("Size");
                            let mut pct = o.w * 100.0;
                            if ui.add(egui::Slider::new(&mut pct, overlay::MIN_W * 100.0..=100.0).suffix("% wide").max_decimals(0)).changed() {
                                o.w = pct / 100.0;
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("Opacity");
                        ui.add(egui::Slider::new(&mut o.opacity, 0..=100).suffix("%"));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Fade in");
                        ui.add(egui::DragValue::new(&mut o.fade_in).range(0.0..=2.0).speed(0.01).max_decimals(2).suffix(" s"));
                        ui.label("Fade out");
                        ui.add(egui::DragValue::new(&mut o.fade_out).range(0.0..=2.0).speed(0.01).max_decimals(2).suffix(" s"));
                    });
                    match o.kind {
                        Kind::Animated => {
                            ui.horizontal(|ui| {
                                ui.selectable_value(&mut o.play_once, false, "Loop");
                                ui.selectable_value(&mut o.play_once, true, "Play once").on_hover_text("Plays through once, then holds the last frame");
                            });
                        }
                        Kind::Video => {
                            if o.has_audio {
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut o.audio_on, "Sound").on_hover_text("Mix this video's own sound into the clip");
                                    ui.add_enabled(o.audio_on, egui::Slider::new(&mut o.volume_pct, 0..=200).suffix("%"));
                                });
                            } else {
                                theme::hint(ui, "This video has no sound.");
                            }
                            ui.checkbox(&mut o.follow_speed, "Follow clip speed")
                                .on_hover_text("On: it slows down and speeds up with your speed zones. Off: it always plays at normal speed.");
                            theme::hint(ui, "The preview shows a still from the video. The export plays it.");
                        }
                        Kind::Text => {
                            ui.add(egui::TextEdit::multiline(&mut o.text.text).desired_rows(2).desired_width(f32::INFINITY));
                            ui.horizontal(|ui| {
                                ui.label("Font");
                                egui::ComboBox::from_id_salt(("font", i)).width(ui.available_width()).selected_text(&o.text.font_name).show_ui(ui, |ui| {
                                    for f in &fonts {
                                        if ui.selectable_label(o.text.font_name == f.name, &f.name).clicked() {
                                            o.text.font_name = f.name.clone();
                                            o.text.font_file = Some(f.file.clone());
                                        }
                                    }
                                });
                            });
                            ui.horizontal(|ui| {
                                ui.label("Size");
                                let mut pct = o.text.size * 100.0;
                                if ui.add(egui::Slider::new(&mut pct, 2.0..=30.0).suffix("% tall").max_decimals(1)).changed() {
                                    o.text.size = pct / 100.0;
                                }
                                color_edit(ui, &mut o.text.color);
                            });
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut o.text.outline, "Outline");
                                if o.text.outline {
                                    color_edit(ui, &mut o.text.outline_color);
                                    let mut pct = o.text.outline_w * 100.0;
                                    if ui.add(egui::DragValue::new(&mut pct).range(1.0..=25.0).speed(0.2).max_decimals(0).suffix("% thick")).changed() {
                                        o.text.outline_w = pct / 100.0;
                                    }
                                }
                                ui.checkbox(&mut o.text.shadow, "Shadow");
                            });
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut o.text.background, "Background box");
                                if o.text.background {
                                    color_edit(ui, &mut o.text.bg_color);
                                    ui.add(egui::Slider::new(&mut o.text.bg_opacity, 0..=100).suffix("%"));
                                }
                            });
                        }
                        Kind::Image => {}
                    }
                });
                if !selected && r.response.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
                    if act.is_none() {
                        act = Some(Act::SelectQuiet(i));
                    }
                }
            }
            // Add buttons
            ui.add_enabled_ui(!full, |ui| {
                ui.columns(3, |cols| {
                    if cols[0].add(egui::Button::new("+ Image or GIF").min_size(egui::vec2(cols[0].available_width(), 26.0))).clicked() {
                        act = Some(Act::AddFile(false));
                    }
                    if cols[1].add(egui::Button::new("+ Video").min_size(egui::vec2(cols[1].available_width(), 26.0))).clicked() {
                        act = Some(Act::AddFile(true));
                    }
                    let label = match self.settings.keybinds.get(Action::NewTextOverlay) {
                        Some(b) => format!("+ Text ({})", b.display()),
                        None => "+ Text".into(),
                    };
                    if cols[2].add(egui::Button::new(label).min_size(egui::vec2(cols[2].available_width(), 26.0))).clicked() {
                        act = Some(Act::AddText);
                    }
                });
            });
            let klipy_label = match self.settings.keybinds.get(Action::OpenKlipy) {
                Some(b) => format!("Search KLIPY for a GIF   ({})", b.display()),
                None => "Search KLIPY for a GIF".into(),
            };
            if ui
                .add_enabled(!full, egui::Button::new(klipy_label).min_size(egui::vec2(ui.available_width(), 26.0)))
                .on_hover_text("Find a GIF online and drop it on your clip")
                .clicked()
            {
                act = Some(Act::Klipy);
            }
            if full {
                theme::hint(ui, "You can have up to 20 overlays. Delete one to add another.");
            }
            // Assets folder
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Assets").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.settings.assets_dir.is_some() {
                        if ui.small_button("Open").on_hover_text("Open the Assets folder in Explorer").clicked() {
                            if let Some(d) = &self.settings.assets_dir {
                                winutil::open_folder(d);
                            }
                        }
                        if ui.small_button("Refresh").clicked() {
                            self.rescan_assets();
                        }
                        if ui.small_button("Change").clicked() {
                            act = Some(Act::AddAsset(PathBuf::new()));
                        }
                    }
                });
            });
            if self.settings.assets_dir.is_none() {
                theme::hint(ui, "Keep your GIFs, pictures and videos in one folder, and they'll be listed here to add with one click.");
                if ui.button("Choose assets folder").clicked() {
                    act = Some(Act::AddAsset(PathBuf::new()));
                }
            } else {
                let list = self.assets().to_vec();
                if list.is_empty() {
                    theme::hint(ui, "Nothing in this folder yet. Pictures, GIFs and videos you put there appear here.");
                }
                egui::ScrollArea::vertical().id_salt("assets").max_height(140.0).show(ui, |ui| {
                    for p in list {
                        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_uppercase();
                        let name = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                        let r = ui
                            .add_enabled(!full, egui::Button::new(RichText::new(format!("{ext:<4}  {name}")).small()).frame(false))
                            .on_hover_text("Click to add it as an overlay at the playhead");
                        if r.clicked() {
                            act = Some(Act::AddAsset(p));
                        }
                    }
                });
            }
        });
        if let Some(why) = blocked {
            if self.media.is_some() {
                theme::warn(ui, why);
            }
        }
        match act {
            None => {}
            Some(Act::Select(i, end)) => {
                self.select_overlay(i, end);
                if let Some((s, e)) = self.overlays[i].times() {
                    let fps = self.media.as_ref().and_then(|m| m.video.as_ref()).map(|v| v.fps).unwrap_or(30.0).max(1.0);
                    self.pause();
                    self.seek(if end { (e - 1.0 / fps).max(s) } else { s });
                }
            }
            Some(Act::SelectQuiet(i)) => self.select_overlay(i, false),
            Some(Act::Delete(i)) => {
                self.sel_overlay = Some(i);
                self.delete_selected_overlay();
            }
            Some(Act::Move(i, d)) => {
                let j = (i as isize + d) as usize;
                self.overlays.swap(i, j);
                self.sel_overlay = Some(j);
            }
            Some(Act::StartPan(i)) => {
                let o = &mut self.overlays[i];
                o.pan = true;
                o.b = o.a;
                o.b_drawn = false;
                self.select_overlay(i, true);
                if let Some((s, e)) = self.overlays[i].times() {
                    self.pause();
                    self.seek((e - 0.05).max(s));
                }
            }
            Some(Act::AddText) => self.new_text_overlay(),
            Some(Act::Klipy) => self.open_klipy(),
            Some(Act::AddFile(video)) => self.pick_overlay_file(video),
            Some(Act::AddAsset(p)) => {
                if p.as_os_str().is_empty() {
                    self.choose_assets_folder();
                } else {
                    self.add_media_overlay(p);
                }
            }
        }
    }

    // ------------------------------------------------------------------ on the video

    /// Make sure a text overlay's font is loaded into egui; returns its family.
    fn text_family(&mut self, file: Option<&Path>, ctx: &egui::Context) -> egui::FontFamily {
        let Some(file) = file else { return egui::FontFamily::Proportional };
        let name = format!("ov:{}", file.display());
        if !self.egui_fonts.contains(&name) {
            self.egui_fonts.insert(name.clone());
            if let Ok(bytes) = std::fs::read(file) {
                ctx.add_font(egui::epaint::text::FontInsert {
                    name: name.clone(),
                    data: egui::FontData::from_owned(bytes),
                    families: vec![egui::epaint::text::InsertFontFamily { family: egui::FontFamily::Name(name.clone().into()), priority: egui::epaint::text::FontPriority::Highest }],
                });
                // Fonts take effect next frame.
                ctx.request_repaint();
                return egui::FontFamily::Proportional;
            }
            self.bad_fonts.insert(name.clone());
        }
        if self.bad_fonts.contains(&name) { egui::FontFamily::Proportional } else { egui::FontFamily::Name(name.into()) }
    }

    /// Draw one overlay at `p` (frame fractions) with `alpha`; returns its rect.
    fn paint_overlay(&mut self, painter: &egui::Painter, vr: egui::Rect, i: usize, p: Pos, alpha: f64, anim_t: f64) -> egui::Rect {
        let o = self.overlays[i].clone();
        let tl = egui::pos2(vr.left() + p.x as f32 * vr.width(), vr.top() + p.y as f32 * vr.height());
        if o.kind == Kind::Text {
            let ctx = painter.ctx().clone();
            let family = self.text_family(o.text.font_file.as_deref(), &ctx);
            let px = (o.text.size * vr.height() as f64) as f32;
            let font = egui::FontId::new(px.max(4.0), family);
            let galley = |c: Color32| painter.layout_no_wrap(o.text.text.clone(), font.clone(), c);
            let main = galley(with_alpha(rgb(o.text.color), alpha));
            let rect = egui::Rect::from_min_size(tl, main.size());
            if o.text.background {
                painter.rect_filled(rect.expand(px / 4.0), 0.0, with_alpha(rgb(o.text.bg_color), alpha * o.text.bg_opacity as f64 / 100.0));
            }
            if o.text.shadow {
                let d = (px / 15.0).max(1.0);
                painter.galley(tl + egui::vec2(d, d), galley(with_alpha(Color32::BLACK, alpha * 0.6)), Color32::BLACK);
            }
            if o.text.outline {
                let w = (o.text.outline_w as f32 * px).max(1.0);
                let g = galley(with_alpha(rgb(o.text.outline_color), alpha));
                for k in 0..12 {
                    let a = k as f32 / 12.0 * std::f32::consts::TAU;
                    painter.galley(tl + egui::vec2(a.cos() * w, a.sin() * w), g.clone(), Color32::BLACK);
                }
            }
            painter.galley(tl, main, Color32::WHITE);
            return rect;
        }
        let w = o.w as f32 * vr.width();
        let rect = egui::Rect::from_min_size(tl, egui::vec2(w, w * o.aspect as f32));
        let anim = o.path.as_ref().and_then(|path| self.previews.get(path, o.kind == Kind::Video, &self.ffmpeg, painter.ctx()));
        match anim {
            Some(a) => {
                let tex = a.frame_at(anim_t, o.play_once);
                painter.image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), with_alpha(Color32::WHITE, alpha));
            }
            None => {
                painter.rect_filled(rect, 2.0, with_alpha(Color32::from_gray(60), alpha * 0.7));
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Loading…", egui::FontId::proportional(12.0), Color32::WHITE);
            }
        }
        rect
    }

    /// Overlays over the video (both views while editing), and dragging the
    /// selected one. Returns true if it used the pointer.
    pub(super) fn overlays_on_video(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: egui::Rect, vr: egui::Rect) -> bool {
        if self.overlays.is_empty() || self.overlays_blocked_reason().is_some() || (self.view == View::Result && self.exported_file.is_some()) {
            return false;
        }
        let painter = ui.painter_at(rect);
        let pos = self.position();
        let map = self.time_map();
        let editing_still = self.paused();
        let mut sel_rect = None;
        for i in 0..self.overlays.len() {
            let o = &self.overlays[i];
            let Some((s, e)) = o.times() else { continue };
            let selected = self.sel_overlay == Some(i);
            let active = pos >= s && pos < e;
            // While paused, the selected overlay sits at the position being edited.
            let (p, alpha, anim_t) = if selected && editing_still {
                let p = if self.edit_end_pos && o.pan { o.end_pos() } else { o.a };
                // Full opacity while in its time (faint outside it), ignoring fades, so it's easy to place.
                (p, if active { (o.opacity as f64 / 100.0).max(0.35) } else { 0.35 }, 0.0)
            } else if active {
                let (u, len) = if o.follows_speed() { (pos - s, e - s) } else { (map.to_out(pos) - map.to_out(s), map.out_len(s, e)) };
                (overlay::pos_at(o, u / len.max(1e-6)), overlay::alpha_at(o, u, len), u)
            } else {
                continue;
            };
            let r = self.paint_overlay(&painter, vr, i, p, alpha, anim_t);
            if selected {
                sel_rect = Some(r);
            }
        }
        let Some(i) = self.sel_overlay.filter(|&i| i < self.overlays.len()) else { return false };
        let Some(r) = sel_rect else { return false };
        let col = overlay_color(i);
        let o = self.overlays[i].clone();
        painter.rect_stroke(r, 0.0, Stroke::new(2.0, col), egui::StrokeKind::Outside);
        let media = o.kind != Kind::Text;
        if media {
            for c in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()] {
                painter.rect_filled(egui::Rect::from_center_size(c, egui::vec2(8.0, 8.0)), 1.0, col);
            }
        }
        let mut label = format!("{} {}", o.kind.label(), i + 1);
        if o.pan {
            label.push_str(if self.edit_end_pos { " - end position" } else { " - start position" });
        }
        painter.text(r.left_top() + egui::vec2(0.0, -4.0), egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(12.0), col);
        if !editing_still {
            return false;
        }

        // ---- dragging
        let to_frac = |p: egui::Pos2| (((p.x - vr.left()) / vr.width()) as f64, ((p.y - vr.top()) / vr.height()) as f64);
        let cur = if self.edit_end_pos && o.pan { o.b } else { o.a };
        let (w, h) = (r.width() as f64 / vr.width() as f64, r.height() as f64 / vr.height() as f64);
        let corners = [
            (r.left_top(), (cur.x + w, cur.y + h), egui::CursorIcon::ResizeNwSe),
            (r.right_top(), (cur.x, cur.y + h), egui::CursorIcon::ResizeNeSw),
            (r.left_bottom(), (cur.x + w, cur.y), egui::CursorIcon::ResizeNeSw),
            (r.right_bottom(), (cur.x, cur.y), egui::CursorIcon::ResizeNwSe),
        ];
        let corner_at = |p: egui::Pos2| if media { corners.iter().find(|c| c.0.distance(p) < 10.0).copied() } else { None };
        let mut used = false;
        if let Some(p) = resp.hover_pos() {
            if let Some(c) = corner_at(p) {
                ui.ctx().set_cursor_icon(c.2);
                used = true;
            } else if r.contains(p) || self.ov_drag.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
                used = true;
            }
        }
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                let f = to_frac(p);
                self.ov_drag = if let Some(c) = corner_at(p) {
                    Some(OvDrag::Resize(c.1))
                } else if r.contains(p) {
                    Some(OvDrag::Move((f.0 - cur.x, f.1 - cur.y)))
                } else {
                    None
                };
            }
        }
        if resp.dragged() {
            if let (Some(d), Some(p)) = (self.ov_drag, resp.interact_pointer_pos()) {
                let f = to_frac(p);
                let ratio = self.frame_aspect(); // frame h / w
                let ov = &mut self.overlays[i];
                let np = match d {
                    OvDrag::Move(off) => Pos { x: (f.0 - off.0).clamp(-w + 0.02, 0.98), y: (f.1 - off.1).clamp(-h + 0.02, 0.98) },
                    OvDrag::Resize(anchor) => {
                        let nw = (f.0 - anchor.0).abs().clamp(overlay::MIN_W, 1.0);
                        let nh = nw * ov.aspect / ratio;
                        ov.w = nw;
                        Pos { x: if f.0 < anchor.0 { anchor.0 - nw } else { anchor.0 }, y: if f.1 < anchor.1 { anchor.1 - nh } else { anchor.1 } }
                    }
                };
                if self.edit_end_pos && ov.pan {
                    ov.b = np;
                    ov.b_drawn = true;
                } else {
                    ov.a = np;
                    if !ov.pan {
                        ov.b = np;
                    }
                }
                used = true;
            }
        }
        if resp.drag_stopped() && self.ov_drag.take().is_some() {
            used = true;
        }
        used || self.ov_drag.is_some()
    }

    // ------------------------------------------------------------------ timeline

    pub(super) fn show_overlay_bands(&self) -> bool {
        !self.overlays.is_empty() && !(self.view == View::Result && self.exported_file.is_some())
    }

    /// One band per overlay, stacked in a single row like zones. Click to
    /// select, drag an edge to change its time.
    pub(super) fn overlay_bands_ui(&mut self, ui: &mut egui::Ui, axis: &Axis) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, theme::CARD.gamma_multiply(0.6));
        let enabled = self.overlays_blocked_reason().is_none();
        let n = self.overlays.len().max(1);
        // Overlays may overlap in time, so each gets its own thin lane.
        let lane_h = (rect.height() / n as f32).max(3.0);
        let mut spans: Vec<(usize, egui::Rect)> = Vec::new();
        for (i, o) in self.overlays.iter().enumerate() {
            let Some((s, e)) = o.times() else { continue };
            if e <= s {
                continue;
            }
            let y0 = rect.top() + i as f32 * lane_h;
            let r = egui::Rect::from_x_y_ranges(axis.x_of(s)..=axis.x_of(e).max(axis.x_of(s) + 2.0), y0..=(y0 + lane_h - 0.5).min(rect.bottom()));
            let sel = self.sel_overlay == Some(i);
            let mut col = overlay_color(i).gamma_multiply(if sel { 0.95 } else { 0.55 });
            if !enabled {
                col = col.gamma_multiply(0.4);
            }
            painter.rect_filled(r, 1.0, col);
            if sel {
                painter.rect_stroke(r, 1.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
            }
            if r.height() >= 11.0 && r.width() > 14.0 {
                painter.text(r.center(), egui::Align2::CENTER_CENTER, format!("{}", i + 1), egui::FontId::proportional(10.0), Color32::BLACK);
            }
            spans.push((i, r));
        }
        if !enabled {
            return;
        }
        let hit = |p: egui::Pos2| spans.iter().rev().find(|s| s.1.expand2(egui::vec2(5.0, 0.0)).contains(p)).copied();
        let edge_at = |p: egui::Pos2| -> Option<(usize, bool)> {
            let (i, r) = hit(p)?;
            if (p.x - r.left()).abs() < 5.0 {
                Some((i, true))
            } else if (p.x - r.right()).abs() < 5.0 {
                Some((i, false))
            } else {
                None
            }
        };
        if let Some(p) = resp.hover_pos() {
            if edge_at(p).is_some() || self.ov_band_drag.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            } else if let Some((i, _)) = hit(p) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                let o = &self.overlays[i];
                if let Some((s, e)) = o.times() {
                    resp.clone().on_hover_text(format!("{} {}: {} - {}  {}", o.kind.label(), i + 1, timecode::format_short(s), timecode::format_short(e), o.name()));
                }
            }
        }
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.ov_band_drag = edge_at(p);
                if let Some((i, _)) = self.ov_band_drag {
                    self.select_overlay(i, false);
                }
            }
        }
        if resp.dragged() {
            if let (Some((i, is_start)), Some(p)) = (self.ov_band_drag, resp.interact_pointer_pos()) {
                let t = axis.t_of(p.x);
                let o = &mut self.overlays[i];
                if is_start {
                    o.start_text = timecode::format_compact(t);
                } else {
                    o.end_text = timecode::format_compact(t);
                }
                self.scrub_to(t);
            }
        }
        if resp.drag_stopped() {
            self.ov_band_drag = None;
            self.end_scrub();
        }
        if resp.clicked() {
            if let Some((i, _)) = resp.interact_pointer_pos().and_then(hit) {
                self.select_overlay(i, false);
                if let Some((s, _)) = self.overlays[i].times() {
                    self.seek(s);
                }
            }
        }
    }

    /// Copies of the overlays for an export, with each text written to a file
    /// (drawtext reads it from there, so no escaping can go wrong).
    pub(super) fn overlays_for_export(&self) -> Result<Vec<Overlay>, String> {
        if self.overlays_blocked_reason().is_some() {
            return Ok(Vec::new());
        }
        let dir = std::env::temp_dir().join("cheapedit-text");
        let mut out = self.overlays.clone();
        for (i, o) in out.iter_mut().enumerate().filter(|(_, o)| o.kind == Kind::Text) {
            std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't prepare the text overlays: {e}"))?;
            let f = dir.join(format!("text{i}.txt"));
            std::fs::write(&f, o.text.text.trim_end()).map_err(|e| format!("Couldn't prepare the text overlays: {e}"))?;
            o.text_file = Some(f);
        }
        Ok(out)
    }
}
