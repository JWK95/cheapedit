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

/// Simple: one still box. Precise: the camera moves (your points, or tracking).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    #[default]
    Simple,
    Precise,
}

/// A Precise point after the first: at the time in `text`, the view is `b`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Key {
    pub text: String,
    pub b: ZBox,
}

impl Default for Key {
    fn default() -> Self {
        Key { text: String::new(), b: ZBox::centered(0.5) }
    }
}

impl Key {
    pub fn t(&self) -> Option<f64> {
        timecode::parse(&self.text).ok()
    }
}

/// Most points one Precise zone may have (the first included).
pub const MAX_POINTS: usize = 20;
/// Two points closer together than this count as the same time.
pub const SAME_TIME: f64 = 0.01;

/// One zone as the user edits it (times are text, like the trim boxes).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Zone {
    pub start_text: String,
    pub end_text: String,
    pub smooth: bool,
    pub glide: f64,
    pub mode: Mode,
    /// Precise: follow what's in the first box (tracked by `tracker`)
    /// instead of moving through `points`.
    pub follow: bool,
    /// The box: Simple's box, and Precise's point 1 (at the zone start).
    pub a: ZBox,
    /// Whether the user has drawn the box yet.
    pub a_drawn: bool,
    /// Precise points after the first, in time order.
    pub points: Vec<Key>,
    /// 0 = follow every wobble, 1 = a slow, steady glide.
    pub smoothness: f64,
    /// Where the target's centre was found, over time (original time).
    pub track_points: Vec<Point>,
    /// Places the user redrew the box; tracking restarts from each.
    pub locks: Vec<Lock>,
    /// Follow automatically: a loop drawn around just the thing to follow,
    /// inside the box (fractions of the box). Empty = follow the whole box.
    pub lasso: Vec<[f64; 2]>,
    // v0.6 montages, read by `upgrade` and never written.
    #[serde(rename = "pan", skip_serializing)]
    old_pan: bool,
    #[serde(rename = "b", skip_serializing)]
    old_b: Option<ZBox>,
    #[serde(rename = "b_drawn", skip_serializing)]
    old_b_drawn: bool,
    #[serde(rename = "track", skip_serializing)]
    old_track: bool,
}

/// A fix: at time `t` the view is box `b`, with the target in the loop
/// (fractions of `b`; empty = the loop from before, or the whole box).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Lock {
    pub t: f64,
    pub b: ZBox,
    #[serde(default)]
    pub lasso: Vec<[f64; 2]>,
}

/// The rectangle around a loop drawn in box `b` (frame fractions x, y, w, h).
/// No loop = the box itself.
pub fn loop_rect(lasso: &[[f64; 2]], b: ZBox) -> (f64, f64, f64, f64) {
    if lasso.is_empty() {
        return (b.x, b.y, b.s, b.s);
    }
    let (mut u0, mut v0, mut u1, mut v1) = (1.0f64, 1.0f64, 0.0f64, 0.0f64);
    for p in lasso {
        u0 = u0.min(p[0]);
        v0 = v0.min(p[1]);
        u1 = u1.max(p[0]);
        v1 = v1.max(p[1]);
    }
    (b.x + u0 * b.s, b.y + v0 * b.s, (u1 - u0).max(0.0) * b.s, (v1 - v0).max(0.0) * b.s)
}

/// How far the box's centre is from its loop's centre: the zoom keeps this
/// framing while the loop's target moves.
fn loop_offset(lasso: &[[f64; 2]], b: ZBox) -> (f64, f64) {
    let (x, y, w, h) = loop_rect(lasso, b);
    (b.x + b.s / 2.0 - (x + w / 2.0), b.y + b.s / 2.0 - (y + h / 2.0))
}

/// Smallest loop the tracker can follow (fraction of the frame, each way).
pub const MIN_LOOP: f64 = 0.03;

/// Turn a freehand stroke (frame fractions) into a loop inside box `b`:
/// cut off at the box's edges, with points closer than a hair dropped.
/// Err says why it can't be used.
pub fn make_loop(stroke: &[(f64, f64)], b: ZBox) -> Result<Vec<[f64; 2]>, &'static str> {
    if stroke.len() < 3 {
        return Err("That was a click, not a loop. Hold the mouse button down and draw around the thing to follow.");
    }
    let inside = stroke.iter().filter(|p| p.0 >= b.x && p.0 <= b.x + b.s && p.1 >= b.y && p.1 <= b.y + b.s).count();
    if inside * 2 < stroke.len() {
        return Err("Most of that loop was outside the box. Draw it inside the coloured box, around the thing to follow.");
    }
    let mut out: Vec<[f64; 2]> = Vec::new();
    for &(x, y) in stroke {
        let p = [((x - b.x) / b.s).clamp(0.0, 1.0), ((y - b.y) / b.s).clamp(0.0, 1.0)];
        if out.last().is_none_or(|q| (q[0] - p[0]).abs().max((q[1] - p[1]).abs()) > 0.004) {
            out.push(p);
        }
    }
    // Keep it light to save: at most 200 points.
    if out.len() > 200 {
        let step = out.len().div_ceil(200);
        out = out.into_iter().step_by(step).collect();
    }
    let (_, _, w, h) = loop_rect(&out, b);
    if out.len() < 3 || w < MIN_LOOP || h < MIN_LOOP {
        return Err(if w < MIN_LOOP / 3.0 || h < MIN_LOOP / 3.0 {
            "That was a line, not a loop. Draw all the way around the thing to follow."
        } else {
            "That loop is too small to follow. Draw it a bit bigger, around the whole thing."
        });
    }
    Ok(out)
}

impl Zone {
    pub fn new(start: f64, end: f64) -> Zone {
        Zone {
            start_text: timecode::format_compact(start),
            end_text: timecode::format_compact(end),
            smooth: true,
            glide: DEFAULT_GLIDE,
            mode: Mode::Simple,
            follow: false,
            a: ZBox::centered(0.5),
            a_drawn: false,
            points: Vec::new(),
            smoothness: 0.5,
            track_points: Vec::new(),
            locks: Vec::new(),
            lasso: Vec::new(),
            old_pan: false,
            old_b: None,
            old_b_drawn: false,
            old_track: false,
        }
    }
    /// A zone saved by v0.6: a pan becomes Precise with two points (the
    /// second one glide before the end, so it moves as before) and a
    /// tracked zone becomes Precise, Follow automatically.
    pub fn upgrade(&mut self) {
        if self.old_track {
            self.mode = Mode::Precise;
            self.follow = true;
        } else if let (true, true, Some(b), Some((_, e))) = (self.old_pan, self.old_b_drawn, self.old_b, self.times()) {
            self.mode = Mode::Precise;
            self.points = vec![Key { text: timecode::format_compact(e - self.glide_used()), b }];
        }
        self.old_pan = false;
        self.old_b = None;
        self.old_b_drawn = false;
        self.old_track = false;
    }
    /// Precise, following what's in the box.
    pub fn follows(&self) -> bool {
        self.mode == Mode::Precise && self.follow
    }
    /// Precise, moving through the user's points.
    pub fn uses_points(&self) -> bool {
        self.mode == Mode::Precise && !self.follow
    }
    /// Following, with a path to follow.
    pub fn tracked(&self) -> bool {
        self.follows() && !self.track_points.is_empty()
    }
    pub fn times(&self) -> Option<(f64, f64)> {
        Some((timecode::parse(&self.start_text).ok()?, timecode::parse(&self.end_text).ok()?))
    }
    /// The loop in use at `t` (a fix can redraw it; otherwise it carries on),
    /// relative to its box.
    pub fn loop_at(&self, t: f64) -> &[[f64; 2]] {
        let mut cur: &[[f64; 2]] = &self.lasso;
        for l in self.locks.iter().filter(|l| l.t <= t + 1e-6) {
            if !l.lasso.is_empty() {
                cur = &l.lasso;
            }
        }
        cur
    }
    /// Where tracking starts from the zone start (`None`) or fix `k`: the
    /// rectangle around the loop, frame fractions x, y, w, h.
    pub fn start_rect(&self, k: Option<usize>) -> (f64, f64, f64, f64) {
        match k.and_then(|k| self.locks.get(k)) {
            Some(l) => loop_rect(self.loop_at(l.t), l.b),
            None => loop_rect(&self.lasso, self.a),
        }
    }
    /// The framing kept around the target, from the start and from each fix:
    /// (from time, box centre minus loop centre).
    fn track_offsets(&self) -> Vec<(f64, (f64, f64))> {
        let mut out = vec![(f64::NEG_INFINITY, loop_offset(&self.lasso, self.a))];
        out.extend(self.locks.iter().map(|l| (l.t, loop_offset(self.loop_at(l.t), l.b))));
        out
    }
    pub fn glide_used(&self) -> f64 {
        if self.smooth { self.glide } else { 0.0 }
    }
    /// Point `k` (0 = the first, at the zone start): its time and box.
    pub fn point(&self, k: usize) -> Option<(f64, ZBox)> {
        if k == 0 {
            return Some((self.times()?.0, self.a));
        }
        let p = self.points.get(k - 1)?;
        Some((p.t()?, p.b))
    }
    /// Why each point after the first isn't used (None = used).
    pub fn point_problems(&self) -> Vec<Option<String>> {
        let times = self.times();
        let g = self.glide_used();
        let mut used: Vec<(usize, f64)> = Vec::new();
        self.points
            .iter()
            .enumerate()
            .map(|(j, p)| {
                let t = match timecode::parse(&p.text) {
                    Ok(t) => t,
                    Err(e) => return Some(format!("Time: {e}.")),
                };
                let (s, e) = times?;
                let why = if t <= s || t > e + 0.001 {
                    Some("Outside the zone, so not used.".to_string())
                } else if t <= s + g + SAME_TIME {
                    Some("Inside the glide in, so not used. Move it later.".to_string())
                } else if t > e - g + 0.001 {
                    Some("Inside the glide out, so not used. Move it earlier.".to_string())
                } else {
                    used.iter().find(|u| (u.1 - t).abs() < SAME_TIME).map(|u| format!("Same time as point {}, so not used.", u.0 + 2))
                };
                if why.is_none() {
                    used.push((j, t));
                }
                why
            })
            .collect()
    }
    /// The points the camera moves through: point 1 one glide after the
    /// start, then every usable point, in time order.
    fn keys(&self, start: f64, glide: f64) -> Vec<(f64, ZBox)> {
        let mut k = vec![(start + glide, self.a)];
        for (p, why) in self.points.iter().zip(self.point_problems()) {
            if let (None, Some(t)) = (why, p.t()) {
                k.push((t, p.b));
            }
        }
        k.sort_by(|a, b| a.0.total_cmp(&b.0));
        k
    }
    /// Add a point at `t` with box `b`; returns its number (1 = the second point).
    pub fn add_point(&mut self, t: f64, b: ZBox) -> usize {
        self.points.push(Key { text: timecode::format_compact(t), b });
        self.sort_points(self.points.len())
    }
    /// Put the points in time order (unreadable times last). Returns where
    /// point `keep` (0 = the first, which never moves) ended up.
    pub fn sort_points(&mut self, keep: usize) -> usize {
        let mut idx: Vec<usize> = (0..self.points.len()).collect();
        idx.sort_by(|&a, &b| self.points[a].t().unwrap_or(f64::INFINITY).total_cmp(&self.points[b].t().unwrap_or(f64::INFINITY)));
        let new_keep = if keep == 0 { 0 } else { idx.iter().position(|&i| i + 1 == keep).map_or(0, |p| p + 1) };
        self.points = idx.iter().map(|&i| self.points[i].clone()).collect();
        new_keep
    }
    pub fn resolve(&self) -> Option<Resolved> {
        let (start, end) = self.times()?;
        let glide = self.glide_used();
        let still = Resolved { start, end, glide, a: self.a, b: self.a, path: None };
        let path = if self.tracked() {
            track_path(&self.track_points, &self.locks, &self.track_offsets(), self.smoothness, self.a.s, start, end, MAX_TRACK_KEYS)
        } else if self.uses_points() {
            let keys = self.keys(start, glide);
            if keys.len() < 2 {
                return Some(still);
            }
            curve_path(&keys, MAX_TRACK_KEYS)
        } else {
            Vec::new()
        };
        if path.is_empty() {
            return Some(still);
        }
        let a = box_at(&path, start + glide);
        let b = box_at(&path, end - glide);
        Some(Resolved { start, end, glide, a, b, path: Some(Arc::new(path)) })
    }
    /// The strongest enlargement this zone uses.
    pub fn max_zoom(&self) -> f64 {
        let mut z = self.a.zoom();
        if self.uses_points() {
            for (p, why) in self.points.iter().zip(self.point_problems()) {
                if why.is_none() {
                    z = z.max(p.b.zoom());
                }
            }
        }
        z
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
/// within `tol` of every point (Ramer-Douglas-Peucker on position and size over time).
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
            let d = (e.x - p.1.x).abs().max((e.y - p.1.y).abs()).max((e.s - p.1.s).abs());
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
/// `offsets` (from time, x/y) shift the box off the target to keep the
/// framing a loop set up; the last one starting at or before a point applies.
#[allow(clippy::too_many_arguments)]
pub fn track_path(points: &[Point], locks: &[Lock], offsets: &[(f64, (f64, f64))], smoothness: f64, s: f64, start: f64, end: f64, max_keys: usize) -> Vec<(f64, ZBox)> {
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
    let off = |t: f64| offsets.iter().rev().find(|o| o.0 <= t + 1e-6).map_or((0.0, 0.0), |o| o.1);
    let boxes: Vec<(f64, ZBox)> = smoothed
        .iter()
        .map(|p| {
            let (dx, dy) = off(p.t);
            (p.t, ZBox { x: p.x + dx - s / 2.0, y: p.y + dy - s / 2.0, s }.clamped())
        })
        .collect();
    let mut tol = 0.002;
    loop {
        let out = simplify(&boxes, tol);
        if out.len() <= max_keys.max(2) {
            return out;
        }
        tol *= 1.5;
    }
}

/// A flowing camera path through `keys` (time, box): a cubic curve that
/// passes through every key without stopping, easing only at the first and
/// last (Catmull-Rom tangents, timed by the keys). Sampled and cut down to at
/// most `max_keys` straight-line keyframes, like a tracked path.
pub fn curve_path(keys: &[(f64, ZBox)], max_keys: usize) -> Vec<(f64, ZBox)> {
    let n = keys.len();
    if n < 2 {
        return keys.to_vec();
    }
    let v = |b: ZBox| [b.x, b.y, b.s];
    let tangent = |i: usize| -> [f64; 3] {
        if i == 0 || i == n - 1 {
            return [0.0; 3];
        }
        let (p, q) = (v(keys[i - 1].1), v(keys[i + 1].1));
        let dt = (keys[i + 1].0 - keys[i - 1].0).max(1e-9);
        [(q[0] - p[0]) / dt, (q[1] - p[1]) / dt, (q[2] - p[2]) / dt]
    };
    let mut pts = vec![keys[0]];
    for i in 0..n - 1 {
        let (t0, h) = (keys[i].0, keys[i + 1].0 - keys[i].0);
        let (p0, p1, m0, m1) = (v(keys[i].1), v(keys[i + 1].1), tangent(i), tangent(i + 1));
        let steps = ((h * 30.0).ceil() as usize).max(2);
        for k in 1..=steps {
            let u = k as f64 / steps as f64;
            let (u2, u3) = (u * u, u * u * u);
            let (h00, h10, h01, h11) = (2.0 * u3 - 3.0 * u2 + 1.0, u3 - 2.0 * u2 + u, 3.0 * u2 - 2.0 * u3, u3 - u2);
            let c = |j: usize| h00 * p0[j] + h10 * h * m0[j] + h01 * p1[j] + h11 * h * m1[j];
            let b = if k == steps { keys[i + 1].1 } else { ZBox { x: c(0), y: c(1), s: c(2) }.clamped() };
            pts.push((t0 + h * u, b));
        }
    }
    let mut tol = 0.002;
    loop {
        let out = simplify(&pts, tol);
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
}

impl ZoneFix {
    pub fn label(self) -> String {
        match self {
            ZoneFix::ShrinkToTrim(_) => "Shrink zone to fit".into(),
            ZoneFix::Delete(i) => format!("Delete zone {}", i + 1),
            ZoneFix::StartAfter(i, j) => format!("Start zone {} after zone {}", i + 1, j + 1),
            ZoneFix::FitGlide(_) => "Use a shorter glide".into(),
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
        if z.follows() && z.a_drawn && z.track_points.is_empty() {
            p(format!("Zone {n} is set to Follow automatically but hasn't been tracked yet. Click Track."), vec![]);
        }
        if !z.a_drawn {
            p(format!("Zone {n} has no box yet. Click Draw box, then drag on the video."), vec![ZoneFix::Delete(i)]);
        }
        if z.uses_points() {
            for (k, key) in z.points.iter().enumerate() {
                if let Err(err) = timecode::parse(&key.text) {
                    p(format!("Zone {n} point {}: {err}.", k + 2), vec![]);
                }
            }
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
        let path = track_path(&pts, &[], &[], 0.5, 0.4, 10.0, 13.0, MAX_TRACK_KEYS);
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
        assert!(track_path(&wild, &[], &[], 0.0, 0.3, 0.0, 20.0, MAX_TRACK_KEYS).len() <= MAX_TRACK_KEYS);
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
    fn undrawn_boxes_and_bad_point_times_block_export() {
        let mut z = zone("1", "3");
        z.a_drawn = false;
        assert_eq!(check(&[z.clone()], None)[0].fixes, vec![ZoneFix::Delete(0)]);
        z.a_drawn = true;
        z.mode = Mode::Precise;
        z.points.push(Key { text: "x".into(), b: ZBox::centered(0.4) });
        assert_eq!(check(&[z.clone()], None).len(), 1);
        // Hidden while Simple is showing.
        z.mode = Mode::Simple;
        assert!(check(&[z], None).is_empty());
    }

    #[test]
    fn points_flow_through_every_point_and_match_ffmpeg() {
        let mut z = zone("10", "20");
        z.mode = Mode::Precise;
        let (p1, p2, p3) = (ZBox { x: 0.1, y: 0.1, s: 0.5 }, ZBox { x: 0.4, y: 0.2, s: 0.4 }, ZBox { x: 0.2, y: 0.5, s: 0.3 });
        z.a = p1;
        // Added out of order: kept in time order.
        assert_eq!(z.add_point(16.0, p3), 1);
        assert_eq!(z.add_point(13.0, p2), 1);
        assert_eq!(z.points[0].text, "13");
        let r = z.resolve().unwrap();
        let path = r.path.clone().unwrap();
        assert!(path.len() <= MAX_TRACK_KEYS);
        let near = |a: ZBox, b: ZBox| (a.x - b.x).abs() < 0.003 && (a.y - b.y).abs() < 0.003 && (a.s - b.s).abs() < 0.003;
        let segs = segments(&[r]);
        assert!(near(sample(&segs, 10.5), p1), "point 1, one glide in");
        assert!(near(sample(&segs, 13.0), p2), "{:?}", sample(&segs, 13.0));
        assert!(near(sample(&segs, 16.0), p3));
        assert!(near(sample(&segs, 18.0), p3), "holds after the last point");
        // Doesn't stop at a middle point: still moving just either side of it.
        let (before, after) = (sample(&segs, 12.9), sample(&segs, 13.1));
        assert!(after.y - before.y > 0.005, "{before:?} {after:?}");
        for c in [Comp::X, Comp::Y, Comp::S] {
            let e = expr(&segs, c, "it", 10.0);
            let mut t = 0.0;
            while t < 11.0 {
                assert!((comp(sample(&segs, t + 10.0), c) - eval(&e, t)).abs() < 1e-3, "t={t}");
                t += 0.04;
            }
        }
        assert!((z.max_zoom() - 1.0 / 0.3).abs() < 1e-9);
    }

    #[test]
    fn points_that_cant_be_used_are_kept_and_skipped() {
        let mut z = zone("10", "20");
        z.mode = Mode::Precise;
        let b = ZBox::centered(0.2);
        for t in ["9", "10.3", "19.8", "15", "15.001", "zz"] {
            z.points.push(Key { text: t.into(), b });
        }
        let why = z.point_problems();
        assert!(why[0].as_ref().unwrap().starts_with("Outside"));
        assert!(why[1].as_ref().unwrap().contains("glide in"));
        assert!(why[2].as_ref().unwrap().contains("glide out"));
        assert_eq!(why[3], None);
        assert!(why[4].as_ref().unwrap().contains("point 5"));
        assert!(why[5].is_some());
        // Only point 1 and the one at 15 are used: still a moving path.
        assert!(z.resolve().unwrap().path.is_some());
        // Simple ignores them all.
        z.mode = Mode::Simple;
        assert!(z.resolve().unwrap().path.is_none());
        // Follow automatically without a track yet: a still box.
        z.mode = Mode::Precise;
        z.follow = true;
        assert!(z.resolve().unwrap().path.is_none());
    }

    #[test]
    fn loop_is_cut_to_the_box_and_must_be_big_enough() {
        let b = ZBox { x: 0.2, y: 0.2, s: 0.4 };
        // A square loop poking out past the right edge of the box.
        let stroke = [(0.3, 0.3), (0.7, 0.3), (0.7, 0.4), (0.3, 0.4), (0.3, 0.3)];
        let l = make_loop(&stroke, b).unwrap();
        assert!(l.iter().all(|p| p[0] <= 1.0 && p[1] <= 1.0));
        let (x, y, w, h) = loop_rect(&l, b);
        assert!((x - 0.3).abs() < 1e-9 && (y - 0.3).abs() < 1e-9);
        assert!((x + w - 0.6).abs() < 1e-9, "cut at the box edge: {}", x + w);
        assert!((h - 0.1).abs() < 1e-9);
        // Each way it can fail says why.
        assert!(make_loop(&[(0.3, 0.3), (0.32, 0.3), (0.32, 0.32), (0.3, 0.32)], b).unwrap_err().contains("too small"));
        assert!(make_loop(&[(0.3, 0.3)], b).unwrap_err().contains("click"));
        assert!(make_loop(&[(0.3, 0.3), (0.4, 0.3), (0.5, 0.3), (0.55, 0.3)], b).unwrap_err().contains("line"));
        assert!(make_loop(&[(0.7, 0.7), (0.9, 0.7), (0.9, 0.9), (0.3, 0.3)], b).unwrap_err().contains("outside"));
        // No loop: the box itself.
        assert_eq!(loop_rect(&[], b), (0.2, 0.2, 0.4, 0.4));
    }

    #[test]
    fn loop_keeps_the_framing_around_the_target() {
        let mut z = zone("0", "4");
        z.mode = Mode::Precise;
        z.follow = true;
        z.smooth = false;
        z.a = ZBox { x: 0.1, y: 0.1, s: 0.5 };
        // The target is a small thing in the top-left quarter of the box.
        z.lasso = vec![[0.1, 0.1], [0.3, 0.1], [0.3, 0.3], [0.1, 0.3]];
        let (x, y, w, h) = z.start_rect(None);
        assert!((x - 0.15).abs() < 1e-9 && (y - 0.15).abs() < 1e-9 && (w - 0.1).abs() < 1e-9 && (h - 0.1).abs() < 1e-9);
        // It moves right; the box follows with the same framing (box centre
        // 0.15 right and below the target's centre).
        z.track_points = (0..=120).map(|k| Point { t: k as f64 / 30.0, x: 0.2 + 0.05 * k as f64 / 30.0, y: 0.2 }).collect();
        z.smoothness = 0.0;
        let path = z.resolve().unwrap().path.unwrap();
        let at = box_at(&path, 2.0);
        assert!((at.x + 0.25 - (0.3 + 0.15)).abs() < 0.004, "{at:?}");
        assert!((at.y + 0.25 - 0.35).abs() < 0.004, "{at:?}");
        // A fix with a new loop (the target turned): from then on its framing applies.
        z.locks.push(Lock { t: 3.0, b: ZBox { x: 0.2, y: 0.1, s: 0.5 }, lasso: vec![[0.4, 0.4], [0.6, 0.4], [0.6, 0.6], [0.4, 0.6]] });
        assert_eq!(z.loop_at(1.0), &z.lasso[..]);
        assert_eq!(z.loop_at(3.5)[0], [0.4, 0.4]);
        let path = z.resolve().unwrap().path.unwrap();
        let after = box_at(&path, 3.6);
        // Loop centred in the box: the box centres on the target.
        assert!((after.x + 0.25 - (0.2 + 0.05 * 3.6)).abs() < 0.004, "{after:?}");
        // A fix without a loop keeps the loop from before.
        z.locks[0].lasso.clear();
        assert_eq!(z.loop_at(3.5), &z.lasso[..]);
        // Old fixes (no loop saved) still load.
        let l: Lock = serde_json::from_str(r#"{"t":1.0,"b":{"x":0.1,"y":0.1,"s":0.5}}"#).unwrap();
        assert!(l.lasso.is_empty());
    }

    #[test]
    fn two_points_move_like_the_old_pan() {
        let (a, b) = (ZBox { x: 0.0, y: 0.0, s: 0.5 }, ZBox { x: 0.5, y: 0.5, s: 0.4 });
        let old = segments(&[r(10.0, 14.0, 0.5, a, b)]);
        let json = r#"{"start_text":"10","end_text":"14","pan":true,"a":{"x":0.0,"y":0.0,"s":0.5},"b":{"x":0.5,"y":0.5,"s":0.4},"a_drawn":true,"b_drawn":true}"#;
        let mut z: Zone = serde_json::from_str(json).unwrap();
        z.upgrade();
        assert_eq!(z.mode, Mode::Precise);
        assert_eq!(z.points[0].text, "13.5");
        let new = segments(&[z.resolve().unwrap()]);
        let mut t = 9.5;
        while t < 14.5 {
            let (o, n) = (sample(&old, t), sample(&new, t));
            assert!((o.x - n.x).abs() < 0.003 && (o.s - n.s).abs() < 0.003, "t={t}");
            t += 0.05;
        }
        // Saving drops the old fields.
        assert!(!serde_json::to_string(&z).unwrap().contains("\"pan\""));
        // A v0.6 tracked zone becomes Follow automatically.
        let mut t: Zone = serde_json::from_str(r#"{"start_text":"1","end_text":"3","track":true,"a_drawn":true}"#).unwrap();
        t.upgrade();
        assert!(t.follows());
        // A plain one stays Simple.
        let mut s: Zone = serde_json::from_str(r#"{"start_text":"1","end_text":"3","a_drawn":true,"b_drawn":false}"#).unwrap();
        s.upgrade();
        assert_eq!(s.mode, Mode::Simple);
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
