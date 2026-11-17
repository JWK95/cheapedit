//! Speed zones: slow motion or fast forward between two times.
//!
//! The export splits the trimmed clip at the zone edges, gives each piece its
//! own speed, and joins the pieces again. `TimeMap` converts between
//! original-video time and finished-video time, for the timeline and the
//! "= in final video" hints.
//!
//! Pure (no I/O), so all of it is unit-tested.

use crate::timecode;

pub const MAX_ZONES: usize = 20;
pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 4.0;
/// Shortest zone allowed.
pub const MIN_LEN: f64 = 0.1;

/// One zone as the user edits it (times are text, like the trim boxes).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpeedZone {
    pub start_text: String,
    pub end_text: String,
    /// Covers the whole trim, and follows it when the trim changes.
    pub whole: bool,
    pub speed: f64,
    /// Make in-between frames (minterpolate) so slow motion isn't choppy.
    pub smooth: bool,
    /// Silence the audio for this zone.
    pub mute: bool,
}

impl SpeedZone {
    pub fn new(start: f64, end: f64) -> SpeedZone {
        SpeedZone {
            start_text: timecode::format_compact(start),
            end_text: timecode::format_compact(end),
            whole: false,
            speed: 0.5,
            smooth: false,
            mute: false,
        }
    }
    pub fn whole_clip() -> SpeedZone {
        SpeedZone { whole: true, speed: 2.0, ..SpeedZone::new(0.0, 0.0) }
    }
    /// The zone's times; a whole-clip zone takes the trim's.
    pub fn times(&self, trim: Option<(f64, f64)>) -> Option<(f64, f64)> {
        if self.whole {
            return trim;
        }
        Some((timecode::parse(&self.start_text).ok()?, timecode::parse(&self.end_text).ok()?))
    }
    pub fn resolve(&self, trim: Option<(f64, f64)>) -> Option<Resolved> {
        let (start, end) = self.times(trim)?;
        (end > start).then_some(Resolved { start, end, speed: self.speed.clamp(MIN_SPEED, MAX_SPEED), smooth: self.smooth && self.speed < 1.0, mute: self.mute })
    }
    /// "0.5x", "2x", "1.25x".
    pub fn label(&self) -> String {
        format!("{}x", num(self.speed))
    }
}

/// A zone with its times worked out (original-video seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resolved {
    pub start: f64,
    pub end: f64,
    pub speed: f64,
    pub smooth: bool,
    pub mute: bool,
}

/// Converts between original-video time and finished-video time.
/// Built from zones that don't overlap (overlapping ones are skipped, so the
/// map always runs forwards; the export is blocked until they're fixed).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TimeMap {
    zones: Vec<Resolved>,
}

impl TimeMap {
    pub fn new(zones: &[Resolved]) -> TimeMap {
        let mut zs: Vec<Resolved> = zones.iter().copied().filter(|z| z.end > z.start && z.speed > 0.0).collect();
        zs.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut out: Vec<Resolved> = Vec::new();
        for z in zs {
            if out.last().is_none_or(|p| z.start >= p.end - 1e-9) {
                out.push(z);
            }
        }
        TimeMap { zones: out }
    }
    pub fn is_identity(&self) -> bool {
        self.zones.iter().all(|z| z.speed == 1.0)
    }
    /// Where original time `t` lands, counting the whole source from 0.
    pub fn to_out(&self, t: f64) -> f64 {
        let mut u = t;
        for z in &self.zones {
            let inside = (t - z.start).clamp(0.0, z.end - z.start);
            u += inside * (1.0 / z.speed - 1.0);
        }
        u
    }
    /// The original time that lands at finished time `u` (the inverse of `to_out`).
    pub fn to_src(&self, u: f64) -> f64 {
        let mut shift = 0.0; // to_out(t) - t, for t before the current zone
        for z in &self.zones {
            let out_start = z.start + shift;
            if u < out_start {
                return u - shift;
            }
            let out_end = out_start + (z.end - z.start) / z.speed;
            if u < out_end {
                return z.start + (u - out_start) * z.speed;
            }
            shift += (z.end - z.start) * (1.0 / z.speed - 1.0);
        }
        u - shift
    }
    /// Finished length of the stretch from `start` to `end`.
    pub fn out_len(&self, start: f64, end: f64) -> f64 {
        (self.to_out(end) - self.to_out(start)).max(0.0)
    }
    /// The zone at original time `t`, if any.
    pub fn zone_at(&self, t: f64) -> Option<&Resolved> {
        self.zones.iter().find(|z| t >= z.start && t < z.end)
    }
    pub fn zones(&self) -> &[Resolved] {
        &self.zones
    }
}

/// One stretch of the exported clip at one speed. Times are seconds from the
/// trim start (the export's clock); `end` is None for "to the end".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub start: f64,
    pub end: Option<f64>,
    pub speed: f64,
    pub smooth: bool,
    pub mute: bool,
}

impl Piece {
    fn plain(&self) -> bool {
        self.speed == 1.0 && !self.mute
    }
}

/// The trim from `start` to `end`, cut into pieces at the zone edges.
/// Gaps between zones are 1x pieces.
pub fn pieces(map: &TimeMap, start: f64, end: f64) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut at = start;
    let mut push = |a: f64, b: f64, z: Option<&Resolved>| {
        if b - a > 1e-3 {
            out.push(Piece {
                start: a - start,
                end: Some(b - start),
                speed: z.map_or(1.0, |z| z.speed),
                smooth: z.is_some_and(|z| z.smooth),
                mute: z.is_some_and(|z| z.mute),
            });
        }
    };
    for z in map.zones() {
        let (zs, ze) = (z.start.max(start), z.end.min(end));
        if ze <= zs {
            continue;
        }
        push(at, zs, None);
        push(zs, ze, Some(z));
        at = ze;
    }
    push(at, end, None);
    if let Some(last) = out.last_mut() {
        last.end = None;
    }
    out
}

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.to_string() }
}

/// The video filters for one piece's speed (after its timestamps start at 0).
fn video_speed(p: &Piece, fps: f64) -> String {
    let mut f = if p.speed == 1.0 { "setpts=PTS-STARTPTS".to_string() } else { format!("setpts=(PTS-STARTPTS)/{}", num(p.speed)) };
    if p.smooth {
        f.push_str(&format!(",minterpolate=fps={}:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1", num(fps)));
    }
    f
}

fn trim_args(p: &Piece) -> String {
    match p.end {
        Some(e) => format!("start={}:end={}", num(p.start), num(e)),
        None => format!("start={}", num(p.start)),
    }
}

/// Video filters that apply the speeds, for a simple (`-vf`) filter chain:
/// one input, one output. None when nothing changes speed. `fps` is the
/// frame rate smooth slow motion fills up to.
pub fn video_chain(pieces: &[Piece], fps: f64) -> Option<String> {
    if pieces.iter().all(|p| p.speed == 1.0) {
        return None;
    }
    if let [p] = pieces {
        // One piece covering the whole clip: no split needed.
        let mut f = format!("setpts=PTS/{}", num(p.speed));
        if p.smooth {
            f.push_str(&format!(",minterpolate=fps={}:mi_mode=mci:mc_mode=aobmc:me_mode=bidir:vsbmc=1", num(fps)));
        }
        return Some(f);
    }
    let n = pieces.len();
    let mut parts = Vec::new();
    let outs: String = (0..n).map(|i| format!("[vs{i}]")).collect();
    parts.push(format!("split={n}{outs}"));
    for (i, p) in pieces.iter().enumerate() {
        parts.push(format!("[vs{i}]trim={},{}[vp{i}]", trim_args(p), video_speed(p, fps)));
    }
    let ins: String = (0..n).map(|i| format!("[vp{i}]")).collect();
    parts.push(format!("{ins}concat=n={n}:v=1:a=0"));
    Some(parts.join(";"))
}

/// `atempo` only accepts 0.5–100, so slower speeds are chained (0.25 = 0.5 × 0.5).
pub fn atempo_chain(speed: f64) -> Vec<String> {
    let mut out = Vec::new();
    let mut r = speed;
    while r < 0.5 {
        out.push("atempo=0.5".to_string());
        r /= 0.5;
    }
    out.push(format!("atempo={}", num(r)));
    out
}

fn audio_speed(p: &Piece, change_pitch: bool, sample_rate: u32) -> Vec<String> {
    let mut f = Vec::new();
    if p.speed != 1.0 {
        if change_pitch {
            f.push(format!("asetrate={},aresample={sample_rate}", (sample_rate as f64 * p.speed).round() as u32));
        } else {
            f.extend(atempo_chain(p.speed));
        }
    }
    if p.mute {
        f.push("volume=0".into());
    }
    f
}

/// Audio filters that apply the speeds (and muted zones), as one item of a
/// simple filter chain. None when nothing changes.
pub fn audio_chain(pieces: &[Piece], change_pitch: bool, sample_rate: u32) -> Option<String> {
    if pieces.iter().all(Piece::plain) {
        return None;
    }
    if let [p] = pieces {
        return Some(audio_speed(p, change_pitch, sample_rate).join(","));
    }
    let n = pieces.len();
    let mut parts = Vec::new();
    let outs: String = (0..n).map(|i| format!("[as{i}]")).collect();
    parts.push(format!("asplit={n}{outs}"));
    for (i, p) in pieces.iter().enumerate() {
        let mut f = vec![format!("atrim={}", trim_args(p)), "asetpts=PTS-STARTPTS".into()];
        f.extend(audio_speed(p, change_pitch, sample_rate));
        parts.push(format!("[as{i}]{}[ap{i}]", f.join(",")));
    }
    let ins: String = (0..n).map(|i| format!("[ap{i}]")).collect();
    parts.push(format!("{ins}concat=n={n}:v=0:a=1"));
    Some(parts.join(";"))
}

// ---------------------------------------------------------------- checks

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedFix {
    /// Cut the zone down to the part inside the trim.
    ShrinkToTrim(usize),
    Delete(usize),
    /// Move zone `.0`'s start to zone `.1`'s end.
    StartAfter(usize, usize),
}

impl SpeedFix {
    pub fn label(self) -> String {
        match self {
            SpeedFix::ShrinkToTrim(_) => "Shrink speed zone to fit".into(),
            SpeedFix::Delete(i) => format!("Delete speed zone {}", i + 1),
            SpeedFix::StartAfter(i, j) => format!("Start speed zone {} after speed zone {}", i + 1, j + 1),
        }
    }
    pub fn apply(self, zones: &mut Vec<SpeedZone>, trim: Option<(f64, f64)>) {
        match self {
            SpeedFix::ShrinkToTrim(i) => {
                if let (Some(z), Some((ts, te))) = (zones.get_mut(i), trim) {
                    if let Some((s, e)) = z.times(trim) {
                        z.start_text = timecode::format_compact(s.max(ts));
                        z.end_text = timecode::format_compact(e.min(te));
                    }
                }
            }
            SpeedFix::Delete(i) => {
                if i < zones.len() {
                    zones.remove(i);
                }
            }
            SpeedFix::StartAfter(i, j) => {
                if let Some(end) = zones.get(j).and_then(|z| z.times(trim)).map(|t| t.1) {
                    if let Some(z) = zones.get_mut(i) {
                        z.whole = false;
                        z.start_text = timecode::format_compact(end);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpeedProblem {
    pub message: String,
    pub fixes: Vec<SpeedFix>,
}

/// Problems that block the export. `trim` is the trim range, if valid.
pub fn check(zones: &[SpeedZone], trim: Option<(f64, f64)>) -> Vec<SpeedProblem> {
    let mut out = Vec::new();
    let mut p = |message: String, fixes: Vec<SpeedFix>| out.push(SpeedProblem { message, fixes });
    let mut ok: Vec<(usize, f64, f64)> = Vec::new();
    for (i, z) in zones.iter().enumerate() {
        let n = i + 1;
        let (s, e) = if z.whole {
            match trim {
                Some(t) => t,
                None => continue,
            }
        } else {
            match (timecode::parse(&z.start_text), timecode::parse(&z.end_text)) {
                (Err(err), _) => {
                    p(format!("Speed zone {n} start: {err}."), vec![]);
                    continue;
                }
                (_, Err(err)) => {
                    p(format!("Speed zone {n} end: {err}."), vec![]);
                    continue;
                }
                (Ok(s), Ok(e)) => (s, e),
            }
        };
        if s >= e {
            p(format!("Speed zone {n}: start must be before end."), vec![SpeedFix::Delete(i)]);
            continue;
        }
        if let Some((ts, te)) = trim {
            if s < ts - 0.0005 || e > te + 0.0005 {
                let mut fixes = Vec::new();
                if s.max(ts) < e.min(te) {
                    fixes.push(SpeedFix::ShrinkToTrim(i));
                }
                fixes.push(SpeedFix::Delete(i));
                p(
                    format!(
                        "Speed zone {n} ({}–{}) isn't inside your trim ({}–{}).",
                        timecode::format_short(s),
                        timecode::format_short(e),
                        timecode::format_short(ts),
                        timecode::format_short(te)
                    ),
                    fixes,
                );
            }
        }
        if e - s < MIN_LEN - 1e-9 {
            p(format!("Speed zone {n} is too short — make it at least {MIN_LEN} s long."), vec![SpeedFix::Delete(i)]);
        }
        ok.push((i, s, e));
    }
    ok.sort_by(|a, b| a.1.total_cmp(&b.1));
    for w in ok.windows(2) {
        let ((i, _, ie), (j, _, je)) = (w[0], w[1]);
        if w[1].1 < ie - 1e-9 {
            let fixes = if ie < je { vec![SpeedFix::StartAfter(j, i)] } else { vec![SpeedFix::Delete(j)] };
            p(format!("Speed zones {} and {} overlap. Speed zones can't share time.", i + 1, j + 1), fixes);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: f64, end: f64, speed: f64) -> Resolved {
        Resolved { start, end, speed, smooth: false, mute: false }
    }

    #[test]
    fn map_stretches_and_inverts() {
        let m = TimeMap::new(&[r(10.0, 14.0, 0.5), r(20.0, 30.0, 2.0)]);
        assert_eq!(m.to_out(5.0), 5.0);
        assert_eq!(m.to_out(12.0), 14.0); // 2 s at half speed = 4 s
        assert_eq!(m.to_out(14.0), 18.0);
        assert_eq!(m.to_out(25.0), 26.5); // +4, then 5 s at 2x = 2.5 s
        assert_eq!(m.to_out(40.0), 39.0);
        for t in [0.0, 9.9, 10.0, 11.3, 14.0, 17.0, 20.0, 27.5, 30.0, 45.0] {
            assert!((m.to_src(m.to_out(t)) - t).abs() < 1e-9, "t={t}");
        }
        assert_eq!(m.out_len(10.0, 30.0), 19.0); // 8 + 6 + 5
        assert!(TimeMap::new(&[]).is_identity());
    }

    #[test]
    fn overlapping_zones_are_skipped_in_the_map() {
        let m = TimeMap::new(&[r(10.0, 20.0, 0.5), r(15.0, 25.0, 2.0)]);
        assert_eq!(m.zones().len(), 1);
    }

    #[test]
    fn pieces_fill_gaps() {
        let m = TimeMap::new(&[r(12.0, 14.0, 0.5), r(16.0, 20.0, 2.0)]);
        let p = pieces(&m, 10.0, 20.0);
        let spans: Vec<(f64, Option<f64>, f64)> = p.iter().map(|p| (p.start, p.end, p.speed)).collect();
        assert_eq!(spans, vec![(0.0, Some(2.0), 1.0), (2.0, Some(4.0), 0.5), (4.0, Some(6.0), 1.0), (6.0, None, 2.0)]);
        // A whole-trim zone is one piece.
        let m = TimeMap::new(&[r(10.0, 20.0, 2.0)]);
        assert_eq!(pieces(&m, 10.0, 20.0).len(), 1);
    }

    #[test]
    fn filter_chains() {
        let m = TimeMap::new(&[r(10.0, 20.0, 0.25)]);
        let p = pieces(&m, 10.0, 20.0);
        assert_eq!(video_chain(&p, 60.0).unwrap(), "setpts=PTS/0.25");
        assert_eq!(audio_chain(&p, false, 48000).unwrap(), "atempo=0.5,atempo=0.5");
        assert_eq!(audio_chain(&p, true, 48000).unwrap(), "asetrate=12000,aresample=48000");

        let z = Resolved { start: 12.0, end: 14.0, speed: 0.5, smooth: true, mute: true };
        let p = pieces(&TimeMap::new(&[z]), 10.0, 20.0);
        let v = video_chain(&p, 60.0).unwrap();
        assert!(v.starts_with("split=3[vs0][vs1][vs2];[vs0]trim=start=0:end=2,setpts=PTS-STARTPTS[vp0];"), "{v}");
        assert!(v.contains("[vs1]trim=start=2:end=4,setpts=(PTS-STARTPTS)/0.5,minterpolate=fps=60:"), "{v}");
        assert!(v.contains("[vs2]trim=start=4,setpts=PTS-STARTPTS[vp2]"), "{v}");
        assert!(v.ends_with("[vp0][vp1][vp2]concat=n=3:v=1:a=0"), "{v}");
        let a = audio_chain(&p, false, 48000).unwrap();
        assert!(a.contains("[as1]atrim=start=2:end=4,asetpts=PTS-STARTPTS,atempo=0.5,volume=0[ap1]"), "{a}");
        assert!(a.ends_with("concat=n=3:v=0:a=1"), "{a}");

        // A muted 1x zone changes the audio but not the video.
        let z = Resolved { start: 12.0, end: 14.0, speed: 1.0, smooth: false, mute: true };
        let p = pieces(&TimeMap::new(&[z]), 10.0, 20.0);
        assert!(video_chain(&p, 30.0).is_none());
        assert!(audio_chain(&p, false, 48000).is_some());
        assert!(audio_chain(&pieces(&TimeMap::new(&[]), 10.0, 20.0), false, 48000).is_none());
    }

    #[test]
    fn whole_clip_follows_trim() {
        let z = SpeedZone::whole_clip();
        assert_eq!(z.resolve(Some((5.0, 9.0))).unwrap().start, 5.0);
        assert!(check(&[z], Some((5.0, 9.0))).is_empty());
    }

    #[test]
    fn checks_and_fixes() {
        let mut zones = vec![SpeedZone::new(10.0, 15.0), SpeedZone::new(14.0, 20.0)];
        let probs = check(&zones, None);
        assert_eq!(probs.len(), 1);
        assert_eq!(probs[0].fixes, vec![SpeedFix::StartAfter(1, 0)]);
        SpeedFix::StartAfter(1, 0).apply(&mut zones, None);
        assert!(check(&zones, None).is_empty(), "touching is fine");

        let mut zones = vec![SpeedZone::new(45.0, 50.0)];
        let probs = check(&zones, Some((30.0, 48.0)));
        assert_eq!(probs[0].fixes, vec![SpeedFix::ShrinkToTrim(0), SpeedFix::Delete(0)]);
        SpeedFix::ShrinkToTrim(0).apply(&mut zones, Some((30.0, 48.0)));
        assert!(check(&zones, Some((30.0, 48.0))).is_empty());

        assert_eq!(check(&[SpeedZone::new(1.0, 1.05)], None).len(), 1);
    }
}

impl Default for SpeedZone {
    fn default() -> Self {
        SpeedZone::new(0.0, 0.0)
    }
}
