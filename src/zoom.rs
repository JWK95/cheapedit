//! Zoom zones: zoom into a box on the picture for part of the clip.
//!
//! The export uses FFmpeg's `zoompan` filter with time expressions. The live
//! preview drives mpv's zoom/pan from `sample`, which evaluates the same
//! segments the FFmpeg expressions are built from, so the two always agree.
//!
//! Pure (no I/O), so all of it is unit-tested.

use crate::timecode;
use crate::tracker::Point;
use std::sync::Arc;

pub const MAX_ZONES: usize = 20;
/// A box can't be smaller than this fraction of the frame (10× zoom).
pub const MIN_SIZE: f64 = 0.1;
/// Above this enlargement the picture visibly softens.
pub const WARN_ZOOM: f64 = 3.0;
pub const DEFAULT_GLIDE: f64 = 0.5;
/// Shortest zone allowed when Smooth is off.
pub const MIN_HARD_LEN: f64 = 0.1;

/// A box on the frame, as fractions of the frame: top-left corner and size.
/// The size is the same fraction of the width and the height, so the box
/// always has the video's shape.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ZBox {
    pub x: f64,
    pub y: f64,
    pub s: f64,
}

impl ZBox {
    pub const FULL: ZBox = ZBox { x: 0.0, y: 0.0, s: 1.0 };

    pub fn centered(s: f64) -> ZBox {
        ZBox { x: (1.0 - s) / 2.0, y: (1.0 - s) / 2.0, s }
    }
    pub fn zoom(self) -> f64 {
        1.0 / self.s
    }
    /// Keep the size within limits and the box inside the frame.
    pub fn clamped(self) -> ZBox {
        let s = self.s.clamp(MIN_SIZE, 1.0);
        ZBox { x: self.x.clamp(0.0, 1.0 - s), y: self.y.clamp(0.0, 1.0 - s), s }
    }
    /// A box with one corner fixed at `anchor` and the opposite corner
    /// following the pointer `p` (both as frame fractions).
    pub fn from_drag(anchor: (f64, f64), p: (f64, f64)) -> ZBox {
        let (ax, ay) = (anchor.0.clamp(0.0, 1.0), anchor.1.clamp(0.0, 1.0));
        let (dx, dy) = (p.0 - ax, p.1 - ay);
        let room_x = if dx >= 0.0 { 1.0 - ax } else { ax };
        let room_y = if dy >= 0.0 { 1.0 - ay } else { ay };
        let s = dx.abs().max(dy.abs()).min(room_x).min(room_y).max(MIN_SIZE);
        let x = if dx >= 0.0 { ax } else { ax - s };
        let y = if dy >= 0.0 { ay } else { ay - s };
        ZBox { x, y, s }.clamped()
    }
    fn lerp(a: ZBox, b: ZBox, e: f64) -> ZBox {
        ZBox { x: a.x + (b.x - a.x) * e, y: a.y + (b.y - a.y) * e, s: a.s + (b.s - a.s) * e }
    }
}

/// One zone as the user edits it (times are text, like the trim boxes).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Zone {
    pub start_text: String,
    pub end_text: String,
    pub smooth: bool,
    pub glide: f64,
    pub pan: bool,
    /// The box (the start box when panning).
    pub a: ZBox,
    /// The end box, used only when panning.
    pub b: ZBox,
    /// Whether the user has drawn each box yet.
    pub a_drawn: bool,
    pub b_drawn: bool,
    /// Follow what's in the start box (tracked by `tracker`).
    pub track: bool,
    /// 0 = follow every wobble, 1 = a slow, steady glide.
    pub smoothness: f64,
    /// Where the target's centre was found, over time (original time).
    pub track_points: Vec<Point>,
    /// Places the user redrew the box; tracking restarts from each.
    pub locks: Vec<Lock>,
}

/// A fix: at time `t` the target is in box `b`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Lock {
    pub t: f64,
    pub b: ZBox,
}

impl Zone {
    pub fn new(start: f64, end: f64) -> Zone {
        let a = ZBox::centered(0.5);
        Zone {
            start_text: timecode::format_compact(start),
            end_text: timecode::format_compact(end),
            smooth: true,
            glide: DEFAULT_GLIDE,
            pan: false,
            a,
            b: a,
            a_drawn: false,
            b_drawn: false,
            track: false,
            smoothness: 0.5,
            track_points: Vec::new(),
            locks: Vec::new(),
        }
    }
    /// Tracked, with a path to follow.
    pub fn tracked(&self) -> bool {
        self.track && !self.track_points.is_empty()
    }
    pub fn times(&self) -> Option<(f64, f64)> {
        Some((timecode::parse(&self.start_text).ok()?, timecode::parse(&self.end_text).ok()?))
    }
    pub fn resolve(&self) -> Option<Resolved> {
        let (start, end) = self.times()?;
        let glide = if self.smooth { self.glide } else { 0.0 };
        if self.tracked() {
            let path = track_path(&self.track_points, &self.locks, self.smoothness, self.a.s, start, end, MAX_TRACK_KEYS);
            if !path.is_empty() {
                let a = box_at(&path, start + glide);
                let b = box_at(&path, end - glide);
                return Some(Resolved { start, end, glide, a, b, path: Some(Arc::new(path)) });
            }
        }
        Some(Resolved { start, end, glide, a: self.a, b: if self.pan { self.b } else { self.a }, path: None })
    }
    /// The strongest enlargement this zone uses.
    pub fn max_zoom(&self) -> f64 {
        if self.pan { self.a.zoom().max(self.b.zoom()) } else { self.a.zoom() }
    }
}

/// A zone with its times worked out. Glide is 0 for a hard cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub start: f64,
    pub end: f64,
    pub glide: f64,
    pub a: ZBox,
    pub b: ZBox,
    /// A tracked zone's camera path: keyframes the view moves between in
    /// straight lines (a and b are where it is after the glide in and before
    /// the glide out).
    pub path: Option<Arc<Vec<(f64, ZBox)>>>,
}

/// Most keyframes one tracked zone's path may have. Each one adds to the
/// FFmpeg command line, which Windows limits to 32,767 characters.
pub const MAX_TRACK_KEYS: usize = 40;

/// The box at `t` along a keyframed path (held before the first and after the last).
pub fn box_at(path: &[(f64, ZBox)], t: f64) -> ZBox {
    match path.iter().position(|k| k.0 > t) {
        None => path.last().map_or(ZBox::FULL, |k| k.1),
        Some(0) => path[0].1,
        Some(i) => {
            let (a, b) = (path[i - 1], path[i]);
            ZBox::lerp(a.1, b.1, (t - a.0) / (b.0 - a.0).max(1e-9))
        }
    }
}

/// Gaussian smoothing of the centres over time; `sigma` in seconds.
fn smooth_points(pts: &[Point], sigma: f64) -> Vec<Point> {
    if sigma <= 1e-6 || pts.len() < 3 {
        return pts.to_vec();
    }
    pts.iter()
        .map(|p| {
            let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
            for q in pts {
                let d = q.t - p.t;
                if d.abs() > 3.0 * sigma {
                    continue;
                }
                let w = (-0.5 * (d / sigma).powi(2)).exp();
                sx += q.x * w;
                sy += q.y * w;
                sw += w;
            }
            Point { t: p.t, x: sx / sw, y: sy / sw }
        })
        .collect()
}

/// Keep only the keyframes needed so straight lines between them stay
/// within `tol` of every point (Ramer-Douglas-Peucker on x and y over time).
fn simplify(pts: &[(f64, ZBox)], tol: f64) -> Vec<(f64, ZBox)> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0, pts.len() - 1)];
    while let Some((i, j)) = stack.pop() {
        let (a, b) = (pts[i], pts[j]);
        let mut worst = (0.0, 0);
        for (k, p) in pts.iter().enumerate().take(j).skip(i + 1) {
            let e = ZBox::lerp(a.1, b.1, (p.0 - a.0) / (b.0 - a.0).max(1e-9));
            let d = (e.x - p.1.x).abs().max((e.y - p.1.y).abs());
            if d > worst.0 {
                worst = (d, k);
            }
        }
        if worst.0 > tol {
            keep[worst.1] = true;
            stack.push((i, worst.1));
            stack.push((worst.1, j));
        }
    }
    pts.iter().zip(keep).filter(|x| x.1).map(|x| *x.0).collect()
}

/// A tracked zone's camera path: the target's centres, smoothed (separately
/// between fixes, so a fix is never blurred away), as boxes of size `s`
/// kept inside the frame, cut down to at most `max_keys` keyframes.
pub fn track_path(points: &[Point], locks: &[Lock], smoothness: f64, s: f64, start: f64, end: f64, max_keys: usize) -> Vec<(f64, ZBox)> {
    let pts: Vec<Point> = points.iter().copied().filter(|p| p.t >= start - 1e-6 && p.t <= end + 1e-6).collect();
    if pts.is_empty() {
        return Vec::new();
    }
    let sigma = smoothness.clamp(0.0, 1.0) * 0.8;
    let mut cuts: Vec<f64> = locks.iter().map(|l| l.t).filter(|&t| t > start && t < end).collect();
    cuts.sort_by(f64::total_cmp);
    let mut smoothed = Vec::new();
    let mut from = 0;
    for c in cuts.iter().copied().chain([f64::INFINITY]) {
        let to = pts.iter().position(|p| p.t >= c - 1e-6).unwrap_or(pts.len()).max(from);
        smoothed.extend(smooth_points(&pts[from..to], sigma));
        from = to;
    }
    let boxes: Vec<(f64, ZBox)> = smoothed.iter().map(|p| (p.t, ZBox { x: p.x - s / 2.0, y: p.y - s / 2.0, s }.clamped())).collect();
    let mut tol = 0.002;
    loop {
        let out = simplify(&boxes, tol);
        if out.len() <= max_keys.max(2) {
            return out;
        }
        tol *= 1.5;
    }
}

/// From `t0` to `t1` the view moves (eased) from `from` to `to`.
/// Outside every segment the full frame is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub t0: f64,
    pub t1: f64,
    pub from: ZBox,
    pub to: ZBox,
    /// Straight-line move (tracked paths) instead of slow-fast-slow.
    pub linear: bool,
}

/// Two smooth zones close enough together glide straight from one box to
/// the next instead of zooming fully out in between.
fn merges(a: &Resolved, b: &Resolved) -> bool {
    a.glide > 0.0 && b.glide > 0.0 && b.start - a.end <= a.glide.max(b.glide) + 1e-9
}

pub fn segments(zones: &[Resolved]) -> Vec<Seg> {
    let mut zs = zones.to_vec();
    zs.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out = Vec::new();
    let mut push = |t0: f64, t1: f64, from: ZBox, to: ZBox, linear: bool| {
        if t1 > t0 + 1e-9 {
            out.push(Seg { t0, t1, from, to, linear });
        }
    };
    for (i, z) in zs.iter().enumerate() {
        let g = z.glide;
        let merged_in = i > 0 && merges(&zs[i - 1], z);
        if !merged_in {
            push(z.start, z.start + g, ZBox::FULL, z.a, false);
        }
        match &z.path {
            Some(path) => {
                // Straight lines between the keyframes inside the glides.
                let (t0, t1) = (z.start + g, z.end - g);
                let mut keys: Vec<(f64, ZBox)> = vec![(t0, z.a)];
                keys.extend(path.iter().copied().filter(|k| k.0 > t0 + 1e-6 && k.0 < t1 - 1e-6));
                keys.push((t1, z.b));
                for w in keys.windows(2) {
                    push(w[0].0, w[1].0, w[0].1, w[1].1, true);
                }
            }
            None => push(z.start + g, z.end - g, z.a, z.b, false),
        }
        match zs.get(i + 1) {
            Some(next) if merges(z, next) => push(z.end - g, next.start + next.glide, z.b, next.a, false),
            _ => push(z.end - g, z.end, z.b, ZBox::FULL, false),
        }
    }
    out
}

/// Slow–fast–slow.
fn ease(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

/// The view at time `t` (seconds on the original video).
pub fn sample(segs: &[Seg], t: f64) -> ZBox {
    for s in segs {
        if t >= s.t0 && t < s.t1 {
            let p = (t - s.t0) / (s.t1 - s.t0);
            return ZBox::lerp(s.from, s.to, if s.linear { p.clamp(0.0, 1.0) } else { ease(p) });
        }
    }
    ZBox::FULL
}

#[derive(Clone, Copy)]
enum Comp {
    X,
    Y,
    S,
}

fn comp(b: ZBox, c: Comp) -> f64 {
    match c {
        Comp::X => b.x,
        Comp::Y => b.y,
        Comp::S => b.s,
    }
}

fn num(v: f64) -> String {
    let s = format!("{v:.5}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
}

/// An FFmpeg expression for one part of the view over time. `tv` is the time
/// variable and `offset` is subtracted from every zone time (the export's
/// clock starts at the trim start).
fn expr(segs: &[Seg], c: Comp, tv: &str, offset: f64) -> String {
    let full = num(comp(ZBox::FULL, c));
    let mut out = String::new();
    let mut close = 0;
    for s in segs {
        let (t0, t1) = (s.t0 - offset, s.t1 - offset);
        // Before this segment: full frame.
        out.push_str(&format!("if(lt({tv},{}),{full},", num(t0)));
        let (f, d) = (comp(s.from, c), comp(s.to, c) - comp(s.from, c));
        let value = if d.abs() < 1e-9 {
            num(f)
        } else if s.linear {
            format!("{}+{}*(({tv}-{})/{})", num(f), num(d), num(t0), num(t1 - t0))
        } else {
            let p = format!("(({tv}-{})/{})", num(t0), num(t1 - t0));
            format!("{}+{}*{p}*{p}*(3-2*{p})", num(f), num(d))
        };
        out.push_str(&format!("if(lt({tv},{}),{value},", num(t1)));
        close += 2;
    }
    out.push_str(&full);
    out.push_str(&")".repeat(close));
    out
}

/// The `zoompan` filter for these zones, or None if there are none.
/// `offset` is the trim start; `w`×`h` and `fps` are the source's.
pub fn zoompan(zones: &[Resolved], offset: f64, w: u32, h: u32, fps: f64) -> Option<String> {
    if zones.is_empty() {
        return None;
    }
    let segs = segments(zones);
    let s = expr(&segs, Comp::S, "it", offset);
    let x = expr(&segs, Comp::X, "it", offset);
    let y = expr(&segs, Comp::Y, "it", offset);
    let fps = num(fps);
    Some(format!("zoompan=z='1/({s})':x='iw*({x})':y='ih*({y})':d=1:s={w}x{h}:fps={fps}"))
}

// ---------------------------------------------------------------- checks

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneFix {
    /// Cut the zone down to the part inside the trim.
    ShrinkToTrim(usize),
    Delete(usize),
    /// Move zone `.0`'s start to zone `.1`'s end.
    StartAfter(usize, usize),
    /// Make the glide short enough for the zone.
    FitGlide(usize),
    /// Stop panning: keep the start box for the whole zone.
    NoBoxChange(usize),
}

impl ZoneFix {
    pub fn label(self) -> String {
        match self {
            ZoneFix::ShrinkToTrim(_) => "Shrink zone to fit".into(),
            ZoneFix::Delete(i) => format!("Delete zone {}", i + 1),
            ZoneFix::StartAfter(i, j) => format!("Start zone {} after zone {}", i + 1, j + 1),
            ZoneFix::FitGlide(_) => "Use a shorter glide".into(),
            ZoneFix::NoBoxChange(_) => "No box change".into(),
        }
    }
    /// Apply to the zone list. `trim` is needed for ShrinkToTrim.
    pub fn apply(self, zones: &mut Vec<Zone>, trim: Option<(f64, f64)>) {
        match self {
            ZoneFix::ShrinkToTrim(i) => {
                if let (Some(z), Some((ts, te))) = (zones.get_mut(i), trim) {
                    if let Some((s, e)) = z.times() {
                        z.start_text = timecode::format_compact(s.max(ts));
                        z.end_text = timecode::format_compact(e.min(te));
                    }
                }
            }
            ZoneFix::Delete(i) => {
                if i < zones.len() {
                    zones.remove(i);
                }
            }
            ZoneFix::StartAfter(i, j) => {
                if let Some(end) = zones.get(j).and_then(|z| z.times()).map(|t| t.1) {
                    if let Some(z) = zones.get_mut(i) {
                        z.start_text = timecode::format_compact(end);
                    }
                }
            }
            ZoneFix::NoBoxChange(i) => {
                if let Some(z) = zones.get_mut(i) {
                    z.pan = false;
                }
            }
            ZoneFix::FitGlide(i) => {
                if let Some(z) = zones.get_mut(i) {
                    if let Some((s, e)) = z.times() {
                        z.glide = fit_glide(e - s);
                    }
                }
            }
        }
    }
}

/// The longest glide (in 0.05 s steps) that fits a zone this long.
fn fit_glide(len: f64) -> f64 {
    // The small nudge stops 0.6 / 2 / 0.05 = 5.999… flooring to 5.
    ((len / 2.0) / 0.05 + 1e-6).floor() * 0.05
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZoneProblem {
    pub message: String,
    pub fixes: Vec<ZoneFix>,
}

/// Problems that block the export. `trim` is the trim range, if valid.
pub fn check(zones: &[Zone], trim: Option<(f64, f64)>) -> Vec<ZoneProblem> {
    let mut out = Vec::new();
    let mut p = |message: String, fixes: Vec<ZoneFix>| out.push(ZoneProblem { message, fixes });
    let mut ok: Vec<(usize, f64, f64)> = Vec::new();
    for (i, z) in zones.iter().enumerate() {
        let n = i + 1;
        let start = timecode::parse(&z.start_text);
        let end = timecode::parse(&z.end_text);
        let (s, e) = match (start, end) {
            (Err(err), _) => {
                p(format!("Zone {n} start: {err}."), vec![]);
                continue;
            }
            (_, Err(err)) => {
                p(format!("Zone {n} end: {err}."), vec![]);
                continue;
            }
            (Ok(s), Ok(e)) => (s, e),
        };
        if s >= e {
            p(format!("Zone {n}: start must be before end."), vec![ZoneFix::Delete(i)]);
            continue;
        }
        if z.track && z.a_drawn && z.track_points.is_empty() {
            p(format!("Zone {n} is set to Track but hasn't been tracked yet. Click Track."), vec![]);
        }
        if !z.a_drawn {
            p(format!("Zone {n} has no box yet. Click Draw box, then drag on the video."), vec![ZoneFix::Delete(i)]);
        } else if z.pan && !z.b_drawn {
            p(format!("Zone {n}'s end box isn't drawn yet. Draw it, or choose No box change."), vec![ZoneFix::NoBoxChange(i)]);
        }
        if let Some((ts, te)) = trim {
            if s < ts - 0.0005 || e > te + 0.0005 {
                let mut fixes = Vec::new();
                if s.max(ts) < e.min(te) {
                    fixes.push(ZoneFix::ShrinkToTrim(i));
                }
                fixes.push(ZoneFix::Delete(i));
                p(
                    format!(
                        "Zone {n} ({}–{}) isn't inside your trim ({}–{}).",
                        timecode::format_short(s),
                        timecode::format_short(e),
                        timecode::format_short(ts),
                        timecode::format_short(te)
                    ),
                    fixes,
                );
            }
        }
        let len = e - s;
        if z.smooth && len < 2.0 * z.glide - 1e-9 {
            let fixes = if fit_glide(len) >= 0.05 { vec![ZoneFix::FitGlide(i)] } else { vec![] };
            p(
                format!(
                    "Zone {n} is too short for its {} s glide — it needs to be at least {} s long.",
                    num(z.glide),
                    num(2.0 * z.glide)
                ),
                fixes,
            );
        } else if !z.smooth && len < MIN_HARD_LEN - 1e-9 {
            p(format!("Zone {n} is too short — make it at least {MIN_HARD_LEN} s long."), vec![ZoneFix::Delete(i)]);
        }
        ok.push((i, s, e));
    }
    ok.sort_by(|a, b| a.1.total_cmp(&b.1));
    for w in ok.windows(2) {
        let ((i, _, ie), (j, _, je)) = (w[0], w[1]);
        if w[1].1 < ie - 1e-9 {
            let fixes = if ie < je { vec![ZoneFix::StartAfter(j, i)] } else { vec![] };
            p(format!("Zones {} and {} overlap. Zones can't share time.", i + 1, j + 1), fixes);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: f64, end: f64, glide: f64, a: ZBox, b: ZBox) -> Resolved {
        Resolved { start, end, glide, a, b, path: None }
    }

    #[test]
    fn tracked_path_is_smoothed_simplified_and_matches_ffmpeg() {
        // A target moving right at a steady pace, with a little jitter.
        let pts: Vec<Point> = (0..=90).map(|k| {
            let t = 10.0 + k as f64 / 30.0;
            Point { t, x: 0.2 + 0.1 * (t - 10.0) + if k % 2 == 0 { 0.004 } else { -0.004 }, y: 0.5 }
        }).collect();
        let path = track_path(&pts, &[], 0.5, 0.4, 10.0, 13.0, MAX_TRACK_KEYS);
        assert!(path.len() <= 10, "a steady move needs few keyframes: {}", path.len());
        // Centre follows the target; the box keeps its size and stays inside.
        let mid = box_at(&path, 11.5);
        assert!((mid.x + 0.2 - 0.35).abs() < 0.02, "{mid:?}");
        assert_eq!(mid.s, 0.4);
        let zone = Resolved { start: 10.0, end: 13.0, glide: 0.3, a: box_at(&path, 10.3), b: box_at(&path, 12.7), path: Some(Arc::new(path)) };
        let segs = segments(&[zone]);
        assert!(segs.iter().any(|s| s.linear));
        for c in [Comp::X, Comp::Y, Comp::S] {
            let e = expr(&segs, c, "it", 9.0);
            let mut t = 0.0;
            while t < 5.0 {
                let want = comp(sample(&segs, t + 9.0), c);
                assert!((want - eval(&e, t)).abs() < 1e-3, "t={t}");
                t += 0.03;
            }
        }
        // Lots of wobble with no smoothing still stays under the keyframe cap.
        let wild: Vec<Point> = (0..600).map(|k| Point { t: k as f64 / 30.0, x: 0.5 + 0.2 * ((k * 7919) % 13) as f64 / 13.0 - 0.1, y: 0.5 }).collect();
        assert!(track_path(&wild, &[], 0.0, 0.3, 0.0, 20.0, MAX_TRACK_KEYS).len() <= MAX_TRACK_KEYS);
    }

    fn close(a: ZBox, b: ZBox) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.s - b.s).abs() < 1e-6
    }
    /// Evaluate an FFmpeg-style expression made by `expr`, for tests.
    fn eval(e: &str, t: f64) -> f64 {
        // Tiny evaluator for if/lt/+-*/ and parentheses.
        struct P<'a> {
            s: &'a [u8],
            i: usize,
            t: f64,
        }
        impl P<'_> {
            fn peek(&self) -> u8 {
                *self.s.get(self.i).unwrap_or(&0)
            }
            fn eat(&mut self, c: u8) {
                assert_eq!(self.peek() as char, c as char, "at {}", self.i);
                self.i += 1;
            }
            fn sum(&mut self) -> f64 {
                let mut v = self.prod();
                loop {
                    match self.peek() {
                        b'+' => {
                            self.i += 1;
                            v += self.prod()
                        }
                        b'-' => {
                            self.i += 1;
                            v -= self.prod()
                        }
                        _ => return v,
                    }
                }
            }
            fn prod(&mut self) -> f64 {
                let mut v = self.atom();
                loop {
                    match self.peek() {
                        b'*' => {
                            self.i += 1;
                            v *= self.atom()
                        }
                        b'/' => {
                            self.i += 1;
                            v /= self.atom()
                        }
                        _ => return v,
                    }
                }
            }
            fn atom(&mut self) -> f64 {
                let rest = &self.s[self.i..];
                if rest.starts_with(b"if(") {
                    self.i += 3;
                    let c = self.sum();
                    self.eat(b',');
                    let a = self.sum();
                    self.eat(b',');
                    let b = self.sum();
                    self.eat(b')');
                    return if c != 0.0 { a } else { b };
                }
                if rest.starts_with(b"lt(") {
                    self.i += 3;
                    let a = self.sum();
                    self.eat(b',');
                    let b = self.sum();
                    self.eat(b')');
                    return if a < b { 1.0 } else { 0.0 };
                }
                if rest.starts_with(b"it") {
                    self.i += 2;
                    return self.t;
                }
                if self.peek() == b'(' {
                    self.i += 1;
                    let v = self.sum();
                    self.eat(b')');
                    return v;
                }
                if self.peek() == b'-' {
                    self.i += 1;
                    return -self.atom();
                }
                let start = self.i;
                while self.peek().is_ascii_digit() || self.peek() == b'.' {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i]).unwrap().parse().unwrap()
            }
        }
        let mut p = P { s: e.as_bytes(), i: 0, t };
        let v = p.sum();
        assert_eq!(p.i, e.len());
        v
    }

    #[test]
    fn smooth_glide_inside_zone() {
        let bx = ZBox { x: 0.2, y: 0.3, s: 0.5 };
        let segs = segments(&[r(45.0, 50.0, 0.5, bx, bx)]);
        assert!(close(sample(&segs, 44.9), ZBox::FULL));
        assert!(close(sample(&segs, 45.0), ZBox::FULL));
        assert!(close(sample(&segs, 45.5), bx));
        assert!(close(sample(&segs, 49.4), bx));
        assert!(close(sample(&segs, 50.0), ZBox::FULL));
        let mid = sample(&segs, 45.25);
        assert!(mid.s < 1.0 && mid.s > 0.5);
    }

    #[test]
    fn hard_cut_and_pan() {
        let a = ZBox { x: 0.0, y: 0.0, s: 0.5 };
        let b = ZBox { x: 0.5, y: 0.5, s: 0.4 };
        let segs = segments(&[r(10.0, 14.0, 0.0, a, b)]);
        assert!(close(sample(&segs, 9.99), ZBox::FULL));
        assert!(close(sample(&segs, 10.0), a));
        assert!(close(sample(&segs, 12.0), ZBox::lerp(a, b, 0.5)));
        assert!(close(sample(&segs, 14.0), ZBox::FULL));
        // Eased: a quarter of the way through has moved less than a quarter.
        assert!(sample(&segs, 11.0).x < 0.125);
    }

    #[test]
    fn touching_zones_glide_between_boxes() {
        let a = ZBox::centered(0.5);
        let b = ZBox { x: 0.0, y: 0.0, s: 0.3 };
        let segs = segments(&[r(50.0, 55.0, 0.5, b, b), r(45.0, 50.0, 0.5, a, a)]);
        // Never fully zoomed out between the two.
        for t in [49.6, 49.9, 50.0, 50.2, 50.4] {
            assert!(sample(&segs, t).s < 0.6, "t={t}");
        }
        assert!(close(sample(&segs, 50.5), b));
        // A long gap zooms out in between.
        let segs = segments(&[r(45.0, 50.0, 0.5, a, a), r(52.0, 55.0, 0.5, b, b)]);
        assert!(close(sample(&segs, 51.0), ZBox::FULL));
    }

    #[test]
    fn ffmpeg_expression_matches_sample() {
        let a = ZBox { x: 0.1, y: 0.2, s: 0.5 };
        let b = ZBox { x: 0.4, y: 0.3, s: 0.35 };
        let zones = [r(3.0, 8.0, 0.5, a, b), r(8.2, 9.0, 0.3, b, b), r(12.0, 13.0, 0.0, a, a)];
        let segs = segments(&zones);
        let offset = 2.0; // trim start: FFmpeg's clock starts here
        for c in [Comp::X, Comp::Y, Comp::S] {
            let e = expr(&segs, c, "it", offset);
            let mut t = 0.0;
            while t < 14.0 {
                let want = comp(sample(&segs, t + offset), c);
                let got = eval(&e, t);
                assert!((want - got).abs() < 1e-3, "t={t} want {want} got {got}");
                t += 0.05;
            }
        }
    }

    #[test]
    fn zoompan_filter_shape() {
        let bx = ZBox::centered(0.5);
        let f = zoompan(&[r(1.0, 4.0, 0.5, bx, bx)], 0.0, 1920, 1080, 30.0).unwrap();
        assert!(f.starts_with("zoompan=z='1/("));
        assert!(f.ends_with(":d=1:s=1920x1080:fps=30"));
        assert!(zoompan(&[], 0.0, 1920, 1080, 30.0).is_none());
        // 20 busy zones stay well under Windows' 32,767-character command line.
        let many: Vec<Resolved> = (0..MAX_ZONES).map(|i| r(i as f64 * 3.0, i as f64 * 3.0 + 2.5, 0.5, bx, ZBox::centered(0.3))).collect();
        assert!(zoompan(&many, 0.0, 1920, 1080, 30.0).unwrap().len() < 20_000);
    }

    #[test]
    fn drawing_boxes() {
        let b = ZBox::from_drag((0.2, 0.2), (0.6, 0.4));
        assert!(close(b, ZBox { x: 0.2, y: 0.2, s: 0.4 }));
        // Dragging up-left anchors the bottom-right.
        let b = ZBox::from_drag((0.5, 0.5), (0.3, 0.4));
        assert!(close(b, ZBox { x: 0.3, y: 0.3, s: 0.2 }));
        // Never bigger than the frame, never smaller than the minimum.
        let b = ZBox::from_drag((0.8, 0.8), (2.0, 2.0));
        assert!(close(b, ZBox { x: 0.8, y: 0.8, s: 0.2 }));
        assert_eq!(ZBox::from_drag((0.5, 0.5), (0.5, 0.5)).s, MIN_SIZE);
    }

    fn zone(s: &str, e: &str) -> Zone {
        let mut z = Zone::new(0.0, 1.0);
        z.start_text = s.into();
        z.end_text = e.into();
        z.a_drawn = true;
        z
    }

    #[test]
    fn undrawn_boxes_block_export() {
        let mut z = zone("1", "3");
        z.a_drawn = false;
        assert_eq!(check(&[z.clone()], None)[0].fixes, vec![ZoneFix::Delete(0)]);
        z.a_drawn = true;
        z.pan = true;
        let mut zones = vec![z];
        assert_eq!(check(&zones, None)[0].fixes, vec![ZoneFix::NoBoxChange(0)]);
        ZoneFix::NoBoxChange(0).apply(&mut zones, None);
        assert!(check(&zones, None).is_empty());
    }

    #[test]
    fn zone_outside_trim_offers_fixes() {
        let mut zones = vec![zone("45", "50")];
        let probs = check(&zones, Some((30.0, 48.0)));
        assert_eq!(probs.len(), 1);
        assert_eq!(probs[0].fixes, vec![ZoneFix::ShrinkToTrim(0), ZoneFix::Delete(0)]);
        ZoneFix::ShrinkToTrim(0).apply(&mut zones, Some((30.0, 48.0)));
        assert_eq!(zones[0].times(), Some((45.0, 48.0)));
        assert!(check(&zones, Some((30.0, 48.0))).is_empty());
        // Entirely outside: only delete.
        let probs = check(&[zone("50", "55")], Some((30.0, 48.0)));
        assert_eq!(probs[0].fixes, vec![ZoneFix::Delete(0)]);
    }

    #[test]
    fn overlaps_and_short_zones() {
        let mut zones = vec![zone("10", "15"), zone("14", "20")];
        let probs = check(&zones, None);
        assert_eq!(probs.len(), 1);
        assert_eq!(probs[0].fixes, vec![ZoneFix::StartAfter(1, 0)]);
        ZoneFix::StartAfter(1, 0).apply(&mut zones, None);
        assert!(check(&zones, None).is_empty(), "touching is fine");

        let mut zones = vec![zone("10", "10.6")];
        let probs = check(&zones, None);
        assert_eq!(probs[0].fixes, vec![ZoneFix::FitGlide(0)]);
        ZoneFix::FitGlide(0).apply(&mut zones, None);
        assert!((zones[0].glide - 0.3).abs() < 1e-9);
        assert!(check(&zones, None).is_empty());

        let mut hard = zone("10", "10.05");
        hard.smooth = false;
        assert_eq!(check(&[hard], None).len(), 1);
        assert_eq!(check(&[zone("x", "2")], None)[0].fixes, vec![]);
    }
}

impl Default for Zone {
    fn default() -> Self {
        Zone::new(0.0, 0.0)
    }
}
