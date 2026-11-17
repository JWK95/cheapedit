//! The fonts installed in Windows, for text overlays. Read from the registry
//! (machine-wide and per-user installs); nothing is changed.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    /// e.g. "Arial Bold"
    pub name: String,
    pub file: PathBuf,
}

/// "Arial Bold (TrueType)" -> "Arial Bold".
fn clean_name(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for suffix in [" (TrueType)", " (OpenType)", " (All res)"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            s = stripped.to_string();
        }
    }
    s
}

fn usable(file: &std::path::Path) -> bool {
    file.extension().and_then(|e| e.to_str()).is_some_and(|e| ["ttf", "otf", "ttc"].contains(&e.to_ascii_lowercase().as_str()))
}

/// Every usable font, sorted by name.
pub fn installed() -> Vec<Font> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let system_fonts = windir.join("Fonts");
    let mut out: Vec<Font> = Vec::new();
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let Ok(key) = RegKey::predef(hive).open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts") else { continue };
        for (name, value) in key.enum_values().flatten() {
            let file = PathBuf::from(value.to_string().trim_matches('\0').trim());
            // Machine-wide entries are just file names in Windows\Fonts.
            let file = if file.is_absolute() { file } else { system_fonts.join(file) };
            if usable(&file) && file.is_file() {
                out.push(Font { name: clean_name(&name), file });
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_lose_their_type() {
        assert_eq!(clean_name("Arial Bold (TrueType)"), "Arial Bold");
        assert_eq!(clean_name("Bahnschrift (OpenType)"), "Bahnschrift");
    }

    #[test]
    fn windows_has_fonts() {
        let f = installed();
        assert!(f.iter().any(|f| f.name == "Arial"), "{} fonts", f.len());
    }
}
