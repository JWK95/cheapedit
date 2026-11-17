//! Audio waveforms for the timeline, worked out in the background by FFmpeg.
//!
//! Each audio track is decoded to 8 kHz mono and boiled down to one peak per
//! 1/100 s, so even a long recording is a small list of numbers. Opening
//! another file cancels the work for the previous one.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Peaks per second of audio.
pub const RATE: f64 = 100.0;
const SAMPLE_RATE: usize = 8000;
const PER_PEAK: usize = SAMPLE_RATE / RATE as usize;

/// The peaks of one track: loudest sample (0-1) in each 1/100 s.
pub type Peaks = Vec<f32>;

#[derive(Default)]
pub struct Waveforms {
    path: Option<PathBuf>,
    /// One slot per audio track, filled in when its scan finishes.
    tracks: Arc<Mutex<Vec<Option<Arc<Peaks>>>>>,
    cancel: Arc<AtomicBool>,
}

impl Waveforms {
    /// Start reading `count` audio tracks of `path` (cancelling any earlier file).
    pub fn load(&mut self, ffmpeg: &Path, path: &Path, count: usize, ctx: &eframe::egui::Context) {
        self.cancel.store(true, Ordering::SeqCst);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.tracks = Arc::new(Mutex::new(vec![None; count]));
        self.path = Some(path.to_path_buf());
        for i in 0..count {
            let (ffmpeg, path, ctx) = (ffmpeg.to_path_buf(), path.to_path_buf(), ctx.clone());
            let (tracks, cancel) = (self.tracks.clone(), self.cancel.clone());
            std::thread::spawn(move || {
                if let Some(peaks) = read_peaks(&ffmpeg, &path, i, &cancel) {
                    if !cancel.load(Ordering::SeqCst) {
                        if let Some(slot) = tracks.lock().unwrap().get_mut(i) {
                            *slot = Some(Arc::new(peaks));
                        }
                        ctx.request_repaint();
                    }
                }
            });
        }
    }

    /// Track `i`'s peaks, once they're ready.
    pub fn track(&self, i: usize) -> Option<Arc<Peaks>> {
        self.tracks.lock().unwrap().get(i).cloned().flatten()
    }
}

fn read_peaks(ffmpeg: &Path, path: &Path, track: usize, cancel: &AtomicBool) -> Option<Peaks> {
    let mut child = crate::winutil::hidden_command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-vn", "-sn", "-dn", "-map", &format!("0:a:{track}"), "-ac", "1", "-ar", &SAMPLE_RATE.to_string(), "-f", "s16le", "-acodec", "pcm_s16le", "pipe:1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let mut peaks = Vec::new();
    let mut buf = vec![0u8; PER_PEAK * 2 * 256];
    let (mut peak, mut n, mut odd): (i32, usize, Option<u8>) = (0, 0, None);
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        let got = match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(g) => g,
        };
        let mut bytes = &buf[..got];
        // A read can end halfway through a sample.
        if let Some(lo) = odd.take() {
            let s = i16::from_le_bytes([lo, bytes[0]]) as i32;
            peak = peak.max(s.abs());
            n += 1;
            bytes = &bytes[1..];
        }
        let mut chunks = bytes.chunks_exact(2);
        for c in &mut chunks {
            peak = peak.max((i16::from_le_bytes([c[0], c[1]]) as i32).abs());
            n += 1;
            if n == PER_PEAK {
                peaks.push(peak as f32 / 32768.0);
                (peak, n) = (0, 0);
            }
        }
        odd = chunks.remainder().first().copied();
    }
    if n > 0 {
        peaks.push(peak as f32 / 32768.0);
    }
    let _ = child.wait();
    (!peaks.is_empty()).then_some(peaks)
}

/// The loudest peak between two times (seconds), or None if the range has none.
pub fn peak_between(peaks: &[f32], t0: f64, t1: f64) -> Option<f32> {
    let a = ((t0 * RATE).floor().max(0.0) as usize).min(peaks.len());
    let b = ((t1 * RATE).ceil().max(0.0) as usize).clamp(a, peaks.len());
    if a == b {
        return peaks.get(a).copied();
    }
    peaks[a..b].iter().copied().reduce(f32::max)
}

/// How tall to draw a peak (0-1). Decibel-shaped, so quiet talking still
/// shows and loud moments don't just flatten at the top.
pub fn height(peak: f32) -> f32 {
    if peak <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * peak.log10();
    ((db + 48.0) / 48.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_in_ranges() {
        let p = vec![0.1, 0.5, 0.2, 0.9];
        assert_eq!(peak_between(&p, 0.0, 0.02), Some(0.5));
        assert_eq!(peak_between(&p, 0.02, 0.04), Some(0.9));
        assert_eq!(peak_between(&p, 0.005, 0.006), Some(0.1));
        assert_eq!(peak_between(&p, 5.0, 6.0), None);
    }

    #[test]
    fn heights() {
        assert_eq!(height(0.0), 0.0);
        assert_eq!(height(1.0), 1.0);
        assert!((height(0.5) - 0.875).abs() < 0.01);
        assert_eq!(height(0.001), 0.0);
    }

    /// Reads a real file's waveform with the bundled FFmpeg.
    #[test]
    fn reads_peaks_with_ffmpeg() {
        let ffmpeg = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/deps/ffmpeg.exe");
        if !ffmpeg.exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("cheapedit_wave_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("two.mkv");
        // Track 0: 1 s of silence then 1 s of tone. Track 1: tone all the way.
        let ok = crate::winutil::hidden_command(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "aevalsrc=if(gte(t\\,1)\\,0.5*sin(2*PI*440*t)\\,0):d=2", "-f", "lavfi", "-i", "sine=f=220:d=2"])
            .args(["-map", "0", "-map", "1", "-c:a", "flac"])
            .arg(&f)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let cancel = AtomicBool::new(false);
        let a = read_peaks(&ffmpeg, &f, 0, &cancel).unwrap();
        let b = read_peaks(&ffmpeg, &f, 1, &cancel).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!((a.len() as i64 - 200).abs() <= 2, "{} peaks", a.len());
        assert!(peak_between(&a, 0.1, 0.9).unwrap() < 0.01, "silent first second");
        assert!(peak_between(&a, 1.1, 1.9).unwrap() > 0.4, "tone second second");
        assert!(peak_between(&b, 0.1, 0.9).unwrap() > 0.05, "track 1 plays throughout");
    }
}

/// Loudest moment of a file's first audio track between two times (tests).
#[cfg(test)]
pub fn tests_peak(ffmpeg: &Path, file: &Path, t0: f64, t1: f64) -> f32 {
    let p = read_peaks(ffmpeg, file, 0, &AtomicBool::new(false)).unwrap_or_default();
    peak_between(&p, t0, t1).unwrap_or(0.0)
}
