//! Update checks against this project's GitHub releases.
//!
//! - The only network traffic cheapedit ever makes. It goes to github.com,
//!   sends nothing about you, and can be switched off.
//! - Uses Windows' own `curl.exe` (built into Windows 10/11), so cheapedit
//!   ships no networking code of its own.
//! - Nothing installs without a click. Downloads are checked against the
//!   SHA-256 published with the release before the installer is started.
//! - Dev builds never check.

use crate::log::Log;
use crate::winutil;
use eframe::egui;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const REPO: &str = "JWK95/cheapedit";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

pub fn releases_page() -> String {
    format!("https://github.com/{REPO}/releases")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub installer_name: String,
    pub installer_url: String,
    pub sha_url: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { release: Release, got: u64 },
    Ready { release: Release, file: PathBuf },
    /// The installer has been started; cheapedit is closing.
    Installing,
    Failed(String),
}

pub struct Updater {
    state: Arc<Mutex<State>>,
    log: Log,
    /// "Later" hides the banner for this session.
    pub dismissed: bool,
}

/// "v1.2.3" / "1.2.3" → (1, 2, 3). Anything after a '-' is ignored.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let s = s.split('-').next()?;
    let mut it = s.split('.').map(|p| p.parse::<u64>().ok());
    let v = (it.next()??, it.next().unwrap_or(Some(0))?, it.next().unwrap_or(Some(0))?);
    Some(v)
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Pick out what we need from GitHub's "latest release" JSON.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("GitHub sent something unexpected ({e})."))?;
    let tag = v["tag_name"].as_str().ok_or("The latest release has no version tag.")?;
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let name_of = |a: &serde_json::Value| a["name"].as_str().unwrap_or("").to_string();
    let installer = assets
        .iter()
        .find(|a| {
            let n = name_of(a).to_ascii_lowercase();
            n.starts_with("cheapedit-setup") && n.ends_with(".exe")
        })
        .ok_or("The latest release has no installer attached.")?;
    let installer_name = name_of(installer);
    let sha = assets
        .iter()
        .find(|a| name_of(a).eq_ignore_ascii_case(&format!("{installer_name}.sha256")))
        .ok_or("The latest release has no checksum file, so it can't be verified.")?;
    Ok(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        notes: v["body"].as_str().unwrap_or("").trim().to_string(),
        installer_url: installer["browser_download_url"].as_str().unwrap_or("").to_string(),
        sha_url: sha["browser_download_url"].as_str().unwrap_or("").to_string(),
        size: installer["size"].as_u64().unwrap_or(0),
        installer_name,
    })
}

/// First word of a `.sha256` file ("<hex>  <name>"), lowercased.
pub fn parse_sha_file(text: &str) -> Option<String> {
    let h = text.split_whitespace().next()?.to_ascii_lowercase();
    (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

fn curl() -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join(r"System32\curl.exe")
}

fn curl_base(max_secs: u32) -> std::process::Command {
    let mut c = winutil::hidden_command(&curl());
    c.args(["--silent", "--show-error", "--fail", "--location", "--proto", "=https", "--max-time", &max_secs.to_string()]);
    c.args(["-H", &format!("User-Agent: cheapedit/{CURRENT}")]);
    c
}

fn fetch_text(url: &str) -> Result<String, String> {
    let out = curl_base(20).arg(url).output().map_err(|e| format!("Couldn't run Windows' curl.exe ({e})."))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(friendly_curl_error(&err));
    }
    String::from_utf8(out.stdout).map_err(|_| "GitHub sent something unexpected.".into())
}

fn friendly_curl_error(err: &str) -> String {
    if err.contains("Could not resolve host") {
        "No internet connection (couldn't reach github.com).".into()
    } else if err.contains("timed out") {
        "GitHub took too long to answer. Try again later.".into()
    } else if err.contains("404") {
        "No releases have been published yet.".into()
    } else if err.contains("403") || err.contains("429") {
        "GitHub is limiting requests right now. Try again in an hour.".into()
    } else if err.is_empty() {
        "The download failed.".into()
    } else {
        format!("The download failed: {err}")
    }
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

impl Updater {
    pub fn new(log: Log) -> Updater {
        Updater { state: Arc::new(Mutex::new(State::Idle)), log, dismissed: false }
    }

    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn busy(&self) -> bool {
        matches!(self.state(), State::Checking | State::Downloading { .. } | State::Ready { .. } | State::Installing)
    }

    /// Ask GitHub for the latest release, in the background.
    pub fn check(&mut self, ctx: &egui::Context) {
        if winutil::is_dev() || self.busy() {
            return;
        }
        self.dismissed = false;
        *self.state.lock().unwrap() = State::Checking;
        let (state, log, ctx) = (self.state.clone(), self.log.clone(), ctx.clone());
        std::thread::spawn(move || {
            let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
            let result = fetch_text(&url).and_then(|j| parse_release(&j));
            let next = match result {
                Ok(r) if is_newer(&r.version, CURRENT) => {
                    log.info(format!("Update available: cheapedit {} (you have {CURRENT}).", r.version));
                    State::Available(r)
                }
                Ok(r) => {
                    log.info(format!("Update check: up to date (latest is {}).", r.version));
                    State::UpToDate
                }
                Err(e) => {
                    log.error(format!("Update check failed: {e}"));
                    State::Failed(e)
                }
            };
            *state.lock().unwrap() = next;
            ctx.request_repaint();
        });
    }

    /// Download the installer and its checksum, then verify it.
    pub fn download(&mut self, ctx: &egui::Context) {
        let State::Available(release) = self.state() else { return };
        *self.state.lock().unwrap() = State::Downloading { release: release.clone(), got: 0 };
        let (state, log, ctx) = (self.state.clone(), self.log.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = (|| -> Result<PathBuf, String> {
                let dir = std::env::temp_dir().join("cheapedit-update");
                let _ = std::fs::remove_dir_all(&dir);
                std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make a download folder ({e})."))?;
                // Only a plain file name from GitHub is used for the local path.
                let name = Path::new(&release.installer_name).file_name().ok_or("Bad installer name.")?;
                let file = dir.join(name);

                let expected = fetch_text(&release.sha_url).ok().and_then(|t| parse_sha_file(&t)).ok_or("Couldn't get the release checksum, so the download can't be verified.")?;

                let mut child = curl_base(600).arg("--output").arg(&file).arg(&release.installer_url).spawn().map_err(|e| format!("Couldn't run Windows' curl.exe ({e})."))?;
                let status = loop {
                    if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
                        break s;
                    }
                    let got = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                    *state.lock().unwrap() = State::Downloading { release: release.clone(), got };
                    ctx.request_repaint();
                    std::thread::sleep(Duration::from_millis(150));
                };
                if !status.success() {
                    return Err("The download failed. Check your connection and try again.".into());
                }
                let actual = sha256_file(&file).map_err(|e| format!("Couldn't read the download ({e})."))?;
                if actual != expected {
                    let _ = std::fs::remove_file(&file);
                    return Err("The download was damaged (checksum didn't match), so it was deleted. Try again.".into());
                }
                Ok(file)
            })();
            let next = match result {
                Ok(file) => {
                    log.info(format!("Downloaded and verified {}.", file.display()));
                    State::Ready { release, file }
                }
                Err(e) => {
                    log.error(format!("Update download failed: {e}"));
                    State::Failed(e)
                }
            };
            *state.lock().unwrap() = next;
            ctx.request_repaint();
        });
    }

    /// Start the verified installer. It updates in place, keeps your settings,
    /// and reopens cheapedit when it's done. The caller must then close the app.
    pub fn launch_installer(&mut self) -> Result<(), String> {
        let State::Ready { file, release } = self.state() else { return Err("Nothing to install.".into()) };
        std::process::Command::new(&file)
            .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/RELAUNCH=1"])
            .spawn()
            .map_err(|e| format!("Couldn't start the installer ({e})."))?;
        *self.state.lock().unwrap() = State::Installing;
        self.log.info(format!("Started the installer for cheapedit {}; closing so it can update.", release.version));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.2"), Some((0, 2, 0)));
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn picks_installer_and_checksum() {
        let json = r#"{"tag_name":"v0.2.0","body":"Notes","assets":[
            {"name":"cheapedit-Setup-0.2.0.exe.sha256","browser_download_url":"https://x/s","size":90},
            {"name":"cheapedit-Setup-0.2.0.exe","browser_download_url":"https://x/i","size":4000}]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(r.installer_url, "https://x/i");
        assert_eq!(r.sha_url, "https://x/s");
        assert_eq!(r.size, 4000);
    }

    #[test]
    fn refuses_release_without_checksum() {
        let json = r#"{"tag_name":"v0.2.0","assets":[{"name":"cheapedit-Setup-0.2.0.exe","browser_download_url":"https://x/i"}]}"#;
        assert!(parse_release(json).is_err());
    }

    #[test]
    fn sha_file() {
        let h = "a".repeat(64);
        assert_eq!(parse_sha_file(&format!("{h}  cheapedit-Setup.exe\n")), Some(h));
        assert_eq!(parse_sha_file("nope"), None);
    }
}
