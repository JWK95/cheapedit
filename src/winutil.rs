//! The only module that talks to Windows directly.
//! Everything here is read-only: cheapedit never changes system settings.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Stops a console window flashing up when we start ffmpeg/ffprobe.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A `Command` that runs hidden (no console window).
pub fn hidden_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// The PATH as it is *right now* in the registry (system + user), plus this
/// process's own PATH. Reading the registry means "Check again" sees PATH
/// edits the user just made, without restarting cheapedit.
pub fn current_path_dirs() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let mut raw: Vec<String> = Vec::new();
    if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment")
    {
        if let Ok(v) = k.get_value::<String, _>("Path") {
            raw.push(v);
        }
    }
    if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment") {
        if let Ok(v) = k.get_value::<String, _>("Path") {
            raw.push(v);
        }
    }
    if let Ok(v) = std::env::var("PATH") {
        raw.push(v);
    }

    let mut out: Vec<PathBuf> = Vec::new();
    for list in raw {
        for entry in list.split(';') {
            let entry = expand_env(entry.trim().trim_matches('"'));
            if entry.is_empty() {
                continue;
            }
            let p = PathBuf::from(entry);
            if !out.iter().any(|q| q.as_os_str().eq_ignore_ascii_case(p.as_os_str())) {
                out.push(p);
            }
        }
    }
    out
}

/// Expand `%VAR%` references (registry PATH values are often unexpanded).
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Find a file (e.g. `ffmpeg.exe`) in the current PATH.
pub fn find_on_path(file: &str) -> Option<PathBuf> {
    current_path_dirs().into_iter().map(|d| d.join(file)).find(|p| p.is_file())
}

/// Free bytes available to this user on the drive holding `dir`.
pub fn free_disk_space(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free: u64 = 0;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

/// Can we create files in `dir`? (Checks by actually trying.)
pub fn is_writable_dir(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(format!(".cheapedit-write-test-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Open Explorer with `file` selected.
pub fn reveal_in_explorer(file: &Path) {
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(file.as_os_str());
    let _ = Command::new("explorer").arg(arg).spawn();
}

/// Open a folder in Explorer.
pub fn open_folder(dir: &Path) {
    let _ = Command::new("explorer").arg(dir).spawn();
}

/// True for development builds (`cargo build` / `cargo run`, no `--release`).
/// Dev builds keep their own settings and logs and never check for updates,
/// so testing never disturbs the installed copy.
pub const fn is_dev() -> bool {
    cfg!(debug_assertions)
}

/// `%APPDATA%\cheapedit` (or `cheapedit-dev` for dev builds), created if missing.
pub fn app_data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join(if is_dev() { "cheapedit-dev" } else { "cheapedit" });
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// OpenGL function lookup for libmpv's renderer (needs the GL context current).
pub unsafe fn gl_proc_address(name: &std::ffi::CStr) -> *mut std::ffi::c_void {
    use windows_sys::Win32::Graphics::OpenGL::wglGetProcAddress;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    unsafe {
        let p = wglGetProcAddress(name.as_ptr() as *const u8);
        if let Some(f) = p {
            let addr = f as usize;
            if !matches!(addr, 1 | 2 | 3 | usize::MAX) {
                return addr as *mut _;
            }
        }
        let lib = LoadLibraryA(c"opengl32.dll".as_ptr() as *const u8);
        if lib.is_null() {
            return std::ptr::null_mut();
        }
        match GetProcAddress(lib, name.as_ptr() as *const u8) {
            Some(f) => f as usize as *mut _,
            None => std::ptr::null_mut(),
        }
    }
}
