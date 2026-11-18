//! The main editor screen: preview on the left, tools on the right.

use crate::clips;
use crate::discord::{self, Plan, Preset};
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
use crate::speed::{self, SpeedZone, TimeMap};
use crate::zoom::{self, ZBox, Zone};
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::tracks::{self, TrackMix};
use crate::waveform;

mod klipy_ui;
mod montage_ui;
mod track_ui;
mod overlay_ui;
use overlay_ui::OvDrag;

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
    pitch_correct: bool,
    volume: u32,
    /// Preview-only listening volume (not part of the export).
    listen: u32,
    mute: bool,
    no_audio: bool,
    no_video: bool,
    /// How a file's several audio tracks are mixed.
    mix: tracks::PreviewAudio,
}

/// Everything that affects an export; if it changes, an exported preview is stale.
#[derive(Clone, PartialEq)]
struct ExportSig {
    file: PathBuf,
    start: String,
    end: String,
    until_end: bool,
    opts: Options,
    zones: Vec<Zone>,
    speed_zones: Vec<SpeedZone>,
    overlays: Vec<crate::overlay::Overlay>,
    audio: tracks::ExportAudio,
}

/// What a drag on the video is doing to the selected zone's box.
#[derive(Clone, Copy, PartialEq)]
enum BoxDrag {
    /// Drawing a new box from this corner (frame fractions).
    Draw((f64, f64)),
    /// Resizing from a corner; the opposite corner stays here.
    Resize((f64, f64)),
    /// Moving; pointer offset from the box's top-left.
    Move((f64, f64)),
}

const ZONE_COLORS: [Color32; 6] = [
    Color32::from_rgb(0x5e, 0xc8, 0xf0),
    Color32::from_rgb(0xf0, 0xc7, 0x5e),
    Color32::from_rgb(0x8b, 0xe0, 0x7a),
    Color32::from_rgb(0xe0, 0x7a, 0xd8),
    Color32::from_rgb(0xf0, 0x8a, 0x5e),
    Color32::from_rgb(0x9a, 0x8c, 0xf5),
];

fn zone_color(i: usize) -> Color32 {
    ZONE_COLORS[i % ZONE_COLORS.len()]
}

/// Where a `w`x`h` video sits inside `rect` (mpv letterboxes it).
fn fit_rect(rect: egui::Rect, w: f32, h: f32) -> egui::Rect {
    if w <= 0.0 || h <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / w).min(rect.height() / h);
    egui::Rect::from_center_size(rect.center(), egui::vec2(w * scale, h * scale))
}

/// The timeline's horizontal axis. Times are original-video seconds; on
/// screen they're spaced by finished-video time, so a slow zone is wider and
/// a fast one narrower. For an exported file the map is plain.
struct Axis {
    left: f32,
    width: f32,
    map: TimeMap,
    duration: f64,
    /// Finished length of the whole source.
    total: f64,
}

impl Axis {
    fn new(xs: egui::Rangef, map: TimeMap, duration: f64) -> Axis {
        let total = map.to_out(duration).max(0.001);
        Axis { left: xs.min, width: xs.span(), map, duration, total }
    }
    fn x_of(&self, t: f64) -> f32 {
        self.left + (self.map.to_out(t.clamp(0.0, self.duration)) / self.total).clamp(0.0, 1.0) as f32 * self.width
    }
    fn t_of(&self, x: f32) -> f64 {
        self.t_at(((x - self.left) / self.width).clamp(0.0, 1.0) as f64)
    }
    /// The original time a fraction (0-1) of the way along.
    fn t_at(&self, f: f64) -> f64 {
        self.map.to_src(f * self.total).clamp(0.0, self.duration)
    }
}

/// Speed band colours: blue for slow, orange for fast, grey for 1x.
fn speed_color(speed: f64) -> Color32 {
    if speed < 1.0 {
        Color32::from_rgb(0x5e, 0xa8, 0xf0)
    } else if speed > 1.0 {
        Color32::from_rgb(0xf0, 0x9a, 0x4e)
    } else {
        Color32::from_rgb(0x9a, 0x9a, 0xa8)
    }
}

/// "= 0:14.2 in final video" for a typed original time, when speed zones
/// change where it lands. `s0` is the trim start.
fn final_text(map: &TimeMap, s0: f64, text: &str) -> Option<String> {
    if map.is_identity() {
        return None;
    }
    let t = timecode::parse(text).ok()?;
    Some(format!("= {} in final video", timecode::format_short((map.to_out(t) - map.to_out(s0)).max(0.0))))
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
    zones: Vec<Zone>,
    sel_zone: Option<usize>,
    /// Which point of the selected zone is being edited (0 = its first box).
    edit_point: usize,
    /// The next drag on the video draws a fresh box.
    drawing: bool,
    box_drag: Option<BoxDrag>,
    /// Dragging a zone band's edge on the timeline: (zone, is start).
    band_drag: Option<(usize, bool)>,
    /// Dragging a Precise point's marker on a zone band: (zone, point).
    point_drag: Option<(usize, usize)>,
    /// Where a timeline drag is pointing. While set, the playhead shows this
    /// instead of mpv's position, so it sticks to the cursor even when mpv's
    /// seeks lag behind. After release it holds (with the release time) until
    /// mpv's exact seek lands, so the playhead doesn't jump back.
    scrub: Option<(f64, Option<Instant>)>,
    /// The zoom last given to mpv for the live preview.
    live_zoom: ZBox,
    /// A file was just loaded: keep checking mpv until its first frame is
    /// ready, so it shows even while paused.
    awaiting_frame: Option<Instant>,
    /// Ghost line: where playback last started (original video time).
    ghost: Option<f64>,
    /// Paused last frame, and where; a pause-to-play change sets the ghost.
    was_paused: bool,
    paused_pos: f64,
    /// The Clips drawer's file list.
    library: clips::Library,
    /// The running export's size cap, and whether it's already the retry.
    export_cap: Option<(u64, bool)>,
    /// A short message after Copy file ("Copied" or what went wrong).
    copy_note: Option<(String, Instant)>,
    /// The drawer's Finished files list: the output folder.
    finished: clips::Library,
    /// Which sources have been exported (green ticks).
    history: crate::history::History,
    /// The file just exported, highlighted in Finished for a few seconds.
    fresh: Option<(PathBuf, Instant)>,
    /// (clips folder, output folder) the drawer lists were last scanned for.
    lists_for: Option<(Option<PathBuf>, Option<PathBuf>)>,
    /// One entry per audio track of the open file.
    tracks: Vec<TrackMix>,
    /// The track mix the player was last given. Only updated while no mouse
    /// button is down, so dragging a volume slider doesn't restart mpv's
    /// audio on every frame.
    preview_mix: tracks::PreviewAudio,
    waves: waveform::Waveforms,
    /// Height the track lanes took last frame.
    lanes_h: f32,
    /// To wake the UI from background work.
    egui_ctx: egui::Context,
    /// A Discord export that's still over its cap after the retry.
    over_cap: Option<String>,
    speed_zones: Vec<SpeedZone>,
    sel_speed: Option<usize>,
    /// Dragging a speed band's edge on the timeline: (zone, is start).
    speed_band_drag: Option<(usize, bool)>,
    /// The time map the timeline is drawn with. Only updated while no mouse
    /// button is down, so the timeline re-flows when a drag ends instead of
    /// moving under the cursor.
    shown_map: TimeMap,
    /// The speed and zone mute last given to mpv for the live preview.
    live_speed: (f64, bool),
    overlays: Vec<crate::overlay::Overlay>,
    sel_overlay: Option<usize>,
    /// Editing the selected overlay's end position rather than its start.
    edit_end_pos: bool,
    ov_drag: Option<OvDrag>,
    /// Dragging an overlay band's edge: (overlay, is start).
    ov_band_drag: Option<(usize, bool)>,
    previews: crate::previews::Previews,
    /// Installed fonts, read the first time a text overlay needs them.
    fonts: Option<Vec<crate::fonts::Font>>,
    /// Font files already given to egui for text previews (and ones that failed).
    egui_fonts: std::collections::HashSet<String>,
    bad_fonts: std::collections::HashSet<String>,
    /// The Assets folder's files, and when/for which folder they were listed.
    assets: Vec<PathBuf>,
    assets_for: Option<PathBuf>,
    assets_scanned: Instant,
    show_klipy: bool,
    show_features: bool,
    klipy_key_visible: bool,
    klipy: klipy_ui::KlipyUi,
    /// Montage mode is on (the strip shows, Finish montage replaces Export).
    montage_on: bool,
    montage: Option<montage_ui::MontageState>,
    montage_name_text: String,
    montage_note: Option<String>,
    /// A montage card being dragged to a new place.
    montage_drag: Option<usize>,
    /// The running montage export's sources (ticked when it's done) and work folder.
    montage_sources: Vec<PathBuf>,
    montage_temp: Option<PathBuf>,
    track_job: Option<track_ui::TrackJob>,
    /// What the last tracking run did (or why it failed).
    track_note: Option<String>,
    /// Which device the last run used, for Features.
    tracker_last: Option<String>,
    /// Fixing a tracked zone: its index, and the box being dragged.
    fix_zone: Option<usize>,
    fix_box: Option<ZBox>,
    /// Where the fix box was grabbed (offset from its corner).
    fix_grab: Option<(f64, f64)>,
    /// The next drag on the video draws a loop (in the selected zone's box, or the fix box).
    lasso_mode: bool,
    /// The loop being drawn (frame fractions).
    lasso_stroke: Vec<(f64, f64)>,
    /// Why the last loop drawn couldn't be used (shown until the next try).
    lasso_error: Option<String>,
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
            zones: Vec::new(),
            sel_zone: None,
            edit_point: 0,
            drawing: false,
            box_drag: None,
            band_drag: None,
            point_drag: None,
            scrub: None,
            live_zoom: ZBox::FULL,
            awaiting_frame: None,
            ghost: None,
            was_paused: true,
            paused_pos: 0.0,
            library: clips::Library::default(),
            export_cap: None,
            copy_note: None,
            finished: clips::Library::default(),
            history: crate::history::History::load(),
            fresh: None,
            lists_for: None,
            tracks: Vec::new(),
            preview_mix: tracks::PreviewAudio::Normal,
            waves: waveform::Waveforms::default(),
            lanes_h: 0.0,
            egui_ctx: ctx.clone(),
            over_cap: None,
            speed_zones: Vec::new(),
            sel_speed: None,
            speed_band_drag: None,
            shown_map: TimeMap::default(),
            live_speed: (1.0, false),
            overlays: Vec::new(),
            sel_overlay: None,
            edit_end_pos: false,
            ov_drag: None,
            ov_band_drag: None,
            previews: crate::previews::Previews::default(),
            fonts: None,
            egui_fonts: Default::default(),
            bad_fonts: Default::default(),
            assets: Vec::new(),
            assets_for: None,
            assets_scanned: Instant::now() - Duration::from_secs(60),
            show_klipy: false,
            show_features: false,
            klipy_key_visible: false,
            klipy: Default::default(),
            montage_on: false,
            montage: None,
            montage_name_text: String::new(),
            montage_note: None,
            montage_drag: None,
            montage_sources: Vec::new(),
            montage_temp: None,
            track_job: None,
            track_note: None,
            tracker_last: None,
            fix_zone: None,
            fix_box: None,
            fix_grab: None,
            lasso_mode: false,
            lasso_stroke: Vec::new(),
            lasso_error: None,
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
        if self.discord_active() {
            o = match self.discord_plan() {
                Some(Ok(p)) => o.with_plan(&p),
                _ => o.with_preset_basics(),
            };
        }
        if let Some(m) = &self.media {
            if m.video.is_none() {
                o.av = AvMode::AudioOnly;
            } else if m.audio.is_none() && o.av != AvMode::Both {
                o.av = AvMode::Both;
            }
        }
        o
    }

    /// A Discord preset is chosen and applies (the file has a picture).
    fn discord_active(&self) -> bool {
        self.settings.options.preset.cap_mb().is_some() && self.media.as_ref().is_none_or(|m| m.video.is_some())
    }

    /// The Discord preset's plan for the current trim and speed. `Err` holds
    /// the longest output length (seconds) that fits.
    fn discord_plan(&self) -> Option<Result<Plan, f64>> {
        let cap = self.settings.options.preset.cap_bytes()?;
        let m = self.media.as_ref()?;
        let v = m.video.as_ref()?;
        let (s, e) = self.trim()?;
        let o = &self.settings.options;
        let has_audio = m.audio.is_some() && !matches!(o.av, AvMode::RemoveAudio | AvMode::Mute);
        Some(discord::plan(cap, job::output_duration(s, e, &self.active_speeds()), v.width, v.height, v.fps, has_audio))
    }

    /// Why the speed tools are greyed out, if they are.
    fn speed_blocked_reason(&self) -> Option<&'static str> {
        match &self.media {
            None => Some("Open a video first."),
            _ if self.effective_opts().mode == CutMode::Fast => Some("Switch to Precise to change speed. Fast mode copies the clip untouched."),
            _ => None,
        }
    }

    /// Speed zones used by the export and preview (empty when speed doesn't apply).
    fn active_speeds(&self) -> Vec<speed::Resolved> {
        if self.speed_blocked_reason().is_some() {
            return Vec::new();
        }
        let trim = self.trim();
        self.speed_zones.iter().filter_map(|z| z.resolve(trim)).collect()
    }

    /// Original time <-> finished time, from the current speed zones.
    fn time_map(&self) -> TimeMap {
        TimeMap::new(&self.active_speeds())
    }

    /// Zoom works when the export re-encodes a picture.
    fn zoom_allowed(&self) -> bool {
        self.zoom_blocked_reason().is_none()
    }

    /// Why the zoom tools are greyed out, if they are.
    fn zoom_blocked_reason(&self) -> Option<&'static str> {
        let o = self.effective_opts();
        match &self.media {
            None => Some("Open a video first."),
            Some(m) if m.video.is_none() => Some("This file has no picture to zoom."),
            _ if o.mode == CutMode::Fast => Some("Switch to Precise to zoom. Fast mode copies the video untouched."),
            _ if o.av == AvMode::AudioOnly => Some("Zoom needs video, and you've chosen Audio only."),
            _ => None,
        }
    }

    /// Zones used by the export and preview (empty when zoom doesn't apply).
    fn active_zones(&self) -> Vec<zoom::Resolved> {
        if !self.zoom_allowed() {
            return Vec::new();
        }
        self.zones.iter().filter_map(Zone::resolve).collect()
    }

    /// The automatic name: the game (or the source's own name) and the time now.
    fn auto_name(&self, m: &MediaInfo) -> String {
        let game = clips::game_for_name(self.settings.clips_dir.as_deref(), &m.path)
            .unwrap_or_else(|| m.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into()));
        let name = job::auto_name(&game, winutil::now_stamp());
        job::sanitize_name(&name).unwrap_or(name)
    }

    fn caps(&self) -> Option<Caps> {
        self.caps.lock().unwrap().clone()
    }

    fn trim(&self) -> Option<(f64, f64)> {
        let m = self.media.as_ref()?;
        validate::trim_range(m, &self.start_text, &self.end_text, self.until_end).ok()
    }

    fn position(&self) -> f64 {
        if let Some((t, _)) = self.scrub {
            return t;
        }
        self.video.as_ref().and_then(|v| v.player.time_pos()).unwrap_or(0.0)
    }

    fn paused(&self) -> bool {
        self.video.as_ref().and_then(|v| v.player.paused()).unwrap_or(true)
    }

    fn export_sig(&self) -> Option<ExportSig> {
        Some(ExportSig {
            file: self.media.as_ref()?.path.clone(),
            start: self.start_text.clone(),
            end: self.end_text.clone(),
            until_end: self.until_end,
            opts: self.effective_opts(),
            zones: self.zones.clone(),
            speed_zones: self.speed_zones.clone(),
            overlays: self.overlays.clone(),
            audio: tracks::export_audio(&self.tracks),
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
        self.trim()?;
        let o = self.effective_opts();
        let name = job::sanitize_name(&self.name_text).unwrap_or_else(|| self.auto_name(m));
        Some(job::unique_path(dir, &name, o.output_container().ext()))
    }

    fn problems(&self) -> Vec<validate::Problem> {
        let o = self.effective_opts();
        let caps = self.caps();
        let space = match (&self.media, self.trim(), self.dir_status.2) {
            (Some(m), Some((s, e)), Some(free)) => {
                let out = PathBuf::new();
                let speeds = self.active_speeds();
                let j = Job { media: m, opts: &o, start: s, end: e, output: &out, video_encoder: None, can_tonemap: false, zones: &[], tracks: &self.tracks, speed_zones: &speeds, overlays: &[], montage: None };
                Some((job::estimate_bytes(&j), free))
            }
            _ => None,
        };
        let mut out = validate::check(&validate::Input {
            media: self.media.as_ref(),
            start_text: &self.start_text,
            end_text: &self.end_text,
            until_end: self.until_end,
            opts: &o,
            output_dir: self.settings.output_dir.as_deref(),
            caps: caps.as_ref(),
            space,
            dir_writable: self.dir_status.1,
            zones: &self.zones,
            speed_zones: &self.speed_zones,
            overlays: if self.overlays_blocked_reason().is_some() { &[] } else { &self.overlays },
        });
        if let Some(Err(max_out)) = self.discord_plan() {
            // The trim is in original-video time, so go back through the speed zones.
            let map = self.time_map();
            let s = self.trim().map_or(0.0, |t| t.0);
            let max_trim = map.to_src(map.to_out(s) + max_out) - s;
            out.push(validate::Problem {
                message: format!(
                    "Too long for {} ({} MB) even at 480p. Trim it to under {}.",
                    self.settings.options.preset.label(),
                    self.settings.options.preset.cap_mb().unwrap_or(0),
                    timecode::format_short(max_trim)
                ),
                fixes: Vec::new(),
            });
        }
        out
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
                self.cancel_track();
                self.fix_zone = None;
                self.zones.clear();
                self.sel_zone = None;
                self.speed_zones.clear();
                self.sel_speed = None;
                self.overlays.clear();
                self.sel_overlay = None;
                self.drawing = false;
                self.ghost = None;
                self.was_paused = true;
                if m.video.as_ref().is_some_and(|v| v.hdr) {
                    self.log.info("HDR video: exports will be converted to standard colour (SDR).");
                }
                self.tracks = vec![TrackMix::default(); m.audio_tracks.len()];
                self.preview_mix = tracks::preview_audio(&self.tracks);
                self.waves.load(&self.ffmpeg, &m.path, m.audio_tracks.len(), &self.egui_ctx);
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
            Fix::Zone(z) => {
                let trim = self.trim();
                z.apply(&mut self.zones, trim);
                if self.sel_zone.is_some_and(|i| i >= self.zones.len()) || matches!(z, zoom::ZoneFix::Delete(_)) {
                    self.sel_zone = None;
                    self.drawing = false;
                }
            }
            Fix::Overlay(z) => {
                let trim = self.trim();
                z.apply(&mut self.overlays, trim);
                if self.sel_overlay.is_some_and(|i| i >= self.overlays.len()) || matches!(z, crate::overlay::OverlayFix::Delete(_)) {
                    self.sel_overlay = None;
                }
            }
            Fix::Speed(z) => {
                let trim = self.trim();
                z.apply(&mut self.speed_zones, trim);
                if self.sel_speed.is_some_and(|i| i >= self.speed_zones.len()) || matches!(z, speed::SpeedFix::Delete(_)) {
                    self.sel_speed = None;
                }
            }
        }
    }

    fn start_export(&mut self, ctx: &egui::Context) {
        self.start_export_scaled(ctx, None);
    }

    /// `retry`: re-encode after a capped export came out too big, into the
    /// same file, with the video bitrate scaled by the factor.
    fn start_export_scaled(&mut self, ctx: &egui::Context, retry: Option<(f64, PathBuf)>) {
        if self.exporting() || !self.problems().is_empty() {
            return;
        }
        let out = match &retry {
            Some((_, p)) => Some(p.clone()),
            None => self.output_path_preview(),
        };
        let (Some(m), Some((start, end)), Some(out), Some(caps)) = (self.media.clone(), self.trim(), out, self.caps()) else {
            return;
        };
        let retry = retry.map(|r| r.0);
        let mut o = self.effective_opts();
        let cap = if self.discord_active() { o.preset.cap_bytes() } else { None };
        if let Some(f) = retry {
            o.custom_kbps = ((o.custom_kbps as f64 * f) as u32).max(100);
            self.log.info(format!("Re-encoding at {} kbps video so it fits under {} MB.", o.custom_kbps, o.preset.cap_mb().unwrap_or(0)));
        }
        let encodes_video = o.mode == CutMode::Precise && o.av != AvMode::AudioOnly && m.video.is_some();
        let enc = if encodes_video { caps.pick(o.vcodec, o.use_gpu) } else { None };
        let zones = self.active_zones();
        let speeds = self.active_speeds();
        let overlays = match self.overlays_for_export() {
            Ok(o) => o,
            Err(e) => {
                self.log.error(&e);
                self.last_outcome = Some((Outcome::Failed(e), out));
                return;
            }
        };
        if !overlays.is_empty() {
            self.log.info(format!("{} overlay(s)", overlays.len()));
        }
        let j = Job { media: &m, opts: &o, start, end, output: &out, video_encoder: enc, can_tonemap: caps.zscale, zones: &zones, tracks: &self.tracks, speed_zones: &speeds, overlays: &overlays, montage: None };
        if !zones.is_empty() {
            self.log.info(format!("Zooming {} zone(s)", zones.len()));
        }
        if !speeds.is_empty() {
            let list: Vec<String> = speeds.iter().map(|z| format!("{}-{} at {}x", timecode::format_short(z.start), timecode::format_short(z.end), z.speed)).collect();
            self.log.info(format!("Speed zones: {}", list.join(", ")));
        }
        let args = job::build_args(&j);
        let total = job::output_duration(start, end, &speeds);
        if let Some(e) = enc {
            self.log.info(format!("Encoding video with {e}"));
        }
        match Export::start(&self.ffmpeg, args, out.clone(), total, self.log.clone(), ctx.clone()) {
            Ok(e) => {
                self.export = Some(e);
                self.export_sig = self.export_sig();
                self.export_cap = cap.map(|c| (c, retry.is_some()));
                self.last_outcome = None;
                self.over_cap = None;
            }
            Err(e) => {
                self.log.error(&e);
                self.last_outcome = Some((Outcome::Failed(e), out));
            }
        }
    }

    /// A finished export: check a size cap (re-encoding once if it's over),
    /// then show the result.
    fn finish_export(&mut self, ctx: &egui::Context) {
        let Some(e) = &self.export else { return };
        let Some(outcome) = e.progress().outcome else { return };
        let path = e.output.clone();
        self.export = None;
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if outcome == Outcome::Done {
            if let Some((cap, false)) = self.export_cap {
                if bytes > cap {
                    self.log.info(format!("{} came out at {}, over the {} cap. Trying again smaller.", path.display(), mb(bytes), mb(cap)));
                    let _ = std::fs::remove_file(&path);
                    self.start_export_scaled(ctx, Some((discord::retry_scale(cap, bytes), path.clone())));
                    if self.export.is_some() {
                        return;
                    }
                }
            }
            if let Some((cap, _)) = self.export_cap {
                if bytes > cap {
                    let msg = format!("Still {} after the retry, over the {} cap. Discord may refuse it.", mb(bytes), mb(cap));
                    self.log.error(&msg);
                    self.over_cap = Some(msg);
                }
            }
            self.exported_file = Some(path.clone());
            self.view = View::Result;
            if !self.montage_sources.is_empty() {
                for s in &self.montage_sources {
                    self.history.record(s, winutil::now_stamp());
                }
                self.history.save();
            } else if let Some(m) = &self.media {
                self.history.record(&m.path, winutil::now_stamp());
                self.history.save();
            }
            self.fresh = Some((path.clone(), Instant::now()));
            self.rescan_lists(ctx);
        }
        self.export_cap = None;
        self.montage_sources.clear();
        if let Some(t) = self.montage_temp.take() {
            // Its files are already deleted; the folder goes if it's empty.
            let _ = std::fs::remove_dir(t);
        }
        self.last_outcome = Some((outcome, path));
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
            Action::Export if !self.montage_on => self.start_export(ctx),
            Action::OpenFile => self.open_dialog(),
            Action::ShowShortcuts => self.show_keys = true,
            Action::NewZone => self.new_zone(),
            Action::AddZonePoint if self.zoom_allowed() => {
                // The selected zone, or else the one under the playhead.
                let pos = self.position();
                let i = self.sel_zone.filter(|&i| i < self.zones.len()).or_else(|| self.zones.iter().position(|z| z.times().is_some_and(|(s, e)| pos > s && pos < e)));
                if let Some(i) = i {
                    self.add_zone_point(i, pos);
                }
            }
            Action::NewSpeedZone => self.new_speed_zone(),
            Action::NewTextOverlay => self.new_text_overlay(),
            Action::OpenKlipy => self.open_klipy(),
            Action::AddToMontage => self.add_to_montage(),
            Action::DeleteZone => {
                if self.sel_overlay.is_some() {
                    self.delete_selected_overlay();
                } else if self.sel_speed.is_some() {
                    self.delete_selected_speed();
                } else if let Some(i) = self.sel_zone.filter(|&i| self.edit_point > 0 && self.zones.get(i).is_some_and(|z| z.uses_points())) {
                    self.remove_zone_point(i, self.edit_point);
                } else {
                    self.delete_selected_zone();
                }
            }
            _ => {}
        }
    }

    fn seek(&self, t: f64) {
        if let Some(v) = &self.video {
            v.player.command(&["seek", &t.to_string(), "absolute+exact"]);
        }
    }

    /// Follow a drag on the timeline. Exact seeks decode from the previous
    /// keyframe, so sending one per frame piles up and the picture falls
    /// seconds behind the mouse. While dragging, jump to keyframes (cheap)
    /// and only send a seek when the target actually moved.
    fn scrub_to(&mut self, t: f64) {
        if self.scrub.is_some_and(|(s, settling)| settling.is_none() && (s - t).abs() < 1e-3) {
            return;
        }
        self.scrub = Some((t, None));
        if let Some(v) = &self.video {
            v.player.command(&["seek", &t.to_string(), "absolute+keyframes"]);
        }
    }

    /// The drag ended: land exactly on the frame under the cursor.
    fn end_scrub(&mut self) {
        if let Some((t, None)) = self.scrub {
            self.seek(t);
            self.scrub = Some((t, Some(Instant::now())));
        }
    }

    /// Drop the held scrub position once mpv has caught up with it.
    fn settle_scrub(&mut self, ctx: &egui::Context) {
        let Some((t, Some(since))) = self.scrub else { return };
        let v = self.video.as_ref();
        let seeking = v.and_then(|v| v.player.seeking()).unwrap_or(false);
        let there = v.and_then(|v| v.player.time_pos()).is_some_and(|p| (p - t).abs() < 0.25);
        if (!seeking && there) || since.elapsed() > Duration::from_millis(1500) {
            self.scrub = None;
        } else {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn pause(&self) {
        if let Some(v) = &self.video {
            v.player.set_property("pause", "yes");
        }
    }

    /// Add a zone at the playhead (inside the trim, after any zone already
    /// there) and get ready to draw its box.
    fn new_zone(&mut self) {
        if !self.zoom_allowed() || self.zones.len() >= zoom::MAX_ZONES {
            return;
        }
        let Some((ts, te)) = self.trim() else { return };
        let mut times: Vec<(f64, f64)> = self.zones.iter().filter_map(|z| z.times()).collect();
        times.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut s = self.position().clamp(ts, te);
        for &(zs, ze) in &times {
            if s >= zs && s < ze {
                s = ze;
            }
        }
        let mut e = (s + 5.0).min(te);
        if let Some(&(next, _)) = times.iter().find(|t| t.0 > s) {
            e = e.min(next);
        }
        if e - s < 1.0 {
            s = (e - 1.0).max(ts);
        }
        self.zones.push(Zone::new(s, e));
        self.sel_zone = Some(self.zones.len() - 1);
        self.edit_point = 0;
        self.drawing = true;
        self.lasso_mode = false;
        self.view = View::Original;
        self.pause();
        self.seek(s);
    }

    /// Add a speed zone at the playhead (inside the trim, after any speed
    /// zone already there).
    fn new_speed_zone(&mut self) {
        if self.speed_blocked_reason().is_some() || self.speed_zones.len() >= speed::MAX_ZONES {
            return;
        }
        let Some((ts, te)) = self.trim() else { return };
        let trim = Some((ts, te));
        let mut times: Vec<(f64, f64)> = self.speed_zones.iter().filter_map(|z| z.times(trim)).collect();
        times.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut s = self.position().clamp(ts, te);
        for &(zs, ze) in &times {
            if s >= zs && s < ze {
                s = ze;
            }
        }
        let mut e = (s + 3.0).min(te);
        if let Some(&(next, _)) = times.iter().find(|t| t.0 > s) {
            e = e.min(next);
        }
        if e - s < 1.0 {
            s = (e - 1.0).max(ts);
        }
        self.speed_zones.push(SpeedZone::new(s, e));
        self.select_speed(self.speed_zones.len() - 1);
    }

    fn select_speed(&mut self, i: usize) {
        self.sel_speed = Some(i);
        self.sel_zone = None;
        self.sel_overlay = None;
        self.drawing = false;
    }

    fn delete_selected_speed(&mut self) {
        if let Some(i) = self.sel_speed.take() {
            if i < self.speed_zones.len() {
                self.speed_zones.remove(i);
            }
        }
    }

    fn delete_selected_zone(&mut self) {
        if self.sel_zone.is_some() {
            self.cancel_track();
            self.fix_zone = None;
        }
        if let Some(i) = self.sel_zone.take() {
            if i < self.zones.len() {
                self.zones.remove(i);
            }
        }
        self.drawing = false;
        self.lasso_mode = false;
        self.box_drag = None;
    }

    /// Select a zone and jump to its start.
    fn select_zone(&mut self, i: usize) {
        self.sel_zone = Some(i);
        self.sel_speed = None;
        self.sel_overlay = None;
        self.edit_point = 0;
        self.drawing = false;
        self.lasso_mode = false;
        if let Some((s, _)) = self.zones.get(i).and_then(|z| z.times()) {
            self.seek(s);
        }
    }

    /// How long one frame of the source lasts.
    fn frame_len(&self) -> f64 {
        1.0 / self.media.as_ref().and_then(|m| m.video.as_ref()).map_or(30.0, |v| v.fps.max(1.0))
    }

    /// Show zone `i`'s point `k` (0 = its first box) on its frame, ready to
    /// draw a new box (`draw`) or to move/resize the one there.
    fn goto_zone_box(&mut self, i: usize, k: usize, draw: bool) {
        self.sel_zone = Some(i);
        self.edit_point = k;
        self.drawing = draw;
        self.lasso_mode = false;
        self.view = View::Original;
        self.pause();
        if let Some(t) = self.zones.get(i).and_then(|z| point_frame(z, k, self.frame_len())) {
            self.seek(t);
        }
    }

    /// Add a Precise point to zone `i` at time `t`, starting as a copy of
    /// where the camera is then, and jump there to adjust it. A Simple
    /// zone switches to Precise first.
    fn add_zone_point(&mut self, i: usize, t: f64) {
        let Some(z) = self.zones.get_mut(i) else { return };
        if add_point_blocked(z, t).is_some() {
            return;
        }
        if !z.uses_points() {
            z.mode = zoom::Mode::Precise;
            z.follow = false;
        }
        let cam = z.resolve().map(|r| zoom::sample(&zoom::segments(&[r]), t)).filter(|b| *b != ZBox::FULL).unwrap_or(z.a);
        let k = z.add_point(t, cam);
        self.goto_zone_box(i, k, false);
    }

    /// Remove zone `i`'s point `k` (1 or later).
    fn remove_zone_point(&mut self, i: usize, k: usize) {
        let Some(z) = self.zones.get_mut(i) else { return };
        if k == 0 || k > z.points.len() {
            return;
        }
        z.points.remove(k - 1);
        if self.sel_zone == Some(i) && self.edit_point >= k {
            self.edit_point = if self.edit_point == k { 0 } else { self.edit_point - 1 };
        }
    }

    /// Remember where playback starts: when the player goes from paused to
    /// playing, the spot it was paused at becomes the ghost line. Only while
    /// watching the source, since the exported file has its own timeline.
    fn track_ghost(&mut self) {
        if self.media.is_none() || !self.can_mark() {
            self.was_paused = true;
            return;
        }
        let paused = self.paused();
        if paused {
            self.paused_pos = self.position();
        } else if self.was_paused {
            self.ghost = Some(self.paused_pos);
        }
        self.was_paused = paused;
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
            pitch_correct: true,
            volume: 100,
            listen: self.settings.preview_volume,
            mute: false,
            no_audio: false,
            no_video: false,
            mix: if exported { tracks::PreviewAudio::Normal } else { self.preview_mix.clone() },
        };
        match self.view {
            View::Original => neutral(src, false),
            View::Result => match &self.exported_file {
                Some(f) => neutral(Some(f.clone()), true),
                None => PreviewState {
                    file: src,
                    exported: false,
                    range: self.trim(),
                    pitch_correct: !o.change_pitch,
                    volume: if o.mode == CutMode::Fast { 100 } else { o.volume_pct },
                    listen: self.settings.preview_volume,
                    mute: o.av == AvMode::Mute,
                    no_audio: o.av == AvMode::RemoveAudio,
                    no_video: o.av == AvMode::AudioOnly,
                    mix: self.preview_mix.clone(),
                },
            },
        }
    }

    /// The zoom the live Result preview should show right now.
    fn wanted_live_zoom(&self) -> ZBox {
        let live = self.view == View::Result && self.exported_file.is_none();
        if !live || self.settings.zoom_static_preview {
            return ZBox::FULL;
        }
        let zones = self.active_zones();
        if zones.is_empty() {
            return ZBox::FULL;
        }
        zoom::sample(&zoom::segments(&zones), self.position())
    }

    fn apply_preview(&mut self) {
        // An export made with different settings no longer shows "the result".
        if self.exported_file.is_some() && self.export_sig != self.export_sig() {
            self.exported_file = None;
        }
        // Zoom preview: mpv's own zoom and pan, from the same maths as the
        // FFmpeg filter. Changes every frame, so it's applied separately.
        let z = self.wanted_live_zoom();
        if z != self.live_zoom {
            if let Some(v) = &self.video {
                let p = &v.player;
                p.set_property("video-zoom", &(1.0 / z.s).log2().to_string());
                p.set_property("video-pan-x", &(0.5 - (z.x + z.s / 2.0)).to_string());
                p.set_property("video-pan-y", &(0.5 - (z.y + z.s / 2.0)).to_string());
            }
            self.live_zoom = z;
        }
        let want = self.desired_preview();
        // Speed zones: mpv's own speed (and mute for muted zones), following
        // the playhead. Like the zoom, this changes as it plays.
        let live = self.view == View::Result && self.exported_file.is_none();
        let ls = if live {
            self.time_map().zone_at(self.position()).map_or((1.0, false), |z| (z.speed, z.mute))
        } else {
            (1.0, false)
        };
        if ls != self.live_speed {
            if let Some(v) = &self.video {
                v.player.set_property("speed", &ls.0.to_string());
                v.player.set_property("mute", if want.mute || ls.1 { "yes" } else { "no" });
            }
            self.live_speed = ls;
        }
        if self.applied.as_ref() == Some(&want) {
            return;
        }
        let Some(v) = &self.video else { return };
        let p = &v.player;
        let prev = self.applied.clone();
        // Several audio tracks: mpv mixes them with a filter graph. Set it
        // before loading a file, so a graph naming track 2 never meets a file
        // that only has one.
        let graph = match (&want.mix, want.no_audio) {
            (tracks::PreviewAudio::Graph(g), false) => g.as_str(),
            _ => "",
        };
        p.set_property("lavfi-complex", graph);
        if graph.is_empty() {
            let silent = want.no_audio || want.mix == tracks::PreviewAudio::Silent;
            p.set_property("aid", if silent { "no" } else { "auto" });
        }
        if want.file != self.loaded {
            if let Some(f) = &want.file {
                p.load_file(f);
                self.awaiting_frame = Some(Instant::now());
            }
            self.loaded = want.file.clone();
        }
        p.set_property("speed", &self.live_speed.0.to_string());
        p.set_property("audio-pitch-correction", if want.pitch_correct { "yes" } else { "no" });
        // The listening volume scales whatever the export would sound like.
        let heard = want.volume as u64 * self.settings.preview_volume as u64 / 100;
        p.set_property("volume", &heard.to_string());
        p.set_property("mute", if want.mute || self.live_speed.1 { "yes" } else { "no" });
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
        if let Some(v) = &mut self.video {
            let ev = v.player.poll_events();
            if ev.file_loaded {
                self.applied = None; // re-apply track/loop settings to the new file
            }
            if ev.file_loaded || ev.restarted {
                v.force_frames = 3;
            }
            if ev.restarted {
                self.awaiting_frame = None;
            }
        }
        let mix = tracks::preview_audio(&self.tracks);
        if mix != self.preview_mix && !ctx.input(|i| i.pointer.any_down()) {
            self.preview_mix = mix;
        }
        self.settle_scrub(&ctx);
        self.track_ghost();
        if let Some(t) = self.awaiting_frame {
            if t.elapsed() < Duration::from_secs(5) {
                ctx.request_repaint_after(Duration::from_millis(30));
            } else {
                self.awaiting_frame = None;
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
        self.finish_export(&ctx);
        self.poll_klipy();
        self.poll_track();
        self.sync_montage();
        if self.library.poll(&ctx) {
            self.catch_up_history();
        }
        self.finished.poll(&ctx);
        self.refresh_dir_status();

        egui::Panel::top("topbar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let open = self.settings.clips_open;
                // The ☰ glyph isn't in egui's built-in fonts, so the icon is drawn:
                // leading spaces leave room for it.
                let btn = ui.selectable_label(open, "      Clips");
                let r = btn.rect;
                let (x0, x1, cy) = (r.left() + 10.0, r.left() + 24.0, r.center().y);
                for dy in [-4.5, 0.0, 4.5] {
                    ui.painter().line_segment([egui::pos2(x0, cy + dy), egui::pos2(x1, cy + dy)], Stroke::new(1.6, theme::TEXT));
                }
                if btn.on_hover_text(if open { "Hide the Clips drawer" } else { "Show your clips, grouped by game" }).clicked() {
                    self.settings.clips_open = !open;
                    if !open {
                        self.rescan_lists(&ctx);
                    }
                }
                ui.add_space(4.0);
                ui.label(RichText::new("cheapedit").strong().size(16.0).color(theme::ACCENT));
                if winutil::is_dev() {
                    dev_badge(ui);
                }
                ui.add_space(8.0);
                if ui.button("Open video").on_hover_text(self.binding_hint(Action::OpenFile)).clicked() {
                    self.open_dialog();
                }
                if ui
                    .selectable_label(self.montage_on, " Montage ")
                    .on_hover_text("Edit several clips one after another and join them into one file")
                    .clicked()
                {
                    self.montage_on = !self.montage_on;
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
                    if ui.button("Features").on_hover_text("KLIPY GIF search and other extras").clicked() {
                        self.show_features = true;
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

        if self.settings.clips_open {
            let r = egui::Panel::left("clips")
                .resizable(true)
                .default_size(self.settings.clips_width)
                .size_range(190.0..=480.0)
                .show(ui, |ui| self.clips_ui(ui, frame));
            let w = r.response.rect.width().round();
            if (w - self.settings.clips_width).abs() >= 1.0 {
                self.settings.clips_width = w;
            }
        }

        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.preview_ui(ui, frame);
        });

        if self.show_log {
            self.log_window(&ctx);
        }
        if self.show_features {
            self.features_window(&ctx);
        }
        if self.montage_on && self.montage.is_none() {
            self.montage_picker(&ctx);
        }
        if self.show_klipy {
            self.klipy_window(&ctx);
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
            theme::hint(ui, "Live preview shows your trim, speed, audio, zoom and overlays. Quality and format changes only show in the exported file.");
        }
        ui.add_space(4.0);

        // Leave room below the video for the timeline, controls and shortcut strip.
        let rows = self.show_bands() as u8 + self.show_speed_bands() as u8 + self.show_overlay_bands() as u8;
        let below = 160.0 + rows as f32 * 22.0 + self.lanes_h + self.montage_strip_height();
        let avail = ui.available_size();
        let video_h = (avail.y - below).max(120.0);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail.x, video_h), Sense::click_and_drag());
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
        let mut on_overlay = false;
        if let Some((w, h)) = self.media.as_ref().and_then(|m| m.video.as_ref()).map(|v| (v.width as f32, v.height as f32)) {
            if self.video.is_some() {
                let vr = fit_rect(rect, w, h);
                self.zoom_overlay(ui, &resp, rect, vr);
                on_overlay = self.overlays_on_video(ui, &resp, rect, vr);
            }
        }
        if resp.clicked() && has_media && !on_overlay {
            self.handle_action(Action::PlayPause, &ui.ctx().clone());
        }

        ui.add_space(6.0);
        self.timeline_ui(ui);
        if self.montage_on {
            ui.add_space(6.0);
            self.montage_strip_ui(ui);
        }
        ui.add_space(4.0);
        self.transport_ui(ui);
        ui.add_space(2.0);
        self.shortcut_strip(ui);
    }

    /// Zone bands under the timeline: shown while editing the source.
    fn show_bands(&self) -> bool {
        !self.zones.is_empty() && !(self.view == View::Result && self.exported_file.is_some())
    }

    fn show_speed_bands(&self) -> bool {
        !self.speed_zones.is_empty() && !(self.view == View::Result && self.exported_file.is_some())
    }

    /// Speed bands under the timeline, labelled with their speed. Click to
    /// select, drag an edge to change the time (the timeline re-flows when
    /// you let go).
    fn speed_bands_ui(&mut self, ui: &mut egui::Ui, axis: &Axis) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, theme::CARD.gamma_multiply(0.6));
        let enabled = self.speed_blocked_reason().is_none();
        let trim = self.trim();
        let mut spans: Vec<(usize, egui::Rect, bool)> = Vec::new();
        for (i, z) in self.speed_zones.iter().enumerate() {
            let Some((s, e)) = z.times(trim) else { continue };
            if e <= s {
                continue;
            }
            let r = egui::Rect::from_x_y_ranges(axis.x_of(s)..=axis.x_of(e).max(axis.x_of(s) + 2.0), rect.y_range());
            let sel = self.sel_speed == Some(i);
            let mut col = speed_color(z.speed).gamma_multiply(if sel { 0.95 } else { 0.55 });
            if !enabled {
                col = col.gamma_multiply(0.4);
            }
            painter.rect_filled(r, 2.0, col);
            if sel {
                painter.rect_stroke(r, 2.0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Inside);
            }
            let mut label = z.label();
            if z.mute {
                label.push_str(" muted");
            }
            if r.width() > 30.0 {
                painter.text(r.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.0), Color32::BLACK);
            }
            spans.push((i, r, z.whole));
        }
        if !enabled {
            return;
        }
        let edge_at = |p: egui::Pos2| -> Option<(usize, bool)> {
            spans.iter().filter(|s| !s.2).find_map(|&(i, r, _)| {
                if (p.x - r.left()).abs() < 5.0 {
                    Some((i, true))
                } else if (p.x - r.right()).abs() < 5.0 {
                    Some((i, false))
                } else {
                    None
                }
            })
        };
        let zone_at = |p: egui::Pos2| spans.iter().find(|s| s.1.contains(p)).map(|s| s.0);
        if let Some(p) = resp.hover_pos() {
            if edge_at(p).is_some() || self.speed_band_drag.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            } else if zone_at(p).is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some(i) = zone_at(p) {
                let z = &self.speed_zones[i];
                if let Some((s, e)) = z.times(trim) {
                    resp.clone().on_hover_text(format!("Speed zone {}: {} - {} at {}", i + 1, timecode::format_short(s), timecode::format_short(e), z.label()));
                }
            }
        }
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.speed_band_drag = edge_at(p);
                if let Some((i, _)) = self.speed_band_drag {
                    self.select_speed(i);
                }
            }
        }
        if resp.dragged() {
            if let (Some((i, is_start)), Some(p)) = (self.speed_band_drag, resp.interact_pointer_pos()) {
                let t = axis.t_of(p.x);
                let z = &mut self.speed_zones[i];
                if is_start {
                    z.start_text = timecode::format_compact(t);
                } else {
                    z.end_text = timecode::format_compact(t);
                }
                self.scrub_to(t);
            }
        }
        if resp.drag_stopped() {
            self.speed_band_drag = None;
            self.end_scrub();
        }
        if resp.clicked() {
            if let Some(i) = resp.interact_pointer_pos().and_then(zone_at) {
                self.select_speed(i);
                if let Some((s, _)) = self.speed_zones[i].times(trim) {
                    self.seek(s);
                }
            }
        }
    }

    /// Zoom boxes on the video, and dragging them.
    fn zoom_overlay(&mut self, ui: &mut egui::Ui, resp: &egui::Response, rect: egui::Rect, vr: egui::Rect) {
        let painter = ui.painter_at(rect);
        let live = self.view == View::Result && self.exported_file.is_none();
        // mpv's zoom spills into the black bars; keep the bars black.
        if live && self.live_zoom != ZBox::FULL {
            for r in [
                egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), vr.top())),
                egui::Rect::from_min_max(egui::pos2(rect.left(), vr.bottom()), rect.max),
                egui::Rect::from_min_max(egui::pos2(rect.left(), vr.top()), egui::pos2(vr.left(), vr.bottom())),
                egui::Rect::from_min_max(egui::pos2(vr.right(), vr.top()), egui::pos2(rect.right(), vr.bottom())),
            ] {
                painter.rect_filled(r, 0.0, Color32::BLACK);
            }
        }
        if self.zones.is_empty() || !self.zoom_allowed() {
            return;
        }
        let to_screen = |b: ZBox| {
            egui::Rect::from_min_size(
                egui::pos2(vr.left() + b.x as f32 * vr.width(), vr.top() + b.y as f32 * vr.height()),
                egui::vec2(b.s as f32 * vr.width(), b.s as f32 * vr.height()),
            )
        };
        let font = egui::FontId::proportional(12.0);
        let pos = self.position();

        if live {
            if self.settings.zoom_static_preview {
                let b = zoom::sample(&zoom::segments(&self.active_zones()), pos);
                if b != ZBox::FULL {
                    let r = to_screen(b);
                    painter.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
                    painter.text(r.left_top() + egui::vec2(5.0, 4.0), egui::Align2::LEFT_TOP, format!("Zoom {:.1}x", b.zoom()), font, Color32::WHITE);
                }
            }
            return;
        }
        if self.view != View::Original {
            return;
        }

        // Other zones, faintly, while the playhead is inside their time.
        for (i, z) in self.zones.iter().enumerate() {
            if Some(i) == self.sel_zone {
                continue;
            }
            if let Some((s, e)) = z.times() {
                if pos >= s && pos < e && z.a_drawn {
                    let col = zone_color(i).gamma_multiply(0.55);
                    let r = to_screen(z.a);
                    painter.rect_stroke(r, 0.0, Stroke::new(1.5, col), egui::StrokeKind::Inside);
                    painter.text(r.left_top() + egui::vec2(5.0, 3.0), egui::Align2::LEFT_TOP, format!("{}", i + 1), font.clone(), col);
                }
            }
        }

        let Some(i) = self.sel_zone.filter(|&i| i < self.zones.len()) else { return };
        let col = zone_color(i);
        let z = self.zones[i].clone();
        if self.fix_zone == Some(i) {
            self.fix_on_video(ui, resp, vr, i);
            return;
        }
        if self.lasso_mode && z.follows() {
            self.start_loop_on_video(ui, resp, vr, i);
            return;
        }
        // A tracked zone after its first frame: show where the tracked box is.
        let first_frame = 1.0 / self.media.as_ref().and_then(|m| m.video.as_ref()).map_or(30.0, |v| v.fps.max(1.0));
        if z.tracked() && !self.drawing && self.box_drag.is_none() && z.times().is_some_and(|(s, e)| pos > s + first_frame && pos < e) {
            if let Some(path) = z.resolve().and_then(|r| r.path) {
                let b = zoom::box_at(&path, pos);
                let r = to_screen(b);
                painter.rect_stroke(r, 0.0, Stroke::new(2.0, col), egui::StrokeKind::Inside);
                painter.text(r.left_top() + egui::vec2(6.0, 5.0), egui::Align2::LEFT_TOP, format!("Zone {} - tracked", i + 1), font.clone(), col);
                // The loop rides along where the target is.
                let lasso = z.loop_at(pos);
                if !lasso.is_empty() {
                    track_ui::paint_loop(&painter, vr, lasso, b, col.gamma_multiply(0.8));
                }
            }
            return;
        }
        // Precise points: the point being edited, when the playhead is on it.
        let multi = z.uses_points() && !z.points.is_empty();
        let k = if multi && self.edit_point <= z.points.len() { self.edit_point } else { 0 };
        if multi && !self.drawing && self.box_drag.is_none() {
            let on_point = point_frame(&z, k, first_frame).is_some_and(|t| (pos - t).abs() < first_frame * 0.75);
            if !on_point {
                // Elsewhere: where the camera is, not editable.
                let cam = z.resolve().map(|r| zoom::sample(&zoom::segments(&[r]), pos)).filter(|b| *b != ZBox::FULL);
                if let (Some(b), true) = (cam, z.times().is_some_and(|(s, e)| pos >= s && pos < e)) {
                    let r = to_screen(b);
                    painter.rect_stroke(r, 0.0, Stroke::new(1.5, col.gamma_multiply(0.8)), egui::StrokeKind::Inside);
                    painter.text(r.left_top() + egui::vec2(6.0, 5.0), egui::Align2::LEFT_TOP, format!("Zone {} - camera", i + 1), font.clone(), col);
                }
                return;
            }
        }
        let cur = if k == 0 { z.a } else { z.points[k - 1].b };
        let cur_drawn = k > 0 || z.a_drawn;
        let r = to_screen(cur);
        if multi {
            // The points either side, thin, with arrows showing the way the camera goes.
            for j in [k.checked_sub(1), Some(k + 1)].into_iter().flatten().filter(|&j| j <= z.points.len()) {
                let ob = if j == 0 { z.a } else { z.points[j - 1].b };
                let other = to_screen(ob);
                painter.rect_stroke(other, 0.0, Stroke::new(1.0, col.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                painter.text(other.left_top() + egui::vec2(5.0, 3.0), egui::Align2::LEFT_TOP, format!("point {}", j + 1), font.clone(), col.gamma_multiply(0.8));
                let (from, to) = if j < k { (other.center(), r.center()) } else { (r.center(), other.center()) };
                if from.distance(to) > 12.0 {
                    painter.arrow(from, to - from, Stroke::new(1.5, col.gamma_multiply(0.7)));
                }
            }
        }
        if cur_drawn || self.box_drag.is_some() {
            painter.rect_filled(r, 0.0, col.gamma_multiply(0.08));
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, col), egui::StrokeKind::Inside);
            for c in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()] {
                painter.rect_filled(egui::Rect::from_center_size(c, egui::vec2(8.0, 8.0)), 1.0, col);
            }
            let mut label = format!("Zone {}", i + 1);
            if multi {
                label.push_str(&format!(" - point {}", k + 1));
            }
            label.push_str(&format!("  {:.1}x", cur.zoom()));
            painter.text(r.left_top() + egui::vec2(6.0, 5.0), egui::Align2::LEFT_TOP, label, font.clone(), col);
            if z.follows() && !z.lasso.is_empty() {
                track_ui::paint_loop(&painter, vr, &z.lasso, cur, col);
            }
        }
        if self.drawing && self.box_drag.is_none() {
            let which = if multi { format!("point {}'s box", k + 1) } else { "box".to_string() };
            let msg = format!("Drag on the video to draw zone {}'s {which}", i + 1);
            let at = egui::pos2(vr.center().x, vr.top() + 18.0);
            let galley = painter.layout_no_wrap(msg, egui::FontId::proportional(14.0), Color32::WHITE);
            let bg = egui::Rect::from_center_size(at, galley.size() + egui::vec2(16.0, 8.0));
            painter.rect_filled(bg, 4.0, Color32::from_black_alpha(190));
            painter.galley(bg.min + egui::vec2(8.0, 4.0), galley, Color32::WHITE);
        }

        // ---- interaction
        let to_frac = |p: egui::Pos2| (((p.x - vr.left()) / vr.width()) as f64, ((p.y - vr.top()) / vr.height()) as f64);
        let corners = [
            (r.left_top(), (cur.x + cur.s, cur.y + cur.s), egui::CursorIcon::ResizeNwSe),
            (r.right_top(), (cur.x, cur.y + cur.s), egui::CursorIcon::ResizeNeSw),
            (r.left_bottom(), (cur.x + cur.s, cur.y), egui::CursorIcon::ResizeNeSw),
            (r.right_bottom(), (cur.x, cur.y), egui::CursorIcon::ResizeNwSe),
        ];
        let corner_at = |p: egui::Pos2| if cur_drawn { corners.iter().find(|c| c.0.distance(p) < 10.0).copied() } else { None };
        if let Some(p) = resp.hover_pos() {
            let icon = if self.box_drag.is_some() || self.drawing {
                egui::CursorIcon::Crosshair
            } else if let Some(c) = corner_at(p) {
                c.2
            } else if cur_drawn && r.contains(p) {
                egui::CursorIcon::Move
            } else {
                egui::CursorIcon::Crosshair
            };
            ui.ctx().set_cursor_icon(icon);
        }
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                let f = to_frac(p);
                self.box_drag = Some(if self.drawing {
                    BoxDrag::Draw(f)
                } else if let Some(c) = corner_at(p) {
                    BoxDrag::Resize(c.1)
                } else if cur_drawn && r.contains(p) {
                    BoxDrag::Move((f.0 - cur.x, f.1 - cur.y))
                } else {
                    BoxDrag::Draw(f)
                });
            }
        }
        if resp.dragged() {
            if let (Some(d), Some(p)) = (self.box_drag, resp.interact_pointer_pos()) {
                let f = to_frac(p);
                let nb = match d {
                    BoxDrag::Draw(a) | BoxDrag::Resize(a) => ZBox::from_drag(a, f),
                    BoxDrag::Move(off) => ZBox { x: f.0 - off.0, y: f.1 - off.1, s: cur.s }.clamped(),
                };
                let z = &mut self.zones[i];
                if k == 0 {
                    z.a = nb;
                    z.a_drawn = true;
                } else {
                    z.points[k - 1].b = nb;
                }
            }
        }
        if resp.drag_stopped() {
            self.box_drag = None;
            self.drawing = false;
        }
    }

    /// Numbered, coloured zone bands under the timeline. Click to select,
    /// drag an edge to change the time.
    fn zone_bands_ui(&mut self, ui: &mut egui::Ui, x_of: &dyn Fn(f64) -> f32, t_of: &dyn Fn(f32) -> f64) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, theme::CARD.gamma_multiply(0.6));
        let enabled = self.zoom_allowed();
        let mut spans: Vec<(usize, egui::Rect)> = Vec::new();
        let mut marks: Vec<(usize, usize, f32)> = Vec::new();
        for (i, z) in self.zones.iter().enumerate() {
            let Some((s, e)) = z.times() else { continue };
            if e <= s {
                continue;
            }
            let r = egui::Rect::from_x_y_ranges(x_of(s)..=x_of(e).max(x_of(s) + 2.0), rect.y_range());
            let sel = self.sel_zone == Some(i);
            let mut col = zone_color(i).gamma_multiply(if sel { 0.95 } else { 0.55 });
            if !enabled {
                col = col.gamma_multiply(0.4);
            }
            painter.rect_filled(r, 2.0, col);
            if sel {
                painter.rect_stroke(r, 2.0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Inside);
            }
            if r.width() > 12.0 {
                painter.text(r.center(), egui::Align2::CENTER_CENTER, format!("{}", i + 1), egui::FontId::proportional(11.0), Color32::BLACK);
            }
            spans.push((i, r));
            // Precise points: a diamond for each (point 1 sits on the zone start).
            if z.uses_points() && !z.points.is_empty() {
                let problems = z.point_problems();
                for k in 0..=z.points.len() {
                    let Some(t) = (if k == 0 { Some(s) } else { z.points[k - 1].t() }) else { continue };
                    let x = x_of(t.clamp(s, e)).clamp(r.left() + 3.0, r.right() - 3.0);
                    let used = k == 0 || problems[k - 1].is_none();
                    let on = sel && self.edit_point == k;
                    marks.push((i, k, x));
                    let c = egui::pos2(x, rect.center().y);
                    let d = if on { 5.5 } else { 4.5 };
                    let shape = vec![c + egui::vec2(0.0, -d), c + egui::vec2(d, 0.0), c + egui::vec2(0.0, d), c + egui::vec2(-d, 0.0)];
                    let fill = if on { Color32::WHITE } else if used { Color32::from_black_alpha(200) } else { Color32::TRANSPARENT };
                    let stroke = Stroke::new(1.0, if on { Color32::BLACK } else if used { Color32::WHITE } else { Color32::from_gray(140) });
                    painter.add(egui::Shape::convex_polygon(shape, fill, stroke));
                }
            }
        }
        if !enabled {
            return;
        }
        // A point marker under the pointer (point 1 only for clicking: its edge drags the zone).
        let mark_at = |p: egui::Pos2, movable: bool| marks.iter().filter(|m| !movable || m.1 > 0).find(|m| (p.x - m.2).abs() < 5.0 && rect.contains(p)).map(|m| (m.0, m.1));
        let edge_at = |p: egui::Pos2| -> Option<(usize, bool)> {
            spans.iter().find_map(|&(i, r)| {
                if (p.x - r.left()).abs() < 5.0 {
                    Some((i, true))
                } else if (p.x - r.right()).abs() < 5.0 {
                    Some((i, false))
                } else {
                    None
                }
            })
        };
        let zone_at = |p: egui::Pos2| spans.iter().find(|s| s.1.contains(p)).map(|s| s.0);
        if let Some(p) = resp.hover_pos() {
            if mark_at(p, true).is_some() || edge_at(p).is_some() || self.band_drag.is_some() || self.point_drag.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            } else if zone_at(p).is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some((i, k)) = mark_at(p, false) {
                if let Some((t, _)) = self.zones[i].point(k) {
                    let how = if k == 0 { "click to edit" } else { "click to edit, drag to move" };
                    resp.clone().on_hover_text(format!("Zone {} point {}: {} ({how})", i + 1, k + 1, timecode::format_short(t)));
                }
            } else if let Some(i) = zone_at(p) {
                if let Some((s, e)) = self.zones[i].times() {
                    let more = if self.zones[i].uses_points() { "\nDouble-click to add a point there" } else { "" };
                    resp.clone().on_hover_text(format!("Zone {}: {} - {}{more}", i + 1, timecode::format_short(s), timecode::format_short(e)));
                }
            }
        }
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                if let Some((i, k)) = mark_at(p, true) {
                    self.point_drag = Some((i, k));
                    self.sel_zone = Some(i);
                    self.edit_point = k;
                    self.drawing = false;
                } else {
                    self.band_drag = edge_at(p);
                    if let Some((i, _)) = self.band_drag {
                        self.sel_zone = Some(i);
                        self.edit_point = 0;
                        self.drawing = false;
                    }
                }
            }
        }
        if resp.dragged() {
            if let (Some((i, k)), Some(p)) = (self.point_drag, resp.interact_pointer_pos()) {
                // Kept inside the zone; the card says if it lands somewhere unusable.
                if let Some((s, e)) = self.zones[i].times() {
                    let t = t_of(p.x).clamp(s, e);
                    if let Some(key) = self.zones[i].points.get_mut(k - 1) {
                        key.text = timecode::format_compact(t);
                    }
                    self.scrub_to(t);
                }
            }
            if let (Some((i, is_start)), Some(p)) = (self.band_drag, resp.interact_pointer_pos()) {
                let t = t_of(p.x);
                let z = &mut self.zones[i];
                if is_start {
                    z.start_text = timecode::format_compact(t);
                } else {
                    z.end_text = timecode::format_compact(t);
                }
                self.scrub_to(t);
            }
        }
        if resp.drag_stopped() {
            self.band_drag = None;
            if let Some((i, k)) = self.point_drag.take() {
                self.edit_point = self.zones[i].sort_points(k);
            }
            self.end_scrub();
        }
        if resp.double_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                if let Some(i) = zone_at(p).filter(|&i| self.zones[i].uses_points() && mark_at(p, false).is_none()) {
                    self.add_zone_point(i, t_of(p.x));
                }
            }
        } else if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                if let Some((i, k)) = mark_at(p, false) {
                    self.goto_zone_box(i, k, false);
                } else if let Some(i) = zone_at(p) {
                    self.select_zone(i);
                }
            }
        }
    }

    fn timeline_ui(&mut self, ui: &mut egui::Ui) {
        let exported_view = self.view == View::Result && self.exported_file.is_some();
        let duration = if exported_view {
            self.video.as_ref().and_then(|v| v.player.duration()).unwrap_or(1.0)
        } else {
            self.media.as_ref().map(|m| m.duration).unwrap_or(1.0)
        }
        .max(0.001);
        let pos = self.position();
        // Re-flow only when no mouse button is down, so nothing moves mid-drag.
        let live_map = self.time_map();
        if !ui.input(|i| i.pointer.any_down()) {
            self.shown_map = live_map.clone();
        }
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect.expand(2.0));
        painter.rect_filled(rect, 5.0, theme::CARD);
        let axis = Axis::new(rect.x_range(), if exported_view { TimeMap::default() } else { self.shown_map.clone() }, duration);
        let x_of = |t: f64| axis.x_of(t);
        let t_of = |x: f32| axis.t_of(x);

        let range = if exported_view { None } else { self.trim() };
        if let Some((s, e)) = range {
            let r = egui::Rect::from_x_y_ranges(x_of(s)..=x_of(e), rect.y_range());
            painter.rect_filled(r, 0.0, theme::ACCENT_DIM.gamma_multiply(0.8));
        }
        if self.settings.show_waveform && !exported_view {
            // What the export will sound like: the tracks that are on, at their volumes.
            let heard: Vec<(Arc<waveform::Peaks>, f32)> = if self.tracks.len() < 2 {
                self.waves.track(0).map(|p| (p, 1.0)).into_iter().collect()
            } else {
                self.tracks.iter().enumerate().filter(|(_, t)| t.on).filter_map(|(i, t)| Some((self.waves.track(i)?, t.gain() as f32))).collect()
            };
            if !heard.is_empty() {
                let level = |t0: f64, t1: f64| heard.iter().map(|(p, g)| waveform::peak_between(p, t0, t1).unwrap_or(0.0) * g).sum::<f32>().min(1.0);
                draw_wave(&painter, rect.shrink2(egui::vec2(0.0, 4.0)), &|f| axis.t_at(f), Color32::from_white_alpha(46), level);
            }
        }
        if let Some((s, e)) = range {
            for x in [x_of(s), x_of(e)] {
                painter.rect_filled(egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(6.0, rect.height() + 4.0)), 2.0, theme::CORAL);
            }
        }
        if self.media.is_some() {
            if let (true, Some(g)) = (self.settings.show_ghost && !exported_view, self.ghost) {
                // Thin, faint "you started here" line with a small notch on top.
                let gx = x_of(g);
                let col = Color32::from_white_alpha(110);
                painter.line_segment([egui::pos2(gx, rect.top()), egui::pos2(gx, rect.bottom())], Stroke::new(1.0, col));
                painter.add(egui::Shape::convex_polygon(
                    vec![egui::pos2(gx - 4.0, rect.top()), egui::pos2(gx + 4.0, rect.top()), egui::pos2(gx, rect.top() + 5.0)],
                    col,
                    Stroke::NONE,
                ));
            }
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
                    if resp.clicked() {
                        self.scrub = None;
                        self.seek(t);
                    } else {
                        self.scrub_to(t);
                    }
                }
            }
            if resp.drag_stopped() || resp.clicked() {
                self.drag = None;
                self.end_scrub();
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
        if self.show_speed_bands() && !exported_view {
            ui.add_space(3.0);
            self.speed_bands_ui(ui, &axis);
        }
        if self.show_bands() && !exported_view {
            ui.add_space(3.0);
            self.zone_bands_ui(ui, &|t| axis.x_of(t), &|x| axis.t_of(x));
        }
        if self.show_overlay_bands() && !exported_view {
            ui.add_space(3.0);
            self.overlay_bands_ui(ui, &axis);
        }
        // Measured, so the video above shrinks to make room for the lanes.
        let lanes_top = ui.cursor().top();
        if self.tracks.len() > 1 && !exported_view && self.media.is_some() {
            self.track_lanes_ui(ui, &axis, range);
        }
        let lanes_h = ui.cursor().top() - lanes_top;
        if (lanes_h - self.lanes_h).abs() > 0.5 {
            self.lanes_h = lanes_h;
            ui.ctx().request_repaint();
        }
        let click_hint = |a: Action| self.settings.keybinds.get(a).map(|b| b.display());
        let mut hint = match (click_hint(Action::TimelineStart), click_hint(Action::TimelineEnd)) {
            (Some(s), Some(e)) => format!("{s}: set start  ·  {e}: set end"),
            (Some(s), None) => format!("{s}: set start"),
            (None, Some(e)) => format!("{e}: set end"),
            (None, None) => String::new(),
        };
        let mut hint_col = theme::MUTED;
        // Mid-drag: the length the clip will have when you let go.
        if let (false, Some((s, e))) = (exported_view, range) {
            let (was, will) = (self.shown_map.out_len(s, e), live_map.out_len(s, e));
            if (was - will).abs() > 0.005 {
                hint = format!("New length {} to {}", timecode::format_short(was), timecode::format_short(will));
                hint_col = theme::CORAL;
            }
        }
        ui.horizontal(|ui| {
            let mono = |t: f64| RichText::new(timecode::format_short(t)).small().monospace().color(theme::MUTED);
            let left = ui.label(mono(0.0)).rect;
            if self.media.is_some() && !hint.is_empty() {
                // Centred under the timeline, regardless of the end-time label's width.
                let y = left.center().y;
                ui.painter().text(egui::pos2(rect.center().x, y), egui::Align2::CENTER_CENTER, &hint, egui::FontId::proportional(12.0), hint_col);
            }
            let mut show_wave = self.settings.show_waveform;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(mono(axis.total));
                if self.media.as_ref().is_some_and(|m| m.audio.is_some()) && !exported_view {
                    ui.add_space(8.0);
                    if ui
                        .selectable_label(show_wave, RichText::new("Waveform").small())
                        .on_hover_text("Show the audio's waveform on the timeline, so you can spot the loud moments")
                        .clicked()
                    {
                        show_wave = !show_wave;
                    }
                }
            });
            self.settings.show_waveform = show_wave;
        });
    }

    /// One lane per audio track under the timeline: its name, on/off, Solo
    /// and volume, over its own waveform. Clicking a lane's waveform seeks.
    fn track_lanes_ui(&mut self, ui: &mut egui::Ui, axis: &Axis, range: Option<(f64, f64)>) {
        let pos = self.position();
        let names: Vec<Option<String>> = self.media.as_ref().map(|m| m.audio_tracks.iter().map(|t| t.title.clone()).collect()).unwrap_or_default();
        let soloing = self.tracks.iter().any(|t| t.solo);
        let xs = egui::Rangef::new(axis.left, axis.left + axis.width);
        let x_of = |t: f64| axis.x_of(t);
        let t_of = |x: f32| axis.t_of(x);
        for i in 0..self.tracks.len() {
            ui.add_space(2.0);
            let mut t = self.tracks[i];
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let name = match names.get(i).cloned().flatten() {
                    Some(n) => format!("Track {} · {n}", i + 1),
                    None => format!("Track {}", i + 1),
                };
                ui.checkbox(&mut t.on, RichText::new(name).small().strong())
                    .on_hover_text("Include this track in the export (and the preview)");
                let solo = ui
                    .selectable_label(t.solo, RichText::new("Solo").small())
                    .on_hover_text("Hear only this track while you edit. Doesn't change the export.");
                if solo.clicked() {
                    t.solo = !t.solo;
                }
                ui.add_enabled(t.on, egui::Slider::new(&mut t.volume_pct, 0..=200).suffix("%").show_value(true))
                    .on_hover_text("This track's volume in the mix");
                if t.volume_pct != 100 && ui.small_button("Reset").on_hover_text("Back to 100%").clicked() {
                    t.volume_pct = 100;
                }
                if soloing && !t.solo {
                    theme::hint(ui, "muted by Solo");
                }
            });
            self.tracks[i] = t;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), Sense::click_and_drag());
            let rect = egui::Rect::from_x_y_ranges(xs, rect.y_range());
            let painter = ui.painter_at(rect.expand(2.0));
            painter.rect_filled(rect, 4.0, theme::CARD);
            if let Some((s, e)) = range {
                painter.rect_filled(egui::Rect::from_x_y_ranges(x_of(s)..=x_of(e), rect.y_range()), 0.0, theme::ACCENT_DIM.gamma_multiply(0.45));
            }
            let audible = if soloing { t.solo } else { t.on };
            let col = if audible { theme::ACCENT.gamma_multiply(0.9) } else { Color32::from_white_alpha(28) };
            match self.waves.track(i) {
                Some(p) => draw_wave(&painter, rect.shrink2(egui::vec2(0.0, 2.0)), &|f| axis.t_at(f), col, |a, b| waveform::peak_between(&p, a, b).unwrap_or(0.0) * t.gain() as f32),
                None => {
                    painter.text(rect.left_center() + egui::vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, "Reading audio…", egui::FontId::proportional(11.0), theme::MUTED);
                }
            }
            let px = x_of(pos);
            painter.line_segment([egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())], Stroke::new(1.5, Color32::WHITE));
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if let Some(p) = resp.interact_pointer_pos() {
                let t = t_of(p.x);
                if resp.clicked() {
                    self.scrub = None;
                    self.seek(t);
                } else if resp.dragged() {
                    self.scrub_to(t);
                }
            }
            if resp.drag_stopped() {
                self.end_scrub();
            }
        }
        if self.tracks.iter().all(|t| !t.on) {
            ui.label(RichText::new("Every track is off, so the export will have no audio.").small().color(theme::WARN));
        }
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
                    ui.add_space(10.0);
                    let ghost = self.ghost.filter(|_| self.settings.show_ghost);
                    let go_tip = match ghost {
                        Some(g) => format!("Move the playhead to the ghost line ({})", timecode::format_short(g)),
                        None => "Move the playhead to the ghost line. It appears where you start playing.".into(),
                    };
                    if ui.add_enabled(ghost.is_some(), egui::Button::new("Go to ghost")).on_hover_text(&go_tip).on_disabled_hover_text(&go_tip).clicked() {
                        if let Some(g) = ghost {
                            self.seek(g);
                            self.paused_pos = g;
                        }
                    }
                    let play_tip = match ghost {
                        Some(g) => format!("Jump back to the ghost line ({}) and play from there", timecode::format_short(g)),
                        None => "Play from the ghost line. It appears where you start playing.".into(),
                    };
                    if ui.add_enabled(ghost.is_some(), egui::Button::new("Play at ghost")).on_hover_text(&play_tip).on_disabled_hover_text(&play_tip).clicked() {
                        if let Some(g) = ghost {
                            self.seek(g);
                            // Starting from the ghost keeps the ghost where it is.
                            self.paused_pos = g;
                            if let Some(v) = &self.video {
                                v.player.set_property("pause", "no");
                            }
                        }
                    }
                    let tip = match ghost {
                        Some(g) => format!("Set the start to the ghost line ({}), where you last pressed play", timecode::format_short(g)),
                        None => "Set the start to the ghost line. It appears where you start playing.".into(),
                    };
                    if ui.add_enabled(ghost.is_some(), egui::Button::new("Start at ghost")).on_hover_text(&tip).on_disabled_hover_text(&tip).clicked() {
                        if let Some(g) = ghost {
                            self.start_text = timecode::format_compact(g);
                        }
                    }
                });
                if ui
                    .selectable_label(self.settings.show_ghost, "Ghost")
                    .on_hover_text("Show a faint line on the timeline where you last started playing")
                    .clicked()
                {
                    self.settings.show_ghost = !self.settings.show_ghost;
                }

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
            // Preview-only listening volume, at the right of this row.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().slider_width = 100.0;
                let hint = "How loud the preview plays. Only changes what you hear here. Your clip's volume is set in Audio and video.";
                ui.add(egui::Slider::new(&mut self.settings.preview_volume, 0..=100).show_value(false)).on_hover_text(hint);
                ui.label(RichText::new(format!("Listen {}%", self.settings.preview_volume)).small().color(theme::MUTED)).on_hover_text(hint);
            });
        });
    }

    // ------------------------------------------------------------------ right side

    fn tools_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let caps = self.caps();
        let media = self.media.clone();
        let busy = self.exporting();

        if self.montage_on {
            self.montage_panel_ui(ui);
            ui.separator();
        }

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
        let auto = match &media {
            Some(m) => self.auto_name(m),
            None => "automatic name".into(),
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
            let len = job::output_duration(s, e, &self.active_speeds());
            ui.label(format!("New clip length: {}", if len < 60.0 { format!("{len:.2} s") } else { timecode::format(len) }));
        }
        ui.separator();

        // 4. Zoom
        self.zoom_ui(ui);
        ui.separator();

        // Overlays
        self.overlays_ui(ui);
        ui.separator();

        // 4. Cut mode
        theme::section(ui, "Cut mode");
        let discord = self.discord_active();
        let preset = self.settings.options.preset;
        let o = &mut self.settings.options;
        ui.add_enabled_ui(!discord, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut o.mode, CutMode::Precise, "  Precise  ");
                ui.selectable_value(&mut o.mode, CutMode::Fast, "  Fast  ");
            });
        });
        let mode = if discord { CutMode::Precise } else { o.mode };
        if discord {
            theme::hint(ui, format!("Set by {}: it always re-encodes, so the file can fit the size.", preset.label()));
        }
        match mode {
            CutMode::Precise => theme::hint(ui, "Cuts are exact to the frame. Slower, and quality depends on your settings."),
            CutMode::Fast => theme::hint(
                ui,
                "Done in about a second with no quality loss, but the start snaps to the nearest earlier keyframe, so your clip may begin slightly early.",
            ),
        }
        let fast = mode == CutMode::Fast;
        ui.separator();

        // 5. Speed zones
        self.speed_ui(ui);
        ui.separator();

        // 6. Audio and video
        theme::section(ui, "Audio and video");
        let has_audio = media.as_ref().map(|m| m.audio.is_some()).unwrap_or(true);
        let has_video = media.as_ref().map(|m| m.video.is_some()).unwrap_or(true);
        let o = &mut self.settings.options;
        let mut av = if !has_video {
            AvMode::AudioOnly
        } else if !has_audio || (discord && o.av == AvMode::AudioOnly) {
            AvMode::Both
        } else {
            o.av
        };
        let shown = av;
        ui.add_enabled_ui(has_audio && has_video, |ui| {
            egui::ComboBox::from_id_salt("avmode").width(ui.available_width()).selected_text(av.label()).show_ui(ui, |ui| {
                for m in AvMode::ALL {
                    let blocked = discord && m == AvMode::AudioOnly;
                    let r = ui.add_enabled(!blocked, egui::Button::selectable(av == m, m.label()));
                    if r.clicked() {
                        av = m;
                    }
                    r.on_disabled_hover_text("Not available with Discord presets.");
                }
            });
        });
        // Only a real choice is saved, so a preset never changes your own settings.
        if has_audio && has_video && av != shown {
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

        // 7. Quality and codec, with the preset on top
        theme::section(ui, "Quality and codec");
        if has_video {
            ui.horizontal(|ui| {
                ui.label("Preset");
                let o = &mut self.settings.options;
                egui::ComboBox::from_id_salt("preset").width(ui.available_width()).selected_text(o.preset.label()).show_ui(ui, |ui| {
                    for p in Preset::ALL {
                        ui.selectable_value(&mut o.preset, p, p.label()).on_hover_text(p.hint());
                    }
                });
            });
            theme::hint(ui, self.settings.options.preset.hint());
        }
        if av != AvMode::AudioOnly {
            self.quality_ui(ui, caps.as_ref(), media.as_ref(), fast);
        }
        ui.separator();

        // 8. Estimate, then Export (or Finish montage)
        let problems = if self.montage_on { Vec::new() } else { self.problems() };
        if self.montage_on {
            self.montage_finish_ui(ui, ctx);
        } else {
            self.estimate_ui(ui);
            ui.add_space(4.0);
        }
        if self.media.is_some() && !self.montage_on {
            for pr in &problems {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&pr.message).color(theme::WARN));
                    for &fix in &pr.fixes {
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
        if !self.montage_on && ui.add_enabled(can, theme::primary_button(&export_label).min_size(egui::vec2(ui.available_width(), 36.0))).clicked() {
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
                    if let Some(n) = &self.over_cap {
                        theme::warn(ui, n.clone());
                    }
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

    fn speed_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::section(ui, "Speed");
            if !self.speed_zones.is_empty() {
                ui.label(RichText::new(format!("{} / {}", self.speed_zones.len(), speed::MAX_ZONES)).small().color(theme::MUTED));
            }
        });
        let blocked = self.speed_blocked_reason();
        let pos = self.position();
        let trim = self.trim();
        let fin_map = self.time_map();
        let fin_s0 = trim.map_or(0.0, |t| t.0);
        let sel = self.sel_speed;
        let (mut select, mut select_quiet, mut delete, mut add, mut whole) = (None, None, None, false, false);
        ui.add_enabled_ui(blocked.is_none(), |ui| {
            if self.speed_zones.is_empty() {
                theme::hint(ui, "Slow motion or fast forward for part of the clip. Add a zone, set its start, end and speed. The timeline stretches to show the new length.");
            }
            for (i, z) in self.speed_zones.iter_mut().enumerate() {
                let col = speed_color(z.speed);
                let selected = sel == Some(i);
                let frame = theme::card().stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { col } else { theme::BORDER }));
                let r = frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 5.0, col);
                        if ui.selectable_label(selected, RichText::new(format!("Speed {}", i + 1)).strong().color(col)).on_hover_text("Select and jump to its start").clicked() {
                            select = Some(i);
                        }
                        ui.label(RichText::new(z.label()).small().color(theme::MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Delete").clicked() {
                                delete = Some(i);
                            }
                        });
                    });
                    if z.whole {
                        theme::hint(ui, "Whole clip: covers your trim, and follows it if the trim changes.");
                    } else {
                        ui.columns(2, |cols| {
                            for (c, (label, text)) in [("Start", &mut z.start_text), ("End", &mut z.end_text)].into_iter().enumerate() {
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
                    }
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut z.speed, speed::MIN_SPEED..=speed::MAX_SPEED).step_by(0.25).show_value(false));
                        ui.add(egui::DragValue::new(&mut z.speed).range(speed::MIN_SPEED..=speed::MAX_SPEED).speed(0.01).max_decimals(2).suffix("×"));
                        if z.speed != 1.0 && ui.small_button("Reset").on_hover_text("Back to normal speed").clicked() {
                            z.speed = 1.0;
                        }
                    });
                    if let Some((s, e)) = z.times(trim) {
                        if e > s {
                            theme::hint(ui, format!("{} becomes {}", timecode::format_short(e - s), timecode::format_short((e - s) / z.speed.clamp(speed::MIN_SPEED, speed::MAX_SPEED))));
                        }
                    }
                    ui.add_enabled_ui(z.speed < 1.0, |ui| {
                        ui.checkbox(&mut z.smooth, "Smooth slow-mo")
                            .on_hover_text("Makes in-between frames so slow motion doesn't look choppy.")
                            .on_disabled_hover_text("Only for slow motion (under 1×).");
                    });
                    if z.smooth && z.speed < 1.0 {
                        theme::warn(ui, "Smooth slow-mo makes the export much slower.");
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Audio").small().color(theme::MUTED));
                        ui.selectable_value(&mut z.mute, false, "Keep").on_hover_text("Keep the sound, sped up or slowed down. Voices keep their normal pitch unless Change pitch is ticked.");
                        ui.selectable_value(&mut z.mute, true, "Mute").on_hover_text("Silence this zone.");
                    });
                });
                if !selected && r.response.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
                    select_quiet = Some(i);
                }
            }
            let full = self.speed_zones.len() >= speed::MAX_ZONES;
            let label = match self.settings.keybinds.get(Action::NewSpeedZone) {
                Some(b) => format!("+ New speed zone   ({})", b.display()),
                None => "+ New speed zone".into(),
            };
            ui.horizontal(|ui| {
                let w = if self.speed_zones.is_empty() { (ui.available_width() - 8.0) * 0.6 } else { ui.available_width() };
                if ui
                    .add_enabled(!full, egui::Button::new(label).min_size(egui::vec2(w, 26.0)))
                    .on_disabled_hover_text("You can have up to 20 speed zones. Delete one to add another.")
                    .clicked()
                {
                    add = true;
                }
                if self.speed_zones.is_empty()
                    && ui
                        .add(egui::Button::new("Whole clip").min_size(egui::vec2(ui.available_width(), 26.0)))
                        .on_hover_text("One speed for the whole trimmed clip")
                        .clicked()
                {
                    whole = true;
                }
            });
            if !self.speed_zones.is_empty() {
                ui.checkbox(&mut self.settings.options.change_pitch, "Change pitch with speed")
                    .on_hover_text("Off: voices sound normal at any speed. On: faster = higher, like a sped-up tape.");
            }
        });
        if let Some(why) = blocked {
            if self.media.is_some() {
                theme::warn(ui, why);
            }
        }
        if let Some(i) = delete {
            self.sel_speed = Some(i);
            self.delete_selected_speed();
        } else if add {
            self.new_speed_zone();
        } else if whole {
            self.speed_zones.push(SpeedZone::whole_clip());
            self.select_speed(0);
        } else if let Some(i) = select {
            self.select_speed(i);
            if let Some((s, _)) = self.speed_zones[i].times(trim) {
                self.seek(s);
            }
        } else if let Some(i) = select_quiet {
            self.select_speed(i);
        }
    }

    fn zoom_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::section(ui, "Zoom");
            if !self.zones.is_empty() {
                ui.label(RichText::new(format!("{} / {}", self.zones.len(), zoom::MAX_ZONES)).small().color(theme::MUTED));
            }
        });
        let blocked = self.zoom_blocked_reason();
        let pos = self.position();
        let edit_point = self.edit_point;
        let sel_zone = self.sel_zone;
        let drawing_now = self.drawing;
        let fin_map = self.time_map();
        let fin_s0 = self.trim().map_or(0.0, |t| t.0);
        let (mut select, mut select_quiet, mut delete, mut draw) = (None, None, None, None);
        let mut pick = None;
        let (mut tab_changed, mut resort, mut remove_point, mut add_point) = (None, None, None, None);
        let tctx = self.track_ctx();
        let mut track_acts: Vec<(usize, track_ui::TrackAct)> = Vec::new();
        ui.add_enabled_ui(blocked.is_none(), |ui| {
            if self.zones.is_empty() {
                theme::hint(ui, "Zoom into part of the picture for a stretch of time. Add a zone, then draw a box on the video.");
            }
            for (i, z) in self.zones.iter_mut().enumerate() {
                let col = zone_color(i);
                let selected = sel_zone == Some(i);
                let frame = theme::card().stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { col } else { theme::BORDER }));
                let r = frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 5.0, col);
                        if ui.selectable_label(selected, RichText::new(format!("Zone {}", i + 1)).strong().color(col)).on_hover_text("Select and jump to its start").clicked() {
                            select = Some(i);
                        }
                        ui.label(RichText::new(format!("{:.1}x", z.max_zoom())).small().color(theme::MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Delete").clicked() {
                                delete = Some(i);
                            }
                        });
                    });
                    // Simple / Precise tabs.
                    ui.horizontal(|ui| {
                        if ui.selectable_label(z.mode == zoom::Mode::Simple, "Simple").on_hover_text("One box: the same zoom from start to end").clicked() && z.mode != zoom::Mode::Simple {
                            z.mode = zoom::Mode::Simple;
                            tab_changed = Some(i);
                        }
                        if ui.selectable_label(z.mode == zoom::Mode::Precise, "Precise").on_hover_text("The camera moves: through points you set, or following something").clicked() && z.mode != zoom::Mode::Precise {
                            z.mode = zoom::Mode::Precise;
                            tab_changed = Some(i);
                        }
                    });
                    if z.mode == zoom::Mode::Precise {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Move by").small().color(theme::MUTED));
                            if ui.selectable_label(!z.follow, "My points").on_hover_text("The camera flows through points you place").clicked() && z.follow {
                                z.follow = false;
                                tab_changed = Some(i);
                            }
                            if ui.selectable_label(z.follow, "Follow automatically").on_hover_text("The camera follows something moving, like a player").clicked() && !z.follow {
                                z.follow = true;
                                tab_changed = Some(i);
                            }
                        });
                    }
                    let points = z.uses_points();
                    // The first box (Simple's box, Precise's point 1).
                    let edit_k = if selected { edit_point } else { usize::MAX };
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let label = if points { "Point 1  (zone start)" } else if z.follows() { "Box to follow" } else { "Box" };
                        ui.label(RichText::new(label).small().color(theme::MUTED));
                        box_buttons(ui, z.a_drawn, edit_k == 0, drawing_now, &mut draw, &mut pick, (i, 0));
                    });
                    if points {
                        let problems = z.point_problems();
                        // Where each point is reached (point 1: one glide in), for "Insert between".
                        let start_at = z.times().map(|t| t.0 + z.glide_used());
                        let reached: Vec<Option<f64>> = std::iter::once(start_at)
                            .chain(z.points.iter().zip(&problems).map(|(p, why)| if why.is_none() { p.t() } else { None }))
                            .collect();
                        // Above point k+1's row: halfway between point k and point k+1.
                        let insert_at: Vec<(usize, f64)> = (1..reached.len())
                            .filter_map(|k| {
                                let mid = (reached[k - 1]? + reached[k]?) / 2.0;
                                add_point_blocked(z, mid).is_none().then_some((k, mid))
                            })
                            .collect();
                        for (j, key) in z.points.iter_mut().enumerate() {
                            let k = j + 1;
                            ui.separator();
                            if let Some(&(_, mid)) = insert_at.iter().find(|x| x.0 == k)
                                && ui
                                    .small_button(format!("+ Insert between {k} and {}", k + 1))
                                    .on_hover_text(format!("Add a point halfway, at {}", timecode::format_short(mid)))
                                    .clicked()
                            {
                                add_point = Some((i, mid));
                            }
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("Point {}", k + 1)).small().color(if problems[j].is_some() { theme::MUTED } else { col }));
                                let r = ui.add(egui::TextEdit::singleline(&mut key.text).desired_width(90.0));
                                if r.lost_focus() {
                                    resort = Some(i);
                                }
                                if ui.small_button("Use playhead").clicked() {
                                    key.text = timecode::format_compact(pos);
                                    resort = Some(i);
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                box_buttons(ui, true, edit_k == k, drawing_now, &mut draw, &mut pick, (i, k));
                                if ui.small_button("Remove").on_hover_text("Remove this point").clicked() {
                                    remove_point = Some((i, k));
                                }
                            });
                            match &problems[j] {
                                Some(why) => theme::warn(ui, why.clone()),
                                None => {
                                    if let Some(f) = final_text(&fin_map, fin_s0, &key.text) {
                                        theme::hint(ui, f);
                                    }
                                }
                            }
                        }
                        let blocked_add = add_point_blocked(z, pos);
                        if ui
                            .add_enabled(blocked_add.is_none(), egui::Button::new("+ Add point at playhead").min_size(egui::vec2(ui.available_width(), 22.0)))
                            .on_hover_text("Pause where the camera should be somewhere else, click this, then move or resize the box")
                            .on_disabled_hover_text(blocked_add.unwrap_or(""))
                            .clicked()
                        {
                            add_point = Some((i, pos));
                        }
                        if z.points.is_empty() {
                            theme::hint(ui, "Pause where the thing you're zooming on has moved and add a point. The camera flows smoothly through every point.");
                        }
                    }
                    ui.columns(2, |cols| {
                        for (c, (label, text)) in [("Start", &mut z.start_text), ("End", &mut z.end_text)].into_iter().enumerate() {
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
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut z.smooth, "Smooth").on_hover_text("Glide in and out of the zoom. Off: cut straight in and out.");
                        if z.smooth {
                            ui.add(egui::DragValue::new(&mut z.glide).range(0.1..=2.0).speed(0.01).max_decimals(2).suffix(" s glide"));
                        }
                    });
                    if z.follows() {
                        if let Some(a) = track_ui::track_controls(ui, i, z, &tctx) {
                            track_acts.push((i, a));
                        }
                    }
                    if z.max_zoom() > zoom::WARN_ZOOM + 1e-6 {
                        theme::warn(ui, "Over 3x zoom: the picture will look soft.");
                    }
                });
                if !selected && r.response.contains_pointer() && ui.input(|i| i.pointer.primary_pressed()) {
                    select_quiet = Some(i);
                }
            }
            let full = self.zones.len() >= zoom::MAX_ZONES;
            let label = match self.settings.keybinds.get(Action::NewZone) {
                Some(b) => format!("+ New zone   ({})", b.display()),
                None => "+ New zone".into(),
            };
            ui.add_enabled(!full, egui::Button::new(label).min_size(egui::vec2(ui.available_width(), 26.0)))
                .on_disabled_hover_text("You can have up to 20 zones. Delete one to add another.")
                .clicked()
                .then(|| draw = Some((usize::MAX, 0)));
            if !self.zones.is_empty() {
                ui.checkbox(&mut self.settings.zoom_static_preview, "Static zoom preview")
                    .on_hover_text("Result shows box outlines instead of zooming live.");
                theme::hint(
                    ui,
                    if self.settings.zoom_static_preview {
                        "Result shows where each zoom lands as an outline. The export still zooms."
                    } else {
                        "Result shows the zoom live. If playback stutters, tick Static zoom preview. The export is not affected."
                    },
                );
            }
        });
        if let Some(why) = blocked {
            if self.media.is_some() {
                theme::warn(ui, why);
            }
        }
        if let Some(n) = &self.track_note {
            theme::hint(ui, n.clone());
        }
        for (i, a) in track_acts {
            self.apply_track_act(i, a);
        }
        if let Some(i) = tab_changed {
            // Only point 1 exists outside "My points".
            if self.sel_zone == Some(i) && !self.zones[i].uses_points() {
                self.edit_point = 0;
            }
        }
        if let Some(i) = resort {
            let keep = if self.sel_zone == Some(i) { self.edit_point } else { 0 };
            let k = self.zones[i].sort_points(keep);
            if self.sel_zone == Some(i) {
                self.edit_point = k;
            }
        }
        if let Some(i) = delete {
            self.sel_zone = Some(i);
            self.delete_selected_zone();
        } else if let Some((i, k)) = remove_point {
            self.remove_zone_point(i, k);
        } else if let Some((i, t)) = add_point {
            self.add_zone_point(i, t);
        } else if let Some((i, k)) = draw {
            if i == usize::MAX {
                self.new_zone();
            } else {
                self.goto_zone_box(i, k, true);
            }
        } else if let Some((i, k)) = pick {
            self.goto_zone_box(i, k, false);
        } else if let Some(i) = select {
            self.select_zone(i);
        } else if let Some(i) = select_quiet {
            self.sel_zone = Some(i);
            self.sel_speed = None;
            self.edit_point = 0;
        }
    }

    /// Codec and quality controls. With a Discord preset they show the
    /// preset's choices, greyed out; your own choices come back with Custom.
    fn quality_ui(&mut self, ui: &mut egui::Ui, caps: Option<&Caps>, media: Option<&MediaInfo>, fast: bool) {
        let discord = self.discord_active();
        let shown = self.effective_opts();
        let saved = discord.then(|| std::mem::replace(&mut self.settings.options, shown));
        let label = self.settings.options.preset.label();
        ui.add_enabled_ui(!discord, |ui| self.quality_body(ui, caps, media, fast));
        if let Some(s) = saved {
            self.settings.options = s;
            theme::hint(ui, format!("Set by {label}. Choose Custom to change these."));
        }
    }

    fn quality_body(&mut self, ui: &mut egui::Ui, caps: Option<&Caps>, media: Option<&MediaInfo>, fast: bool) {
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

    fn drag_out(&mut self, frame: &eframe::Frame, path: &Path, ctx: &egui::Context) {
        if let Err(e) = winutil::drag_file_out(frame, path) {
            self.log.error(&e);
        }
        // Windows ran the drag itself, so egui never saw the button come up.
        ctx.stop_dragging();
    }

    fn copy_file(&mut self, path: &Path) {
        let note = match winutil::copy_file_to_clipboard(path) {
            Ok(()) => "Copied. Paste it with Ctrl+V".to_string(),
            Err(e) => {
                self.log.error(&e);
                e
            }
        };
        self.copy_note = Some((note, Instant::now()));
    }

    /// Size, resolution, frame rate and bitrates the export will come out at.
    fn estimate_ui(&self, ui: &mut egui::Ui) {
        let (Some(m), Some((s, e))) = (self.media.as_ref(), self.trim()) else { return };
        let o = self.effective_opts();
        let out = PathBuf::new();
        let speeds = self.active_speeds();
        let j = Job { media: m, opts: &o, start: s, end: e, output: &out, video_encoder: None, can_tonemap: false, zones: &[], tracks: &self.tracks, speed_zones: &speeds, overlays: &[], montage: None };
        let est = job::estimate(&j);
        let dur = job::output_duration(s, e, &speeds);
        let plan = if self.discord_active() { self.discord_plan().and_then(Result::ok) } else { None };
        let cap = self.settings.options.preset;
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Estimate").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match &plan {
                    Some(p) => {
                        ui.label(RichText::new(format!("fits under {} MB", cap.cap_mb().unwrap_or(0))).small().color(theme::OK));
                        ui.label(RichText::new(mb(p.bytes(dur))).strong().size(15.0));
                    }
                    None => {
                        ui.label(RichText::new(format!("≈ {}", validate::bytes(est.bytes))).strong().size(15.0));
                    }
                });
            });
            egui::Grid::new("estimate").num_columns(2).spacing(egui::vec2(12.0, 2.0)).show(ui, |ui| {
                let row = |ui: &mut egui::Ui, k: &str, v: String| {
                    ui.label(RichText::new(k).small().color(theme::MUTED));
                    ui.label(RichText::new(v).small());
                    ui.end_row();
                };
                if let Some((w, h)) = est.size {
                    row(ui, "Resolution", format!("{w}×{h}"));
                }
                if let Some(f) = est.fps {
                    row(ui, "Frame rate", format!("{} fps", probe::fmt_fps(f)));
                }
                if est.video_copied {
                    row(ui, "Video", "copied as it is".into());
                } else if let Some(k) = est.video_kbps {
                    row(ui, "Video", if est.exact_bitrate { kbps(k) } else { format!("≈ {}", kbps(k)) });
                }
                match est.audio_kbps {
                    Some(k) => row(ui, "Audio", kbps(k)),
                    None if m.audio.is_none() || o.av == AvMode::RemoveAudio => row(ui, "Audio", "none".into()),
                    None if o.av == AvMode::Mute => row(ui, "Audio", "silent track".into()),
                    None if o.mode == CutMode::Fast => row(ui, "Audio", "copied as it is".into()),
                    None => row(ui, "Audio", "lossless".into()),
                }
            });
            if plan.is_some() {
                theme::hint(ui, "Picked to fit the size. Longer clips step down from 1080p60 to 1080p30, 720p30, then 480p30.");
            } else if !est.exact_bitrate && !est.video_copied {
                theme::hint(ui, "A rough guess: the real size depends on how busy the video is.");
            }
        });
    }

    fn choose_clips_folder(&mut self, ctx: &egui::Context) {
        let mut d = rfd::FileDialog::new().set_title("Choose the folder your clips are saved in");
        if let Some(dir) = self.settings.clips_dir.as_ref().or(self.settings.last_open_dir.as_ref()) {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_folder() {
            self.settings.clips_dir = Some(p);
            self.rescan_lists(ctx);
        }
    }

    /// Rescan both drawer lists: Editing (the clips folder and its subfolders,
    /// minus the output folder) and Finished (the output folder itself).
    fn rescan_lists(&mut self, ctx: &egui::Context) {
        let (clips_dir, out_dir) = (self.settings.clips_dir.clone(), self.settings.output_dir.clone());
        if let Some(dir) = &clips_dir {
            self.library.rescan(dir, clips::ScanOpts { flat: false, exclude: out_dir.clone() }, ctx);
        }
        if let Some(dir) = &out_dir {
            self.finished.rescan(dir, clips::ScanOpts { flat: true, exclude: None }, ctx);
        }
        self.lists_for = Some((clips_dir, out_dir));
    }

    /// Once, when history is first kept: mark sources that already have an
    /// old-style export (`<source name>_…`) in the output folder.
    fn catch_up_history(&mut self) {
        if self.history.caught_up {
            return;
        }
        let Some(out) = self.settings.output_dir.clone() else { return };
        let exports: Vec<(String, Option<job::Stamp>)> = clips::scan(&out, &clips::ScanOpts { flat: true, exclude: None })
            .into_iter()
            .flat_map(|g| g.clips)
            .map(|c| (c.name, winutil::local_stamp(c.modified)))
            .collect();
        let found = self.history.catch_up(self.library.all(), &exports);
        self.history.save();
        self.log.info(format!("Export history started: {found} clip(s) already had exports in the output folder."));
    }

    /// The drawer: Editing files on top, Finished files below, with a
    /// divider you can drag between them.
    fn clips_ui(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let ctx = ui.ctx().clone();
        let want = (self.settings.clips_dir.clone(), self.settings.output_dir.clone());
        if self.lists_for.as_ref() != Some(&want) {
            self.rescan_lists(&ctx);
        }
        let w = ui.available_width();
        let total = ui.available_height();
        let top_y = ui.cursor().top();
        const DIVIDER: f32 = 10.0;
        let top_h = ((total - DIVIDER) * self.settings.clips_split.clamp(0.15, 0.85)).round();
        ui.allocate_ui_with_layout(egui::vec2(w, top_h), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_size(egui::vec2(w, top_h));
            self.editing_half(ui, &ctx);
        });
        let (r, resp) = ui.allocate_exact_size(egui::vec2(w, DIVIDER), Sense::drag());
        let col = if resp.hovered() || resp.dragged() { theme::ACCENT } else { theme::BORDER };
        ui.painter().line_segment([egui::pos2(r.left(), r.center().y), egui::pos2(r.right(), r.center().y)], Stroke::new(if resp.dragged() { 2.0 } else { 1.0 }, col));
        let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text("Drag to resize");
        // Follow the pointer itself, so the divider stays under the cursor
        // (adding up small per-frame moves lost them to rounding).
        if let (true, Some(p)) = (resp.dragged(), resp.interact_pointer_pos()) {
            let f = (p.y - top_y - DIVIDER / 2.0) / (total - DIVIDER).max(1.0);
            self.settings.clips_split = f.clamp(0.15, 0.85);
            ctx.request_repaint();
        }
        let bottom_h = ui.available_height();
        ui.allocate_ui_with_layout(egui::vec2(w, bottom_h), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_min_size(egui::vec2(w, bottom_h));
            self.finished_half(ui, frame, &ctx);
        });
    }

    /// A drawer half's heading, Refresh button, Audio checkbox and folder line.
    /// Returns true if "Change" was clicked.
    fn half_header(&mut self, ui: &mut egui::Ui, title: &str, dir: Option<&Path>, scanning: bool, show_audio: &mut bool, ctx: &egui::Context) -> bool {
        let mut change = false;
        ui.horizontal(|ui| {
            theme::section(ui, title);
            if dir.is_some() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Refresh").on_hover_text("Look for new files").clicked() {
                        self.rescan_lists(ctx);
                    }
                    ui.checkbox(show_audio, RichText::new("Audio").small())
                        .on_hover_text("Also list audio files (MP3, WAV, M4A…). Untick to show only videos.");
                    if scanning {
                        ui.spinner();
                    }
                });
            }
        });
        if let Some(dir) = dir {
            ui.horizontal(|ui| {
                let w = ui.available_width() - 64.0;
                ui.allocate_ui_with_layout(egui::vec2(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(dir.display().to_string()).monospace().small().color(theme::MUTED)).truncate())
                        .on_hover_text(dir.display().to_string());
                });
                change = ui.small_button("Change").clicked();
            });
        }
        change
    }

    fn editing_half(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let dir = self.settings.clips_dir.clone();
        let mut show_audio = self.settings.editing_show_audio;
        if self.half_header(ui, "Editing files", dir.as_deref(), self.library.scanning(), &mut show_audio, ctx) {
            self.choose_clips_folder(ctx);
        }
        self.settings.editing_show_audio = show_audio;
        if dir.is_none() {
            theme::hint(ui, "Choose the folder your game clips are saved in, for example Videos\\NVIDIA. Clips are grouped by game, newest first.");
            if ui.button("Choose clips folder").clicked() {
                self.choose_clips_folder(ctx);
            }
            return;
        }
        if !self.library.all().any(|c| show_audio || !c.audio) {
            theme::hint(ui, if self.library.scanning() { "Looking for clips…" } else { "No videos in this folder yet." });
            return;
        }
        let current = self.media.as_ref().map(|m| m.path.clone());
        let list = ClipList { id: "editing", current: current.as_deref(), history: Some(&self.history), fresh: None, draggable: false, show_audio };
        let act = list.show(ui, &self.library);
        self.do_list_actions(act, None, ctx);
    }

    fn finished_half(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, ctx: &egui::Context) {
        let dir = self.settings.output_dir.clone();
        let mut show_audio = self.settings.finished_show_audio;
        if self.half_header(ui, "Finished files", dir.as_deref(), self.finished.scanning(), &mut show_audio, ctx) {
            self.choose_folder();
        }
        self.settings.finished_show_audio = show_audio;
        if let Some((note, at)) = &self.copy_note {
            if at.elapsed() < Duration::from_secs(4) {
                ui.label(RichText::new(note).small().color(theme::OK));
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
        if dir.is_none() {
            theme::hint(ui, "Your exports show here. Choose the output folder they're saved to.");
            if ui.button("Choose output folder").clicked() {
                self.choose_folder();
            }
            return;
        }
        if !self.finished.all().any(|c| show_audio || !c.audio) {
            theme::hint(ui, if self.finished.scanning() { "Looking…" } else { "Nothing exported yet." });
            return;
        }
        let fresh = self.fresh.as_ref().filter(|(_, at)| at.elapsed() < Duration::from_secs(5)).map(|(p, at)| (p.clone(), *at));
        if fresh.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let current = self.media.as_ref().map(|m| m.path.clone());
        let list = ClipList { id: "finished", current: current.as_deref(), history: None, fresh: fresh.as_ref().map(|(p, at)| (p.as_path(), *at)), draggable: true, show_audio };
        let act = list.show(ui, &self.finished);
        self.do_list_actions(act, Some(frame), ctx);
    }

    fn do_list_actions(&mut self, act: ListActions, frame: Option<&eframe::Frame>, ctx: &egui::Context) {
        if let (Some(p), Some(frame)) = (act.drag, frame) {
            self.drag_out(frame, &p, ctx);
        }
        if let Some(p) = act.open {
            if self.media.as_ref().map(|m| &m.path) != Some(&p) {
                self.open(p);
            }
        }
        if let Some(p) = act.reveal {
            winutil::reveal_in_explorer(&p);
        }
        if let Some(p) = act.copy {
            self.copy_file(&p);
        }
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

/// Draw box / Select box / Redraw for one zone box (`at` = zone, point).
fn box_buttons(ui: &mut egui::Ui, drawn: bool, this_box: bool, drawing: bool, draw: &mut Option<(usize, usize)>, pick: &mut Option<(usize, usize)>, at: (usize, usize)) {
    if !drawn && ui.add(egui::Button::selectable(this_box && drawing, "Draw box")).on_hover_text("Drag on the video to draw this box").clicked() {
        *draw = Some(at);
    }
    if ui.add_enabled(drawn, egui::Button::selectable(this_box && !drawing, "Select box")).on_hover_text("Jump here and show this box on the video to move or resize it").clicked() {
        *pick = Some(at);
    }
    if ui.add_enabled(drawn, egui::Button::new("Redraw")).on_hover_text("Throw this box away and draw a new one").clicked() {
        *draw = Some(at);
    }
}

/// Where to show zone point `k` (0 = the first box): its time, kept on a
/// frame inside the zone.
fn point_frame(z: &Zone, k: usize, frame: f64) -> Option<f64> {
    let (s, e) = z.times()?;
    let t = z.point(k).map_or(s, |p| p.0);
    Some(t.min(e - frame).max(s))
}

/// Why a Precise point can't be added to `z` at `pos` (None = it can).
fn add_point_blocked(z: &Zone, pos: f64) -> Option<&'static str> {
    let (s, e) = z.times()?;
    if z.points.len() + 1 >= zoom::MAX_POINTS {
        return Some("A zone can have up to 20 points. Remove one to add another.");
    }
    if pos <= s || pos >= e {
        return Some("Move the playhead inside the zone first");
    }
    if !z.a_drawn {
        return Some("Draw point 1's box first");
    }
    if z.points.iter().any(|p| p.t().is_some_and(|t| (t - pos).abs() < zoom::SAME_TIME)) {
        return Some("There's already a point here");
    }
    None
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

/// What the user did in a drawer list this frame.
#[derive(Default)]
struct ListActions {
    open: Option<PathBuf>,
    drag: Option<PathBuf>,
    reveal: Option<PathBuf>,
    copy: Option<PathBuf>,
}

/// A drawer list: files grouped by game, newest first.
struct ClipList<'a> {
    id: &'static str,
    /// The file open in the editor (highlighted).
    current: Option<&'a Path>,
    /// Ticks for sources that have been exported (Editing list only).
    history: Option<&'a crate::history::History>,
    /// The file just exported, and when (Finished list only).
    fresh: Option<(&'a Path, Instant)>,
    /// Rows can be dragged out to other apps (Finished list only).
    draggable: bool,
    /// List audio files as well as videos.
    show_audio: bool,
}

impl ClipList<'_> {
    fn show(&self, ui: &mut egui::Ui, lib: &clips::Library) -> ListActions {
        let mut act = ListActions::default();
        let now = std::time::SystemTime::now();
        egui::ScrollArea::vertical().id_salt(self.id).auto_shrink(false).show(ui, |ui| {
            for g in &lib.groups {
                let shown: Vec<&clips::Clip> = g.clips.iter().filter(|c| self.show_audio || !c.audio).collect();
                if shown.is_empty() {
                    continue;
                }
                let holds_fresh = self.fresh.is_some_and(|(p, _)| shown.iter().any(|c| c.path == p));
                let mut header = egui::CollapsingHeader::new(RichText::new(format!("{}  ({})", g.game, shown.len())).strong())
                    .id_salt((self.id, &g.game))
                    .default_open(true);
                if holds_fresh {
                    header = header.open(Some(true));
                }
                header.show(ui, |ui| {
                    for c in shown {
                        let tick = self.history.and_then(|h| h.get(&c.path)).map(crate::history::describe);
                        let fresh = self.fresh.filter(|(p, _)| *p == c.path).map(|(_, at)| at);
                        let resp = self.row(ui, c, now, tick);
                        if let Some(at) = fresh {
                            // Fade the highlight out over a few seconds.
                            let a = (1.0 - at.elapsed().as_secs_f32() / 5.0).clamp(0.0, 1.0);
                            ui.painter().rect_stroke(resp.rect, 4.0, Stroke::new(2.0, theme::OK.gamma_multiply(a)), egui::StrokeKind::Inside);
                            if at.elapsed() < Duration::from_millis(300) {
                                resp.scroll_to_me(Some(egui::Align::Center));
                            }
                        }
                        if self.draggable && resp.drag_started() {
                            act.drag = Some(c.path.clone());
                        } else if resp.clicked() {
                            act.open = Some(c.path.clone());
                        }
                        resp.context_menu(|ui| {
                            if ui.button("Show in folder").clicked() {
                                act.reveal = Some(c.path.clone());
                                ui.close();
                            }
                            if ui.button("Copy file").on_hover_text("Then press Ctrl+V in Discord or Explorer").clicked() {
                                act.copy = Some(c.path.clone());
                                ui.close();
                            }
                        });
                    }
                });
            }
        });
        act
    }

    /// One file: name (and a green tick if it's been exported), then
    /// "2 h ago · 45 MB".
    fn row(&self, ui: &mut egui::Ui, c: &clips::Clip, now: std::time::SystemTime, tick: Option<String>) -> egui::Response {
        let current = self.current == Some(c.path.as_path());
        let w = ui.available_width();
        let sense = if self.draggable { Sense::click_and_drag() } else { Sense::click() };
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 36.0), sense);
        let painter = ui.painter_at(rect);
        if current {
            painter.rect_filled(rect, 4.0, theme::ACCENT_DIM.gamma_multiply(0.6));
        } else if resp.hovered() {
            painter.rect_filled(rect, 4.0, theme::CARD);
        }
        let pad = 6.0;
        let tick_w = if tick.is_some() { 20.0 } else { 0.0 };
        // One line only: cut long names with an ellipsis.
        let mut job = egui::text::LayoutJob::simple_singleline(c.name.clone(), egui::FontId::proportional(13.0), if current { Color32::WHITE } else { theme::TEXT });
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - pad * 2.0 - tick_w);
        let name = ui.fonts_mut(|f| f.layout_job(job));
        painter.galley(rect.left_top() + egui::vec2(pad, 3.0), name, theme::TEXT);
        let sub = format!("{} · {}", clips::relative_time(c.modified, now), validate::bytes(c.size));
        painter.text(rect.left_bottom() + egui::vec2(pad, -3.0), egui::Align2::LEFT_BOTTOM, sub, egui::FontId::proportional(11.5), theme::MUTED);
        let how = if self.draggable { "Click to open · drag into another app · right-click for more" } else { "Click to open · right-click for more" };
        let resp = resp.on_hover_cursor(if self.draggable { egui::CursorIcon::Grab } else { egui::CursorIcon::PointingHand });
        if let Some(t) = tick {
            let tr = egui::Rect::from_center_size(egui::pos2(rect.right() - pad - 7.0, rect.top() + 11.0), egui::vec2(16.0, 16.0));
            check_mark(&painter, tr, theme::OK);
            // The tick has its own tooltip; it sits on top of the row.
            ui.interact(tr, ui.scope_id().with((self.id, "tick", &c.path)), Sense::hover()).on_hover_text(t);
        }
        resp.on_hover_text(format!("{}\n{how}", c.path.display()))
    }
}

/// A waveform: one bar per pixel column, mirrored around the middle.
/// `level(t0, t1)` gives the loudest moment (0-1) between two times.
/// `t_at(f)` is the time a fraction `f` (0-1) of the way along.
fn draw_wave(painter: &egui::Painter, rect: egui::Rect, t_at: &dyn Fn(f64) -> f64, color: Color32, level: impl Fn(f64, f64) -> f32) {
    let cols = rect.width().max(1.0) as usize;
    let mid = rect.center().y;
    let half = rect.height() / 2.0;
    for c in 0..cols {
        let (t0, t1) = (t_at(c as f64 / cols as f64), t_at((c + 1) as f64 / cols as f64));
        let h = waveform::height(level(t0, t1.max(t0 + 1e-3))) * half;
        if h >= 0.5 {
            let x = rect.left() + c as f32 + 0.5;
            painter.line_segment([egui::pos2(x, mid - h), egui::pos2(x, mid + h)], Stroke::new(1.0, color));
        }
    }
}

/// A tick drawn with lines. egui's built-in fonts have no check-mark glyph
/// (it would show as an empty box), so it's painted instead.
fn check_mark(painter: &egui::Painter, r: egui::Rect, color: Color32) {
    let p = |x: f32, y: f32| egui::pos2(r.left() + x * r.width(), r.top() + y * r.height());
    let stroke = Stroke::new(2.0, color);
    painter.line_segment([p(0.15, 0.55), p(0.4, 0.8)], stroke);
    painter.line_segment([p(0.4, 0.8), p(0.88, 0.22)], stroke);
}

/// 3552 → "3,552 kbps".
fn kbps(k: u32) -> String {
    let s = k.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    format!("{out} kbps")
}

/// Bytes as megabytes with one decimal, e.g. "8.4 MB".
fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1e6)
}
