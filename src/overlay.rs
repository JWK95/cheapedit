//! Overlays: images, GIFs, videos and text on top of the clip.
//!
//! Each overlay has a start and end time (original-video seconds, like zones)
//! and a start position, plus an optional end position it glides to. Media
//! overlays become extra FFmpeg inputs joined with `overlay`; text uses
//! `drawtext`. The builders here are pure, so they're unit-tested.

use crate::timecode;
use std::path::{Path, PathBuf};

pub const MAX_OVERLAYS: usize = 20;
/// Smallest media overlay, as a fraction of the frame width.
pub const MIN_W: f64 = 0.03;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    Image,
    /// An animated GIF or WebP.
    Animated,
    Video,
    Text,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Image => "Image",
            Kind::Animated => "GIF",
            Kind::Video => "Video",
            Kind::Text => "Text",
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TextStyle {
    pub text: String,
    /// The font's display name, e.g. "Arial Bold".
    pub font_name: String,
    pub font_file: Option<PathBuf>,
    /// Font size as a fraction of the frame height.
    pub size: f64,
    pub color: [u8; 3],
    pub outline: bool,
    pub outline_color: [u8; 3],
    /// Outline width as a fraction of the font size.
    pub outline_w: f64,
    pub shadow: bool,
    pub background: bool,
    pub bg_color: [u8; 3],
    pub bg_opacity: u8,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            text: "Your text".into(),
            font_name: "Arial Bold".into(),
            font_file: None,
            size: 0.08,
            color: [255, 255, 255],
            outline: true,
            outline_color: [0, 0, 0],
            outline_w: 0.06,
            shadow: false,
            background: false,
            bg_color: [0, 0, 0],
            bg_opacity: 60,
        }
    }
}

/// A position on the frame: the overlay's top-left corner, as fractions of
/// the frame's width and height.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Pos {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Overlay {
    pub kind: Kind,
    /// The file, for everything but text.
    pub path: Option<PathBuf>,
    /// Height / width of the media in pixels (text: worked out from the preview).
    pub aspect: f64,
    /// The media has a sound track (videos only).
    pub has_audio: bool,
    pub text: TextStyle,
    pub start_text: String,
    pub end_text: String,
    /// Start position, and the end position when moving.
    pub a: Pos,
    pub b: Pos,
    /// Width as a fraction of the frame width (media; text uses its size).
    pub w: f64,
    /// Glide from the start position to the end position.
    pub pan: bool,
    pub b_drawn: bool,
    pub opacity: u8,
    pub fade_in: f64,
    pub fade_out: f64,
    /// GIFs: play once and hold the last frame, instead of looping.
    pub play_once: bool,
    /// Videos: include the overlay's own sound.
    pub audio_on: bool,
    pub volume_pct: u32,
    /// Videos: speed up and slow down with the speed zones.
    pub follow_speed: bool,
    /// Text: a UTF-8 file holding the text, written just before an export
    /// (drawtext reads it, which avoids escaping the text itself).
    #[serde(skip)]
    pub text_file: Option<PathBuf>,
}

impl Overlay {
    fn base(kind: Kind, start: f64, end: f64) -> Overlay {
        Overlay {
            kind,
            path: None,
            aspect: 1.0,
            has_audio: false,
            text: TextStyle::default(),
            start_text: timecode::format_compact(start),
            end_text: timecode::format_compact(end),
            a: Pos { x: 0.35, y: 0.35 },
            b: Pos { x: 0.35, y: 0.35 },
            w: 0.3,
            pan: false,
            b_drawn: false,
            opacity: 100,
            fade_in: 0.0,
            fade_out: 0.0,
            play_once: false,
            audio_on: false,
            volume_pct: 100,
            follow_speed: false,
            text_file: None,
        }
    }
    /// A media overlay, centred, a third of the frame wide.
    /// `aspect` is height / width in pixels; `frame_aspect` the video's.
    pub fn media(kind: Kind, path: PathBuf, aspect: f64, frame_aspect: f64, has_audio: bool, start: f64, end: f64) -> Overlay {
        let mut o = Overlay::base(kind, start, end);
        o.path = Some(path);
        o.aspect = aspect.max(0.01);
        o.has_audio = has_audio;
        // Keep it inside the frame: shrink tall things.
        o.w = 0.33f64.min(0.8 / (o.aspect * frame_aspect.max(0.01)));
        let h = o.w * o.aspect * frame_aspect;
        o.a = Pos { x: (1.0 - o.w) / 2.0, y: ((1.0 - h) / 2.0).max(0.0) };
        o.b = o.a;
        o
    }
    pub fn text(start: f64, end: f64) -> Overlay {
        let mut o = Overlay::base(Kind::Text, start, end);
        o.a = Pos { x: 0.1, y: 0.1 };
        o.b = o.a;
        o
    }
    /// What the card and band call it.
    pub fn name(&self) -> String {
        match &self.path {
            Some(p) => p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
            None => {
                let first = self.text.text.lines().next().unwrap_or("").trim();
                let short: String = first.chars().take(24).collect();
                if short.is_empty() { "(empty text)".into() } else { format!("\u{201c}{short}\u{201d}") }
            }
        }
    }
    pub fn times(&self) -> Option<(f64, f64)> {
        Some((timecode::parse(&self.start_text).ok()?, timecode::parse(&self.end_text).ok()?))
    }
    /// The end position actually used.
    pub fn end_pos(&self) -> Pos {
        if self.pan && self.b_drawn { self.b } else { self.a }
    }
    /// Moves with the speed zones (applied before speed in the export).
    pub fn follows_speed(&self) -> bool {
        self.kind == Kind::Video && self.follow_speed
    }
    /// The overlay's sound goes into the export.
    pub fn sound(&self) -> bool {
        self.kind == Kind::Video && self.audio_on && self.has_audio
    }
}

/// Where the overlay is at `p` (0-1 through its time), eased slow-fast-slow.
pub fn pos_at(o: &Overlay, p: f64) -> Pos {
    let p = p.clamp(0.0, 1.0);
    let e = p * p * (3.0 - 2.0 * p);
    let (a, b) = (o.a, o.end_pos());
    Pos { x: a.x + (b.x - a.x) * e, y: a.y + (b.y - a.y) * e }
}

/// How visible the overlay is `u` seconds into its `len`-second span.
pub fn alpha_at(o: &Overlay, u: f64, len: f64) -> f64 {
    let mut a = o.opacity as f64 / 100.0;
    if o.fade_in > 0.0 {
        a *= (u / o.fade_in).clamp(0.0, 1.0);
    }
    if o.fade_out > 0.0 {
        a *= ((len - u) / o.fade_out).clamp(0.0, 1.0);
    }
    a
}

// ---------------------------------------------------------------- FFmpeg

fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.to_string() }
}

fn even(v: f64) -> u32 {
    ((v / 2.0).round() as u32).max(1) * 2
}

/// A file path as a filter option value: forward slashes, escaped for the
/// option parser and then for the filtergraph parser (FFmpeg's two levels).
pub fn filter_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let esc = |s: &str, special: &str| {
        let mut out = String::new();
        for c in s.chars() {
            if special.contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    };
    esc(&esc(&s, "\\':"), "\\'[],;")
}

fn hex(c: [u8; 3]) -> String {
    format!("0x{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn is_gif(o: &Overlay) -> bool {
    o.path.as_ref().and_then(|p| p.extension()).is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

/// Input options for a media overlay. `fps` is the clip's frame rate (for
/// still images) and `max_len` how long any overlay could need to run.
pub fn input_args(o: &Overlay, fps: f64, max_len: f64) -> Vec<String> {
    let path = o.path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let mut a: Vec<String> = Vec::new();
    match o.kind {
        Kind::Image => a.extend(["-loop".into(), "1".into(), "-framerate".into(), num(fps), "-t".into(), num(max_len)]),
        // GIFs have their own loop switch. Anything else (WebP) loops with the
        // `loop` filter in `media_filters`: -stream_loop hangs on animated WebP.
        Kind::Animated if !o.play_once && is_gif(o) => a.extend(["-ignore_loop".into(), "0".into(), "-t".into(), num(max_len)]),
        _ => {}
    }
    a.extend(["-i".into(), path]);
    a
}

/// Pixel size of a media overlay on a `fw`x`fh` frame.
pub fn pixel_size(o: &Overlay, fw: u32, fh: u32) -> (u32, u32) {
    let w = even(o.w * fw as f64).min(even(fw as f64));
    let h = even(w as f64 * o.aspect).min(even(fh as f64 * 4.0));
    (w, h)
}

/// The eased position as FFmpeg expressions (x, y), in pixels, for `t` in
/// the export's clock, with the overlay showing from `t0` to `t1`.
fn pos_exprs(o: &Overlay, t0: f64, t1: f64) -> (String, String) {
    let (a, b) = (o.a, o.end_pos());
    if a == b {
        return (format!("W*{}", num(a.x)), format!("H*{}", num(a.y)));
    }
    let p = format!("clip((t-{})/{},0,1)", num(t0), num((t1 - t0).max(0.001)));
    let e = format!("({p}*{p}*(3-2*{p}))");
    (format!("W*({}+{}*{e})", num(a.x), num(b.x - a.x)), format!("H*({}+{}*{e})", num(a.y), num(b.y - a.y)))
}

/// Prepare media input `input` (scale, timing, fades, opacity) into `[label]`.
pub fn media_filters(o: &Overlay, input: usize, label: &str, t0: f64, t1: f64, fw: u32, fh: u32) -> String {
    let (w, h) = pixel_size(o, fw, fh);
    let shift = format!("setpts=PTS-STARTPTS+{}/TB", num(t0));
    let mut f = if o.kind == Kind::Animated && !o.play_once && !is_gif(o) {
        // Repeat the (already shrunk) frames forever; the export's -t stops it.
        vec![format!("scale={w}:{h}"), "format=rgba".into(), "loop=loop=-1:size=32767".into(), shift]
    } else {
        vec![shift, format!("scale={w}:{h}"), "format=rgba".into()]
    };
    if o.fade_in > 0.0 {
        f.push(format!("fade=t=in:st={}:d={}:alpha=1", num(t0), num(o.fade_in)));
    }
    if o.fade_out > 0.0 {
        f.push(format!("fade=t=out:st={}:d={}:alpha=1", num((t1 - o.fade_out).max(t0)), num(o.fade_out)));
    }
    if o.opacity < 100 {
        f.push(format!("colorchannelmixer=aa={}", num(o.opacity as f64 / 100.0)));
    }
    format!("[{input}:v]{}[{label}]", f.join(","))
}

/// The `overlay` filter that places a prepared media overlay.
pub fn overlay_filter(o: &Overlay, t0: f64, t1: f64) -> String {
    let (x, y) = pos_exprs(o, t0, t1);
    // Stills and GIFs hold their last frame; a finished video disappears.
    let eof = if o.kind == Kind::Video { "pass" } else { "repeat" };
    format!("overlay=x='{x}':y='{y}':enable='between(t,{},{})':eof_action={eof}:eval=frame", num(t0), num(t1))
}

/// The `drawtext` filter for a text overlay. `fh` is the frame height.
pub fn drawtext(o: &Overlay, t0: f64, t1: f64, fh: u32) -> String {
    let st = &o.text;
    let size = (st.size * fh as f64).round().max(4.0);
    let (x, y) = pos_exprs(o, t0, t1);
    let mut f = vec![
        format!("textfile={}", o.text_file.as_deref().map(filter_path).unwrap_or_default()),
        "expansion=none".into(),
        format!("fontsize={}", num(size)),
        format!("fontcolor={}", hex(st.color)),
        format!("x='{x}'"),
        format!("y='{y}'"),
    ];
    if let Some(ff) = &st.font_file {
        f.insert(1, format!("fontfile={}", filter_path(ff)));
    }
    if st.outline {
        f.push(format!("borderw={}", num((st.outline_w * size).round().max(1.0))));
        f.push(format!("bordercolor={}", hex(st.outline_color)));
    }
    if st.shadow {
        let d = num((size / 15.0).round().max(1.0));
        f.push(format!("shadowx={d}:shadowy={d}:shadowcolor=0x000000@0.6"));
    }
    if st.background {
        f.push(format!("box=1:boxcolor={}@{}:boxborderw={}", hex(st.bg_color), num(st.bg_opacity as f64 / 100.0), num((size / 4.0).round())));
    }
    // Opacity and fades.
    let mut alpha = num(o.opacity as f64 / 100.0);
    if o.fade_in > 0.0 {
        alpha.push_str(&format!("*clip((t-{})/{},0,1)", num(t0), num(o.fade_in)));
    }
    if o.fade_out > 0.0 {
        alpha.push_str(&format!("*clip(({}-t)/{},0,1)", num(t1), num(o.fade_out)));
    }
    if alpha != "1" {
        f.push(format!("alpha='{alpha}'"));
    }
    f.push(format!("enable='between(t,{},{})'", num(t0), num(t1)));
    format!("drawtext={}", f.join(":"))
}

/// A video overlay's sound, placed at `t0` and cut at `t1`, into `[label]`.
pub fn audio_filters(o: &Overlay, input: usize, label: &str, t0: f64, t1: f64) -> String {
    let ms = (t0 * 1000.0).round() as u64;
    format!(
        "[{input}:a]asetpts=PTS-STARTPTS,atrim=end={},adelay={ms}:all=1,volume={}[{label}]",
        num((t1 - t0).max(0.0)),
        num(o.volume_pct as f64 / 100.0)
    )
}

// ---------------------------------------------------------------- checks

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayFix {
    ShrinkToTrim(usize),
    Delete(usize),
    /// Stop moving: stay at the start position.
    NoMove(usize),
}

impl OverlayFix {
    pub fn label(self) -> String {
        match self {
            OverlayFix::ShrinkToTrim(_) => "Shrink overlay to fit".into(),
            OverlayFix::Delete(i) => format!("Delete overlay {}", i + 1),
            OverlayFix::NoMove(_) => "No position change".into(),
        }
    }
    pub fn apply(self, list: &mut Vec<Overlay>, trim: Option<(f64, f64)>) {
        match self {
            OverlayFix::ShrinkToTrim(i) => {
                if let (Some(o), Some((ts, te))) = (list.get_mut(i), trim) {
                    if let Some((s, e)) = o.times() {
                        o.start_text = timecode::format_compact(s.max(ts));
                        o.end_text = timecode::format_compact(e.min(te));
                    }
                }
            }
            OverlayFix::Delete(i) => {
                if i < list.len() {
                    list.remove(i);
                }
            }
            OverlayFix::NoMove(i) => {
                if let Some(o) = list.get_mut(i) {
                    o.pan = false;
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlayProblem {
    pub message: String,
    pub fixes: Vec<OverlayFix>,
}

/// Problems that block the export. `trim` is the trim range, if valid.
pub fn check(list: &[Overlay], trim: Option<(f64, f64)>) -> Vec<OverlayProblem> {
    let mut out = Vec::new();
    let mut p = |message: String, fixes: Vec<OverlayFix>| out.push(OverlayProblem { message, fixes });
    for (i, o) in list.iter().enumerate() {
        let n = i + 1;
        let (s, e) = match (timecode::parse(&o.start_text), timecode::parse(&o.end_text)) {
            (Err(err), _) => {
                p(format!("Overlay {n} start: {err}."), vec![]);
                continue;
            }
            (_, Err(err)) => {
                p(format!("Overlay {n} end: {err}."), vec![]);
                continue;
            }
            (Ok(s), Ok(e)) => (s, e),
        };
        if s >= e {
            p(format!("Overlay {n}: start must be before end."), vec![OverlayFix::Delete(i)]);
            continue;
        }
        if let Some(path) = &o.path {
            if !path.is_file() {
                p(format!("Overlay {n}'s file is missing: {}", path.display()), vec![OverlayFix::Delete(i)]);
            }
        }
        if o.kind == Kind::Text {
            if o.text.text.trim().is_empty() {
                p(format!("Overlay {n} has no text."), vec![OverlayFix::Delete(i)]);
            }
            if o.text.font_file.as_ref().is_some_and(|f| !f.is_file()) {
                p(format!("Overlay {n}'s font file is missing. Pick another font."), vec![]);
            }
        }
        if o.pan && !o.b_drawn {
            p(format!("Overlay {n}'s end position isn't set yet. Place it, or choose No position change."), vec![OverlayFix::NoMove(i)]);
        }
        if let Some((ts, te)) = trim {
            if s < ts - 0.0005 || e > te + 0.0005 {
                let mut fixes = Vec::new();
                if s.max(ts) < e.min(te) {
                    fixes.push(OverlayFix::ShrinkToTrim(i));
                }
                fixes.push(OverlayFix::Delete(i));
                p(
                    format!(
                        "Overlay {n} ({}–{}) isn't inside your trim ({}–{}).",
                        timecode::format_short(s),
                        timecode::format_short(e),
                        timecode::format_short(ts),
                        timecode::format_short(te)
                    ),
                    fixes,
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_escaped_twice() {
        // Option level: C\: ; filtergraph level then escapes that backslash: C\\:
        assert_eq!(filter_path(Path::new(r"C:\Windows\Fonts\arial.ttf")), r"C\\:/Windows/Fonts/arial.ttf");
        assert_eq!(filter_path(Path::new(r"C:\Users\O'Brien\a,b.txt")), r"C\\:/Users/O\\\'Brien/a\,b.txt");
    }

    #[test]
    fn media_overlay_filters() {
        let mut o = Overlay::media(Kind::Animated, "C:\\a\\cat.gif".into(), 0.5, 9.0 / 16.0, false, 2.0, 6.0);
        o.w = 0.25;
        o.fade_in = 0.5;
        o.opacity = 80;
        assert_eq!(pixel_size(&o, 1920, 1080), (480, 240));
        let f = media_filters(&o, 1, "ov0", 1.0, 5.0, 1920, 1080);
        assert_eq!(f, "[1:v]setpts=PTS-STARTPTS+1/TB,scale=480:240,format=rgba,fade=t=in:st=1:d=0.5:alpha=1,colorchannelmixer=aa=0.8[ov0]");
        let ov = overlay_filter(&o, 1.0, 5.0);
        assert!(ov.contains("enable='between(t,1,5)'") && ov.contains("eof_action=repeat"), "{ov}");
        assert!(!ov.contains("clip("), "a still position has no motion: {ov}");
        assert_eq!(input_args(&o, 60.0, 30.0), vec!["-ignore_loop", "0", "-t", "30", "-i", "C:\\a\\cat.gif"]);
        o.play_once = true;
        assert_eq!(input_args(&o, 60.0, 30.0), vec!["-i", "C:\\a\\cat.gif"]);
        o.play_once = false;
        o.path = Some("x.webp".into());
        assert_eq!(input_args(&o, 60.0, 30.0), vec!["-i", "x.webp"]);
        assert!(media_filters(&o, 1, "ov0", 1.0, 5.0, 1920, 1080).contains("format=rgba,loop=loop=-1:size=32767,setpts=PTS-STARTPTS+1/TB"));
    }

    #[test]
    fn moving_overlay_eases() {
        let mut o = Overlay::text(0.0, 4.0);
        o.a = Pos { x: 0.1, y: 0.2 };
        o.b = Pos { x: 0.5, y: 0.2 };
        o.pan = true;
        o.b_drawn = true;
        assert!((pos_at(&o, 0.5).x - 0.3).abs() < 1e-9);
        let (x, y) = pos_exprs(&o, 1.0, 3.0);
        assert!(x.starts_with("W*(0.1+0.4*(clip((t-1)/2,0,1)*"), "{x}");
        assert!(y.starts_with("H*(0.2+0*"), "{y}");
    }

    #[test]
    fn text_overlay_filter() {
        let mut o = Overlay::text(0.0, 4.0);
        o.text_file = Some(r"C:\tmp\t1.txt".into());
        o.text.font_file = Some(r"C:\Windows\Fonts\arialbd.ttf".into());
        o.fade_out = 1.0;
        let f = drawtext(&o, 2.0, 6.0, 1080);
        assert!(f.starts_with(r"drawtext=textfile=C\\:/tmp/t1.txt:fontfile=C\\:/Windows/Fonts/arialbd.ttf:expansion=none:fontsize=86:fontcolor=0xFFFFFF:"), "{f}");
        assert!(f.contains("borderw=5:bordercolor=0x000000"), "{f}");
        assert!(f.contains("alpha='1*clip((6-t)/1,0,1)'"), "{f}");
        assert!(f.ends_with("enable='between(t,2,6)'"), "{f}");
        assert_eq!(alpha_at(&o, 3.5, 4.0), 0.5);
    }

    #[test]
    fn overlay_sound() {
        let mut o = Overlay::media(Kind::Video, "cam.mp4".into(), 0.5625, 0.5625, true, 0.0, 3.0);
        assert!(!o.sound(), "off by default");
        o.audio_on = true;
        o.volume_pct = 50;
        assert_eq!(audio_filters(&o, 2, "oa0", 1.5, 4.0), "[2:a]asetpts=PTS-STARTPTS,atrim=end=2.5,adelay=1500:all=1,volume=0.5[oa0]");
    }

    #[test]
    fn checks() {
        let mut list = vec![Overlay::text(5.0, 9.0)];
        list[0].text.text = "  ".into();
        list[0].pan = true;
        let probs = check(&list, Some((6.0, 20.0)));
        assert_eq!(probs.len(), 3, "{probs:?}");
        OverlayFix::ShrinkToTrim(0).apply(&mut list, Some((6.0, 20.0)));
        assert_eq!(list[0].times(), Some((6.0, 9.0)));
    }
}

impl Default for Overlay {
    fn default() -> Self {
        Overlay::base(Kind::Image, 0.0, 0.0)
    }
}
