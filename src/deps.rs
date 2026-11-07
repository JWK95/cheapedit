//! Finding and checking FFmpeg, ffprobe and libmpv on the user's PATH.

use crate::mpv;
use crate::winutil;
use std::path::PathBuf;
use std::sync::Arc;

pub const MIN_FFMPEG_MAJOR: u32 = 6;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Found and usable. `version` is shown to the user.
    Ok { path: PathBuf, version: String, note: Option<String> },
    /// Not on PATH at all.
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
    pub fn all_ok(&self) -> bool {
        self.ffmpeg.is_ok() && self.ffprobe.is_ok() && self.libmpv.is_ok() && self.mpv_api.is_some()
    }
}

pub fn check_all() -> Report {
    let ffmpeg = check_ff_tool("ffmpeg.exe", "ffmpeg");
    let ffprobe = check_ff_tool("ffprobe.exe", "ffprobe");
    let (libmpv, mpv_api) = check_libmpv();
    Report { ffmpeg, ffprobe, libmpv, mpv_api }
}

fn check_ff_tool(file: &str, name: &str) -> Status {
    let Some(path) = winutil::find_on_path(file) else { return Status::Missing };
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
        Some(major) if major >= MIN_FFMPEG_MAJOR => Status::Ok { path, version: ver, note: None },
        Some(major) => Status::Problem {
            path,
            reason: format!("Version {major} is too old — cheapedit needs {MIN_FFMPEG_MAJOR}.0 or newer."),
        },
        // Git/nightly builds look like "N-12345-g…" or "2026-01-01-git-…": these are newer than any release.
        None => Status::Ok {
            path,
            version: ver,
            note: Some("Development build — assumed new enough.".into()),
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

fn check_libmpv() -> (Status, Option<Arc<mpv::Api>>) {
    let Some(path) = winutil::find_on_path("libmpv-2.dll") else {
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
    };
    match mpv::Api::load(&path) {
        Ok(api) => {
            let (major, minor) = api.version();
            if major < 2 {
                return (
                    Status::Problem { path, reason: format!("libmpv API {major}.{minor} is too old — download a current build.") },
                    None,
                );
            }
            (Status::Ok { path, version: format!("client API {major}.{minor}"), note: None }, Some(api))
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
