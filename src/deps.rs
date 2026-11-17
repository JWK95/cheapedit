//! Finding and checking FFmpeg, ffprobe and libmpv.
//!
//! cheapedit ships its own copies in a `deps\` folder next to the exe. Those
//! are used first; the user's PATH is the fallback (or comes first, when they
//! choose "Use my own FFmpeg and libmpv from PATH").

use crate::mpv;
use crate::winutil;
use std::path::PathBuf;
use std::sync::Arc;

pub const MIN_FFMPEG_MAJOR: u32 = 6;

/// Where a dependency was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// cheapedit's own copy, in its `deps\` folder.
    Bundled,
    /// The user's own install, found on PATH.
    Path,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Bundled => "Bundled with cheapedit",
            Origin::Path => "Your own copy, from PATH",
        }
    }
}

/// cheapedit's bundled copies: `deps\` next to the exe. A development
/// build or test run uses the repo's `target\deps` (made by
/// tools\fetch-deps.ps1), so dev matches what the installer ships.
pub fn bundled_dir() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)) {
        dirs.push(exe_dir.join("deps"));
    }
    if winutil::is_dev() {
        dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("deps"));
    }
    dirs.into_iter().find(|d| d.join("ffmpeg.exe").is_file() || d.join("libmpv-2.dll").is_file())
}

/// What's bundled, from `deps\VERSIONS.txt` (one line per program).
pub fn bundled_versions() -> Vec<String> {
    bundled_dir()
        .and_then(|d| std::fs::read_to_string(d.join("VERSIONS.txt")).ok())
        .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// Places to look for `file`, best first.
fn candidates(file: &str, prefer_path: bool) -> Vec<(PathBuf, Origin)> {
    let bundled = bundled_dir().map(|d| d.join(file)).filter(|p| p.is_file()).map(|p| (p, Origin::Bundled));
    let on_path = winutil::find_on_path(file).map(|p| (p, Origin::Path));
    let (first, second) = if prefer_path { (on_path, bundled) } else { (bundled, on_path) };
    first.into_iter().chain(second).collect()
}

/// The path to use for `file` (e.g. "ffmpeg.exe"), without checking it works.
#[cfg(test)]
pub fn find(file: &str) -> Option<PathBuf> {
    candidates(file, false).into_iter().next().map(|c| c.0)
}

/// Try each place in turn: the first that works wins. If none works, the
/// first problem is reported (or Missing when there's nothing anywhere).
fn first_working<T>(cands: Vec<(PathBuf, Origin)>, check: impl Fn(PathBuf, Origin) -> (Status, Option<T>)) -> (Status, Option<T>) {
    let mut first_problem = None;
    for (p, origin) in cands {
        let (status, extra) = check(p, origin);
        if status.is_ok() {
            return (status, extra);
        }
        first_problem.get_or_insert(status);
    }
    (first_problem.unwrap_or(Status::Missing), None)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Found and usable. `version` is shown to the user.
    Ok { path: PathBuf, version: String, note: Option<String>, origin: Origin },
    /// Not bundled and not on PATH.
    Missing,
    /// Found but unusable, with a plain-English reason.
    Problem { path: PathBuf, reason: String },
}

impl Status {
    pub fn is_ok(&self) -> bool {
        matches!(self, Status::Ok { .. })
    }
    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            Status::Ok { path, .. } | Status::Problem { path, .. } => Some(path),
            Status::Missing => None,
        }
    }
}

pub struct Report {
    pub ffmpeg: Status,
    pub ffprobe: Status,
    pub libmpv: Status,
    /// Loaded libmpv, if it checked out.
    pub mpv_api: Option<Arc<mpv::Api>>,
}

impl Report {
    /// One line per dependency, for the log: which copy is used and from where.
    pub fn summary(&self) -> Vec<String> {
        [("FFmpeg", &self.ffmpeg), ("ffprobe", &self.ffprobe), ("libmpv", &self.libmpv)]
            .iter()
            .map(|(name, s)| match s {
                Status::Ok { path, version, origin, .. } => format!("{name} {version} ({}): {}", origin.label(), path.display()),
                Status::Missing => format!("{name}: not found"),
                Status::Problem { path, reason } => format!("{name}: {reason} ({})", path.display()),
            })
            .collect()
    }

    pub fn all_ok(&self) -> bool {
        self.ffmpeg.is_ok() && self.ffprobe.is_ok() && self.libmpv.is_ok() && self.mpv_api.is_some()
    }
}

/// Check all three. `prefer_path`: the user chose their own copies from PATH.
pub fn check_all(prefer_path: bool) -> Report {
    let ffmpeg = first_working(candidates("ffmpeg.exe", prefer_path), |p, o| (check_ff_tool(p, o, "ffmpeg"), None::<()>)).0;
    let ffprobe = first_working(candidates("ffprobe.exe", prefer_path), |p, o| (check_ff_tool(p, o, "ffprobe"), None::<()>)).0;
    let (libmpv, mpv_api) = check_libmpv(prefer_path);
    Report { ffmpeg, ffprobe, libmpv, mpv_api }
}

fn check_ff_tool(path: PathBuf, origin: Origin, name: &str) -> Status {
    let out = match winutil::hidden_command(&path).arg("-hide_banner").arg("-version").output() {
        Ok(o) => o,
        Err(e) => return Status::Problem { path, reason: format!("Windows couldn't run it ({e}).") },
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or("").to_string();
    let prefix = format!("{name} version ");
    let Some(ver) = first.strip_prefix(&prefix) else {
        return Status::Problem { path, reason: format!("This doesn't look like a real {name} (unexpected output).") };
    };
    let ver = ver.split_whitespace().next().unwrap_or("").to_string();
    match parse_major(&ver) {
        Some(major) if major >= MIN_FFMPEG_MAJOR => Status::Ok { path, version: ver, note: None, origin },
        Some(major) => Status::Problem {
            path,
            reason: format!("Version {major} is too old — cheapedit needs {MIN_FFMPEG_MAJOR}.0 or newer."),
        },
        // Git/nightly builds look like "N-12345-g…" or "2026-01-01-git-…": these are newer than any release.
        None => Status::Ok {
            path,
            version: ver,
            note: Some("Development build — assumed new enough.".into()),
            origin,
        },
    }
}

/// "8.1.1-essentials_build" -> 8. Returns None for git builds.
pub fn parse_major(ver: &str) -> Option<u32> {
    let digits: String = ver.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() >= 4 {
        return None; // "N-…" or a date like "2026-…"
    }
    // "7.1" style needs a dot after the major number.
    match ver.chars().nth(digits.len()) {
        Some('.') | Some('-') | None => digits.parse().ok(),
        _ => None,
    }
}

fn check_libmpv(prefer_path: bool) -> (Status, Option<Arc<mpv::Api>>) {
    let cands = candidates("libmpv-2.dll", prefer_path);
    if cands.is_empty() {
        // Common mix-up: the mpv *player* download is on PATH instead of the mpv-dev library.
        if let Some(player) = winutil::find_on_path("mpv.exe") {
            return (
                Status::Problem {
                    path: player.parent().map(|p| p.to_path_buf()).unwrap_or(player),
                    reason: "This folder is on your PATH, but it has the mpv player (mpv.exe), not the library. \
                             Download the file starting with mpv-dev-x86_64 instead and copy its libmpv-2.dll here."
                        .into(),
                },
                None,
            );
        }
        return (Status::Missing, None);
    }
    first_working(cands, load_libmpv)
}

fn load_libmpv(path: PathBuf, origin: Origin) -> (Status, Option<Arc<mpv::Api>>) {
    match mpv::Api::load(&path) {
        Ok(api) => {
            let (major, minor) = api.version();
            if major < 2 {
                return (
                    Status::Problem { path, reason: format!("libmpv API {major}.{minor} is too old — download a current build.") },
                    None,
                );
            }
            (Status::Ok { path, version: format!("client API {major}.{minor}"), note: None, origin }, Some(api))
        }
        Err(reason) => (Status::Problem { path, reason }, None),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_major;

    #[test]
    fn versions() {
        assert_eq!(parse_major("8.1.1-essentials_build-www.gyan.dev"), Some(8));
        assert_eq!(parse_major("6.0"), Some(6));
        assert_eq!(parse_major("7.1-full_build"), Some(7));
        assert_eq!(parse_major("N-118000-gabcdef"), None);
        assert_eq!(parse_major("2026-01-01-git-abc"), None);
    }
}
