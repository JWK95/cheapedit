//! Turns the user's choices into an FFmpeg command line.
//!
//! This is pure (no I/O), so every combination can be unit-tested.

use crate::discord::{Plan, Preset};
use crate::formats::{ACodec, Container, VCodec};
use crate::probe::MediaInfo;
use crate::tracks::ExportAudio;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CutMode {
    Precise,
    Fast,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AvMode {
    Both,
    Mute,
    RemoveAudio,
    AudioOnly,
}

impl AvMode {
    pub const ALL: [AvMode; 4] = [AvMode::Both, AvMode::Mute, AvMode::RemoveAudio, AvMode::AudioOnly];
    pub fn label(self) -> &'static str {
        match self {
            AvMode::Both => "Keep video and audio",
            AvMode::Mute => "Mute audio (silent track)",
            AvMode::RemoveAudio => "Remove audio track",
            AvMode::AudioOnly => "Audio only (MP3)",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            AvMode::Both => "",
            AvMode::Mute => "The video keeps an audio track, but it's silent. Some sites and players expect one.",
            AvMode::RemoveAudio => "The video has no audio track at all.",
            AvMode::AudioOnly => "No video — just the sound, saved as an MP3.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    High,
    Medium,
    Small,
    Custom,
}

impl Quality {
    pub const ALL: [Quality; 4] = [Quality::High, Quality::Medium, Quality::Small, Quality::Custom];
    pub fn label(self) -> &'static str {
        match self {
            Quality::High => "High",
            Quality::Medium => "Medium",
            Quality::Small => "Small file",
            Quality::Custom => "Custom bitrate",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Quality::High => "Looks nearly identical to the original. Bigger files.",
            Quality::Medium => "Good quality, sensible size. Right for most things.",
            Quality::Small => "Noticeably softer, but small. Good for quick sharing.",
            Quality::Custom => "You set the exact bitrate. Higher = better quality and bigger files.",
        }
    }
    fn index(self) -> usize {
        match self {
            Quality::High => 0,
            Quality::Medium | Quality::Custom => 1,
            Quality::Small => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncSpeed {
    Faster,
    Balanced,
    Smaller,
}

impl EncSpeed {
    pub const ALL: [EncSpeed; 3] = [EncSpeed::Faster, EncSpeed::Balanced, EncSpeed::Smaller];
    pub fn label(self) -> &'static str {
        match self {
            EncSpeed::Faster => "Faster export",
            EncSpeed::Balanced => "Balanced",
            EncSpeed::Smaller => "Smaller file (slower)",
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    Same,
    P2160,
    P1440,
    P1080,
    P720,
    P480,
}

impl Resolution {
    pub const ALL: [Resolution; 6] =
        [Resolution::Same, Resolution::P2160, Resolution::P1440, Resolution::P1080, Resolution::P720, Resolution::P480];
    pub fn label(self) -> &'static str {
        match self {
            Resolution::Same => "Same as source",
            Resolution::P2160 => "2160p (4K)",
            Resolution::P1440 => "1440p",
            Resolution::P1080 => "1080p",
            Resolution::P720 => "720p",
            Resolution::P480 => "480p",
        }
    }
    pub fn height(self) -> Option<u32> {
        match self {
            Resolution::Same => None,
            Resolution::P2160 => Some(2160),
            Resolution::P1440 => Some(1440),
            Resolution::P1080 => Some(1080),
            Resolution::P720 => Some(720),
            Resolution::P480 => Some(480),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum FrameRate {
    Same,
    Fps(u32),
}

impl FrameRate {
    pub const ALL: [FrameRate; 7] = [
        FrameRate::Same,
        FrameRate::Fps(120),
        FrameRate::Fps(60),
        FrameRate::Fps(50),
        FrameRate::Fps(30),
        FrameRate::Fps(25),
        FrameRate::Fps(24),
    ];
    pub fn label(self) -> String {
        match self {
            FrameRate::Same => "Same as source".into(),
            FrameRate::Fps(n) => format!("{n} fps"),
        }
    }
}

/// Everything the user can choose (apart from the file, trim and names).
/// Saved between sessions as "last used options".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub mode: CutMode,
    pub change_pitch: bool,
    pub av: AvMode,
    pub volume_pct: u32,
    pub container: Container,
    pub vcodec: VCodec,
    pub acodec: ACodec,
    pub quality: Quality,
    pub custom_kbps: u32,
    pub fps: FrameRate,
    pub resolution: Resolution,
    pub enc_speed: EncSpeed,
    pub audio_kbps: u32,
    pub mp3_kbps: u32,
    pub use_gpu: bool,
    /// Custom, or a Discord size-cap preset that overrides the format,
    /// codec, quality, frame rate and resolution while it's chosen.
    pub preset: Preset,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            mode: CutMode::Precise,
            change_pitch: false,
            av: AvMode::Both,
            volume_pct: 100,
            container: Container::Mp4,
            vcodec: VCodec::H264,
            acodec: ACodec::Aac,
            quality: Quality::Medium,
            custom_kbps: 8000,
            fps: FrameRate::Same,
            resolution: Resolution::Same,
            enc_speed: EncSpeed::Balanced,
            audio_kbps: 192,
            mp3_kbps: 192,
            use_gpu: true,
            preset: Preset::Custom,
        }
    }
}

impl Options {
    pub fn output_container(&self) -> Container {
        if self.av == AvMode::AudioOnly { Container::Mp3 } else { self.container }
    }

    /// These options with a Discord preset's fixed choices applied: Precise,
    /// MP4, H.264, AAC, and the plan's bitrate, resolution and frame rate.
    /// Your own Custom choices are left untouched in the saved settings.
    pub fn with_preset_basics(&self) -> Options {
        let mut o = self.clone();
        if o.preset.cap_mb().is_some() {
            o.mode = CutMode::Precise;
            o.container = Container::Mp4;
            o.vcodec = VCodec::H264;
            o.acodec = ACodec::Aac;
            o.quality = Quality::Custom;
            if o.av == AvMode::AudioOnly {
                o.av = AvMode::Both;
            }
        }
        o
    }

    pub fn with_plan(&self, p: &Plan) -> Options {
        let mut o = self.with_preset_basics();
        o.custom_kbps = p.video_kbps;
        o.audio_kbps = p.audio_kbps.unwrap_or(o.audio_kbps);
        o.resolution = if p.same_height { Resolution::Same } else { Resolution::ALL.into_iter().find(|r| r.height() == Some(p.height)).unwrap_or(Resolution::Same) };
        o.fps = if p.same_fps { FrameRate::Same } else { FrameRate::Fps(p.fps.round() as u32) };
        o
    }
}

/// One export.
pub struct Job<'a> {
    pub media: &'a MediaInfo,
    pub opts: &'a Options,
    pub start: f64,
    pub end: f64,
    pub output: &'a Path,
    /// Resolved video encoder name (e.g. "h264_nvenc"); None when not encoding video.
    pub video_encoder: Option<&'static str>,
    /// Convert HDR to standard colour (zscale is available).
    pub can_tonemap: bool,
    /// Zoom zones (times on the original video). Ignored in Fast mode.
    pub zones: &'a [crate::zoom::Resolved],
    /// Per-track settings when the source has several audio tracks
    /// (empty or one entry: just the first track, as before).
    pub tracks: &'a [crate::tracks::TrackMix],
    /// Speed zones (times on the original video). Ignored in Fast mode.
    pub speed_zones: &'a [crate::speed::Resolved],
    /// Overlays, bottom layer first (times on the original video). Ignored
    /// in Fast mode and without a picture.
    pub overlays: &'a [crate::overlay::Overlay],
    /// One clip of a montage: fit into this frame (black bars, never
    /// stretched) at this frame rate, with 48 kHz stereo sound (silent if the
    /// clip has none), so every clip can be joined without re-encoding.
    pub montage: Option<Frame>,
}

/// A montage's picture size and frame rate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub fps: f64,
}

impl Job<'_> {
    /// The speed zones that apply (none in Fast mode, which copies untouched).
    fn speeds(&self) -> &[crate::speed::Resolved] {
        if self.opts.mode == CutMode::Fast { &[] } else { self.speed_zones }
    }
}

/// Length of the exported clip in seconds.
pub fn output_duration(start: f64, end: f64, zones: &[crate::speed::Resolved]) -> f64 {
    crate::speed::TimeMap::new(zones).out_len(start, end)
}

fn n(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.to_string() }
}

const TONEMAP: &str = "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv";

pub fn build_args(j: &Job) -> Vec<String> {
    let o = j.opts;
    let m = j.media;
    let fast = o.mode == CutMode::Fast;
    let map = crate::speed::TimeMap::new(j.speeds());
    let pieces = crate::speed::pieces(&map, j.start, j.end);
    let retimed = pieces.iter().any(|p| p.speed != 1.0);
    let audio_only = o.av == AvMode::AudioOnly;
    let container = o.output_container();
    let has_v = m.video.is_some() && !audio_only;
    let source = export_audio_of(j);
    let overlays: &[crate::overlay::Overlay] = if has_v && !fast { j.overlays } else { &[] };
    // Each overlay's times on the export's clock: before the speed zones for
    // "follow clip speed" videos, after them for everything else.
    let clock = |ov: &crate::overlay::Overlay| -> (f64, f64) {
        let (s, e) = ov.times().unwrap_or((j.start, j.start));
        if ov.follows_speed() {
            (s - j.start, e - j.start)
        } else {
            let base = map.to_out(j.start);
            (map.to_out(s) - base, map.to_out(e) - base)
        }
    };
    // Media overlays are extra inputs, numbered from 1.
    let mut input_of: Vec<Option<usize>> = Vec::new();
    let mut next_input = 1;
    for ov in overlays {
        if ov.kind == crate::overlay::Kind::Text {
            input_of.push(None);
        } else {
            input_of.push(Some(next_input));
            next_input += 1;
        }
    }
    let overlay_sound = source.is_some() && overlays.iter().any(|ov| ov.sound());
    let mut complex: Vec<String> = Vec::new();

    let mut a: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    push(&["-hide_banner", "-nostdin", "-n", "-loglevel", "warning"]);
    push(&["-ss", &n(j.start), "-t", &n(j.end - j.start), "-i"]);
    a.push(m.path.to_string_lossy().into_owned());
    if !overlays.is_empty() {
        let fps = m.video.as_ref().map_or(30.0, |v| v.fps);
        let longest = (j.end - j.start) / crate::speed::MIN_SPEED + 2.0;
        for ov in overlays.iter().filter(|ov| ov.kind != crate::overlay::Kind::Text) {
            a.extend(crate::overlay::input_args(ov, fps, longest));
        }
    }
    // A montage clip is cut to a whole number of frames, and its sound to
    // exactly the same length, so joined clips never drift out of sync.
    let montage_len = j.montage.map(|fr| {
        let frames = (map.out_len(j.start, j.end) * fr.fps).round().max(1.0);
        (frames as u64, frames / fr.fps)
    });
    // A montage clip without sound gets a silent track, so every clip matches.
    let silent_input = j.montage.is_some() && source.is_none();
    if silent_input {
        let len = n(montage_len.map_or(0.0, |m| m.1));
        a.extend(["-f".into(), "lavfi".into(), "-t".into(), len, "-i".into(), "anullsrc=r=48000:cl=stereo".into()]);
    }
    let silent_index = 1 + overlays.iter().filter(|ov| ov.kind != crate::overlay::Kind::Text).count();
    if has_v {
        a.extend(["-map".into(), if overlays.is_empty() { "0:v:0".into() } else { "[vout]".into() }]);
    }
    match &source {
        Some(ExportAudio::Track(i)) if !overlay_sound => a.extend(["-map".into(), format!("0:a:{i}")]),
        Some(_) => a.extend(["-map".into(), "[aout]".into()]),
        None if silent_input => a.extend(["-map".into(), format!("{silent_index}:a")]),
        _ => {}
    }

    // ---- video ----
    if has_v {
        let v = m.video.as_ref().unwrap();
        if fast {
            a.extend(["-c:v".into(), "copy".into()]);
            if v.codec == "hevc" && matches!(container, Container::Mp4 | Container::Mov) {
                a.extend(["-tag:v".into(), "hvc1".into()]);
            }
        } else {
            let mut pre: Vec<String> = Vec::new();
            if v.hdr && j.can_tonemap {
                pre.push(TONEMAP.into());
            }
            // Zoom before speed, so zone times match the original video. The
            // fps filter first makes the frame rate constant, which zoompan needs.
            if let Some(z) = crate::zoom::zoompan(j.zones, j.start, v.width, v.height, v.fps) {
                pre.push(format!("fps={}", n(v.fps)));
                pre.push(z);
            }
            let speed = crate::speed::video_chain(&pieces, v.fps);
            let mut post: Vec<String> = Vec::new();
            let target_fps = match o.fps {
                FrameRate::Fps(f) => Some(f as f64),
                FrameRate::Same if retimed => Some(v.fps),
                FrameRate::Same => None,
            };
            if let Some(fr) = j.montage {
                let (w, h) = (fr.w, fr.h);
                // Start the clock at 0, so the frame count below is exact.
                post.push("setpts=PTS-STARTPTS".into());
                post.push(format!("scale={w}:{h}:force_original_aspect_ratio=decrease:flags=lanczos"));
                post.push(format!("pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black"));
                post.push("setsar=1".into());
                post.push(format!("fps={}", n(fr.fps)));
                // Hold the last frame if short, then cut to the exact frame count.
                post.push("tpad=stop_mode=clone:stop_duration=1".into());
                post.push(format!("trim=end_frame={}", montage_len.map_or(1, |m| m.0)));
            } else {
                if let Some(f) = target_fps {
                    post.push(format!("fps={}", n(f)));
                }
                if let Some(h) = o.resolution.height() {
                    post.push(format!("scale=-2:{h}:flags=lanczos"));
                }
            }
            post.push(if o.vcodec == VCodec::ProRes { "format=yuv422p10le".into() } else { "format=yuv420p".into() });
            if overlays.is_empty() {
                let vf: Vec<String> = pre.into_iter().chain(speed).chain(post).collect();
                a.extend(["-vf".into(), vf.join(",")]);
            } else {
                // A filter graph: the picture runs through each stage, and
                // each overlay is laid on top in turn.
                let mut g = Chain { parts: &mut complex, cur: "0:v:0".into(), k: 0 };
                if !pre.is_empty() {
                    g.link("", &pre.join(","));
                }
                let order: Vec<usize> = (0..overlays.len())
                    .filter(|&i| overlays[i].follows_speed())
                    .chain([usize::MAX]) // the speed zones go here
                    .chain((0..overlays.len()).filter(|&i| !overlays[i].follows_speed()))
                    .collect();
                for i in order {
                    if i == usize::MAX {
                        if let Some(chain) = &speed {
                            g.link("", chain);
                        }
                        continue;
                    }
                    let ov = &overlays[i];
                    let (t0, t1) = clock(ov);
                    match input_of[i] {
                        None => g.link("", &crate::overlay::drawtext(ov, t0, t1, v.height)),
                        Some(input) => {
                            let label = format!("ov{i}");
                            g.parts.push(crate::overlay::media_filters(ov, input, &label, t0, t1, v.width, v.height));
                            g.link(&format!("[{label}]"), &crate::overlay::overlay_filter(ov, t0, t1));
                        }
                    }
                }
                g.k = usize::MAX;
                g.link("", &post.join(","));
            }
            let enc = j.video_encoder.unwrap_or("libx264");
            a.extend(["-c:v".into(), enc.into()]);
            a.extend(video_quality_args(enc, o));
            if o.vcodec == VCodec::H265 && matches!(container, Container::Mp4 | Container::Mov) {
                a.extend(["-tag:v".into(), "hvc1".into()]);
            }
        }
    } else {
        a.push("-vn".into());
    }

    // ---- audio ----
    if let Some(source) = &source {
        let first = match source {
            ExportAudio::Track(i) => *i,
            ExportAudio::Mix(mix) => mix[0].0,
            ExportAudio::None => 0,
        };
        let track = m.audio_tracks.get(first).or(m.audio.as_ref()).unwrap();
        let src = &track.codec;
        let mixing = matches!(source, ExportAudio::Mix(_)) || overlay_sound;
        let mute = o.av == AvMode::Mute;
        let volume = if fast { 1.0 } else { o.volume_pct as f64 / 100.0 };
        let codec = if audio_only {
            ACodec::Mp3
        } else if fast {
            if !mute && !mixing && container.can_copy_audio(src) {
                a.extend(["-c:a".into(), "copy".into()]);
                return finish(a, complex, container, fast, j.output);
            }
            ACodec::from_ffprobe(src).filter(|c| container.audio_codecs().contains(c) && *c != ACodec::Pcm).unwrap_or(container.default_audio())
        } else {
            o.acodec
        };
        let speed = crate::speed::audio_chain(&pieces, o.change_pitch, track.sample_rate);
        let mut level: Vec<String> = Vec::new();
        if mute {
            level.push("volume=0".into());
        } else if volume != 1.0 {
            level.push(format!("volume={}", n(volume)));
        }
        if let Some((_, len)) = montage_len {
            // Same length as the picture, to the sample.
            level.push("aresample=48000".into());
            level.push("apad".into());
            level.push(format!("atrim=end_sample={}", (len * 48000.0).round() as u64));
        }
        if overlay_sound {
            // Graph: source -> mix in "follow speed" sound -> speed -> mix in
            // the other overlays' sound -> volume -> [aout].
            let mut cur = match source {
                ExportAudio::Track(i) => format!("0:a:{i}"),
                ExportAudio::Mix(mix) => {
                    complex.push(crate::tracks::ffmpeg_mix_graph(mix, &[]).replacen("[aout]", "[amain]", 1));
                    "amain".into()
                }
                ExportAudio::None => unreachable!(),
            };
            let mix_in = |complex: &mut Vec<String>, cur: &mut String, follow: bool, tag: &str| {
                let mut labels = Vec::new();
                for (i, ov) in overlays.iter().enumerate().filter(|(_, ov)| ov.sound() && ov.follows_speed() == follow) {
                    let (t0, t1) = clock(ov);
                    let label = format!("oa{i}");
                    complex.push(crate::overlay::audio_filters(ov, input_of[i].unwrap(), &label, t0, t1));
                    labels.push(label);
                }
                if !labels.is_empty() {
                    let ins: String = labels.iter().map(|l| format!("[{l}]")).collect();
                    complex.push(format!("[{cur}]{ins}amix=inputs={}:duration=first:normalize=0[{tag}]", labels.len() + 1));
                    *cur = tag.into();
                }
            };
            mix_in(&mut complex, &mut cur, true, "af0");
            if let Some(chain) = &speed {
                complex.push(format!("[{cur}]{chain}[af1]"));
                cur = "af1".into();
            }
            mix_in(&mut complex, &mut cur, false, "af2");
            let tail = if level.is_empty() { "anull".to_string() } else { level.join(",") };
            complex.push(format!("[{cur}]{tail}[aout]"));
        } else {
            let af: Vec<String> = speed.into_iter().chain(level).collect();
            if let ExportAudio::Mix(mix) = source {
                complex.push(crate::tracks::ffmpeg_mix_graph(mix, &af));
            } else if !af.is_empty() {
                a.extend(["-af".into(), af.join(",")]);
            }
        }
        a.extend(["-c:a".into(), codec.encoder().into()]);
        if codec.has_bitrate() {
            let kbps = if audio_only { o.mp3_kbps } else { o.audio_kbps };
            a.extend(["-b:a".into(), format!("{kbps}k")]);
        }
        if j.montage.is_some() {
            a.extend(["-ar".into(), "48000".into(), "-ac".into(), "2".into()]);
        }
    } else if silent_input {
        a.extend(["-c:a".into(), o.acodec.encoder().into()]);
        if o.acodec.has_bitrate() {
            a.extend(["-b:a".into(), format!("{}k", o.audio_kbps)]);
        }
        a.extend(["-ar".into(), "48000".into(), "-ac".into(), "2".into()]);
    } else {
        a.push("-an".into());
    }
    if !overlays.is_empty() || j.montage.is_some() {
        // Looping overlay inputs run on; stop at the clip's own length.
        a.extend(["-t".into(), n(montage_len.map_or_else(|| map.out_len(j.start, j.end), |m| m.1))]);
    }
    finish(a, complex, container, fast, j.output)
}

/// Builds a video filter graph one link at a time: each link takes the
/// current picture (plus any `extra` inputs) and names its output.
struct Chain<'a> {
    parts: &'a mut Vec<String>,
    cur: String,
    /// The last link's number; usize::MAX makes the next output `[vout]`.
    k: usize,
}

impl Chain<'_> {
    fn link(&mut self, extra: &str, filters: &str) {
        let next = if self.k == usize::MAX {
            "vout".to_string()
        } else {
            self.k += 1;
            format!("v{}", self.k)
        };
        self.parts.push(format!("[{}]{extra}{filters}[{next}]", self.cur));
        self.cur = next;
    }
}

/// The audio the export takes, or None for no audio track at all.
fn export_audio_of(j: &Job) -> Option<ExportAudio> {
    if j.media.audio.is_none() || j.opts.av == AvMode::RemoveAudio {
        return None;
    }
    match crate::tracks::export_audio(j.tracks) {
        ExportAudio::None => None,
        other => Some(other),
    }
}

fn finish(mut a: Vec<String>, complex: Vec<String>, container: Container, fast: bool, output: &Path) -> Vec<String> {
    if !complex.is_empty() {
        a.extend(["-filter_complex".into(), complex.join(";")]);
    }
    a.extend(["-map_metadata".into(), "0".into()]);
    if matches!(container, Container::Mp4 | Container::Mov) {
        a.extend(["-movflags".into(), "+faststart".into()]);
    }
    if fast {
        a.extend(["-avoid_negative_ts".into(), "make_zero".into()]);
    }
    a.extend(["-progress".into(), "pipe:1".into(), "-nostats".into()]);
    a.push(output.to_string_lossy().into_owned());
    a
}

fn video_quality_args(enc: &str, o: &Options) -> Vec<String> {
    let q = o.quality.index();
    let s = o.enc_speed.index();
    let k = o.custom_kbps;
    let custom = o.quality == Quality::Custom;
    // A size cap needs the peak held at the target, or busy scenes overshoot.
    let peak = if o.preset.cap_mb().is_some() { k } else { k * 3 / 2 };
    let bitrate = || vec!["-b:v".to_string(), format!("{k}k"), "-maxrate".into(), format!("{peak}k"), "-bufsize".into(), format!("{}k", k * 2)];
    let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<String>>();

    let mut out: Vec<String>;
    if enc == "libx264" || enc == "libx265" {
        out = v(&["-preset", ["veryfast", "medium", "slow"][s]]);
        if custom { out.extend(bitrate()) } else { out.extend(v(&["-crf", ["18", "23", "28"][q]])) }
        if enc == "libx265" {
            out.extend(v(&["-x265-params", "log-level=error"]));
        }
    } else if enc.ends_with("_nvenc") {
        out = v(&["-preset", ["p2", "p5", "p7"][s], "-rc", "vbr"]);
        let cq = if enc == "av1_nvenc" { ["22", "28", "34"][q] } else { ["19", "24", "30"][q] };
        if custom { out.extend(bitrate()) } else { out.extend(v(&["-cq", cq, "-b:v", "0"])) }
    } else if enc.ends_with("_amf") {
        out = v(&["-quality", ["speed", "balanced", "quality"][s]]);
        if custom {
            out.extend(v(&["-rc", "vbr_peak"]));
            out.extend(bitrate());
        } else {
            let qp = ["18", "23", "28"][q];
            out.extend(v(&["-rc", "cqp", "-qp_i", qp, "-qp_p", qp]));
        }
    } else if enc == "libsvtav1" {
        out = v(&["-preset", ["10", "8", "5"][s]]);
        if custom { out.extend(bitrate()) } else { out.extend(v(&["-crf", ["24", "32", "40"][q]])) }
    } else if enc == "libaom-av1" {
        out = v(&["-cpu-used", ["8", "6", "4"][s], "-row-mt", "1"]);
        if custom { out.extend(bitrate()) } else { out.extend(v(&["-crf", ["24", "32", "40"][q], "-b:v", "0"])) }
    } else if enc == "libvpx-vp9" {
        out = v(&["-deadline", "good", "-cpu-used", ["5", "3", "1"][s], "-row-mt", "1"]);
        if custom { out.extend(bitrate()) } else { out.extend(v(&["-crf", ["24", "32", "40"][q], "-b:v", "0"])) }
    } else if enc == "prores_ks" {
        // 3 = HQ, 2 = standard, 0 = proxy. ProRes has no bitrate control.
        let profile = if custom { "3" } else { ["3", "2", "0"][q] };
        out = v(&["-profile:v", profile, "-vendor", "apl0"]);
    } else {
        out = Vec::new();
    }
    out
}

/// Rough output size in bytes, for the free-space check.
pub fn estimate_bytes(j: &Job) -> u64 {
    let o = j.opts;
    let dur = output_duration(j.start, j.end, j.speeds());
    let src_bps = j.media.bitrate.unwrap_or(8_000_000) as f64;
    let bps = if o.av == AvMode::AudioOnly {
        o.mp3_kbps as f64 * 1000.0
    } else if o.mode == CutMode::Fast {
        src_bps
    } else if o.vcodec == VCodec::ProRes {
        let px = j.media.video.as_ref().map(|v| (v.width * v.height) as f64).unwrap_or(2_073_600.0);
        px * 60.0 // ~120 Mbps at 1080p for ProRes HQ
    } else if o.quality == Quality::Custom {
        (o.custom_kbps + o.audio_kbps) as f64 * 1000.0
    } else {
        src_bps * [1.2, 0.8, 0.5][o.quality.index()]
    };
    (bps * dur / 8.0) as u64
}

/// What an export will come out as, for the Estimate card.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub bytes: u64,
    /// Output picture size; None for audio only.
    pub size: Option<(u32, u32)>,
    pub fps: Option<f64>,
    /// Video bitrate; a guess unless the quality is a fixed bitrate.
    pub video_kbps: Option<u32>,
    /// Audio bitrate; None when there's no audio or it's lossless.
    pub audio_kbps: Option<u32>,
    /// Fast mode: the video is copied as it is.
    pub video_copied: bool,
    /// The video bitrate is set exactly (Custom bitrate or a Discord preset).
    pub exact_bitrate: bool,
}

pub fn estimate(j: &Job) -> Estimate {
    let o = j.opts;
    let m = j.media;
    let fast = o.mode == CutMode::Fast;
    let audio_only = o.av == AvMode::AudioOnly || m.video.is_none();
    let dur = output_duration(j.start, j.end, j.speeds()).max(0.001);
    let bytes = estimate_bytes(j);
    let has_a = export_audio_of(j).is_some();
    let audio_kbps = if !has_a || o.av == AvMode::Mute && !audio_only {
        // Muted audio is a silent track: next to nothing.
        None
    } else if audio_only {
        Some(o.mp3_kbps)
    } else if fast || !o.acodec.has_bitrate() {
        None
    } else {
        Some(o.audio_kbps)
    };
    let (size, fps, video_kbps) = match m.video.as_ref().filter(|_| !audio_only) {
        None => (None, None, None),
        Some(v) if fast => (Some((v.width, v.height)), Some(v.fps), m.bitrate.map(|b| (b / 1000) as u32)),
        Some(v) => {
            let size = match o.resolution.height() {
                Some(h) => (crate::discord::scaled_width(v.width, v.height, h), h),
                None => (v.width, v.height),
            };
            let fps = match o.fps {
                FrameRate::Fps(f) => f as f64,
                FrameRate::Same => v.fps,
            };
            let kbps = if o.quality == Quality::Custom {
                o.custom_kbps
            } else {
                ((bytes as f64 * 8.0 / dur / 1000.0) as u32).saturating_sub(audio_kbps.unwrap_or(0))
            };
            (Some(size), Some(fps), Some(kbps))
        }
    };
    Estimate { bytes, size, fps, video_kbps, audio_kbps, video_copied: fast && !audio_only, exact_bitrate: !fast && o.quality == Quality::Custom }
}

/// A local date and time, to the minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

impl Stamp {
    /// `17-11-26 21-04` (dd-mm-yy hh-mm). Windows doesn't allow `:` in names.
    pub fn text(&self) -> String {
        format!("{:02}-{:02}-{:02} {:02}-{:02}", self.day, self.month, self.year % 100, self.hour, self.minute)
    }
}

/// `Call of Duty Black Ops 6 17-11-26 21-04`, without extension: the game
/// (or the source's own name when the game isn't known) and when it was exported.
pub fn auto_name(game: &str, at: Stamp) -> String {
    format!("{game} {}", at.text())
}

/// Make a user-typed name safe for Windows. Returns None if nothing usable is left.
pub fn sanitize_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .trim()
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        return None;
    }
    let upper = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT")) && upper.len() == 4 && upper.as_bytes()[3].is_ascii_digit());
    Some(if reserved { format!("{cleaned}_") } else { cleaned })
}

/// `dir/name.ext`, or `dir/name (2).ext`, `(3)`… if taken. Never overwrites.
pub fn unique_path(dir: &Path, name: &str, ext: &str) -> PathBuf {
    // If the user typed the extension themselves, don't double it.
    let suffix = format!(".{ext}");
    let base = if name.to_ascii_lowercase().ends_with(&suffix) { &name[..name.len() - suffix.len()] } else { name };
    let first = dir.join(format!("{base}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|i| dir.join(format!("{base} ({i}).{ext}"))).find(|p| !p.exists()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{AudioInfo, VideoInfo};
    use crate::speed::Resolved;

    fn media() -> MediaInfo {
        MediaInfo {
            path: "C:\\in\\holiday clip.mp4".into(),
            duration: 60.0,
            size_bytes: 60_000_000,
            bitrate: Some(8_000_000),
            video: Some(VideoInfo { codec: "h264".into(), width: 1920, height: 1080, fps: 60.0, hdr: false }),
            audio: Some(AudioInfo { codec: "aac".into(), sample_rate: 48000, channels: 2, title: None }),
            audio_tracks: vec![AudioInfo { codec: "aac".into(), sample_rate: 48000, channels: 2, title: None }],
        }
    }

    fn args(o: &Options, enc: Option<&'static str>) -> String {
        args_speed(o, enc, &[])
    }

    fn whole(speed: f64) -> [Resolved; 1] {
        [Resolved { start: 25.0, end: 45.0, speed, smooth: false, mute: false }]
    }

    /// The trim is 25-45 s.
    fn args_speed(o: &Options, enc: Option<&'static str>, speed_zones: &[Resolved]) -> String {
        let m = media();
        let out = PathBuf::from("C:\\out\\x.mp4");
        let j = Job { media: &m, opts: o, start: 25.0, end: 45.0, output: &out, video_encoder: enc, can_tonemap: true, zones: &[], tracks: &[], speed_zones, overlays: &[], montage: None };
        build_args(&j).join(" ")
    }

    /// Issue #6: game and mic on separate tracks.
    #[test]
    fn multi_track_audio() {
        use crate::tracks::TrackMix;
        let mut m = media();
        m.audio_tracks.push(AudioInfo { codec: "aac".into(), sample_rate: 48000, channels: 1, title: Some("Mic".into()) });
        let out = PathBuf::from(r"C:\out\x.mp4");
        let run = |o: &Options, tracks: &[TrackMix]| {
            build_args(&Job { media: &m, opts: o, start: 0.0, end: 5.0, output: &out, video_encoder: Some("libx264"), can_tonemap: true, zones: &[], tracks, speed_zones: &[], overlays: &[], montage: None }).join(" ")
        };
        let both = [TrackMix::default(), TrackMix { volume_pct: 150, ..Default::default() }];
        let o = Options { volume_pct: 80, ..Options::default() };
        let a = run(&o, &both);
        assert!(a.contains("-map [aout]"), "{a}");
        assert!(a.contains("-filter_complex [0:a:0]volume=1[m0];[0:a:1]volume=1.5[m1];[m0][m1]amix=inputs=2:duration=longest:normalize=0,volume=0.8[aout]"), "{a}");
        assert!(!a.contains("-af "), "{a}");

        // Fast mode can't copy a mix, so the audio is encoded.
        let fast = Options { mode: CutMode::Fast, ..Options::default() };
        let a = run(&fast, &both);
        assert!(a.contains("-c:v copy") && !a.contains("-c:a copy") && a.contains("amix"), "{a}");

        // Just the mic, untouched: a plain stream that Fast mode can copy.
        let mic_only = [TrackMix { on: false, ..Default::default() }, TrackMix::default()];
        let a = run(&fast, &mic_only);
        assert!(a.contains("-map 0:a:1") && a.contains("-c:a copy") && !a.contains("filter_complex"), "{a}");

        // Everything off: no audio track.
        let none = [TrackMix { on: false, ..Default::default() }; 2];
        let a = run(&Options::default(), &none);
        assert!(a.contains("-an") && !a.contains("0:a"), "{a}");
    }

    #[test]
    fn precise_default() {
        let a = args(&Options::default(), Some("libx264"));
        assert!(a.contains("-ss 25 -t 20 -i"));
        assert!(a.contains("-c:v libx264 -preset medium -crf 23"));
        assert!(a.contains("-c:a aac -b:a 192k"));
        assert!(a.contains("-movflags +faststart"));
        assert!(a.contains(" -n "));
    }

    #[test]
    fn fast_copies() {
        let o = Options { mode: CutMode::Fast, volume_pct: 50, ..Default::default() };
        let a = args_speed(&o, None, &whole(2.0));
        assert!(a.contains("-c:v copy"));
        assert!(a.contains("-c:a copy"));
        assert!(!a.contains("setpts"), "speed is ignored in fast mode");
        assert!(a.contains("-avoid_negative_ts make_zero"));
    }

    #[test]
    fn speed_and_pitch() {
        let a = args_speed(&Options::default(), Some("libx264"), &whole(0.25));
        assert!(a.contains("setpts=PTS/0.25,fps=60"));
        assert!(a.contains("-af atempo=0.5,atempo=0.5"));
        let o = Options { change_pitch: true, ..Default::default() };
        assert!(args_speed(&o, Some("libx264"), &whole(2.0)).contains("asetrate=96000,aresample=48000"));
    }

    #[test]
    fn speed_zone_pieces() {
        // 30-32 s at half speed, muted; the rest normal.
        let z = [Resolved { start: 30.0, end: 32.0, speed: 0.5, smooth: false, mute: true }];
        let a = args_speed(&Options { volume_pct: 80, ..Default::default() }, Some("libx264"), &z);
        assert!(a.contains("-vf split=3[vs0][vs1][vs2];[vs0]trim=start=0:end=5,"), "{a}");
        assert!(a.contains("concat=n=3:v=1:a=0,fps=60,format=yuv420p"), "{a}");
        assert!(a.contains("[as1]atrim=start=5:end=7,asetpts=PTS-STARTPTS,atempo=0.5,volume=0[ap1]"), "{a}");
        assert!(a.contains("concat=n=3:v=0:a=1,volume=0.8 "), "{a}");
        assert!((output_duration(25.0, 45.0, &z) - 22.0).abs() < 1e-9);
    }

    #[test]
    fn audio_modes() {
        let o = Options { av: AvMode::Mute, ..Default::default() };
        assert!(args(&o, Some("libx264")).contains("-af volume=0"));
        let o = Options { av: AvMode::RemoveAudio, ..Default::default() };
        let a = args(&o, Some("libx264"));
        assert!(a.contains("-an") && !a.contains("0:a:0"));
        let o = Options { av: AvMode::AudioOnly, volume_pct: 60, ..Default::default() };
        let a = args(&o, None);
        assert!(a.contains("-vn") && a.contains("-af volume=0.6 -c:a libmp3lame -b:a 192k"));
        assert!(!a.contains("movflags"));
    }

    #[test]
    fn nvenc_and_hevc_tag() {
        let o = Options { vcodec: VCodec::H265, quality: Quality::High, ..Default::default() };
        let a = args(&o, Some("hevc_nvenc"));
        assert!(a.contains("-c:v hevc_nvenc -preset p5 -rc vbr -cq 19 -b:v 0"));
        assert!(a.contains("-tag:v hvc1"));
    }

    #[test]
    fn zoom_before_speed() {
        let m = media();
        let out = PathBuf::from("out.mp4");
        let bx = crate::zoom::ZBox::centered(0.5);
        let zones = [crate::zoom::Resolved { start: 30.0, end: 35.0, glide: 0.5, a: bx, b: bx, path: None }];
        let o = Options::default();
        let sz = whole(2.0);
        let j = Job { media: &m, opts: &o, start: 25.0, end: 45.0, output: &out, video_encoder: Some("libx264"), can_tonemap: true, zones: &zones, tracks: &[], speed_zones: &sz, overlays: &[], montage: None };
        let a = build_args(&j).join(" ");
        let zp = a.find("fps=60,zoompan=").expect("zoompan present");
        assert!(zp < a.find("setpts=PTS/2").unwrap());
        assert!(a.contains("if(lt(it,5),"), "zone times are relative to the trim start: {a}");
        let o = Options { mode: CutMode::Fast, ..Default::default() };
        let j = Job { opts: &o, ..j };
        assert!(!build_args(&j).join(" ").contains("zoompan"));
    }

    #[test]
    fn discord_preset_args() {
        let base = Options { preset: Preset::Discord10, container: Container::Mkv, vcodec: VCodec::H265, mode: CutMode::Fast, av: AvMode::AudioOnly, ..Default::default() };
        let plan = crate::discord::plan(10_000_000, 20.0, 1920, 1080, 60.0, true).unwrap();
        let o = base.with_plan(&plan);
        assert_eq!((o.container, o.vcodec, o.mode, o.av), (Container::Mp4, VCodec::H264, CutMode::Precise, AvMode::Both));
        let a = args(&o, Some("libx264"));
        let k = plan.video_kbps;
        assert!(a.contains(&format!("-b:v {k}k -maxrate {k}k")), "peak held at the target: {a}");
        assert!(a.contains("fps=30,format=yuv420p") && !a.contains("scale="), "1080p30 from a 1080p60 source: {a}");
        assert!(a.contains("-c:a aac -b:a 128k"));
        // Custom keeps the usual 1.5x peak.
        let o = Options { quality: Quality::Custom, custom_kbps: 1000, ..Default::default() };
        assert!(args(&o, Some("libx264")).contains("-maxrate 1500k"));
    }

    #[test]
    fn estimates() {
        let m = media();
        let out = PathBuf::from("x.mp4");
        let o = Options { resolution: Resolution::P720, fps: FrameRate::Fps(30), ..Default::default() };
        let j = Job { media: &m, opts: &o, start: 0.0, end: 10.0, output: &out, video_encoder: None, can_tonemap: true, zones: &[], tracks: &[], speed_zones: &[], overlays: &[], montage: None };
        let e = estimate(&j);
        assert_eq!((e.size, e.fps, e.audio_kbps, e.exact_bitrate), (Some((1280, 720)), Some(30.0), Some(192), false));
        let o = Options { mode: CutMode::Fast, ..Default::default() };
        let e = estimate(&Job { opts: &o, ..j });
        assert!(e.video_copied && e.size == Some((1920, 1080)));
    }

    #[test]
    fn atempo_chains() {
        use crate::speed::atempo_chain;
        assert_eq!(atempo_chain(2.0), vec!["atempo=2"]);
        assert_eq!(atempo_chain(0.3), vec!["atempo=0.5", "atempo=0.6"]);
    }

    #[test]
    fn names() {
        let at = Stamp { year: 2026, month: 11, day: 7, hour: 9, minute: 4 };
        assert_eq!(auto_name("Call of Duty Black Ops 6", at), "Call of Duty Black Ops 6 07-11-26 09-04");
        assert_eq!(sanitize_name("  my: clip?. "), Some("my_ clip_".into()));
        assert_eq!(sanitize_name("con"), Some("con_".into()));
        assert_eq!(sanitize_name("   "), None);
    }
}
