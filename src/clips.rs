//! The Clips drawer's data: every video (and audio file) under the user's
//! clips folder, grouped by game, newest first.
//!
//! The game comes from (in order): the folder the clip is in (ShadowPlay
//! saves to `Videos\NVIDIA\<Game>\`), the text before the date in the file
//! name (Xbox Game Bar: `Valorant 2026-11-17 21-04-12.mp4`), or "Unsorted".

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Video files the drawer lists.
pub const EXTS: &[&str] = &["mp4", "mkv", "mov", "webm", "avi", "m4v", "wmv", "flv", "ts", "mts", "m2ts"];
/// Audio files the drawer lists too (each list has a checkbox to hide them).
/// Includes MP3, cheapedit's own audio-only export.
pub const AUDIO_EXTS: &[&str] = &["mp3", "m4a", "wav", "flac", "ogg", "opus"];

pub const UNSORTED: &str = "Unsorted";

/// Folder names that say nothing about the game, so the file name is used instead.
const GENERIC_FOLDERS: &[&str] = &[
    "captures", "clips", "replays", "replay", "videos", "video", "recordings", "recording", "highlights", "nvidia", "obs", "shadowplay", "cheapedit",
    "exports", "export", "output", "gamebar", "game bar", "medal", "outplayed",
];

/// Don't wander forever through huge folder trees.
const MAX_DEPTH: usize = 6;
const MAX_FILES: usize = 20_000;

#[derive(Clone, Debug)]
pub struct Clip {
    pub path: PathBuf,
    pub name: String,
    pub modified: SystemTime,
    pub size: u64,
    /// An audio file rather than a video.
    pub audio: bool,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub game: String,
    pub clips: Vec<Clip>,
}

/// Squash runs of spaces ("Call of Duty  Black Ops 6" has two, where
/// ShadowPlay dropped a colon) and trim separators off the ends.
fn tidy(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.trim_matches(|c: char| c == '-' || c == '_' || c == '.' || c == ' ').to_string()
}

/// Text before the first date in a file stem: `YYYY.MM.DD` / `YYYY-MM-DD` /
/// `YYYY_MM_DD` (recorders), or `dd-mm-yy hh-mm` (cheapedit's own names).
fn name_prefix(stem: &str) -> Option<String> {
    let b = stem.as_bytes();
    let digits = |r: std::ops::Range<usize>| r.clone().all(|i| b.get(i).is_some_and(u8::is_ascii_digit));
    let sep = |i: usize| b.get(i).is_some_and(|c| matches!(c, b'.' | b'-' | b'_'));
    for i in 0..b.len() {
        // Only start at a word boundary, so "Game2" years aren't split mid-word.
        if i > 0 && b[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        let recorder = digits(i..i + 4) && sep(i + 4) && digits(i + 5..i + 7) && sep(i + 7) && digits(i + 8..i + 10);
        let dash = |j: usize| b.get(j) == Some(&b'-');
        let ours = digits(i..i + 2)
            && dash(i + 2)
            && digits(i + 3..i + 5)
            && dash(i + 5)
            && digits(i + 6..i + 8)
            && b.get(i + 8) == Some(&b' ')
            && digits(i + 9..i + 11)
            && dash(i + 11)
            && digits(i + 12..i + 14);
        if recorder || ours {
            let p = tidy(&stem[..i]);
            return (!p.is_empty()).then_some(p);
        }
    }
    None
}

/// Which game a clip belongs to.
pub fn game_of(root: &Path, file: &Path) -> String {
    if let Some(parent) = file.parent() {
        if parent != root {
            if let Some(dir) = parent.file_name().map(|d| tidy(&d.to_string_lossy())) {
                if !dir.is_empty() && !GENERIC_FOLDERS.contains(&dir.to_lowercase().as_str()) {
                    return dir;
                }
            }
        }
    }
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    name_prefix(&stem).unwrap_or_else(|| UNSORTED.into())
}

/// The game to put in an export's name, if it can be worked out. Inside the
/// clips folder this follows the drawer's grouping; elsewhere only the file
/// name is trusted (a folder like "Downloads" isn't a game).
pub fn game_for_name(clips_dir: Option<&Path>, file: &Path) -> Option<String> {
    let game = match clips_dir {
        Some(root) if file.starts_with(root) => game_of(root, file),
        _ => name_prefix(&file.file_stem()?.to_string_lossy())?,
    };
    (game != UNSORTED).then_some(game)
}

/// Same folder? (Windows paths ignore case and a trailing slash.)
pub fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    norm(a) == norm(b)
}

/// `Some(false)` for a video, `Some(true)` for an audio file, `None` for anything else.
fn media_kind(p: &Path) -> Option<bool> {
    let e = p.extension()?.to_string_lossy().to_ascii_lowercase();
    if EXTS.contains(&e.as_str()) {
        Some(false)
    } else if AUDIO_EXTS.contains(&e.as_str()) {
        Some(true)
    } else {
        None
    }
}

/// How far to look.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanOpts {
    /// Only the folder itself, no subfolders (the Finished list).
    pub flat: bool,
    /// A folder to leave out (the output folder, from the Editing list).
    pub exclude: Option<PathBuf>,
}

fn walk(dir: &Path, depth: usize, opts: &ScanOpts, out: &mut Vec<Clip>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            // Skip hidden/system folders like "$RECYCLE.BIN".
            let hidden = entry.file_name().to_string_lossy().starts_with(['.', '$']);
            let excluded = opts.exclude.as_deref().is_some_and(|x| same_dir(x, &path));
            if !opts.flat && depth < MAX_DEPTH && !hidden && !excluded {
                walk(&path, depth + 1, opts, out);
            }
        } else if let (true, Some(audio)) = (ft.is_file(), media_kind(&path)) {
            let Ok(md) = entry.metadata() else { continue };
            out.push(Clip {
                audio,
                name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                modified: md.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                size: md.len(),
                path,
            });
        }
    }
}

/// Group clips by game: newest clip first in each group, and the group with
/// the newest clip first. "Unsorted" always goes last.
pub fn group(root: &Path, clips: Vec<Clip>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for c in clips {
        let game = game_of(root, &c.path);
        match groups.iter_mut().find(|g| g.game.eq_ignore_ascii_case(&game)) {
            Some(g) => g.clips.push(c),
            None => groups.push(Group { game, clips: vec![c] }),
        }
    }
    for g in &mut groups {
        g.clips.sort_by(|a, b| b.modified.cmp(&a.modified));
    }
    let newest = |g: &Group| g.clips.first().map(|c| c.modified).unwrap_or(SystemTime::UNIX_EPOCH);
    groups.sort_by(|a, b| (a.game == UNSORTED).cmp(&(b.game == UNSORTED)).then(newest(b).cmp(&newest(a))));
    groups
}

pub fn scan(root: &Path, opts: &ScanOpts) -> Vec<Group> {
    let mut clips = Vec::new();
    walk(root, 0, opts, &mut clips);
    group(root, clips)
}

/// Scans in the background so a big folder never freezes the window.
#[derive(Default)]
pub struct Library {
    root: Option<PathBuf>,
    result: Arc<Mutex<Option<(PathBuf, Vec<Group>)>>>,
    scanning: Arc<std::sync::atomic::AtomicBool>,
    /// A rescan asked for while one was running; started when it finishes.
    pending: Option<(PathBuf, ScanOpts)>,
    pub groups: Vec<Group>,
}

impl Library {
    pub fn scanning(&self) -> bool {
        self.scanning.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn rescan(&mut self, root: &Path, opts: ScanOpts, ctx: &eframe::egui::Context) {
        if self.root.as_deref() != Some(root) {
            self.groups.clear();
            self.root = Some(root.to_path_buf());
        }
        if self.scanning() {
            self.pending = Some((root.to_path_buf(), opts));
            return;
        }
        self.scanning.store(true, std::sync::atomic::Ordering::SeqCst);
        let (root, result, scanning, ctx) = (root.to_path_buf(), self.result.clone(), self.scanning.clone(), ctx.clone());
        std::thread::spawn(move || {
            let groups = scan(&root, &opts);
            *result.lock().unwrap() = Some((root, groups));
            scanning.store(false, std::sync::atomic::Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    /// Pick up a finished scan (ignoring one for a folder no longer chosen).
    /// True when the list changed.
    pub fn poll(&mut self, ctx: &eframe::egui::Context) -> bool {
        let done = self.result.lock().unwrap().take();
        if !self.scanning() {
            if let Some((root, opts)) = self.pending.take() {
                self.rescan(&root, opts, ctx);
            }
        }
        if let Some((root, groups)) = done {
            if self.root.as_deref() == Some(root.as_path()) {
                self.groups = groups;
                return true;
            }
        }
        false
    }

    /// Every clip, in no particular order.
    pub fn all(&self) -> impl Iterator<Item = &Clip> {
        self.groups.iter().flat_map(|g| g.clips.iter())
    }
}

/// "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago", or a date.
pub fn relative_time(t: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        86_400..172_800 => "yesterday".into(),
        172_800..2_592_000 => format!("{} days ago", secs / 86_400),
        _ => {
            let days = t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
            let (y, m, d) = civil_from_days(days);
            format!("{d} {} {y}", ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(m - 1) as usize])
        }
    }
}

/// Days since 1970-01-01 → (year, month, day). (Howard Hinnant's algorithm.)
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn game_from_shadowplay_folder() {
        let root = Path::new(r"C:\Users\x\Videos\NVIDIA");
        let f = Path::new(r"C:\Users\x\Videos\NVIDIA\Call of Duty  Black Ops 6\Call of Duty  Black Ops 6 2026.11.16 - 22.57.07.17.DVR.mp4");
        assert_eq!(game_of(root, f), "Call of Duty Black Ops 6");
        // Choosing the parent Videos folder still finds the game folder.
        assert_eq!(game_of(Path::new(r"C:\Users\x\Videos"), f), "Call of Duty Black Ops 6");
    }

    #[test]
    fn game_from_file_name() {
        let root = Path::new(r"C:\Clips");
        assert_eq!(game_of(root, Path::new(r"C:\Clips\Valorant 2026-11-17 21-04-12.mp4")), "Valorant");
        assert_eq!(game_of(root, Path::new(r"C:\Clips\Captures\Ready Or Not 2026.07.21 - 23.22.26.07.DVR.mp4")), "Ready Or Not");
        // cheapedit's own exports keep the source name, so they land with their game.
        assert_eq!(
            game_of(root, Path::new(r"C:\Clips\Cheapedit\World Of Warcraft 2026.11.08 - 00.55.08.10.DVR_37.6-1m0.4s.mp4")),
            "World Of Warcraft"
        );
        assert_eq!(game_of(root, Path::new(r"C:\Clips\holiday.mp4")), UNSORTED);
        assert_eq!(game_of(root, Path::new(r"C:\Clips\2026-11-17 21-04-12.mp4")), UNSORTED);
        // cheapedit's new names: game, then dd-mm-yy hh-mm (maybe with a (2)).
        assert_eq!(game_of(root, Path::new(r"C:\Clips\Call of Duty Black Ops 6 17-11-26 21-04 (2).mp4")), "Call of Duty Black Ops 6");
        assert_eq!(game_of(root, Path::new(r"C:\Clips\firefox 17-11-26 21-04.mp4")), "firefox");
    }

    #[test]
    fn game_for_export_names() {
        let clips = Path::new(r"C:\Videos\NVIDIA");
        let inside = Path::new(r"C:\Videos\NVIDIA\Bodycam\Bodycam 2026.11.16 - 19.44.02.15.DVR.mp4");
        assert_eq!(game_for_name(Some(clips), inside).as_deref(), Some("Bodycam"));
        // Outside the clips folder, the folder name isn't trusted.
        assert_eq!(game_for_name(Some(clips), Path::new(r"C:\Downloads\funny.mp4")), None);
        assert_eq!(game_for_name(None, Path::new(r"C:\Downloads\Halo Infinite 2026.07.30 - 00.19.52.08.DVR.mp4")).as_deref(), Some("Halo Infinite"));
        assert!(same_dir(Path::new(r"C:\Videos\Cheapedit\"), Path::new(r"c:\videos\cheapedit")));
    }

    #[test]
    fn groups_newest_first() {
        let root = Path::new(r"C:\C");
        let t = |s: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        let clip = |p: &str, s: u64| Clip { path: PathBuf::from(p), name: p.into(), modified: t(s), size: 1, audio: false };
        let g = group(
            root,
            vec![
                clip(r"C:\C\A\a1.mp4", 10),
                clip(r"C:\C\x.mp4", 99),
                clip(r"C:\C\B\b1.mp4", 50),
                clip(r"C:\C\a\a2.mp4", 30),
            ],
        );
        let names: Vec<_> = g.iter().map(|g| g.game.as_str()).collect();
        assert_eq!(names, ["B", "A", UNSORTED], "newest group first, Unsorted last, folder names merged ignoring case");
        assert_eq!(g[1].clips[0].modified, t(30));
    }

    /// Issue #5: audio-only (MP3) exports were missing from Finished files.
    #[test]
    fn scan_finds_audio_files() {
        let dir = std::env::temp_dir().join(format!("cheapedit_scan_audio_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["clip.mp4", "voice.MP3", "notes.txt"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let mut found: Vec<(String, bool)> = scan(&dir, &ScanOpts { flat: true, exclude: None }).into_iter().flat_map(|g| g.clips).map(|c| (c.name, c.audio)).collect();
        found.sort();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found, [("clip.mp4".to_string(), false), ("voice.MP3".to_string(), true)]);
    }

    /// Prints how a real folder groups. Run with
    /// `CHEAPEDIT_CLIPS_DIR=... cargo test scan_real_folder -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn scan_real_folder() {
        let dir = PathBuf::from(std::env::var_os("CHEAPEDIT_CLIPS_DIR").expect("set CHEAPEDIT_CLIPS_DIR"));
        let started = std::time::Instant::now();
        let groups = scan(&dir, &ScanOpts::default());
        println!("{} groups in {:?}", groups.len(), started.elapsed());
        for g in &groups {
            println!("{} ({}): newest {}", g.game, g.clips.len(), g.clips[0].name);
        }
    }

    #[test]
    fn relative_times() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let ago = |s| relative_time(now - Duration::from_secs(s), now);
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(7200), "2 h ago");
        assert_eq!(ago(100_000), "yesterday");
        assert_eq!(relative_time(SystemTime::UNIX_EPOCH + Duration::from_secs(86_400 * 20_409), now), "17 Nov 2025");
    }
}
