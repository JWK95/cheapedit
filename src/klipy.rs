//! KLIPY GIF search (https://docs.klipy.com/gifs-api). Uses the user's own
//! API key and Windows' own curl.exe, like the update check. Only the search
//! text and the key are sent.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const BASE: &str = "https://api.klipy.com/api/v1";
const PER_PAGE: u32 = 24;

/// One search result.
#[derive(Clone, Debug, PartialEq)]
pub struct Gif {
    pub id: String,
    pub title: String,
    /// Small animated preview for the results grid.
    pub thumb_url: String,
    pub thumb_ext: String,
    /// The file to download: WebP when offered (smaller, keeps transparency), else GIF.
    pub file_url: String,
    pub file_ext: String,
    pub width: u32,
    pub height: u32,
}

/// Letters, digits, `-`, `_`, `.` and `~` stay; everything else is %XX.
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The search URL, or trending when `q` is blank. Pages count from 1.
pub fn url(key: &str, q: &str, page: u32) -> String {
    let key = url_encode(key.trim());
    let q = q.trim();
    if q.is_empty() {
        format!("{BASE}/{key}/gifs/trending?page={page}&per_page={PER_PAGE}&content_filter=off")
    } else {
        format!("{BASE}/{key}/gifs/search?page={page}&per_page={PER_PAGE}&q={}&content_filter=off", url_encode(q))
    }
}

/// `file.<size>.<format>` -> (url, width, height)
fn pick(file: &Value, size: &str, format: &str) -> Option<(String, u32, u32)> {
    let f = file.get(size)?.get(format)?;
    let url = f.get("url")?.as_str()?.to_string();
    let w = f.get("width").and_then(Value::as_u64).unwrap_or(0) as u32;
    let h = f.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
    (!url.is_empty()).then_some((url, w, h))
}

/// Results and whether there's another page.
pub fn parse(json: &str) -> Result<(Vec<Gif>, bool), String> {
    let v: Value = serde_json::from_str(json).map_err(|_| "KLIPY sent something unexpected.".to_string())?;
    if v.get("result").and_then(Value::as_bool) != Some(true) {
        let msg = v.get("message").and_then(Value::as_str).unwrap_or("KLIPY refused the search.");
        return Err(msg.to_string());
    }
    let data = v.get("data").ok_or("KLIPY sent something unexpected.")?;
    let has_next = data.get("has_next").and_then(Value::as_bool).unwrap_or(false);
    let mut out = Vec::new();
    for item in data.get("data").and_then(Value::as_array).into_iter().flatten() {
        // Ads and other non-GIF items are left out.
        if item.get("type").and_then(Value::as_str).is_some_and(|t| t != "gif") {
            continue;
        }
        let Some(file) = item.get("file") else { continue };
        let id = match item.get("id") {
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::String(s)) => s.clone(),
            _ => item.get("slug").and_then(Value::as_str).unwrap_or("").to_string(),
        };
        let thumb = ["sm", "xs", "md"].iter().find_map(|s| pick(file, s, "webp").map(|t| (t, "webp")).or_else(|| pick(file, s, "gif").map(|t| (t, "gif"))));
        let full = ["hd", "md", "sm"].iter().find_map(|s| pick(file, s, "webp").map(|t| (t, "webp")).or_else(|| pick(file, s, "gif").map(|t| (t, "gif"))));
        let (Some((thumb, te)), Some((full, fe))) = (thumb, full) else { continue };
        out.push(Gif {
            id,
            title: item.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
            thumb_url: thumb.0,
            thumb_ext: te.into(),
            file_url: full.0,
            file_ext: fe.into(),
            width: full.1,
            height: full.2,
        });
    }
    Ok((out, has_next))
}

fn friendly(err: &str) -> String {
    if err.contains("Could not resolve host") {
        "No internet connection (couldn't reach KLIPY).".into()
    } else if err.contains("timed out") {
        "KLIPY took too long to answer. Try again.".into()
    } else if err.contains("401") || err.contains("403") || err.contains("404") {
        "KLIPY didn't accept your API key. Check it in Features.".into()
    } else if err.contains("429") {
        "KLIPY's limit for your key is used up for now (test keys allow 100 searches an hour). Try again later.".into()
    } else if err.is_empty() {
        "The search failed.".into()
    } else {
        format!("The search failed: {err}")
    }
}

fn fetch(url: &str) -> Result<String, String> {
    let out = crate::update::curl_base(20).arg(url).output().map_err(|e| format!("Couldn't run Windows' curl.exe ({e})."))?;
    if !out.status.success() {
        return Err(friendly(String::from_utf8_lossy(&out.stderr).trim()));
    }
    String::from_utf8(out.stdout).map_err(|_| "KLIPY sent something unexpected.".into())
}

/// Download `url` to `file` (via a .part file, so a failed download leaves nothing).
pub fn download(url: &str, file: &Path) -> Result<(), String> {
    let part = file.with_extension("part");
    let out = crate::update::curl_base(120).arg("--output").arg(&part).arg(url).output().map_err(|e| format!("Couldn't run Windows' curl.exe ({e})."))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(friendly(String::from_utf8_lossy(&out.stderr).trim()).replace("The search failed", "The download failed"));
    }
    std::fs::rename(&part, file).map_err(|e| format!("Couldn't save the GIF: {e}"))
}

/// `gg-ez_8421.webp`: the search words and the GIF's id, safe for Windows.
pub fn file_name(q: &str, title: &str, id: &str, ext: &str) -> String {
    let words = if q.trim().is_empty() { title } else { q };
    let mut base: String = words
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    base.truncate(40);
    if base.is_empty() {
        base = "klipy".into();
    }
    let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("{base}_{id}.{ext}")
}

#[derive(Default)]
pub struct State {
    pub query: String,
    pub results: Vec<Gif>,
    pub page: u32,
    pub has_next: bool,
    pub busy: bool,
    pub error: Option<String>,
    /// Bumped by each new search, so a slow old answer is thrown away.
    pub generation: u64,
}

/// Searches in the background.
#[derive(Clone, Default)]
pub struct Search {
    pub state: Arc<Mutex<State>>,
}

impl Search {
    /// Start a new search (blank = trending), or `more` for the next page.
    pub fn run(&self, key: &str, q: &str, more: bool, ctx: &eframe::egui::Context) {
        let (url, generation) = {
            let mut s = self.state.lock().unwrap();
            if more {
                if !s.has_next || s.busy {
                    return;
                }
            } else {
                s.generation += 1;
                s.query = q.to_string();
                s.results.clear();
                s.page = 0;
                s.has_next = false;
            }
            s.busy = true;
            s.error = None;
            (url(key, &s.query, s.page + 1), s.generation)
        };
        let (state, ctx) = (self.state.clone(), ctx.clone());
        std::thread::spawn(move || {
            let res = fetch(&url).and_then(|j| parse(&j));
            let mut s = state.lock().unwrap();
            if s.generation != generation {
                return;
            }
            s.busy = false;
            match res {
                Ok((items, next)) => {
                    s.page += 1;
                    s.results.extend(items);
                    s.has_next = next;
                }
                Err(e) => s.error = Some(e),
            }
            ctx.request_repaint();
        });
    }
}

/// Thumbnails are kept in the temp folder while the app runs.
pub fn thumb_path(g: &Gif) -> PathBuf {
    let id: String = g.id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    std::env::temp_dir().join("cheapedit-klipy").join(format!("{id}.{}", g.thumb_ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"result":true,"data":{"data":[
        {"id":8041071659142944,"slug":"hello-hi-662","title":"Hello","type":"gif","file":{
          "hd":{"gif":{"url":"https://s/hd.gif","width":498,"height":498},"webp":{"url":"https://s/hd.webp","width":498,"height":400}},
          "sm":{"gif":{"url":"https://s/sm.gif","width":220,"height":220},"webp":{"url":"https://s/sm.webp","width":220,"height":220}}}},
        {"id":"x1","title":"Gif only","file":{"md":{"gif":{"url":"https://s/md.gif","width":300,"height":200}}}},
        {"type":"ad","id":5,"file":{}}
      ],"current_page":1,"per_page":24,"has_next":true}}"#;

    #[test]
    fn parses_results() {
        let (items, next) = parse(SAMPLE).unwrap();
        assert!(next);
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].file_url.as_str(), items[0].file_ext.as_str(), items[0].height), ("https://s/hd.webp", "webp", 400));
        assert_eq!(items[0].thumb_url, "https://s/sm.webp");
        assert_eq!(items[0].id, "8041071659142944");
        assert_eq!((items[1].file_url.as_str(), items[1].file_ext.as_str()), ("https://s/md.gif", "gif"));
        assert!(parse(r#"{"result":false,"message":"Invalid key"}"#).unwrap_err().contains("Invalid key"));
    }

    #[test]
    fn urls() {
        assert_eq!(url("abc", "", 1), "https://api.klipy.com/api/v1/abc/gifs/trending?page=1&per_page=24&content_filter=off");
        assert_eq!(url("abc", " gg ez! ", 2), "https://api.klipy.com/api/v1/abc/gifs/search?page=2&per_page=24&q=gg%20ez%21&content_filter=off");
    }

    #[test]
    fn names() {
        assert_eq!(file_name("GG ez!", "", "8421", "webp"), "gg-ez_8421.webp");
        assert_eq!(file_name("", "Hello: World", "x/1", "gif"), "hello-world_x1.gif");
        assert_eq!(file_name("???", "", "7", "gif"), "klipy_7.gif");
    }
}
