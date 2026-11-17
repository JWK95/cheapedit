//! Settings saved to `%APPDATA%\cheapedit\settings.toml`.

use crate::job::Options;
use crate::keybinds::Keybinds;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub output_dir: Option<PathBuf>,
    pub last_open_dir: Option<PathBuf>,
    pub options: Options,
    pub keybinds: Keybinds,
    pub updates: UpdatePrefs,
    /// Result shows zoom boxes as outlines instead of zooming live.
    pub zoom_static_preview: bool,
    /// How loud the player is, 0-100. Only affects what you hear while
    /// editing, never the export.
    pub preview_volume: u32,
    /// Show the ghost line where playback last started.
    pub show_ghost: bool,
    /// Draw the audio's waveform in the timeline.
    pub show_waveform: bool,
    /// The folder the Clips drawer lists.
    pub clips_dir: Option<PathBuf>,
    /// The Clips drawer is open.
    pub clips_open: bool,
    /// The Clips drawer's width in points.
    pub clips_width: f32,
    /// Where the drawer's divider sits: the Editing half's share of the height.
    pub clips_split: f32,
    /// List audio files in the drawer's Editing half.
    pub editing_show_audio: bool,
    /// List audio files (like MP3 exports) in the drawer's Finished half.
    pub finished_show_audio: bool,
    /// Use the user's own FFmpeg and libmpv from PATH before the bundled ones.
    pub use_path_deps: bool,
    /// GIFs, pictures and videos to put on clips (listed in Overlays).
    pub assets_dir: Option<PathBuf>,
    /// The user's own KLIPY API key (empty: no GIF search).
    pub klipy_key: String,
    /// Where montage projects are saved.
    pub montage_dir: Option<PathBuf>,
    /// Run tracking on the graphics card (DirectML) instead of the CPU.
    pub tracker_gpu: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            output_dir: None,
            last_open_dir: None,
            options: Options::default(),
            keybinds: Keybinds::default(),
            updates: UpdatePrefs::default(),
            zoom_static_preview: false,
            preview_volume: 100,
            show_ghost: true,
            show_waveform: true,
            clips_dir: None,
            clips_open: true,
            clips_width: 250.0,
            clips_split: 0.5,
            editing_show_audio: true,
            finished_show_audio: true,
            use_path_deps: false,
            assets_dir: None,
            klipy_key: String::new(),
            montage_dir: None,
            tracker_gpu: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatePrefs {
    /// Ask GitHub for a newer version when cheapedit starts.
    pub check_on_startup: bool,
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        UpdatePrefs { check_on_startup: true }
    }
}

fn path() -> PathBuf {
    crate::winutil::app_data_dir().join("settings.toml")
}

impl Settings {
    pub fn load() -> Settings {
        let mut s: Settings = std::fs::read_to_string(path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default();
        // Shortcuts added in a newer version get their default key.
        s.keybinds.fill_missing();
        s
    }
    pub fn save(&self) {
        if let Ok(s) = toml::to_string_pretty(self) {
            // Write to a temp file then rename, so a crash can't leave half a file.
            let p = path();
            let tmp = p.with_extension("toml.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings { output_dir: Some("D:\\out".into()), ..Default::default() };
        let text = toml::to_string_pretty(&s).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
    }

    #[test]
    fn tolerates_missing_fields() {
        let s: Settings = toml::from_str("output_dir = 'D:\\\\x'").unwrap();
        assert_eq!(s.options, Options::default());
        assert!(s.updates.check_on_startup, "update checks default to on");
        assert_eq!(s.preview_volume, 100, "older settings files don't start the preview silent");
    }
}
