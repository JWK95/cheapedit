//! The main editor screen: preview on the left, tools on the right.

use crate::export::{Export, Outcome};
use crate::formats::{ACodec, Caps, Container, VCodec};
use crate::job::{self, AvMode, CutMode, EncSpeed, FrameRate, Job, Options, Quality, Resolution};
use crate::keybinds::Action;
use crate::keymap_ui::KeymapUi;
use crate::log::{Kind, Log};
use crate::probe::{self, MediaInfo};
use crate::settings::Settings;
use crate::theme;
use crate::timecode;
use crate::validate::{self, Fix};
use crate::video_view::VideoView;
use crate::winutil;
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VIDEO_EXTS: &[&str] = &[
    "mp4", "mkv", "mov", "webm", "avi", "m4v", "wmv", "flv", "ts", "mts", "m2ts", "mpg", "mpeg", "3gp", "ogv", "mp3", "wav", "m4a", "flac", "aac", "ogg", "opus",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Original,
    Result,
}

/// What the preview should currently look like. When this changes, mpv is updated.
#[derive(Clone, PartialEq, Debug)]
struct PreviewState {
    file: Option<PathBuf>,
    exported: bool,
    range: Option<(f64, f64)>,
    speed: f64,
    pitch_correct: bool,
    volume: u32,
    mute: bool,
    no_audio: bool,
    no_video: bool,
}

/// Everything that affects an export; if it changes, an exported preview is stale.
#[derive(Clone, PartialEq)]
struct ExportSig {
    file: PathBuf,
    start: String,
    end: String,
    until_end: bool,
    opts: Options,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Start,
    End,
    Scrub,
}

pub enum EditorAction {
    None,
    OpenSetup,
    ShowAbout,
}

pub struct Editor {
    settings: Settings,
    saved: Settings,
    last_save: Instant,
    pub log: Log,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    caps: Arc<Mutex<Option<Caps>>>,
    video: Option<VideoView>,
    media: Option<MediaInfo>,
    open_error: Option<String>,
    start_text: String,
    end_text: String,
    until_end: bool,
    name_text: String,
    view: View,
    exported_file: Option<PathBuf>,
    export_sig: Option<ExportSig>,
    applied: Option<PreviewState>,
    loaded: Option<PathBuf>,
    export: Option<Export>,
    last_outcome: Option<(Outcome, PathBuf)>,
    format_note: Option<String>,
    show_log: bool,
    show_keys: bool,
    keymap: KeymapUi,
    drag: Option<Drag>,
    dir_status: (Option<PathBuf>, bool, Option<u64>),
    dir_checked: Instant,
    pending_open: Option<PathBuf>,
}

impl Editor {
    pub fn new(ffmpeg: PathBuf, ffprobe: PathBuf, mpv: Arc<crate::mpv::Api>, log: Log, ctx: &egui::Context) -> Editor {
        let settings = Settings::load();
        let caps = Arc::new(Mutex::new(None));
        {
            let (caps, ffmpeg, ctx, log) = (caps.clone(), ffmpeg.clone(), ctx.clone(), log.clone());
            std::thread::spawn(move || {
                let c = Caps::detect(&ffmpeg);
                let mut gpu: Vec<_> = c.working_gpu.iter().cloned().collect();
                gpu.sort();
                log.info(format!("Encoder check done. Working GPU encoders: {}", if gpu.is_empty() { "none".into() } else { gpu.join(", ") }));
                *caps.lock().unwrap() = Some(c);
                ctx.request_repaint();
            });
        }
        let (video, open_error) = match VideoView::new(mpv) {
            Ok(v) => (Some(v), None),
            Err(e) => {
                log.error(&e);
                (None, Some(e))
            }
        };
        Editor {
            saved: settings.clone(),
            settings,
            last_save: Instant::now(),
            log,
            ffmpeg,
            ffprobe,
            caps,
            video,
            media: None,
            open_error,
            start_text: "0".into(),
            end_text: String::new(),
            until_end: false,
            name_text: String::new(),
            view: View::Original,
            exported_file: None,
            export_sig: None,
            applied: None,
            loaded: None,
            export: None,
            last_outcome: None,
            format_note: None,
            show_log: false,
            show_keys: false,
            keymap: KeymapUi::default(),
            drag: None,
            dir_status: (None, false, None),
            dir_checked: Instant::now() - Duration::from_secs(60),
            pending_open: None,
        }
    }

    pub fn request_open(&mut self, path: PathBuf) {
        self.pending_open = Some(path);
    }

    pub fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some(e) = &self.export {
            e.cancel();
        }
        if let Some(v) = &mut self.video {
            v.destroy_gl(gl);
        }
        self.settings.save();
    }

    pub fn title(&self) -> String {
        match &self.media {
            Some(m) => format!("cheapedit — {}", m.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()),
            None => "cheapedit".into(),
        }
    }

    pub fn settings_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }

    // ------------------------------------------------------------------ state helpers

    /// The options actually used, after the file's limits are applied
    /// (e.g. a file with no audio can't be "audio only").
    fn effective_opts(&self) -> Options {
        let mut o = self.settings.options.clone();
        if let Some(m) = &self.media {
            if m.video.is_none() {
                o.av = AvMode::AudioOnly;
            } else if m.audio.is_none() && o.av != AvMode::Both {
                o.av = AvMode::Both;
            }
        }
        o
    }

    fn caps(&self) -> Option<Caps> {
        self.caps.lock().unwrap().clone()
    }

    fn trim(&self) -> Option<(f64, f64)> {
        let m = self.media.as_ref()?;
        validate::trim_range(m, &self.start_text, &self.end_text, self.until_end).ok()
    }

    fn position(&self) -> f64 {
        self.video.as_ref().and_then(|v| v.player.get_f64("time-pos")).unwrap_or(0.0)
    }

    fn paused(&self) -> bool {
        self.video.as_ref().and_then(|v| v.player.get_flag("pause")).unwrap_or(true)
    }

    fn export_sig(&self) -> Option<ExportSig> {
        Some(ExportSig {
            file: self.media.as_ref()?.path.clone(),
            start: self.start_text.clone(),
            end: self.end_text.clone(),
            until_end: self.until_end,
            opts: self.effective_opts(),
        })
    }

    fn refresh_dir_status(&mut self) {
        let dir = self.settings.output_dir.clone();
        let changed = dir != self.dir_status.0;
        if changed || self.dir_checked.elapsed() > Duration::from_secs(5) {
            let writable = dir.as_deref().map(winutil::is_writable_dir).unwrap_or(false);
            let free = dir.as_deref().and_then(winutil::free_disk_space);
            self.dir_status = (dir, writable, free);
            self.dir_checked = Instant::now();
        }
    }

    fn output_path_preview(&self) -> Option<PathBuf> {
        let dir = self.settings.output_dir.as_ref()?;
        let m = self.media.as_ref()?;
        let (s, e) = self.trim()?;
        let o = self.effective_opts();
        let name = job::sanitize_name(&self.name_text).unwrap_or_else(|| job::auto_name(&m.path, s, e, o.effective_speed()));
        Some(job::unique_path(dir, &name, o.output_container().ext()))
    }

    fn problems(&self) -> Vec<validate::Problem> {
        let o = self.effective_opts();
        let caps = self.caps();
        let space = match (&self.media, self.trim(), self.dir_status.2) {
            (Some(m), Some((s, e)), Some(free)) => {
                let out = PathBuf::new();
                let j = Job { media: m, opts: &o, start: s, end: e, output: &out, video_encoder: None, can_tonemap: false };
                Some((job::estimate_bytes(&j), free))
            }
            _ => None,
        };
        validate::check(&validate::Input {
            media: self.media.as_ref(),
            start_text: &self.start_text,
            end_text: &self.end_text,
            until_end: self.until_end,
            opts: &o,
            output_dir: self.settings.output_dir.as_deref(),
            caps: caps.as_ref(),
            space,
            dir_writable: self.dir_status.1,
        })
    }

    pub fn exporting(&self) -> bool {
        self.export.as_ref().is_some_and(|e| e.progress().outcome.is_none())
    }

    // ------------------------------------------------------------------ actions

    fn open_dialog(&mut self) {
        let mut d = rfd::FileDialog::new().set_title("Open a video").add_filter("Video and audio", VIDEO_EXTS).add_filter("All files", &["*"]);
        if let Some(dir) = &self.settings.last_open_dir {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_file() {
            self.open(p);
        }
    }

    fn open(&mut self, path: PathBuf) {
        if self.exporting() {
            self.open_error = Some("Wait for the export to finish (or cancel it) before opening another file.".into());
            return;
        }
        self.log.info(format!("Opening {}", path.display()));
        match probe::probe(&self.ffprobe, &path) {
            Ok(m) => {
                self.log.info(format!("{}: {:.2}s, {}", path.display(), m.duration, m.summary()));
                self.settings.last_open_dir = path.parent().map(Path::to_path_buf);
                self.start_text = "0".into();
                self.end_text = timecode::format_compact(m.duration);
                self.until_end = false;
                self.name_text.clear();
                self.view = View::Original;
                self.exported_file = None;
                self.export_sig = None;
                self.last_outcome = None;
                self.open_error = None;
                self.applied = None;
                self.loaded = None;
                if m.video.as_ref().is_some_and(|v| v.hdr) {
                    self.log.info("HDR video: exports will be converted to standard colour (SDR).");
                }
                self.media = Some(m);
            }
            Err(e) => {
                self.log.error(format!("{}: {e}", path.display()));
                self.open_error = Some(e);
            }
        }
    }

    fn choose_folder(&mut self) {
        let mut d = rfd::FileDialog::new().set_title("Choose where exports go");
        if let Some(dir) = self.settings.output_dir.as_ref().or(self.settings.last_open_dir.as_ref()) {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_folder() {
            self.settings.output_dir = Some(p);
        }
    }

    fn apply_fix(&mut self, fix: Fix) {
        match fix {
            Fix::UseEndOfVideo => self.until_end = true,
            Fix::SwitchToPrecise => self.settings.options.mode = CutMode::Precise,
            Fix::ChooseFolder => self.choose_folder(),
            Fix::SwitchToMkv => self.settings.options.container = Container::Mkv,
        }
    }

    fn start_export(&mut self, ctx: &egui::Context) {
        if self.exporting() || !self.problems().is_empty() {
            return;
        }
        let (Some(m), Some((start, end)), Some(out), Some(caps)) = (self.media.clone(), self.trim(), self.output_path_preview(), self.caps()) else {
            return;
        };
        let o = self.effective_opts();
        let encodes_video = o.mode == CutMode::Precise && o.av != AvMode::AudioOnly && m.video.is_some();
        let enc = if encodes_video { caps.pick(o.vcodec, o.use_gpu) } else { None };
        let j = Job { media: &m, opts: &o, start, end, output: &out, video_encoder: enc, can_tonemap: caps.zscale };
        let args = job::build_args(&j);
        let total = job::output_duration(start, end, o.effective_speed());
        if let Some(e) = enc {
            self.log.info(format!("Encoding video with {e}"));
        }
        match Export::start(&self.ffmpeg, args, out.clone(), total, self.log.clone(), ctx.clone()) {
            Ok(e) => {
                self.export = Some(e);
                self.export_sig = self.export_sig();
                self.last_outcome = None;
            }
            Err(e) => {
                self.log.error(&e);
                self.last_outcome = Some((Outcome::Failed(e), out));
            }
        }
    }

    fn handle_action(&mut self, a: Action, ctx: &egui::Context) {
        let Some(v) = &self.video else { return };
        let has_media = self.media.is_some();
        match a {
            Action::PlayPause if has_media => v.player.command(&["cycle", "pause"]),
            Action::FrameBack if has_media => v.player.command(&["frame-back-step"]),
            Action::FrameForward if has_media => v.player.command(&["frame-step"]),
            Action::SeekBack if has_media => v.player.command(&["seek", "-1", "relative+exact"]),
            Action::SeekForward if has_media => v.player.command(&["seek", "1", "relative+exact"]),
            Action::SetStart if self.can_mark() => {
                self.start_text = timecode::format_compact(self.position());
            }
            Action::SetEnd if self.can_mark() => {
                self.end_text = timecode::format_compact(self.position());
                self.until_end = false;
            }
            Action::ToggleView if has_media => self.view = if self.view == View::Original { View::Result } else { View::Original },
            Action::Export => self.start_export(ctx),
            Action::OpenFile => self.open_dialog(),
            Action::ShowShortcuts => self.show_keys = true,
            _ => {}
        }
    }

    /// Start/end can be marked while viewing the source (Original, or the live Result preview).
    fn can_mark(&self) -> bool {
        self.media.is_some() && !(self.view == View::Result && self.exported_file.is_some())
    }

    // ------------------------------------------------------------------ preview

    fn desired_preview(&self) -> PreviewState {
        let o = self.effective_opts();
        let src = self.media.as_ref().map(|m| m.path.clone());
        let neutral = |file: Option<PathBuf>, exported: bool| PreviewState {
            file,
            exported,
            range: None,
            speed: 1.0,
            pitch_correct: true,
            volume: 100,
            mute: false,
            no_audio: false,
            no_video: false,
        };
        match self.view {
            View::Original => neutral(src, false),
            View::Result => match &self.exported_file {
                Some(f) => neutral(Some(f.clone()), true),
                None => PreviewState {
                    file: src,
                    exported: false,
                    range: self.trim(),
                    speed: o.effective_speed(),
                    pitch_correct: !o.change_pitch,
                    volume: if o.mode == CutMode::Fast { 100 } else { o.volume_pct },
                    mute: o.av == AvMode::Mute,
                    no_audio: o.av == AvMode::RemoveAudio,
                    no_video: o.av == AvMode::AudioOnly,
                },
            },
        }
    }

    fn apply_preview(&mut self) {
        // An export made with different settings no longer shows "the result".
        if self.exported_file.is_some() && self.export_sig != self.export_sig() {
            self.exported_file = None;
        }
        let want = self.desired_preview();
        if self.applied.as_ref() == Some(&want) {
            return;
        }
        let Some(v) = &self.video else { return };
        let p = &v.player;
        let prev = self.applied.clone();
        if want.file != self.loaded {
            if let Some(f) = &want.file {
                p.load_file(f);
            }
            self.loaded = want.file.clone();
        }
        p.set_property("speed", &want.speed.to_string());
        p.set_property("audio-pitch-correction", if want.pitch_correct { "yes" } else { "no" });
        p.set_property("volume", &want.volume.to_string());
        p.set_property("mute", if want.mute { "yes" } else { "no" });
        p.set_property("aid", if want.no_audio { "no" } else { "auto" });
        p.set_property("vid", if want.no_video { "no" } else { "auto" });
        match want.range {
            Some((a, b)) => {
                p.set_property("ab-loop-a", &a.to_string());
                p.set_property("ab-loop-b", &b.to_string());
                // Jump into the range when the preview starts or the range moves.
                let range_changed = prev.as_ref().map(|x| x.range) != Some(want.range);
                let pos = self.position();
                if range_changed && (pos < a || pos > b) {
                    p.command(&["seek", &a.to_string(), "absolute+exact"]);
                }
            }
            None => {
                p.set_property("ab-loop-a", "no");
                p.set_property("ab-loop-b", "no");
            }
        }
        self.applied = Some(want);
    }

    // ------------------------------------------------------------------ UI

    pub fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) -> EditorAction {
        let ctx = ui.ctx().clone();
        let mut action = EditorAction::None;

        if let Some(p) = self.pending_open.take() {
            self.open(p);
        }
        if let Some(v) = &self.video {
            let ev = v.player.poll_events();
            if ev.file_loaded {
                self.applied = None; // re-apply track/loop settings to the new file
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = dropped.into_iter().next() {
            self.open(p);
        }
        if self.keymap.capturing() {
            // The key-capture dialog gets the keys, not the shortcuts.
        } else {
            for a in self.settings.keybinds.triggered(&ctx) {
                self.handle_action(a, &ctx);
            }
        }
        // Finished export → show the result.
        if let Some(e) = &self.export {
            let pr = e.progress();
            if let Some(outcome) = pr.outcome {
                if outcome == Outcome::Done {
                    self.exported_file = Some(e.output.clone());
                    self.view = View::Result;
                }
                self.last_outcome = Some((outcome, e.output.clone()));
                self.export = None;
            }
        }
        self.refresh_dir_status();

        egui::Panel::top("topbar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("cheapedit").strong().size(16.0).color(theme::ACCENT));
                if winutil::is_dev() {
                    dev_badge(ui);
                }
                ui.add_space(8.0);
                if ui.button("Open video").on_hover_text(self.binding_hint(Action::OpenFile)).clicked() {
                    self.open_dialog();
                }
                if let Some(m) = &self.media {
                    ui.label(RichText::new(m.path.file_name().unwrap_or_default().to_string_lossy()).strong());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("About").on_hover_text("Version and updates").clicked() {
                        action = EditorAction::ShowAbout;
                    }
                    if ui.button("Setup").on_hover_text("Check FFmpeg and libmpv").clicked() {
                        action = EditorAction::OpenSetup;
                    }
                    if ui.button("Keybindings").clicked() {
                        self.show_keys = true;
                    }
                    let log_label = if self.show_log { "Hide log" } else { "Log" };
                    if ui.button(log_label).clicked() {
                        self.show_log = !self.show_log;
                    }
                });
            });
            ui.add_space(2.0);
        });

        egui::Panel::right("tools").resizable(false).exact_size(370.0).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                // Keep content clear of the scrollbar.
                ui.set_max_width(ui.available_width() - 14.0);
                ui.add_space(4.0);
                self.tools_ui(ui, &ctx);
                ui.add_space(12.0);
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.preview_ui(ui, frame);
        });

        if self.show_log {
            self.log_window(&ctx);
        }
        if self.show_keys {
            let mut open = true;
            self.keymap.show(&ctx, &mut self.settings.keybinds, &mut open);
            self.show_keys = open;
        }

        self.apply_preview();
        if self.settings != self.saved && self.last_save.elapsed() > Duration::from_millis(500) {
            self.settings.save();
            self.saved = self.settings.clone();
            self.last_save = Instant::now();
        }
        if self.exporting() || (!self.paused() && self.media.is_some()) {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        action
    }

    fn binding_hint(&self, a: Action) -> String {
        match self.settings.keybinds.get(a) {
            Some(b) => format!("Shortcut: {}", b.display()),
            None => "No shortcut set".into(),
        }
    }

    // ------------------------------------------------------------------ left side

    fn preview_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let has_media = self.media.is_some();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(has_media, |ui| {
                ui.selectable_value(&mut self.view, View::Original, "  Original  ");
                ui.selectable_value(&mut self.view, View::Result, "  Result  ");
            });
            if self.view == View::Result && has_media {
                let (txt, col) = if self.exported_file.is_some() { ("Exported file", theme::OK) } else { ("Live preview", theme::CORAL) };
                ui.label(RichText::new(txt).small().color(col));
                if let Some(f) = &self.exported_file {
                    if ui.small_button("Open in folder").clicked() {
                        winutil::reveal_in_explorer(f);
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(m) = &self.media {
                    ui.label(RichText::new(m.summary()).small().color(theme::MUTED));
                }
            });
        });
        if self.view == View::Result && has_media && self.exported_file.is_none() {
            theme::hint(ui, "Live preview shows your trim, speed and audio choices. Quality and format changes only show in the exported file.");
        }
        ui.add_space(4.0);

        // Leave room below the video for the timeline, controls and shortcut strip.
        let below = 160.0;
        let avail = ui.available_size();
        let video_h = (avail.y - below).max(120.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail.x, video_h), Sense::click());
        if let Some(v) = &mut self.video {
            if has_media {
                v.paint(ui, frame, rect);
                let audio_only = self.applied.as_ref().is_some_and(|a| a.no_video) || self.media.as_ref().is_some_and(|m| m.video.is_none());
                if audio_only {
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Audio only", egui::FontId::proportional(20.0), theme::MUTED);
                }
            } else {
                ui.painter().rect_filled(rect, 4.0, Color32::BLACK);
            }
            if let Some(e) = &v.error {
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, e, egui::FontId::proportional(15.0), theme::DANGER);
            }
        } else {
            ui.painter().rect_filled(rect, 4.0, Color32::BLACK);
        }
        if !has_media {
            let msg = self.open_error.clone().unwrap_or_else(|| "Open a video, or drag one onto this window".into());
            let col = if self.open_error.is_some() { theme::DANGER } else { theme::MUTED };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(17.0), col);
            if resp.clicked() {
                self.open_dialog();
            }
        } else if let Some(e) = &self.open_error {
            ui.painter().text(rect.center_top() + egui::vec2(0.0, 20.0), egui::Align2::CENTER_CENTER, e, egui::FontId::proportional(14.0), theme::DANGER);
        }
        if resp.clicked() && has_media {
            self.handle_action(Action::PlayPause, &ui.ctx().clone());
        }

        ui.add_space(6.0);
        self.timeline_ui(ui);
        ui.add_space(4.0);
        self.transport_ui(ui);
        ui.add_space(2.0);
        self.shortcut_strip(ui);
    }

    fn timeline_ui(&mut self, ui: &mut egui::Ui) {
        let exported_view = self.view == View::Result && self.exported_file.is_some();
        let duration = if exported_view {
            self.video.as_ref().and_then(|v| v.player.get_f64("duration")).unwrap_or(1.0)
        } else {
            self.media.as_ref().map(|m| m.duration).unwrap_or(1.0)
        }
        .max(0.001);
        let pos = self.position();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_filled(rect, 5.0, theme::CARD);
        let x_of = |t: f64| rect.left() + (t / duration).clamp(0.0, 1.0) as f32 * rect.width();
        let t_of = |x: f32| (((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64) * duration;

        let range = if exported_view { None } else { self.trim() };
        if let Some((s, e)) = range {
            let r = egui::Rect::from_x_y_ranges(x_of(s)..=x_of(e), rect.y_range());
            painter.rect_filled(r, 0.0, theme::ACCENT_DIM.gamma_multiply(0.8));
            for x in [x_of(s), x_of(e)] {
                painter.rect_filled(egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(6.0, rect.height() + 4.0)), 2.0, theme::CORAL);
            }
        }
        if self.media.is_some() {
            let px = x_of(pos);
            painter.line_segment([egui::pos2(px, rect.top() - 2.0), egui::pos2(px, rect.bottom() + 2.0)], Stroke::new(2.0, Color32::WHITE));
        }

        if self.media.is_some() {
            // Holding a timeline-click modifier (default Ctrl = start, Shift = end)
            // puts that point where you click, and dragging keeps moving it.
            let mods = ui.input(|i| i.modifiers);
            let click_mark = if self.can_mark() { self.settings.keybinds.click_action(mods) } else { None };
            if resp.drag_started() || resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    self.drag = Some(Drag::Scrub);
                    if let Some(a) = click_mark {
                        self.drag = Some(if a == Action::TimelineStart { Drag::Start } else { Drag::End });
                    } else if let Some((s, e)) = range {
                        if (p.x - x_of(s)).abs() < 8.0 {
                            self.drag = Some(Drag::Start);
                        } else if (p.x - x_of(e)).abs() < 8.0 {
                            self.drag = Some(Drag::End);
                        }
                    }
                }
            }
            if resp.dragged() || resp.clicked() {
                if let (Some(d), Some(p)) = (self.drag, resp.interact_pointer_pos()) {
                    let t = t_of(p.x);
                    match d {
                        Drag::Start => self.start_text = timecode::format_compact(t),
                        Drag::End => {
                            self.end_text = timecode::format_compact(t);
                            self.until_end = false;
                        }
                        Drag::Scrub => {}
                    }
                    if let Some(v) = &self.video {
                        v.player.command(&["seek", &t.to_string(), "absolute+exact"]);
                    }
                }
            }
            if resp.drag_stopped() || resp.clicked() {
                self.drag = None;
            }
            if let Some(p) = resp.hover_pos() {
                let near_handle = range.is_some_and(|(s, e)| (p.x - x_of(s)).abs() < 8.0 || (p.x - x_of(e)).abs() < 8.0);
                ui.ctx().set_cursor_icon(if near_handle || click_mark.is_some() { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::PointingHand });
                // Show where the start/end would land.
                if let Some(a) = click_mark {
                    let x = p.x.clamp(rect.left(), rect.right());
                    painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], Stroke::new(2.0, theme::CORAL));
                    let label = format!("{} {}", if a == Action::TimelineStart { "Start" } else { "End" }, timecode::format_short(t_of(x)));
                    let (lx, align) = if x > rect.right() - 90.0 { (x - 5.0, egui::Align2::RIGHT_TOP) } else { (x + 5.0, egui::Align2::LEFT_TOP) };
                    painter.text(egui::pos2(lx, rect.top() + 3.0), align, label, egui::FontId::proportional(11.5), Color32::WHITE);
                }
            }
        }
        let click_hint = |a: Action| self.settings.keybinds.get(a).map(|b| b.display());
        let hint = match (click_hint(Action::TimelineStart), click_hint(Action::TimelineEnd)) {
            (Some(s), Some(e)) => format!("{s}: set start  ·  {e}: set end"),
            (Some(s), None) => format!("{s}: set start"),
            (None, Some(e)) => format!("{e}: set end"),
            (None, None) => String::new(),
        };
        ui.horizontal(|ui| {
            let mono = |t: f64| RichText::new(timecode::format_short(t)).small().monospace().color(theme::MUTED);
            let left = ui.label(mono(0.0)).rect;
            if self.media.is_some() && !hint.is_empty() {
                // Centred under the timeline, regardless of the end-time label's width.
                let y = left.center().y;
                ui.painter().text(egui::pos2(rect.center().x, y), egui::Align2::CENTER_CENTER, &hint, egui::FontId::proportional(12.0), theme::MUTED);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(mono(duration));
            });
        });
    }

    fn transport_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let has = self.media.is_some();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(has, |ui| {
                if ui.button("‹ Frame").on_hover_text(self.binding_hint(Action::FrameBack)).clicked() {
                    self.handle_action(Action::FrameBack, &ctx);
                }
                let play = if self.paused() { "  Play  " } else { " Pause " };
                if ui.button(RichText::new(play).strong()).on_hover_text(self.binding_hint(Action::PlayPause)).clicked() {
                    self.handle_action(Action::PlayPause, &ctx);
                }
                if ui.button("Frame ›").on_hover_text(self.binding_hint(Action::FrameForward)).clicked() {
                    self.handle_action(Action::FrameForward, &ctx);
                }
                ui.add_space(10.0);
                ui.add_enabled_ui(self.can_mark(), |ui| {
                    if ui.button("Set start").on_hover_text(self.binding_hint(Action::SetStart)).clicked() {
                        self.handle_action(Action::SetStart, &ctx);
                    }
                    if ui.button("Set end").on_hover_text(self.binding_hint(Action::SetEnd)).clicked() {
                        self.handle_action(Action::SetEnd, &ctx);
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if has {
                    ui.label(RichText::new(timecode::format(self.position())).monospace().size(15.0));
                }
            });
        });
    }

    fn shortcut_strip(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for a in Action::ALL.iter().copied().filter(|a| a.in_strip()) {
                if let Some(b) = self.settings.keybinds.get(a) {
                    key_cap(ui, &b.display());
                    ui.label(RichText::new(a.short()).small().color(theme::MUTED));
                    ui.add_space(6.0);
                }
            }
            let all = match self.settings.keybinds.get(Action::ShowShortcuts) {
                Some(b) => format!("All shortcuts ({})", b.display()),
                None => "All shortcuts".into(),
            };
            if ui.small_button(all).clicked() {
                self.show_keys = true;
            }
        });
    }

    // ------------------------------------------------------------------ right side

    fn tools_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let caps = self.caps();
        let media = self.media.clone();
        let busy = self.exporting();

        // 1. Output folder
        ui.horizontal(|ui| {
            theme::section(ui, "Output folder");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.settings.output_dir.is_none() {
                    ui.label(RichText::new(" Required ").small().color(theme::DANGER).background_color(theme::DANGER.gamma_multiply(0.15)));
                }
            });
        });
        ui.horizontal(|ui| {
            let text = self.settings.output_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "Not chosen yet".into());
            let w = ui.available_width() - 70.0;
            ui.allocate_ui_with_layout(egui::vec2(w, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(text).monospace().small()).truncate());
            });
            if ui.button("Browse").clicked() {
                self.choose_folder();
            }
        });
        theme::hint(ui, "Your original file is never changed.");
        ui.separator();

        // 2. Name
        theme::section(ui, "Output name");
        let auto = match (&media, self.trim()) {
            (Some(m), Some((s, e))) => job::auto_name(&m.path, s, e, self.effective_opts().effective_speed()),
            _ => "automatic name".into(),
        };
        ui.add(egui::TextEdit::singleline(&mut self.name_text).hint_text(auto).desired_width(f32::INFINITY));
        match self.output_path_preview() {
            Some(p) => theme::hint(ui, format!("Saves as {}", p.file_name().unwrap_or_default().to_string_lossy())),
            None => theme::hint(ui, "Leave blank to use the automatic name."),
        }
        ui.separator();

        // 3. Trim
        theme::section(ui, "Trim");
        ui.columns(2, |cols| {
            cols[0].label(RichText::new("Start").small().color(theme::MUTED));
            cols[0].add(egui::TextEdit::singleline(&mut self.start_text).desired_width(f32::INFINITY));
            time_feedback(&mut cols[0], &self.start_text);
            cols[1].label(RichText::new("End").small().color(theme::MUTED));
            cols[1].add_enabled(!self.until_end, egui::TextEdit::singleline(&mut self.end_text).desired_width(f32::INFINITY));
            if self.until_end {
                if let Some(m) = &media {
                    theme::hint(&mut cols[1], timecode::format(m.duration));
                }
            } else {
                time_feedback(&mut cols[1], &self.end_text);
            }
        });
        ui.checkbox(&mut self.until_end, "Until end of video");
        theme::hint(ui, "Type 25, 1:25 or 0:01:25.500. Times are from the original video.");
        if let Some((s, e)) = self.trim() {
            let len = job::output_duration(s, e, self.effective_opts().effective_speed());
            ui.label(format!("New clip length: {}", if len < 60.0 { format!("{len:.2} s") } else { timecode::format(len) }));
        }
        ui.separator();

        // 4. Cut mode
        theme::section(ui, "Cut mode");
        let o = &mut self.settings.options;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut o.mode, CutMode::Precise, "  Precise  ");
            ui.selectable_value(&mut o.mode, CutMode::Fast, "  Fast  ");
        });
        match o.mode {
            CutMode::Precise => theme::hint(ui, "Cuts are exact to the frame. Slower, and quality depends on your settings."),
            CutMode::Fast => theme::hint(
                ui,
                "Done in about a second with no quality loss, but the start snaps to the nearest earlier keyframe, so your clip may begin slightly early.",
            ),
        }
        let fast = o.mode == CutMode::Fast;
        ui.separator();

        // 5. Speed
        theme::section(ui, "Speed");
        ui.add_enabled_ui(!fast, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut o.speed, 0.25..=4.0).step_by(0.25).show_value(false));
                ui.add(egui::DragValue::new(&mut o.speed).range(0.1..=16.0).speed(0.01).max_decimals(2).suffix("×"));
                if o.speed != 1.0 && ui.small_button("Reset").clicked() {
                    o.speed = 1.0;
                }
            });
            ui.checkbox(&mut o.change_pitch, "Change pitch with speed").on_hover_text("Off: voices sound normal at any speed. On: faster = higher, like a sped-up tape.");
        });
        if fast {
            theme::warn(ui, "Switch to Precise to change this.");
        }
        ui.separator();

        // 6. Audio and video
        theme::section(ui, "Audio and video");
        let has_audio = media.as_ref().map(|m| m.audio.is_some()).unwrap_or(true);
        let has_video = media.as_ref().map(|m| m.video.is_some()).unwrap_or(true);
        let o = &mut self.settings.options;
        let mut av = if !has_video { AvMode::AudioOnly } else if !has_audio { AvMode::Both } else { o.av };
        ui.add_enabled_ui(has_audio && has_video, |ui| {
            egui::ComboBox::from_id_salt("avmode").width(ui.available_width()).selected_text(av.label()).show_ui(ui, |ui| {
                for m in AvMode::ALL {
                    ui.selectable_value(&mut av, m, m.label());
                }
            });
        });
        if has_audio && has_video {
            o.av = av;
        }
        if !has_audio {
            theme::hint(ui, "This video has no audio.");
        } else if !has_video {
            theme::hint(ui, "This file is audio only, so it will be saved as an MP3.");
        } else if !av.hint().is_empty() {
            theme::hint(ui, av.hint());
        }
        let audio_kept = has_audio && !matches!(av, AvMode::RemoveAudio | AvMode::Mute);
        if audio_kept {
            ui.add_enabled_ui(!fast, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Volume");
                    ui.add(egui::Slider::new(&mut o.volume_pct, 0..=200).step_by(5.0).suffix("%"));
                });
            });
            if fast && o.volume_pct != 100 {
                theme::warn(ui, "Volume needs Precise mode.");
            }
        }
        if av == AvMode::AudioOnly {
            ui.horizontal(|ui| {
                ui.label("MP3 bitrate");
                egui::ComboBox::from_id_salt("mp3kbps").selected_text(format!("{} kbps", o.mp3_kbps)).show_ui(ui, |ui| {
                    for k in [128, 192, 320] {
                        ui.selectable_value(&mut o.mp3_kbps, k, format!("{k} kbps"));
                    }
                });
            });
            theme::hint(ui, "128 = fine for speech, 192 = good for music, 320 = best MP3 quality.");
        }
        ui.separator();

        // 7. Quality and codec
        if av != AvMode::AudioOnly {
            self.quality_ui(ui, caps.as_ref(), media.as_ref(), fast);
            ui.separator();
        }

        // 8. Export
        ui.add_space(4.0);
        let problems = self.problems();
        if self.media.is_some() {
            for pr in &problems {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&pr.message).color(theme::WARN));
                    if let Some(fix) = pr.fix {
                        if ui.small_button(fix.label()).clicked() {
                            self.apply_fix(fix);
                        }
                    }
                });
            }
        }
        let can = problems.is_empty() && !busy;
        let export_label = match self.settings.keybinds.get(Action::Export) {
            Some(b) => format!("Export clip   ({})", b.display()),
            None => "Export clip".into(),
        };
        if ui.add_enabled(can, theme::primary_button(&export_label).min_size(egui::vec2(ui.available_width(), 36.0))).clicked() {
            self.start_export(ctx);
        }
        if let Some(e) = &self.export {
            let p = e.progress();
            ui.add(egui::ProgressBar::new(p.fraction).show_percentage().fill(theme::ACCENT_DIM));
            ui.horizontal(|ui| {
                let eta = p.eta.map(|s| if s < 60.0 { format!("about {:.0} s left", s.ceil()) } else { format!("about {} left", timecode::format_short(s)) });
                ui.label(RichText::new(eta.unwrap_or_else(|| "Starting…".into())).small().color(theme::MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        e.cancel();
                    }
                });
            });
        } else if let Some((outcome, path)) = &self.last_outcome {
            match outcome {
                Outcome::Done => {
                    ui.label(RichText::new(format!("Saved {}", path.file_name().unwrap_or_default().to_string_lossy())).color(theme::OK));
                    if ui.small_button("Open in folder").clicked() {
                        winutil::reveal_in_explorer(path);
                    }
                }
                Outcome::Cancelled => theme::hint(ui, "Export cancelled. The unfinished file was deleted."),
                Outcome::Failed(msg) => {
                    ui.label(RichText::new(msg).color(theme::DANGER));
                    if ui.small_button("Open log").clicked() {
                        self.show_log = true;
                    }
                }
            }
        } else if self.media.is_some() && problems.is_empty() {
            theme::hint(ui, "Ready.");
        }
    }

    fn quality_ui(&mut self, ui: &mut egui::Ui, caps: Option<&Caps>, media: Option<&MediaInfo>, fast: bool) {
        theme::section(ui, "Quality and codec");
        let o = &mut self.settings.options;

        // Format: always available (also in Fast mode).
        let before = o.container;
        ui.horizontal(|ui| {
            ui.label("Format");
            egui::ComboBox::from_id_salt("container").width(ui.available_width()).selected_text(o.container.label()).show_ui(ui, |ui| {
                for c in Container::ALL.iter().copied().filter(|c| !c.is_audio_only()) {
                    ui.selectable_value(&mut o.container, c, c.label()).on_hover_text(c.hint());
                }
            });
        });
        theme::hint(ui, o.container.hint());
        if o.container != before {
            // Keep the codec choices valid for the new format, and say what changed.
            let mut notes = Vec::new();
            if !o.container.video_codecs().contains(&o.vcodec) {
                let old = o.vcodec;
                o.vcodec = o.container.video_codecs()[0];
                notes.push(format!("{} can't go in {}, so video is now {}.", old.label(), o.container.label(), o.vcodec.label()));
            }
            if !o.container.audio_codecs().contains(&o.acodec) {
                let old = o.acodec;
                o.acodec = o.container.default_audio();
                notes.push(format!("{} audio can't go in {}, so audio is now {}.", old.label(), o.container.label(), o.acodec.label()));
            }
            self.format_note = (!notes.is_empty()).then(|| notes.join(" "));
        }
        if let Some(n) = &self.format_note {
            theme::warn(ui, n.clone());
        }
        if fast {
            if let Some(m) = media {
                if let Some(a) = &m.audio {
                    let o = self.effective_opts();
                    if o.av == AvMode::Both && !o.container.can_copy_audio(&a.codec) {
                        let target = ACodec::from_ffprobe(&a.codec)
                            .filter(|c| o.container.audio_codecs().contains(c) && *c != ACodec::Pcm)
                            .unwrap_or(o.container.default_audio());
                        theme::hint(ui, format!("Audio will be converted to {} for {} (the video is still copied untouched).", target.label(), o.container.label()));
                    }
                }
            }
            theme::warn(ui, "Fast mode copies the video as-is. Switch to Precise to change codec, quality, frame rate or size.");
        }

        let o = &mut self.settings.options;
        ui.add_enabled_ui(!fast, |ui| {
            ui.horizontal(|ui| {
                ui.label("Video codec");
                egui::ComboBox::from_id_salt("vcodec").width(ui.available_width()).selected_text(o.vcodec.label()).show_ui(ui, |ui| {
                    for &v in o.container.video_codecs() {
                        let reason = caps.and_then(|c| c.video_unavailable_reason(v));
                        let r = ui.add_enabled(reason.is_none(), egui::Button::selectable(o.vcodec == v, v.label()));
                        if r.clicked() {
                            o.vcodec = v;
                        }
                        match reason {
                            Some(why) => {
                                r.on_disabled_hover_text(why);
                            }
                            None => {
                                r.on_hover_text(v.hint());
                            }
                        }
                    }
                });
            });
            theme::hint(ui, o.vcodec.hint());
            if let Some(p) = o.container.video_pitfall(o.vcodec) {
                theme::warn(ui, p);
            }

            ui.horizontal(|ui| {
                ui.label("Quality");
                egui::ComboBox::from_id_salt("quality").width(ui.available_width()).selected_text(o.quality.label()).show_ui(ui, |ui| {
                    for q in Quality::ALL {
                        if q == Quality::Custom && !o.vcodec.supports_bitrate() {
                            continue;
                        }
                        ui.selectable_value(&mut o.quality, q, q.label()).on_hover_text(q.hint());
                    }
                });
            });
            if o.quality == Quality::Custom && !o.vcodec.supports_bitrate() {
                o.quality = Quality::High;
            }
            theme::hint(ui, o.quality.hint());
            if o.quality == Quality::Custom {
                ui.horizontal(|ui| {
                    ui.label("Video bitrate");
                    ui.add(egui::DragValue::new(&mut o.custom_kbps).range(100..=200_000).speed(50).suffix(" kbps"));
                });
                theme::hint(ui, "Rough guide: 1080p ≈ 6,000–12,000 kbps, 4K ≈ 20,000–45,000 kbps.");
            }

            ui.horizontal(|ui| {
                ui.label("Frame rate");
                egui::ComboBox::from_id_salt("fps").width(ui.available_width()).selected_text(o.fps.label()).show_ui(ui, |ui| {
                    for f in FrameRate::ALL {
                        ui.selectable_value(&mut o.fps, f, f.label());
                    }
                });
            });
            if let (FrameRate::Fps(f), Some(v)) = (o.fps, media.and_then(|m| m.video.as_ref())) {
                if (f as f64) > v.fps + 0.5 {
                    theme::hint(ui, format!("The source is {} fps — going higher repeats frames, it won't look smoother.", probe::fmt_fps(v.fps)));
                }
            }

            egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Resolution");
                    egui::ComboBox::from_id_salt("res").width(ui.available_width()).selected_text(o.resolution.label()).show_ui(ui, |ui| {
                        for r in Resolution::ALL {
                            ui.selectable_value(&mut o.resolution, r, r.label());
                        }
                    });
                });
                if let (Some(h), Some(v)) = (o.resolution.height(), media.and_then(|m| m.video.as_ref())) {
                    if h > v.height {
                        theme::hint(ui, format!("The source is {}p — scaling up makes the file bigger but not sharper.", v.height));
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Encoder speed");
                    egui::ComboBox::from_id_salt("encspeed").width(ui.available_width()).selected_text(o.enc_speed.label()).show_ui(ui, |ui| {
                        for s in EncSpeed::ALL {
                            ui.selectable_value(&mut o.enc_speed, s, s.label());
                        }
                    });
                });
                theme::hint(ui, "Slower settings squeeze the same quality into a smaller file.");
                ui.horizontal(|ui| {
                    ui.label("Audio codec");
                    egui::ComboBox::from_id_salt("acodec").width(ui.available_width()).selected_text(o.acodec.label()).show_ui(ui, |ui| {
                        for &a in o.container.audio_codecs() {
                            let ok = caps.map(|c| c.audio_ok(a)).unwrap_or(true);
                            let r = ui.add_enabled(ok, egui::Button::selectable(o.acodec == a, a.label()));
                            if r.clicked() {
                                o.acodec = a;
                            }
                            r.on_disabled_hover_text("Your FFmpeg build doesn't include this encoder.");
                        }
                    });
                });
                if let Some(p) = o.container.audio_pitfall(o.acodec) {
                    theme::warn(ui, p);
                }
                if o.acodec.has_bitrate() {
                    ui.horizontal(|ui| {
                        ui.label("Audio bitrate");
                        egui::ComboBox::from_id_salt("akbps").selected_text(format!("{} kbps", o.audio_kbps)).show_ui(ui, |ui| {
                            for k in [96, 128, 160, 192, 256, 320] {
                                ui.selectable_value(&mut o.audio_kbps, k, format!("{k} kbps"));
                            }
                        });
                    });
                }
                ui.checkbox(&mut o.use_gpu, "Use GPU encoder when available");
                match caps {
                    None => theme::hint(ui, "Checking your graphics card…"),
                    Some(c) => {
                        let gpu = c.gpu_encoder(o.vcodec);
                        let used = c.pick(o.vcodec, o.use_gpu);
                        let line = match (o.use_gpu, gpu, used) {
                            (true, Some(g), _) => format!("Using {} — fast, with slightly bigger files than the CPU at the same quality.", gpu_name(g)),
                            (true, None, Some(cpu)) => format!("No working GPU encoder for {} on this PC, so the CPU ({cpu}) is used.", o.vcodec.label()),
                            (false, _, Some(cpu)) => format!("Using the CPU ({cpu}): slower, slightly smaller files."),
                            (_, _, None) => "No encoder available for this codec.".into(),
                        };
                        theme::hint(ui, line);
                    }
                }
            });
        });
        let _ = VCodec::H264;
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_log;
        egui::Window::new("Log").open(&mut open).default_size([760.0, 360.0]).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Copy all").clicked() {
                    let text = self.log.snapshot().iter().map(|e| format!("{} {}", e.time, e.text)).collect::<Vec<_>>().join("\n");
                    ctx.copy_text(text);
                }
                if ui.button("Open log folder").clicked() {
                    winutil::open_folder(&Log::dir());
                }
                if ui.button("Clear").clicked() {
                    self.log.clear();
                }
                theme::hint(ui, "Lines starting with $ are the exact FFmpeg commands — you can paste them into a terminal.");
            });
            ui.separator();
            egui::ScrollArea::both().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
                for e in self.log.snapshot() {
                    let (prefix, col) = match e.kind {
                        Kind::Info => ("", theme::TEXT),
                        Kind::Command => ("$ ", theme::ACCENT),
                        Kind::Output => ("  ", theme::MUTED),
                        Kind::Error => ("! ", theme::DANGER),
                    };
                    ui.label(RichText::new(format!("{} {prefix}{}", e.time, e.text)).monospace().size(12.0).color(col));
                }
            });
        });
        self.show_log = open;
    }
}

fn gpu_name(enc: &str) -> &'static str {
    if enc.ends_with("_nvenc") {
        "your NVIDIA graphics card (NVENC)"
    } else if enc.ends_with("_amf") {
        "your AMD graphics (AMF)"
    } else {
        "your Intel graphics (Quick Sync)"
    }
}

fn time_feedback(ui: &mut egui::Ui, text: &str) {
    match timecode::parse(text) {
        Ok(t) => theme::hint(ui, timecode::format(t)),
        Err(e) => {
            ui.label(RichText::new(e).small().color(theme::DANGER));
        }
    }
}

/// Marks a development build so it's never mistaken for the installed app.
pub fn dev_badge(ui: &mut egui::Ui) {
    egui::Frame::new()
        .fill(theme::WARN.gamma_multiply(0.25))
        .stroke(Stroke::new(1.0, theme::WARN))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new("DEV").small().strong().color(theme::WARN));
        })
        .response
        .on_hover_text("Development build. Uses its own settings and logs, and never checks for updates.");
}

pub fn key_cap(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().monospace());
        });
}
