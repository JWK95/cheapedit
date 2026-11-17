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
///
/// Explorer only understands `/select,"C:\path with spaces\file.mp4"`. The
/// normal `.arg()` would quote the whole thing as `"/select,C:\path..."`,
/// which Explorer ignores, opening Documents instead. So the argument is
/// passed raw. If the file is gone, its folder is opened instead.
pub fn reveal_in_explorer(file: &Path) {
    if !file.exists() {
        if let Some(dir) = file.parent() {
            open_folder(dir);
        }
        return;
    }
    let _ = Command::new("explorer").raw_arg(select_arg(file)).spawn();
}

fn select_arg(file: &Path) -> std::ffi::OsString {
    let mut arg = std::ffi::OsString::from("/select,\"");
    arg.push(file.as_os_str());
    arg.push("\"");
    arg
}

/// Open a folder in Explorer.
pub fn open_folder(dir: &Path) {
    let _ = Command::new("explorer").arg(dir).spawn();
}

/// A moment in this PC's local time (with its time zone and daylight saving).
pub fn local_stamp(t: std::time::SystemTime) -> Option<crate::job::Stamp> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // FILETIME counts 100 ns steps since 1601.
    let ticks = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos() as u64 / 100 + 116_444_736_000_000_000;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc) == 0 || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
            return None;
        }
    }
    Some(crate::job::Stamp { year: local.wYear, month: local.wMonth as u8, day: local.wDay as u8, hour: local.wHour as u8, minute: local.wMinute as u8 })
}

/// The local time now.
pub fn now_stamp() -> crate::job::Stamp {
    local_stamp(std::time::SystemTime::now()).unwrap_or(crate::job::Stamp { year: 2000, month: 1, day: 1, hour: 0, minute: 0 })
}

/// Put a file on the clipboard the way Explorer's Copy does (CF_HDROP),
/// so Ctrl+V pastes the file itself into Discord, Explorer, a browser…
pub fn copy_file_to_clipboard(file: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows_sys::Win32::UI::Shell::DROPFILES;
    const CF_HDROP: u32 = 15;

    if !file.is_file() {
        return Err("That file isn't there any more.".into());
    }
    let file = std::path::absolute(file).map_err(|_| "That file isn't there any more.".to_string())?;
    let plain = file.to_string_lossy().into_owned();
    let wide: Vec<u16> = std::ffi::OsStr::new(&plain).encode_wide().chain([0, 0]).collect();
    let header = std::mem::size_of::<DROPFILES>();
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, header + wide.len() * 2);
        if mem.is_null() {
            return Err("Windows ran out of memory for the clipboard.".into());
        }
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            GlobalFree(mem);
            return Err("Couldn't use the clipboard.".into());
        }
        let df = ptr as *mut DROPFILES;
        std::ptr::write_bytes(ptr, 0, header);
        (*df).pFiles = header as u32;
        (*df).fWide = 1;
        std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr.add(header), wide.len() * 2);
        GlobalUnlock(mem);
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            GlobalFree(mem);
            return Err("Another app is using the clipboard. Try again.".into());
        }
        EmptyClipboard();
        let ok = !SetClipboardData(CF_HDROP, mem).is_null();
        CloseClipboard();
        if !ok {
            GlobalFree(mem);
            return Err("Couldn't put the file on the clipboard.".into());
        }
    }
    Ok(())
}

/// Start dragging a file out of the window, as if from Explorer. Blocks
/// until it's dropped (Windows runs its own loop meanwhile). Must be
/// called while the left mouse button is held down.
pub fn drag_file_out(window: &impl raw_window_handle::HasWindowHandle, file: &Path) -> Result<(), String> {
    drag::start_drag(window, drag::DragItem::Files(vec![file.to_path_buf()]), drag::Image::Raw(Vec::new()), |_, _| {}, drag::Options::default())
        .map_err(|e| format!("Couldn't start dragging: {e}"))
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

#[cfg(test)]
mod tests {
    #[test]
    fn select_arg_quotes_only_the_path() {
        let arg = super::select_arg(std::path::Path::new(r"D:\My Clips\holiday clip_0-10.mp4"));
        assert_eq!(arg, r#"/select,"D:\My Clips\holiday clip_0-10.mp4""#);
    }
}
