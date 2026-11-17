//! Montage mode: edit clips one by one, collect them in a strip, and join
//! them into one file. Projects autosave to the montage folder.

use super::*;
use crate::montage::{self, Montage, Recipe};

/// The open montage project.
pub(super) struct MontageState {
    pub project: Montage,
    pub path: PathBuf,
    /// The card being edited: changes to the open clip save into it.
    pub editing: Option<usize>,
    /// What's on disk, to know when to save.
    pub saved: Montage,
    pub changed_at: Option<Instant>,
}

const STRIP_H: f32 = 74.0;

impl Editor {
    pub(super) fn montage_strip_height(&self) -> f32 {
        if self.montage_on { STRIP_H + 8.0 } else { 0.0 }
    }

    /// The open clip's edits, as a recipe.
    fn capture_recipe(&self) -> Option<Recipe> {
        let m = self.media.as_ref()?;
        let v = m.video.as_ref()?;
        let (s, e) = self.trim()?;
        let o = &self.settings.options;
        Some(Recipe {
            source: m.path.clone(),
            start_text: self.start_text.clone(),
            end_text: self.end_text.clone(),
            until_end: self.until_end,
            zones: self.zones.clone(),
            speed_zones: self.speed_zones.clone(),
            overlays: self.overlays.clone(),
            tracks: self.tracks.clone(),
            av: Some(o.av),
            volume_pct: o.volume_pct,
            change_pitch: o.change_pitch,
            length: job::output_duration(s, e, &self.active_speeds()),
            width: v.width,
            height: v.height,
            fps: v.fps,
        })
    }

    /// Open a card's clip with its edits.
    fn apply_recipe(&mut self, r: &Recipe) {
        self.open(r.source.clone());
        if self.media.as_ref().is_none_or(|m| m.path != r.source) {
            return;
        }
        self.start_text = r.start_text.clone();
        self.end_text = r.end_text.clone();
        self.until_end = r.until_end;
        self.zones = r.zones.clone();
        self.speed_zones = r.speed_zones.clone();
        self.overlays = r.overlays.clone();
        if r.tracks.len() == self.tracks.len() {
            self.tracks = r.tracks.clone();
        }
        let o = &mut self.settings.options;
        if let Some(av) = r.av {
            o.av = av;
        }
        o.volume_pct = r.volume_pct;
        o.change_pitch = r.change_pitch;
    }

    fn set_project(&mut self, project: Montage, path: PathBuf) {
        self.montage = Some(MontageState { saved: project.clone(), project, path, editing: None, changed_at: None });
        self.montage_note = None;
    }

    pub(super) fn add_to_montage(&mut self) {
        if !self.montage_on {
            self.montage_on = true;
        }
        let Some(r) = self.capture_recipe() else {
            self.montage_note = Some(if self.media.as_ref().is_some_and(|m| m.video.is_none()) {
                "A montage needs video clips; this file is audio only.".into()
            } else {
                "Open a clip and set a valid trim first.".into()
            });
            return;
        };
        let Some(st) = self.montage.as_mut() else {
            self.montage_note = Some("Start or open a montage first.".into());
            return;
        };
        st.project.clips.push(r);
        st.editing = None;
        st.changed_at = Some(Instant::now());
        self.montage_note = Some(format!("Added as clip {}. Open the next clip and keep going.", st.project.clips.len()));
    }

    /// Keep the card being edited in step with the open clip, and autosave.
    pub(super) fn sync_montage(&mut self) {
        if !self.montage_on {
            return;
        }
        let current = self.capture_recipe();
        let Some(st) = self.montage.as_mut() else { return };
        if let Some(i) = st.editing {
            match (st.project.clips.get(i), &current) {
                (Some(card), Some(r)) if card.source == r.source => {
                    if card != r {
                        st.project.clips[i] = r.clone();
                        st.changed_at = Some(Instant::now());
                    }
                }
                (Some(card), _) if self.media.as_ref().is_some_and(|m| m.path != card.source) => st.editing = None,
                (None, _) => st.editing = None,
                _ => {}
            }
        }
        if st.project != st.saved && st.changed_at.is_none_or(|t| t.elapsed() > Duration::from_millis(500)) {
            match st.project.save(&st.path) {
                Ok(()) => st.saved = st.project.clone(),
                Err(e) => {
                    self.log.error(&e);
                    self.montage_note = Some(e);
                    st.changed_at = Some(Instant::now() + Duration::from_secs(5));
                }
            }
        } else if st.project != st.saved {
            self.egui_ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn choose_montage_folder(&mut self) {
        let mut d = rfd::FileDialog::new().set_title("Choose where montage projects are saved");
        if let Some(dir) = self.settings.montage_dir.as_ref().or(self.settings.output_dir.as_ref()) {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_folder() {
            self.settings.montage_dir = Some(p);
        }
    }

    /// New / open a montage project (shown while Montage is on with none open).
    pub(super) fn montage_picker(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut chosen: Option<(Montage, PathBuf)> = None;
        egui::Window::new("Montage").open(&mut open).default_width(380.0).resizable(false).collapsible(false).show(ctx, |ui| {
            theme::hint(ui, "Edit a clip, click Add to montage, open the next clip and repeat. Finish montage joins them all into one file.");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Projects folder").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(if self.settings.montage_dir.is_some() { "Change" } else { "Choose" }).clicked() {
                        self.choose_montage_folder();
                    }
                });
            });
            let Some(dir) = self.settings.montage_dir.clone() else {
                theme::hint(ui, "Choose a folder for your montage projects first. Each montage is saved there as you work, so nothing is lost if cheapedit closes.");
                return;
            };
            ui.label(RichText::new(dir.display().to_string()).small().monospace());
            ui.separator();
            ui.label(RichText::new("New montage").strong());
            ui.horizontal(|ui| {
                let w = ui.available_width() - 70.0;
                ui.add(egui::TextEdit::singleline(&mut self.montage_name_text).hint_text("e.g. Valorant highlights").desired_width(w));
                let name = job::sanitize_name(&self.montage_name_text);
                let taken = name.as_ref().is_some_and(|n| montage::project_path(&dir, n).exists());
                if ui.add_enabled(name.is_some() && !taken, egui::Button::new("Create")).clicked() {
                    let n = name.unwrap();
                    chosen = Some((Montage { name: n.clone(), ..Default::default() }, montage::project_path(&dir, &n)));
                }
                if taken {
                    ui.label(RichText::new("taken").small().color(theme::WARN));
                }
            });
            let list = montage::list(&dir);
            if !list.is_empty() {
                ui.separator();
                ui.label(RichText::new("Open a montage").strong());
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for (name, path) in list {
                        if ui.button(&name).clicked() {
                            match Montage::load(&path) {
                                Ok(m) => chosen = Some((m, path)),
                                Err(e) => self.montage_note = Some(e),
                            }
                        }
                    }
                });
            }
            if let Some(n) = &self.montage_note {
                theme::warn(ui, n.clone());
            }
        });
        if let Some((m, path)) = chosen {
            if !path.exists() {
                if let Err(e) = m.save(&path) {
                    self.montage_note = Some(e);
                    return;
                }
            }
            self.log.info(format!("Montage: {}", path.display()));
            self.montage_name_text.clear();
            self.set_project(m, path);
        }
        if !open {
            self.montage_on = false;
        }
    }

    /// The strip of clip cards under the timeline.
    pub(super) fn montage_strip_ui(&mut self, ui: &mut egui::Ui) {
        let Some(st) = &self.montage else {
            let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), STRIP_H), Sense::hover());
            ui.painter().rect_filled(r, 5.0, theme::CARD.gamma_multiply(0.6));
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Start or open a montage", egui::FontId::proportional(13.0), theme::MUTED);
            return;
        };
        let clips = st.project.clips.clone();
        let editing = st.editing;
        let total = st.project.total();
        let (mut open_card, mut remove, mut dup, mut find, mut moved) = (None, None, None, None, None);
        let card_w = 150.0;
        ui.horizontal(|ui| {
            let avail = ui.available_width() - 110.0;
            egui::ScrollArea::horizontal().id_salt("montage_strip").max_width(avail).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut rects = Vec::new();
                    for (i, c) in clips.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(card_w, STRIP_H - 6.0), Sense::click_and_drag());
                        let missing = c.missing();
                        let bg = if missing { theme::DANGER.gamma_multiply(0.25) } else { theme::CARD };
                        let stroke = if editing == Some(i) {
                            Stroke::new(2.0, theme::ACCENT)
                        } else if missing {
                            Stroke::new(1.0, theme::DANGER)
                        } else {
                            Stroke::new(1.0, theme::BORDER)
                        };
                        let p = ui.painter();
                        p.rect_filled(r, 5.0, bg);
                        p.rect_stroke(r, 5.0, stroke, egui::StrokeKind::Inside);
                        p.text(r.left_top() + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP, format!("{}", i + 1), egui::FontId::proportional(16.0), theme::ACCENT);
                        let name: String = c.name().chars().take(18).collect();
                        p.text(r.left_top() + egui::vec2(30.0, 8.0), egui::Align2::LEFT_TOP, name, egui::FontId::proportional(12.0), theme::TEXT);
                        let sub = if missing { "File missing".to_string() } else { timecode::format_short(c.length) };
                        p.text(r.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, sub, egui::FontId::proportional(12.0), if missing { theme::DANGER } else { theme::MUTED });
                        if editing == Some(i) {
                            p.text(r.right_bottom() + egui::vec2(-8.0, -8.0), egui::Align2::RIGHT_BOTTOM, "Editing", egui::FontId::proportional(11.0), theme::ACCENT);
                        }
                        if resp.hovered() && self.montage_drag.is_none() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.drag_started() {
                            self.montage_drag = Some(i);
                        }
                        if resp.clicked() {
                            if missing {
                                find = Some(i);
                            } else {
                                open_card = Some(i);
                            }
                        }
                        let tip = format!("{}\nClick to edit it. Drag to move it. Right-click for more.", c.source.display());
                        resp.clone().on_hover_text(tip);
                        resp.context_menu(|ui| {
                            if missing && ui.button("Find file…").clicked() {
                                find = Some(i);
                                ui.close();
                            }
                            if ui.button("Duplicate").clicked() {
                                dup = Some(i);
                                ui.close();
                            }
                            if ui.button("Remove").clicked() {
                                remove = Some(i);
                                ui.close();
                            }
                        });
                        rects.push(r);
                    }
                    // Dragging: show where it would go, and move it on release.
                    if let Some(from) = self.montage_drag {
                        if let Some(p) = ui.ctx().pointer_interact_pos() {
                            let to = rects.iter().position(|r| p.x < r.center().x).unwrap_or(rects.len());
                            let x = rects.get(to).map(|r| r.left() - 3.0).or(rects.last().map(|r| r.right() + 3.0));
                            if let (Some(x), Some(r0)) = (x, rects.first()) {
                                ui.painter().line_segment([egui::pos2(x, r0.top()), egui::pos2(x, r0.bottom())], Stroke::new(3.0, theme::CORAL));
                            }
                            if ui.input(|i| i.pointer.any_released()) {
                                moved = Some((from, to));
                            }
                        }
                        if !ui.input(|i| i.pointer.any_down()) && moved.is_none() {
                            self.montage_drag = None;
                        }
                    }
                    if clips.is_empty() {
                        ui.label(RichText::new("No clips yet. Edit a clip, then click Add to montage.").color(theme::MUTED));
                    }
                });
            });
            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                ui.label(RichText::new("Total").small().color(theme::MUTED));
                ui.label(RichText::new(timecode::format_short(total)).strong().size(16.0));
                ui.label(RichText::new(format!("{} clip{}", clips.len(), if clips.len() == 1 { "" } else { "s" })).small().color(theme::MUTED));
            });
        });
        let Some(st) = self.montage.as_mut() else { return };
        let mut changed = true;
        if let Some((from, to)) = moved {
            self.montage_drag = None;
            let to = if to > from { to - 1 } else { to };
            if from != to && from < st.project.clips.len() {
                let c = st.project.clips.remove(from);
                st.project.clips.insert(to.min(st.project.clips.len()), c);
                st.editing = st.editing.map(|e| if e == from { to } else if from < e && e <= to { e - 1 } else if to <= e && e < from { e + 1 } else { e });
            }
        } else if let Some(i) = remove {
            st.project.clips.remove(i);
            st.editing = match st.editing {
                Some(e) if e == i => None,
                Some(e) if e > i => Some(e - 1),
                e => e,
            };
        } else if let Some(i) = dup {
            let c = st.project.clips[i].clone();
            st.project.clips.insert(i + 1, c);
        } else {
            changed = false;
        }
        if changed {
            st.changed_at = Some(Instant::now());
        }
        if let Some(i) = find {
            let mut d = rfd::FileDialog::new().set_title("Where is this clip now?").add_filter("Video", VIDEO_EXTS);
            if let Some(dir) = self.settings.clips_dir.as_ref() {
                d = d.set_directory(dir);
            }
            if let Some(p) = d.pick_file() {
                let st = self.montage.as_mut().unwrap();
                st.project.clips[i].source = p;
                st.changed_at = Some(Instant::now());
            }
        } else if let Some(i) = open_card {
            let r = self.montage.as_ref().unwrap().project.clips[i].clone();
            if self.exporting() {
                self.montage_note = Some("Wait for the export to finish first.".into());
            } else {
                self.apply_recipe(&r);
                if let Some(st) = self.montage.as_mut() {
                    st.editing = Some(i);
                }
            }
        }
    }

    /// The Montage section of the tools: project, Add to montage, size.
    pub(super) fn montage_panel_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::section(ui, "Montage");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.montage.is_some() && ui.small_button("Switch").on_hover_text("Start or open another montage").clicked() {
                    self.montage = None;
                }
            });
        });
        let Some(st) = &self.montage else {
            theme::hint(ui, "Start or open a montage in the Montage window.");
            return;
        };
        ui.label(RichText::new(&st.project.name).strong());
        let saved = st.project == st.saved;
        theme::hint(ui, if saved { "Saved automatically." } else { "Saving…" });
        let label = match self.settings.keybinds.get(Action::AddToMontage) {
            Some(b) => format!("Add to montage   ({})", b.display()),
            None => "Add to montage".into(),
        };
        let can_add = self.capture_recipe().is_some();
        if ui
            .add_enabled(can_add, egui::Button::new(label).min_size(egui::vec2(ui.available_width(), 28.0)))
            .on_hover_text("Save this clip with its edits as the next clip in the montage")
            .on_disabled_hover_text("Open a video clip with a valid trim first")
            .clicked()
        {
            self.add_to_montage();
        }
        if let Some(n) = &self.montage_note {
            theme::hint(ui, n.clone());
        }
        if let Some(i) = self.montage.as_ref().and_then(|s| s.editing) {
            theme::hint(ui, format!("Editing clip {}: your changes save into it.", i + 1));
        }
        // Size and frame rate of the finished file.
        let st = self.montage.as_mut().unwrap();
        let first = Montage { frame: None, ..st.project.clone() }.frame();
        let shown = st.project.frame().map(|f| format!("{}x{} {}", f.w, f.h, probe::fmt_fps(f.fps))).unwrap_or_else(|| "from the first clip".into());
        ui.horizontal(|ui| {
            ui.label("Size");
            egui::ComboBox::from_id_salt("montage_size").width(ui.available_width()).selected_text(shown).show_ui(ui, |ui| {
                if ui.selectable_label(st.project.frame.is_none(), "Same as the first clip").clicked() {
                    st.project.frame = None;
                    st.changed_at = Some(Instant::now());
                }
                let fps = first.map_or(30.0, |f| f.fps);
                for (w, h) in [(1920, 1080), (1280, 720), (2560, 1440), (1080, 1920), (1080, 1080)] {
                    for f in [fps, 30.0, 60.0] {
                        if f != fps && [30.0, 60.0].contains(&fps) {
                            continue;
                        }
                        let fr = job::Frame { w, h, fps: f };
                        if ui.selectable_label(st.project.frame == Some(fr), format!("{w}x{h} {}", probe::fmt_fps(f))).clicked() {
                            st.project.frame = Some(fr);
                            st.changed_at = Some(Instant::now());
                        }
                    }
                }
            });
        });
        theme::hint(ui, "Clips that don't match are fitted with black bars, never stretched. Montages always use Precise mode.");
    }

    fn montage_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(st) = &self.montage else {
            return vec!["Start or open a montage first.".into()];
        };
        if st.project.clips.is_empty() {
            out.push("Add at least one clip to the montage.".into());
        }
        for (i, c) in st.project.clips.iter().enumerate() {
            if c.missing() {
                out.push(format!("Clip {}'s file is missing. Click its card to find it, or remove it.", i + 1));
            }
        }
        match self.settings.output_dir.as_deref() {
            None => out.push("Choose an output folder: it's where the montage will be saved.".into()),
            Some(d) if !d.is_dir() => out.push("The output folder doesn't exist any more.".into()),
            Some(_) if !self.dir_status.1 => out.push("cheapedit can't save into that output folder.".into()),
            _ => {}
        }
        match self.caps() {
            None => out.push("Still checking which encoders your FFmpeg has…".into()),
            Some(c) => {
                if let Some(r) = c.video_unavailable_reason(self.settings.options.vcodec) {
                    out.push(r);
                }
            }
        }
        out
    }

    /// The bottom of the tools in Montage mode: problems, Finish, progress.
    pub(super) fn montage_finish_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let problems = self.montage_problems();
        if let Some(st) = &self.montage {
            ui.label(format!("Finished montage: {} long", timecode::format_short(st.project.total())));
        }
        for p in &problems {
            ui.label(RichText::new(p).color(theme::WARN));
        }
        let can = problems.is_empty() && !self.exporting();
        if ui.add_enabled(can, theme::primary_button("Finish montage").min_size(egui::vec2(ui.available_width(), 36.0))).clicked() {
            self.finish_montage(ctx);
        }
    }

    /// Render every clip with its edits, then join them into one file.
    fn finish_montage(&mut self, ctx: &egui::Context) {
        if let Err(e) = self.try_finish_montage(ctx) {
            self.log.error(&e);
            self.last_outcome = Some((Outcome::Failed(e), PathBuf::new()));
        }
    }

    fn try_finish_montage(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let st = self.montage.as_ref().ok_or("No montage open.")?;
        let project = st.project.clone();
        let out_dir = self.settings.output_dir.clone().ok_or("Choose an output folder.")?;
        let caps = self.caps().ok_or("Still checking encoders.")?;
        let frame = project.frame().ok_or("Add a clip first.")?;
        let mut o = self.settings.options.clone();
        o.mode = CutMode::Precise;
        if o.preset.cap_bytes().is_some() {
            let cap = o.preset.cap_bytes().unwrap();
            match discord::plan(cap, project.total(), frame.w, frame.h, frame.fps, true) {
                Ok(p) => o = o.with_plan(&p),
                Err(max) => return Err(format!("Too long for {} even at 480p. Keep the montage under {}.", o.preset.label(), timecode::format_short(max))),
            }
        }
        let container = if o.output_container() == crate::formats::Container::Mp3 { crate::formats::Container::Mp4 } else { o.output_container() };
        let ext = container.ext();
        let enc = caps.pick(o.vcodec, o.use_gpu);
        let name = job::sanitize_name(&job::auto_name(&project.name, winutil::now_stamp())).unwrap_or_else(|| "montage".into());
        let out = job::unique_path(&out_dir, &name, ext);
        let temp = out_dir.join(".cheapedit-montage");
        std::fs::create_dir_all(&temp).map_err(|e| format!("Couldn't make a work folder in the output folder: {e}"))?;
        let mut steps = Vec::new();
        let mut temps = Vec::new();
        let mut parts = Vec::new();
        for (i, r) in project.clips.iter().enumerate() {
            let n = i + 1;
            let m = probe::probe(&self.ffprobe, &r.source).map_err(|e| format!("Clip {n}: {e}"))?;
            if m.video.is_none() {
                return Err(format!("Clip {n} has no picture, so it can't go in a montage."));
            }
            let (s, e) = validate::trim_range(&m, &r.start_text, &r.end_text, r.until_end).map_err(|p| format!("Clip {n}: {}", p.message))?;
            let trim = Some((s, e));
            let first_problem = zoom::check(&r.zones, trim)
                .into_iter()
                .map(|p| p.message)
                .chain(speed::check(&r.speed_zones, trim).into_iter().map(|p| p.message))
                .chain(crate::overlay::check(&r.overlays, trim).into_iter().map(|p| p.message))
                .next();
            if let Some(msg) = first_problem {
                return Err(format!("Clip {n}: {msg} Click its card to fix it."));
            }
            let zones: Vec<zoom::Resolved> = r.zones.iter().filter_map(Zone::resolve).collect();
            let speeds: Vec<speed::Resolved> = r.speed_zones.iter().filter_map(|z| z.resolve(trim)).collect();
            let mut overlays = r.overlays.clone();
            for (k, ov) in overlays.iter_mut().enumerate().filter(|(_, ov)| ov.kind == crate::overlay::Kind::Text) {
                let f = temp.join(format!("text{i}_{k}.txt"));
                std::fs::write(&f, ov.text.text.trim_end()).map_err(|e| format!("Couldn't prepare clip {n}'s text: {e}"))?;
                ov.text_file = Some(f.clone());
                temps.push(f);
            }
            let tracks = if r.tracks.len() == m.audio_tracks.len() { r.tracks.clone() } else { vec![TrackMix::default(); m.audio_tracks.len()] };
            let mut co = o.clone();
            co.av = match r.av.unwrap_or(AvMode::Both) {
                AvMode::AudioOnly => AvMode::Both,
                a => a,
            };
            co.volume_pct = r.volume_pct;
            co.change_pitch = r.change_pitch;
            let part = temp.join(format!("clip{i}.{ext}"));
            let _ = std::fs::remove_file(&part);
            let j = Job { media: &m, opts: &co, start: s, end: e, output: &part, video_encoder: enc, can_tonemap: caps.zscale, zones: &zones, tracks: &tracks, speed_zones: &speeds, overlays: &overlays, montage: Some(frame) };
            steps.push(crate::export::Step { args: job::build_args(&j), total: job::output_duration(s, e, &speeds) });
            temps.push(part.clone());
            parts.push(part);
        }
        let list = temp.join("list.txt");
        std::fs::write(&list, montage::concat_list(&parts)).map_err(|e| format!("Couldn't prepare the join: {e}"))?;
        temps.push(list.clone());
        let total: f64 = steps.iter().map(|s| s.total).sum();
        // The join copies, so it's quick: count it as a small share of the work.
        steps.push(crate::export::Step { args: montage::join_args(&list, &out, matches!(container, crate::formats::Container::Mp4 | crate::formats::Container::Mov)), total: total * 0.05 });
        self.log.info(format!("Finishing montage {:?}: {} clips, {}x{} {} fps", project.name, project.clips.len(), frame.w, frame.h, frame.fps));
        let e = crate::export::Export::start_steps(&self.ffmpeg, steps, out.clone(), temps, self.log.clone(), ctx.clone())?;
        self.export = Some(e);
        self.export_sig = self.export_sig();
        self.export_cap = None;
        self.last_outcome = None;
        self.over_cap = None;
        self.montage_sources = project.clips.iter().map(|c| c.source.clone()).collect();
        self.montage_temp = Some(temp);
        Ok(())
    }
}
