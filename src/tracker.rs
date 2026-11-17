//! Follows something on screen: the VitTrack model (OpenCV model zoo,
//! Apache 2.0) run with ONNX Runtime, on the CPU or the graphics card
//! (DirectML). Frames come from FFmpeg as raw pictures.
//!
//! Pre- and post-processing follow OpenCV's TrackerVit (tracker_vit.cpp):
//! a 128x128 template cut from twice the target's size, a 256x256 search
//! area four times its size, BGR pixels normalised with ImageNet mean/std,
//! and the box read from a 16x16 score map weighted by a Hann window.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const TEMPLATE: usize = 128;
const SEARCH: usize = 256;
const MAP: usize = 16;
/// Below this, the target is treated as lost for that frame (the box stays put).
const MIN_SCORE: f32 = 0.20;
/// OpenCV's mean/std, applied to the channels in BGR order (as OpenCV does).
const MEAN: [f32; 3] = [0.485 * 255.0, 0.456 * 255.0, 0.406 * 255.0];
const STD: [f32; 3] = [0.229 * 255.0, 0.224 * 255.0, 0.225 * 255.0];
/// Frames are tracked at this width (the model only sees small crops anyway).
const WORK_W: u32 = 640;
const MAX_FPS: f64 = 30.0;

/// A picture in BGR bytes.
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub bgr: Vec<u8>,
}

/// A box in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// `crop` around `r` (crop side = sqrt(area) x factor, centred on the box),
/// zero-padded outside the picture, resized to `size` x `size`, as a
/// normalised NCHW tensor. Returns the crop side in pixels too.
pub fn crop(img: &Image, r: Rect, factor: f32, size: usize) -> (Vec<f32>, f32) {
    let side = ((r.w * r.h).sqrt() * factor).ceil().max(1.0);
    let x1 = (r.x + (r.w - side) / 2.0).floor();
    let y1 = (r.y + (r.h - side) / 2.0).floor();
    let scale = side / size as f32;
    let mut out = vec![0f32; 3 * size * size];
    let px = |x: i64, y: i64, c: usize| -> f32 {
        if x < 0 || y < 0 || x >= img.w as i64 || y >= img.h as i64 { 0.0 } else { img.bgr[(y as usize * img.w + x as usize) * 3 + c] as f32 }
    };
    for v in 0..size {
        let sy = y1 + (v as f32 + 0.5) * scale - 0.5;
        let (y0, fy) = (sy.floor(), sy - sy.floor());
        for u in 0..size {
            let sx = x1 + (u as f32 + 0.5) * scale - 0.5;
            let (x0, fx) = (sx.floor(), sx - sx.floor());
            let (xi, yi) = (x0 as i64, y0 as i64);
            for c in 0..3 {
                let top = px(xi, yi, c) * (1.0 - fx) + px(xi + 1, yi, c) * fx;
                let bot = px(xi, yi + 1, c) * (1.0 - fx) + px(xi + 1, yi + 1, c) * fx;
                let val = top * (1.0 - fy) + bot * fy;
                out[c * size * size + v * size + u] = (val - MEAN[c]) / STD[c];
            }
        }
    }
    (out, side)
}

/// OpenCV's centred Hann window, 16x16.
fn hann() -> Vec<f32> {
    let h1: Vec<f32> = (0..MAP).map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI / (MAP as f32 + 1.0) * (i as f32 + 1.0)).cos())).collect();
    let mut out = Vec::with_capacity(MAP * MAP);
    for y in 0..MAP {
        for x in 0..MAP {
            out.push(h1[y] * h1[x]);
        }
    }
    out
}

/// Where the box goes, from the model's three maps. `rect` is the previous
/// box and `side` the search crop's side. None when the score is too low.
pub fn decode(conf: &[f32], size: &[f32], offset: &[f32], rect: Rect, side: f32, img_w: usize, img_h: usize, window: &[f32]) -> Option<(Rect, f32)> {
    let n = MAP * MAP;
    let (mut best, mut score) = (0, f32::MIN);
    for i in 0..n {
        let s = conf[i] * window[i];
        if s > score {
            score = s;
            best = i;
        }
    }
    if score < MIN_SCORE {
        return None;
    }
    let (mx, my) = ((best % MAP) as f32, (best / MAP) as f32);
    let cx = (mx + offset[best]) / MAP as f32;
    let cy = (my + offset[n + best]) / MAP as f32;
    let (w, h) = (size[best], size[n + best]);
    let x0 = rect.x + (rect.w - side) / 2.0;
    let y0 = rect.y + (rect.h - side) / 2.0;
    let mut r = Rect { x: ((cx - w / 2.0) * side + x0).floor(), y: ((cy - h / 2.0) * side + y0).floor(), w: (w * side).floor(), h: (h * side).floor() };
    // Keep it on the picture (at least 10 pixels of it).
    r.w = r.w.clamp(10.0, img_w as f32);
    r.h = r.h.clamp(10.0, img_h as f32);
    r.x = r.x.clamp(10.0 - r.w, img_w as f32 - 10.0);
    r.y = r.y.clamp(10.0 - r.h, img_h as f32 - 10.0);
    Some((r, score))
}

static ENV: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();

fn runtime_dll() -> Option<PathBuf> {
    crate::deps::bundled_dir().map(|d| d.join("onnxruntime.dll")).filter(|p| p.is_file())
}

pub fn model_file() -> Option<PathBuf> {
    crate::deps::bundled_dir().map(|d| d.join("models").join("vittrack.onnx")).filter(|p| p.is_file())
}

/// Tracking can run: the runtime and model are bundled.
pub fn available() -> Result<(), String> {
    runtime_dll().ok_or("ONNX Runtime (onnxruntime.dll) isn't in cheapedit's deps folder, so tracking can't run. Reinstall cheapedit.")?;
    model_file().ok_or("The tracking model (models\\vittrack.onnx) is missing from cheapedit's deps folder. Reinstall cheapedit.")?;
    Ok(())
}

fn env() -> Result<&'static ort::environment::Environment, String> {
    ENV.get_or_init(|| {
        let dll = runtime_dll().ok_or("onnxruntime.dll is missing.")?;
        ort::init_from(&dll).map_err(|e| format!("Couldn't load ONNX Runtime: {e}"))?.with_name("cheapedit").build().map_err(|e| format!("Couldn't start ONNX Runtime: {e}"))
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// The model, loaded. `gpu_used` says where it actually runs.
pub struct Engine {
    session: ort::session::Session,
    pub gpu_used: bool,
    /// Why the GPU wasn't used, when it was asked for.
    pub gpu_note: Option<String>,
}

impl Engine {
    pub fn load(model: &Path, gpu: bool) -> Result<Engine, String> {
        let env = env()?;
        let cpu = || -> Result<ort::session::Session, String> {
            ort::session::Session::builder(env)
                .map_err(|e| e.to_string())?
                .commit_from_file(model)
                .map_err(|e| format!("Couldn't load the tracking model: {e}"))
        };
        if gpu {
            let tried = ort::session::Session::builder(env)
                .map_err(|e| e.to_string())
                .and_then(|b| b.with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()]).map_err(|e| e.to_string()))
                .and_then(|mut b| b.commit_from_file(model).map_err(|e| e.to_string()));
            return match tried {
                Ok(session) => Ok(Engine { session, gpu_used: true, gpu_note: None }),
                Err(e) => Ok(Engine { session: cpu()?, gpu_used: false, gpu_note: Some(format!("GPU failed ({e}), so the CPU was used.")) }),
            };
        }
        Ok(Engine { session: cpu()?, gpu_used: false, gpu_note: None })
    }
}

/// One tracking run: VitTrack's template plus the current box.
pub struct VitTrack {
    template: Vec<f32>,
    rect: Rect,
    window: Vec<f32>,
}

impl VitTrack {
    pub fn new(img: &Image, rect: Rect) -> VitTrack {
        VitTrack { template: crop(img, rect, 2.0, TEMPLATE).0, rect, window: hann() }
    }
    pub fn rect(&self) -> Rect {
        self.rect
    }
    /// Find the target in the next picture. Returns the score (the box stays
    /// where it was if the target is lost).
    pub fn update(&mut self, engine: &mut Engine, img: &Image) -> Result<f32, String> {
        let (search, side) = crop(img, self.rect, 4.0, SEARCH);
        let t = ort::value::Tensor::from_array(([1usize, 3, TEMPLATE, TEMPLATE], self.template.clone())).map_err(|e| e.to_string())?;
        let s = ort::value::Tensor::from_array(([1usize, 3, SEARCH, SEARCH], search)).map_err(|e| e.to_string())?;
        let outputs = engine.session.run(ort::inputs!["template" => t, "search" => s]).map_err(|e| format!("Tracking failed: {e}"))?;
        let get = |name: &str| -> Result<Vec<f32>, String> { Ok(outputs[name].try_extract_tensor::<f32>().map_err(|e| e.to_string())?.1.to_vec()) };
        let (conf, size, offset) = (get("output1")?, get("output2")?, get("output3")?);
        match decode(&conf, &size, &offset, self.rect, side, img.w, img.h, &self.window) {
            Some((r, score)) => {
                self.rect = r;
                Ok(score)
            }
            None => Ok(0.0),
        }
    }
}

/// One tracked moment: original-video time and the target's centre (frame fractions).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Point {
    pub t: f64,
    pub x: f64,
    pub y: f64,
}

pub struct Run {
    pub points: Vec<Point>,
    pub gpu_used: bool,
    pub gpu_note: Option<String>,
    /// Frames where the target was lost (the box held still).
    pub lost: usize,
}

/// Follow the thing in `start_box` (frame fractions: x, y, w, h) from `t0`
/// to `t1` of `src`. `progress` counts per mille; `cancel` stops it.
#[allow(clippy::too_many_arguments)]
pub fn track(ffmpeg: &Path, src: &Path, src_w: u32, src_h: u32, src_fps: f64, t0: f64, t1: f64, start_box: (f64, f64, f64, f64), gpu: bool, progress: &AtomicU32, cancel: &AtomicBool) -> Result<Run, String> {
    available()?;
    let mut engine = Engine::load(&model_file().unwrap(), gpu)?;
    let w = WORK_W.min(src_w).max(16) / 2 * 2;
    let h = ((w as f64 * src_h as f64 / src_w.max(1) as f64 / 2.0).round() as u32 * 2).max(16);
    let fps = src_fps.clamp(1.0, MAX_FPS);
    let len = (t1 - t0).max(0.0);
    let mut child = crate::winutil::hidden_command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-ss", &format!("{t0:.3}"), "-t", &format!("{len:.3}"), "-i"])
        .arg(src)
        .args(["-an", "-vf", &format!("fps={fps},scale={w}:{h}"), "-f", "rawvideo", "-pix_fmt", "bgr24", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't start FFmpeg: {e}"))?;
    let mut out = child.stdout.take().unwrap();
    let frame_bytes = (w * h * 3) as usize;
    let total = (len * fps).ceil().max(1.0);
    let (bx, by, bw, bh) = start_box;
    let mut tracker: Option<VitTrack> = None;
    let mut points = Vec::new();
    let mut lost = 0;
    let mut k = 0u64;
    let result = loop {
        if cancel.load(Ordering::SeqCst) {
            break Err("Tracking cancelled.".to_string());
        }
        let mut buf = vec![0u8; frame_bytes];
        if out.read_exact(&mut buf).is_err() {
            break Ok(());
        }
        let img = Image { w: w as usize, h: h as usize, bgr: buf };
        let t = t0 + k as f64 / fps;
        let tr = match &mut tracker {
            None => {
                let r = Rect { x: (bx * w as f64) as f32, y: (by * h as f64) as f32, w: (bw * w as f64) as f32, h: (bh * h as f64) as f32 };
                tracker.insert(VitTrack::new(&img, r))
            }
            Some(tr) => {
                if tr.update(&mut engine, &img)? < MIN_SCORE {
                    lost += 1;
                }
                tr
            }
        };
        let (cx, cy) = tr.rect().center();
        points.push(Point { t, x: cx as f64 / w as f64, y: cy as f64 / h as f64 });
        k += 1;
        progress.store(((k as f64 / total) * 1000.0).min(999.0) as u32, Ordering::SeqCst);
    };
    let _ = child.kill();
    let _ = child.wait();
    result?;
    if points.is_empty() {
        return Err("FFmpeg gave no pictures for that stretch of the video.".into());
    }
    progress.store(1000, Ordering::SeqCst);
    Ok(Run { points, gpu_used: engine.gpu_used, gpu_note: engine.gpu_note, lost })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_pads_with_zero_and_normalises() {
        // A 4x4 white picture; a crop much bigger than it is mostly padding.
        let img = Image { w: 4, h: 4, bgr: vec![255; 48] };
        let (t, side) = crop(&img, Rect { x: 1.0, y: 1.0, w: 2.0, h: 2.0 }, 4.0, 16);
        assert_eq!(side, 8.0);
        assert_eq!(t.len(), 3 * 16 * 16);
        // Corner: outside the picture, so (0 - mean) / std.
        assert!((t[0] - (0.0 - MEAN[0]) / STD[0]).abs() < 1e-4);
        // Centre: white.
        let c = 8 * 16 + 8;
        assert!((t[c] - (255.0 - MEAN[0]) / STD[0]).abs() < 1e-3);
    }

    #[test]
    fn decode_moves_the_box() {
        let n = MAP * MAP;
        let mut conf = vec![0.0; n];
        // Peak at cell (10, 8): right of centre.
        conf[8 * MAP + 10] = 1.0;
        let size = vec![0.25; 2 * n];
        let offset = vec![0.5; 2 * n];
        let r = Rect { x: 100.0, y: 100.0, w: 40.0, h: 40.0 };
        let side = 160.0;
        let (nr, score) = decode(&conf, &size, &offset, r, side, 640, 360, &vec![1.0; n]).unwrap();
        assert_eq!(score, 1.0);
        // Centre at (10.5/16, 8.5/16) of a 160 crop starting at x0 = 100 + (40-160)/2 = 40.
        let (cx, cy) = nr.center();
        assert!((cx - (40.0 + 10.5 / 16.0 * 160.0)).abs() < 1.0, "{cx}");
        assert!((cy - (40.0 + 8.5 / 16.0 * 160.0)).abs() < 1.0, "{cy}");
        assert!(decode(&vec![0.1; n], &size, &offset, r, side, 640, 360, &vec![1.0; n]).is_none());
    }

    /// A white square moving left to right on a dark, textured background:
    /// the tracker must follow it. Needs the bundled runtime and model.
    #[test]
    #[ignore]
    fn follows_a_moving_square() {
        let ffmpeg = crate::deps::find("ffmpeg.exe").expect("run tools\\fetch-deps.ps1");
        let dir = std::env::temp_dir().join("cheapedit-track-test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("moving.mp4");
        // 640x360, 4 s; an 80x80 square from x=40 to x=520 at y=140.
        let ok = crate::winutil::hidden_command(&ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=4,hue=s=0,eq=brightness=-0.3", "-f", "lavfi", "-i", "color=c=0xFF4040:s=80x80:r=30:d=4"])
            .args(["-filter_complex", "[0:v][1:v]overlay=x='40+120*t':y=140:shortest=1", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&src)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        for gpu in [false, true] {
            let (p, c) = (AtomicU32::new(0), AtomicBool::new(false));
            let started = std::time::Instant::now();
            let run = track(&ffmpeg, &src, 640, 360, 30.0, 0.0, 4.0, (40.0 / 640.0, 140.0 / 360.0, 80.0 / 640.0, 80.0 / 360.0), gpu, &p, &c).unwrap();
            println!("gpu={gpu} used={} note={:?} frames={} lost={} in {:?}", run.gpu_used, run.gpu_note, run.points.len(), run.lost, started.elapsed());
            for pt in run.points.iter().step_by(15) {
                let want_x = (40.0 + 120.0 * pt.t + 40.0) / 640.0;
                println!("  t={:.2} x={:.3} (want {:.3}) y={:.3} (want {:.3})", pt.t, pt.x, want_x, pt.y, 180.0 / 360.0);
            }
            let last = run.points.last().unwrap();
            let want_x = (40.0 + 120.0 * last.t + 40.0) / 640.0;
            assert!((last.x - want_x).abs() < 0.04, "ended at x={} want {want_x}", last.x);
            assert!((last.y - 0.5).abs() < 0.05, "ended at y={}", last.y);
        }
    }
}
