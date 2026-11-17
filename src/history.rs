//! Which source clips have been exported, for the green ticks in the
//! Editing list. Saved to `%APPDATA%\cheapedit\exports.json`.
//!
//! Keyed by the source's full path (ignoring case, as Windows does), so a
//! source that's renamed or moved loses its tick. The tick stays after the
//! export itself is deleted: it records that the clip was dealt with.

use crate::clips::Clip;
use crate::job::Stamp;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Successful exports made from this source.
    pub count: u32,
    /// When the last one was made, as `dd-mm-yy hh-mm`.
    pub last: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub sources: BTreeMap<String, Entry>,
    /// Old exports made before history was kept have been matched up.
    pub caught_up: bool,
}

fn key(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

fn path() -> PathBuf {
    crate::winutil::app_data_dir().join("exports.json")
}

impl History {
    pub fn load() -> History {
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let p = path();
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    pub fn record(&mut self, source: &Path, at: Stamp) {
        let e = self.sources.entry(key(source)).or_default();
        e.count += 1;
        e.last = at.text();
    }

    pub fn get(&self, source: &Path) -> Option<&Entry> {
        self.sources.get(&key(source))
    }

    /// One-time catch-up: an export from before history was kept is named
    /// `<source name>_<trim>...`, so any file in the output folder starting
    /// with a source's name and `_` marks that source as exported.
    /// `exports` are (file name, local time it was made).
    pub fn catch_up<'a>(&mut self, sources: impl Iterator<Item = &'a Clip>, exports: &[(String, Option<Stamp>)]) -> usize {
        let mut found = 0;
        for src in sources {
            let Some(stem) = src.path.file_stem().map(|s| s.to_string_lossy().to_lowercase()) else { continue };
            let prefix = format!("{stem}_");
            let mut hits: Vec<&Option<Stamp>> = exports.iter().filter(|(n, _)| n.to_lowercase().starts_with(&prefix)).map(|(_, t)| t).collect();
            if hits.is_empty() || self.get(&src.path).is_some() {
                continue;
            }
            // Newest last, by (y, m, d, h, min).
            hits.sort_by_key(|t| t.map(|t| (t.year, t.month, t.day, t.hour, t.minute)));
            let last = hits.last().and_then(|t| t.map(|t| t.text())).unwrap_or_default();
            self.sources.insert(key(&src.path), Entry { count: hits.len() as u32, last });
            found += 1;
        }
        self.caught_up = true;
        found
    }
}

/// "Exported 3 times, last 17-11-26 21-04".
pub fn describe(e: &Entry) -> String {
    let times = if e.count == 1 { "once".to_string() } else { format!("{} times", e.count) };
    if e.last.is_empty() { format!("Exported {times}") } else { format!("Exported {times}, last {}", e.last) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn clip(p: &str) -> Clip {
        Clip { path: PathBuf::from(p), name: String::new(), modified: SystemTime::UNIX_EPOCH, size: 0 }
    }

    #[test]
    fn records_and_ignores_case() {
        let mut h = History::default();
        let at = Stamp { year: 2026, month: 11, day: 17, hour: 21, minute: 4 };
        h.record(Path::new(r"C:\V\Clip.mp4"), at);
        h.record(Path::new(r"c:\v\clip.MP4"), at);
        let e = h.get(Path::new(r"C:\V\CLIP.mp4")).unwrap();
        assert_eq!(e.count, 2);
        assert_eq!(describe(e), "Exported 2 times, last 17-11-26 21-04");
    }

    #[test]
    fn catches_up_old_exports() {
        let mut h = History::default();
        let t = |d| Some(Stamp { year: 2026, month: 11, day: d, hour: 1, minute: 0 });
        let exports = vec![
            ("World Of Warcraft 2026.11.08 - 00.55.08.10.DVR_37.6-1m0.4s.mp4".to_string(), t(15)),
            ("World Of Warcraft 2026.11.08 - 00.55.08.10.DVR_10-20.mp4".to_string(), t(16)),
            ("Something else.mp4".to_string(), t(1)),
        ];
        let sources = [clip(r"C:\N\World Of Warcraft\World Of Warcraft 2026.11.08 - 00.55.08.10.DVR.mp4"), clip(r"C:\N\x.mp4")];
        assert_eq!(h.catch_up(sources.iter(), &exports), 1);
        let e = h.get(&sources[0].path).unwrap();
        assert_eq!((e.count, e.last.as_str()), (2, "16-11-26 01-00"));
        assert!(h.get(&sources[1].path).is_none() && h.caught_up);
    }
}
