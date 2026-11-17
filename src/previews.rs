//! Overlay pictures for the live preview: stills, animated GIF/WebP frames,
//! and a still frame for video overlays. Decoded in the background and kept
//! as egui textures, keyed by file.

use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Longest side kept for the preview (the export always uses the real file).
const MAX_SIDE: u32 = 640;
/// Frames kept from an animation.
const MAX_FRAMES: usize = 300;

pub struct Anim {
    /// Each frame and how long it shows, in seconds.
    pub frames: Vec<(egui::TextureHandle, f64)>,
    pub total: f64,
}

impl Anim {
    /// The frame `t` seconds in. `once`: hold the last frame at the end.
    pub fn frame_at(&self, t: f64, once: bool) -> &egui::TextureHandle {
        let n = self.frames.len();
        if n == 1 || self.total <= 0.0 {
            return &self.frames[0].0;
        }
        let mut t = t.max(0.0);
        if once {
            if t >= self.total {
                return &self.frames[n - 1].0;
            }
        } else {
            t %= self.total;
        }
        let mut acc = 0.0;
        for (tex, d) in &self.frames {
            acc += d;
            if t < acc {
                return tex;
            }
        }
        &self.frames[n - 1].0
    }
}

enum Entry {
    Loading,
    Ready(Arc<Anim>),
    Failed,
}

#[derive(Clone, Default)]
pub struct Previews {
    map: Arc<Mutex<HashMap<PathBuf, Entry>>>,
}

impl Previews {
    /// The preview for `path`, starting to load it if needed. `video` files
    /// get a still frame made by FFmpeg.
    pub fn get(&self, path: &Path, video: bool, ffmpeg: &Path, ctx: &egui::Context) -> Option<Arc<Anim>> {
        let mut map = self.map.lock().unwrap();
        match map.get(path) {
            Some(Entry::Ready(a)) => return Some(a.clone()),
            Some(_) => return None,
            None => {}
        }
        map.insert(path.to_path_buf(), Entry::Loading);
        drop(map);
        let (me, path, ffmpeg, ctx) = (self.clone(), path.to_path_buf(), ffmpeg.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let frames = if video { video_still(&ffmpeg, &path) } else { decode(&path) };
            let entry = match frames {
                Some(frames) if !frames.is_empty() => {
                    let name = path.to_string_lossy().into_owned();
                    let frames: Vec<(egui::TextureHandle, f64)> = frames
                        .into_iter()
                        .enumerate()
                        .map(|(i, (img, d))| (ctx.load_texture(format!("{name}#{i}"), img, egui::TextureOptions::LINEAR), d))
                        .collect();
                    let total = frames.iter().map(|f| f.1).sum();
                    Entry::Ready(Arc::new(Anim { frames, total }))
                }
                _ => Entry::Failed,
            };
            me.map.lock().unwrap().insert(path, entry);
            ctx.request_repaint();
        });
        None
    }
}

fn to_color(img: image::RgbaImage) -> egui::ColorImage {
    let img = shrink(img);
    let size = [img.width() as usize, img.height() as usize];
    egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw())
}

fn shrink(img: image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= MAX_SIDE {
        return img;
    }
    let s = MAX_SIDE as f64 / w.max(h) as f64;
    image::imageops::resize(&img, ((w as f64 * s) as u32).max(1), ((h as f64 * s) as u32).max(1), image::imageops::FilterType::Triangle)
}

fn frames_of<'a>(frames: image::Frames<'a>) -> Vec<(egui::ColorImage, f64)> {
    frames
        .take(MAX_FRAMES)
        .filter_map(Result::ok)
        .map(|f| {
            let (num, den) = f.delay().numer_denom_ms();
            // Browsers treat very short GIF delays as 0.1 s; so does FFmpeg's player.
            let mut d = num as f64 / den.max(1) as f64 / 1000.0;
            if d < 0.02 {
                d = 0.1;
            }
            (to_color(f.into_buffer()), d)
        })
        .collect()
}

/// Decode a still or animated picture.
fn decode(path: &Path) -> Option<Vec<(egui::ColorImage, f64)>> {
    use image::AnimationDecoder;
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let reader = || std::io::BufReader::new(std::fs::File::open(path).ok().unwrap());
    if !path.is_file() {
        return None;
    }
    match ext.as_str() {
        "gif" => Some(frames_of(image::codecs::gif::GifDecoder::new(reader()).ok()?.into_frames())),
        "webp" => {
            let d = image::codecs::webp::WebPDecoder::new(reader()).ok()?;
            if d.has_animation() {
                Some(frames_of(d.into_frames()))
            } else {
                Some(vec![(to_color(image::open(path).ok()?.to_rgba8()), 0.0)])
            }
        }
        _ => Some(vec![(to_color(image::open(path).ok()?.to_rgba8()), 0.0)]),
    }
}

/// One frame from a video, half a second in (or the first if it's shorter).
fn video_still(ffmpeg: &Path, path: &Path) -> Option<Vec<(egui::ColorImage, f64)>> {
    let dir = std::env::temp_dir().join("cheapedit-previews");
    std::fs::create_dir_all(&dir).ok()?;
    let out = dir.join(format!("{:x}.png", hash(path)));
    for ss in ["0.5", "0"] {
        let ok = crate::winutil::hidden_command(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", ss, "-i"])
            .arg(path)
            .args(["-frames:v", "1", "-vf", "scale='min(640,iw)':-2"])
            .arg(&out)
            .status()
            .is_ok_and(|s| s.success());
        if ok && out.is_file() {
            let img = image::open(&out).ok()?.to_rgba8();
            let _ = std::fs::remove_file(&out);
            return Some(vec![(to_color(img), 0.0)]);
        }
    }
    None
}

fn hash(p: &Path) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut h);
    h.finish()
}

/// Is this WebP animated? (Stills become Image overlays, animations GIF-like ones.)
pub fn webp_is_animated(path: &Path) -> bool {
    std::fs::File::open(path)
        .ok()
        .and_then(|f| image::codecs::webp::WebPDecoder::new(std::io::BufReader::new(f)).ok())
        .is_some_and(|d| d.has_animation())
}

/// Pixel size of a picture file, read from its header.
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}
