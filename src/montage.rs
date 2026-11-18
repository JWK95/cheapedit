//! Montage projects: a named list of clip "recipes" (each clip's edits),
//! saved as `<name>.cheapmontage` (JSON) in the user's montage folder, and
//! joined into one file when finished.

use crate::job::{AvMode, Frame};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const EXT: &str = "cheapmontage";

/// Everything needed to export one clip the way it was edited.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Recipe {
    pub source: PathBuf,
    pub start_text: String,
    pub end_text: String,
    pub until_end: bool,
    pub zones: Vec<crate::zoom::Zone>,
    pub speed_zones: Vec<crate::speed::SpeedZone>,
    pub overlays: Vec<crate::overlay::Overlay>,
    pub tracks: Vec<crate::tracks::TrackMix>,
    pub av: Option<AvMode>,
    pub volume_pct: u32,
    pub change_pitch: bool,
    /// Kept for the strip: the clip's finished length and the source's picture.
    pub length: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

impl Recipe {
    pub fn name(&self) -> String {
        self.source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into())
    }
    pub fn missing(&self) -> bool {
        !self.source.is_file()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Montage {
    pub name: String,
    pub clips: Vec<Recipe>,
    /// Picture size and frame rate; None = the first clip's.
    pub frame: Option<Frame>,
}

fn even(v: u32) -> u32 {
    (v / 2 * 2).max(2)
}

impl Montage {
    pub fn total(&self) -> f64 {
        self.clips.iter().map(|c| c.length).sum()
    }
    /// The size and frame rate every clip is fitted to.
    pub fn frame(&self) -> Option<Frame> {
        if let Some(f) = self.frame {
            return Some(f);
        }
        let c = self.clips.first()?;
        Some(Frame { w: even(c.width), h: even(c.height), fps: if c.fps > 0.0 { c.fps } else { 30.0 } })
    }
    pub fn load(path: &Path) -> Result<Montage, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("Couldn't open {}: {e}", path.display()))?;
        let mut m: Montage = serde_json::from_str(&text).map_err(|e| format!("{} isn't a montage cheapedit can read ({e}).", path.display()))?;
        m.clips.iter_mut().flat_map(|c| c.zones.iter_mut()).for_each(crate::zoom::Zone::upgrade);
        Ok(m)
    }
    /// Save via a temporary file, so a crash mid-save never loses the project.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("saving");
        std::fs::write(&tmp, text).map_err(|e| format!("Couldn't save the montage: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't save the montage: {e}"))
    }
}

/// `dir/<name>.cheapmontage`
pub fn project_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{EXT}"))
}

/// The montages in `dir`, newest first: (name, file).
pub fn list(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf, std::time::SystemTime)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXT)))
        .map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            (p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), p, t)
        })
        .collect();
    out.sort_by(|a, b| b.2.cmp(&a.2));
    out.into_iter().map(|(n, p, _)| (n, p)).collect()
}

/// The concat demuxer's list file for these clips, in order.
pub fn concat_list(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|f| format!("file '{}'\n", f.to_string_lossy().replace('\\', "/").replace('\'', "'\\''")))
        .collect()
}

/// Join the rendered clips (all made with the same settings) without re-encoding.
pub fn join_args(list_file: &Path, output: &Path, mp4_like: bool) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-n", "-loglevel", "warning", "-f", "concat", "-safe", "0", "-i"].iter().map(|s| s.to_string()).collect();
    a.push(list_file.to_string_lossy().into_owned());
    a.extend(["-map", "0", "-c", "copy"].iter().map(|s| s.to_string()));
    if mp4_like {
        a.extend(["-movflags".into(), "+faststart".into()]);
    }
    a.extend(["-progress".into(), "pipe:1".into(), "-nostats".into()]);
    a.push(output.to_string_lossy().into_owned());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads() {
        let dir = std::env::temp_dir().join("cheapedit-montage-test");
        std::fs::create_dir_all(&dir).unwrap();
        let mut z = crate::zoom::Zone::new(1.0, 2.0);
        z.a_drawn = true;
        let m = Montage {
            name: "Valorant highlights".into(),
            clips: vec![Recipe {
                source: r"C:\clips\a.mp4".into(),
                start_text: "1".into(),
                end_text: "5".into(),
                zones: vec![z],
                speed_zones: vec![crate::speed::SpeedZone::new(2.0, 3.0)],
                overlays: vec![crate::overlay::Overlay::text(1.0, 2.0)],
                length: 5.0,
                width: 1920,
                height: 1080,
                fps: 60.0,
                ..Default::default()
            }],
            frame: None,
        };
        let p = project_path(&dir, &m.name);
        m.save(&p).unwrap();
        assert_eq!(Montage::load(&p).unwrap(), m);
        assert!(list(&dir).iter().any(|(n, _)| n == "Valorant highlights"));
        assert_eq!(m.frame(), Some(Frame { w: 1920, h: 1080, fps: 60.0 }));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn concat_lines() {
        assert_eq!(concat_list(&[r"C:\t\0.mp4".into(), r"C:\it's\1.mp4".into()]), "file 'C:/t/0.mp4'\nfile 'C:/it'\\''s/1.mp4'\n");
    }
}
