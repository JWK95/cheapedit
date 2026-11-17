//! Turns the user's choices into an FFmpeg command line.
//!
//! This is pure (no I/O), so every combination can be unit-tested.

use crate::discord::{Plan, Preset};
use crate::formats::{ACodec, Container, VCodec};
use crate::probe::MediaInfo;
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
    pub speed: f64,
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
            speed: 1.0,
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
    pub fn effective_speed(&self) -> f64 {
        if self.mode == CutMode::Fast { 1.0 } else { self.speed }
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
}

/// Length of the exported clip in seconds.
pub fn output_duration(start: f64, end: f64, speed: f64) -> f64 {
    ((end - start) / speed).max(0.0)
}

fn n(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.to_string() }
}

/// `atempo` only accepts 0.5–100, so slower speeds are chained (0.25 = 0.5 × 0.5).
pub fn atempo_chain(speed: f64) -> Vec<String> {
    let mut out = Vec::new();
    let mut r = speed;
    while r < 0.5 {
        out.push("atempo=0.5".to_string());
        r /= 0.5;
    }
    out.push(format!("atempo={}", n(r)));
    out
}

const TONEMAP: &str = "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv";

pub fn build_args(j: &Job) -> Vec<String> {
    let o = j.opts;
    let m = j.media;
    let fast = o.mode == CutMode::Fast;
    let speed = o.effective_speed();
    let audio_only = o.av == AvMode::AudioOnly;
    let container = o.output_container();
    let has_v = m.video.is_some() && !audio_only;
    let has_a = m.audio.is_some() && o.av != AvMode::RemoveAudio;

    let mut a: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    push(&["-hide_banner", "-nostdin", "-n", "-loglevel", "warning"]);
    push(&["-ss", &n(j.start), "-t", &n(j.end - j.start), "-i"]);
    a.push(m.path.to_string_lossy().into_owned());
    if has_v {
        a.extend(["-map".into(), "0:v:0".into()]);
    }
    if has_a {
        a.extend(["-map".into(), "0:a:0".into()]);
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
            let mut vf: Vec<String> = Vec::new();
            if v.hdr && j.can_tonemap {
                vf.push(TONEMAP.into());
            }
            // Zoom before speed, so zone times match the original video. The
            // fps filter first makes the frame rate constant, which zoompan needs.
            if let Some(z) = crate::zoom::zoompan(j.zones, j.start, v.width, v.height, v.fps) {
                vf.push(format!("fps={}", n(v.fps)));
                vf.push(z);
            }
            if speed != 1.0 {
                vf.push(format!("setpts=PTS/{}", n(speed)));
            }
            let target_fps = match o.fps {
                FrameRate::Fps(f) => Some(f as f64),
                FrameRate::Same if speed != 1.0 => Some(v.fps),
                FrameRate::Same => None,
            };
            if let Some(f) = target_fps {
                vf.push(format!("fps={}", n(f)));
            }
            if let Some(h) = o.resolution.height() {
                vf.push(format!("scale=-2:{h}:flags=lanczos"));
            }
            vf.push(if o.vcodec == VCodec::ProRes { "format=yuv422p10le".into() } else { "format=yuv420p".into() });
            a.extend(["-vf".into(), vf.join(",")]);
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
    if has_a {
        let src = &m.audio.as_ref().unwrap().codec;
        let mute = o.av == AvMode::Mute;
        let volume = if fast { 1.0 } else { o.volume_pct as f64 / 100.0 };
        let codec = if audio_only {
            ACodec::Mp3
        } else if fast {
            if !mute && container.can_copy_audio(src) {
                a.extend(["-c:a".into(), "copy".into()]);
                return finish(a, container, fast, j.output);
            }
            ACodec::from_ffprobe(src).filter(|c| container.audio_codecs().contains(c) && *c != ACodec::Pcm).unwrap_or(container.default_audio())
        } else {
            o.acodec
        };
        let mut af: Vec<String> = Vec::new();
        if speed != 1.0 {
            if o.change_pitch {
                let sr = m.audio.as_ref().unwrap().sample_rate;
                af.push(format!("asetrate={},aresample={sr}", (sr as f64 * speed).round() as u32));
            } else {
                af.extend(atempo_chain(speed));
            }
        }
        if mute {
            af.push("volume=0".into());
        } else if volume != 1.0 {
            af.push(format!("volume={}", n(volume)));
        }
        if !af.is_empty() {
            a.extend(["-af".into(), af.join(",")]);
        }
        a.extend(["-c:a".into(), codec.encoder().into()]);
        if codec.has_bitrate() {
            let kbps = if audio_only { o.mp3_kbps } else { o.audio_kbps };
            a.extend(["-b:a".into(), format!("{kbps}k")]);
        }
    } else {
        a.push("-an".into());
    }
    finish(a, container, fast, j.output)
}

fn finish(mut a: Vec<String>, container: Container, fast: bool, output: &Path) -> Vec<String> {
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
    let dur = output_duration(j.start, j.end, o.effective_speed());
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
    let dur = output_duration(j.start, j.end, o.effective_speed()).max(0.001);
    let bytes = estimate_bytes(j);
    let has_a = m.audio.is_some() && o.av != AvMode::RemoveAudio;
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

    fn media() -> MediaInfo {
        MediaInfo {
            path: "C:\\in\\holiday clip.mp4".into(),
            duration: 60.0,
            size_bytes: 60_000_000,
            bitrate: Some(8_000_000),
            video: Some(VideoInfo { codec: "h264".into(), width: 1920, height: 1080, fps: 60.0, hdr: false }),
            audio: Some(AudioInfo { codec: "aac".into(), sample_rate: 48000, channels: 2 }),
        }
    }

    fn args(o: &Options, enc: Option<&'static str>) -> String {
        let m = media();
        let out = PathBuf::from("C:\\out\\x.mp4");
        let j = Job { media: &m, opts: o, start: 25.0, end: 45.0, output: &out, video_encoder: enc, can_tonemap: true, zones: &[] };
        build_args(&j).join(" ")
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
        let o = Options { mode: CutMode::Fast, speed: 2.0, volume_pct: 50, ..Default::default() };
        let a = args(&o, None);
        assert!(a.contains("-c:v copy"));
        assert!(a.contains("-c:a copy"));
        assert!(!a.contains("setpts"), "speed is ignored in fast mode");
        assert!(a.contains("-avoid_negative_ts make_zero"));
    }

    #[test]
    fn speed_and_pitch() {
        let o = Options { speed: 0.25, ..Default::default() };
        let a = args(&o, Some("libx264"));
        assert!(a.contains("setpts=PTS/0.25,fps=60"));
        assert!(a.contains("-af atempo=0.5,atempo=0.5"));
        let o = Options { speed: 2.0, change_pitch: true, ..Default::default() };
        assert!(args(&o, Some("libx264")).contains("asetrate=96000,aresample=48000"));
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
        let zones = [crate::zoom::Resolved { start: 30.0, end: 35.0, glide: 0.5, a: bx, b: bx }];
        let o = Options { speed: 2.0, ..Default::default() };
        let j = Job { media: &m, opts: &o, start: 25.0, end: 45.0, output: &out, video_encoder: Some("libx264"), can_tonemap: true, zones: &zones };
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
        let j = Job { media: &m, opts: &o, start: 0.0, end: 10.0, output: &out, video_encoder: None, can_tonemap: true, zones: &[] };
        let e = estimate(&j);
        assert_eq!((e.size, e.fps, e.audio_kbps, e.exact_bitrate), (Some((1280, 720)), Some(30.0), Some(192), false));
        let o = Options { mode: CutMode::Fast, ..Default::default() };
        let e = estimate(&Job { opts: &o, ..j });
        assert!(e.video_copied && e.size == Some((1920, 1080)));
    }

    #[test]
    fn atempo_chains() {
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
