//! Discord presets: pick a resolution, frame rate and bitrate so the export
//! fits under a size cap (Discord 10 / 20 / 50 MB).
//!
//! Pure maths, no I/O. The bitrate is budgeted from the clip length with
//! headroom; if the finished file still comes out too big, the editor
//! re-encodes once at a lower bitrate (see `retry_scale`).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    Custom,
    Discord10,
    Discord20,
    Discord50,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Custom, Preset::Discord10, Preset::Discord20, Preset::Discord50];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Custom => "Custom",
            Preset::Discord10 => "Discord 10",
            Preset::Discord20 => "Discord 20",
            Preset::Discord50 => "Discord 50",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Preset::Custom => "You choose the format, codec and quality below.",
            Preset::Discord10 => "Always under 10 MB, the free Discord upload limit. Plays right in the chat.",
            Preset::Discord20 => "Always under 20 MB. Plays right in the Discord chat.",
            Preset::Discord50 => "Always under 50 MB, the Nitro Basic upload limit. Plays right in the chat.",
        }
    }

    /// The size cap in megabytes, for the Discord presets.
    pub fn cap_mb(self) -> Option<u32> {
        match self {
            Preset::Custom => None,
            Preset::Discord10 => Some(10),
            Preset::Discord20 => Some(20),
            Preset::Discord50 => Some(50),
        }
    }

    /// The cap in bytes. Uses 1 MB = 1,000,000 bytes, the smaller reading,
    /// so a file is under the cap however Discord counts.
    pub fn cap_bytes(self) -> Option<u64> {
        self.cap_mb().map(|m| m as u64 * 1_000_000)
    }
}

/// Share of the cap the audio + video bitrates aim for. The rest covers the
/// MP4 container and the encoder overshooting a little.
pub const HEADROOM: f64 = 0.92;

/// Below this much video bitrate even 480p looks like mush, so Export is blocked.
pub const MIN_VIDEO_KBPS: u32 = 250;

/// Resolution/frame-rate steps, best first, with the video bitrate each
/// needs to look decent for fast-moving game footage.
const LADDER: [(u32, u32, u32); 4] = [(1080, 60, 4500), (1080, 30, 3000), (720, 30, 1500), (480, 30, MIN_VIDEO_KBPS)];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    /// Output height; never more than the source.
    pub height: u32,
    pub width: u32,
    /// Output frame rate; never more than the source.
    pub fps: f64,
    /// True when the picture keeps the source's height (no scaling).
    pub same_height: bool,
    /// True when the frame rate is the source's (no fps filter needed).
    pub same_fps: bool,
    pub video_kbps: u32,
    /// None when the export has no audio track.
    pub audio_kbps: Option<u32>,
}

impl Plan {
    /// Expected file size in bytes.
    pub fn bytes(&self, out_secs: f64) -> u64 {
        let kbps = self.video_kbps + self.audio_kbps.unwrap_or(0);
        // ~1.5 % for the MP4 container.
        (kbps as f64 * 1000.0 / 8.0 * out_secs * 1.015) as u64
    }
}

/// Width for `scale=-2:h` (keeps the shape, rounded to an even number).
pub fn scaled_width(src_w: u32, src_h: u32, h: u32) -> u32 {
    if src_h == 0 {
        return src_w;
    }
    let w = (src_w as f64 * h as f64 / src_h as f64 / 2.0).round() as u32 * 2;
    w.max(2)
}

fn audio_kbps_for(total_kbps: f64) -> u32 {
    if total_kbps >= 1500.0 { 128 } else { 96 }
}

/// The longest clip (output seconds) that fits under the cap.
pub fn max_seconds(cap_bytes: u64, has_audio: bool) -> f64 {
    let audio = if has_audio { 96 } else { 0 };
    cap_bytes as f64 * 8.0 * HEADROOM / ((MIN_VIDEO_KBPS + audio) as f64 * 1000.0)
}

/// Plan an export of `out_secs` seconds under `cap_bytes`. `Err` holds the
/// longest output length (seconds) that would fit.
pub fn plan(cap_bytes: u64, out_secs: f64, src_w: u32, src_h: u32, src_fps: f64, has_audio: bool) -> Result<Plan, f64> {
    let out_secs = out_secs.max(0.1);
    let total_kbps = cap_bytes as f64 * 8.0 * HEADROOM / out_secs / 1000.0;
    let audio = has_audio.then(|| audio_kbps_for(total_kbps));
    let video = total_kbps - audio.unwrap_or(0) as f64;
    // A small source (e.g. 720p) uses its own size at the steps above it.
    let step = LADDER.iter().find(|&&(h, fps, need)| {
        let need_here = if h > src_h || fps as f64 > src_fps + 0.5 {
            // Scale what the step needs down to what we'd actually encode.
            let px = |h: u32, f: f64| h as f64 * h as f64 * f;
            (need as f64 * px(h.min(src_h), (fps as f64).min(src_fps)) / px(h, fps as f64)).max(MIN_VIDEO_KBPS as f64)
        } else {
            need as f64
        };
        video >= need_here
    });
    let Some(&(h, fps, _)) = step else {
        return Err(max_seconds(cap_bytes, has_audio));
    };
    let height = h.min(src_h);
    let fps_out = (fps as f64).min(src_fps);
    Ok(Plan {
        height,
        width: scaled_width(src_w, src_h, height),
        fps: fps_out,
        same_height: height == src_h,
        same_fps: (fps_out - src_fps).abs() < 0.01,
        // Bitrates are whole kbps; never round up past the budget.
        video_kbps: (video.floor() as u32).min(50_000),
        audio_kbps: audio,
    })
}

/// After an export that came out at `actual` bytes over `cap` bytes, the
/// factor to scale the video bitrate by for the second try. Aims a bit
/// lower than the plain ratio, so the retry lands safely under.
pub fn retry_scale(cap: u64, actual: u64) -> f64 {
    if actual == 0 {
        return 1.0;
    }
    (cap as f64 * HEADROOM / actual as f64 * 0.95).clamp(0.3, 0.97)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB10: u64 = 10_000_000;

    #[test]
    fn short_clip_keeps_1080p60() {
        let p = plan(MB10, 10.0, 1920, 1080, 60.0, true).unwrap();
        assert_eq!((p.height, p.fps, p.audio_kbps), (1080, 60.0, Some(128)));
        assert!(p.same_height && p.same_fps);
        assert!(p.bytes(10.0) < MB10);
    }

    #[test]
    fn steps_down_as_clips_get_longer() {
        let at = |secs: f64| {
            let p = plan(MB10, secs, 1920, 1080, 60.0, true).unwrap();
            (p.height, p.fps as u32)
        };
        assert_eq!(at(14.0), (1080, 60));
        assert_eq!(at(22.0), (1080, 30));
        assert_eq!(at(40.0), (720, 30));
        assert_eq!(at(150.0), (480, 30));
        assert_eq!(scaled_width(1920, 1080, 480), 854);
    }

    #[test]
    fn too_long_says_how_long_fits() {
        let max = plan(MB10, 600.0, 1920, 1080, 60.0, true).unwrap_err();
        assert!((200.0..240.0).contains(&max), "{max}");
        assert!(plan(MB10, max - 1.0, 1920, 1080, 60.0, true).is_ok());
        // No audio track leaves more room for the picture.
        assert!(max_seconds(MB10, false) > max);
    }

    #[test]
    fn never_upscales() {
        let p = plan(MB10, 5.0, 1280, 720, 30.0, true).unwrap();
        assert_eq!((p.height, p.width, p.fps), (720, 1280, 30.0));
        assert!(p.same_height && p.same_fps);
    }

    #[test]
    fn every_plan_fits() {
        for cap in [10, 20, 50] {
            let cap = cap * 1_000_000;
            for secs in [1.0, 7.5, 30.0, 61.0, 200.0] {
                if let Ok(p) = plan(cap, secs, 2560, 1440, 144.0, true) {
                    assert!(p.bytes(secs) < cap, "{cap} {secs}");
                }
            }
        }
    }

    #[test]
    fn retry_aims_lower() {
        let s = retry_scale(MB10, 11_000_000);
        assert!(s < MB10 as f64 / 11_000_000.0);
    }
}
