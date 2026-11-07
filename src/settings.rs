//! Settings saved to `%APPDATA%\cheapedit\settings.toml`.

use crate::job::Options;
use crate::keybinds::Keybinds;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub output_dir: Option<PathBuf>,
    pub last_open_dir: Option<PathBuf>,
    pub options: Options,
    pub keybinds: Keybinds,
    pub updates: UpdatePrefs,
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
    }
}
