//! The log shown in the in-app log viewer, also written to
//! `%APPDATA%\cheapedit\logs\cheapedit-YYYY-MM-DD.log`.

use std::io::Write;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    Command,
    Output,
    Error,
}

#[derive(Clone)]
pub struct Entry {
    pub time: String,
    pub kind: Kind,
    pub text: String,
}

#[derive(Default)]
struct Inner {
    entries: Vec<Entry>,
    file: Option<std::fs::File>,
}

/// Cheap to clone; shared between the UI and export threads.
#[derive(Clone, Default)]
pub struct Log(Arc<Mutex<Inner>>);

const MAX_ENTRIES: usize = 5000;

impl Log {
    pub fn new() -> Log {
        let dir = crate::winutil::app_data_dir().join("logs");
        let _ = std::fs::create_dir_all(&dir);
        let (date, _) = now_parts();
        let file = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("cheapedit-{date}.log"))).ok();
        Log(Arc::new(Mutex::new(Inner { entries: Vec::new(), file })))
    }

    pub fn dir() -> std::path::PathBuf {
        crate::winutil::app_data_dir().join("logs")
    }

    pub fn push(&self, kind: Kind, text: impl Into<String>) {
        let text = text.into();
        let (_, time) = now_parts();
        let mut g = self.0.lock().unwrap();
        if let Some(f) = g.file.as_mut() {
            let tag = match kind {
                Kind::Info => "INFO",
                Kind::Command => "CMD ",
                Kind::Output => "OUT ",
                Kind::Error => "ERR ",
            };
            let _ = writeln!(f, "{time} {tag} {text}");
        }
        g.entries.push(Entry { time, kind, text });
        // Keep memory bounded in long sessions.
        if g.entries.len() > MAX_ENTRIES {
            let extra = g.entries.len() - MAX_ENTRIES;
            g.entries.drain(..extra);
        }
    }

    pub fn info(&self, t: impl Into<String>) {
        self.push(Kind::Info, t)
    }
    pub fn error(&self, t: impl Into<String>) {
        self.push(Kind::Error, t)
    }

    pub fn snapshot(&self) -> Vec<Entry> {
        self.0.lock().unwrap().entries.clone()
    }
    pub fn clear(&self) {
        self.0.lock().unwrap().entries.clear();
    }
}

/// Local date "YYYY-MM-DD" and time "HH:MM:SS".
fn now_parts() -> (String, String) {
    let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
    (
        format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay),
        format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond),
    )
}
