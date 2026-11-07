//! Reading a video's details with ffprobe.

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct VideoInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub hdr: bool,
}

#[derive(Clone, Debug)]
pub struct AudioInfo {
    pub codec: String,
    pub sample_rate: u32,
    #[allow(dead_code)] // shown in the log via Debug
    pub channels: u32,
}

#[derive(Clone, Debug)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub duration: f64,
    #[allow(dead_code)] // kept for future size estimates
    pub size_bytes: u64,
    /// Overall bitrate in bits per second.
    pub bitrate: Option<u64>,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
}

impl MediaInfo {
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(v) = &self.video {
            parts.push(format!("{}×{}", v.width, v.height));
            parts.push(format!("{} fps", fmt_fps(v.fps)));
            parts.push(codec_display(&v.codec));
            if v.hdr {
                parts.push("HDR".into());
            }
        }
        if let Some(a) = &self.audio {
            parts.push(codec_display(&a.codec));
        } else {
            parts.push("no audio".into());
        }
        parts.join(" · ")
    }
}

pub fn fmt_fps(fps: f64) -> String {
    if (fps - fps.round()).abs() < 0.01 { format!("{}", fps.round() as u32) } else { format!("{fps:.2}") }
}

fn codec_display(c: &str) -> String {
    match c {
        "h264" => "H.264".into(),
        "hevc" => "H.265".into(),
        "av1" => "AV1".into(),
        "vp9" => "VP9".into(),
        "prores" => "ProRes".into(),
        "aac" => "AAC".into(),
        "mp3" => "MP3".into(),
        "opus" => "Opus".into(),
        "flac" => "FLAC".into(),
        other => other.to_uppercase(),
    }
}

/// Run ffprobe on a file.
pub fn probe(ffprobe: &Path, file: &Path) -> Result<MediaInfo, String> {
    let out = crate::winutil::hidden_command(ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(file)
        .output()
        .map_err(|e| format!("Couldn't run ffprobe: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(if err.contains("Invalid data") {
            "This file isn't a video or audio file FFmpeg can read.".into()
        } else {
            format!("FFmpeg couldn't read this file. {}", err.lines().last().unwrap_or("").trim())
        });
    }
    let json: Value = serde_json::from_slice(&out.stdout).map_err(|_| "ffprobe gave unreadable output.".to_string())?;
    let size_bytes = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);
    parse(&json, file, size_bytes)
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn ratio(s: &str) -> Option<f64> {
    let (a, b) = s.split_once('/')?;
    let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
    (b > 0.0 && a > 0.0).then(|| a / b)
}

pub fn parse(json: &Value, file: &Path, size_bytes: u64) -> Result<MediaInfo, String> {
    let streams = json["streams"].as_array().cloned().unwrap_or_default();
    let mut video = None;
    let mut audio = None;
    let mut stream_dur: f64 = 0.0;
    for s in &streams {
        if let Some(d) = num(&s["duration"]) {
            stream_dur = stream_dur.max(d);
        }
        match s["codec_type"].as_str() {
            // Skip cover art embedded in audio files.
            Some("video") if video.is_none() && s["disposition"]["attached_pic"].as_i64() != Some(1) => {
                let fps = s["avg_frame_rate"]
                    .as_str()
                    .and_then(ratio)
                    .or_else(|| s["r_frame_rate"].as_str().and_then(ratio))
                    .unwrap_or(30.0);
                let transfer = s["color_transfer"].as_str().unwrap_or("");
                video = Some(VideoInfo {
                    codec: s["codec_name"].as_str().unwrap_or("unknown").to_string(),
                    width: s["width"].as_u64().unwrap_or(0) as u32,
                    height: s["height"].as_u64().unwrap_or(0) as u32,
                    fps,
                    hdr: matches!(transfer, "smpte2084" | "arib-std-b67"),
                });
            }
            Some("audio") if audio.is_none() => {
                audio = Some(AudioInfo {
                    codec: s["codec_name"].as_str().unwrap_or("unknown").to_string(),
                    sample_rate: num(&s["sample_rate"]).unwrap_or(48000.0) as u32,
                    channels: s["channels"].as_u64().unwrap_or(2) as u32,
                });
            }
            _ => {}
        }
    }
    if video.is_none() && audio.is_none() {
        return Err("This file has no video or audio in it.".into());
    }
    let duration = num(&json["format"]["duration"]).unwrap_or(stream_dur);
    if duration <= 0.0 {
        return Err("Couldn't work out how long this file is.".into());
    }
    Ok(MediaInfo {
        path: file.to_path_buf(),
        duration,
        size_bytes,
        bitrate: num(&json["format"]["bit_rate"]).map(|b| b as u64),
        video,
        audio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffprobe_json() {
        let j: Value = serde_json::from_str(
            r#"{"streams":[
                {"codec_type":"video","codec_name":"hevc","width":1920,"height":1080,"avg_frame_rate":"30000/1001","color_transfer":"smpte2084"},
                {"codec_type":"audio","codec_name":"aac","sample_rate":"44100","channels":2}
              ],"format":{"duration":"60.5","bit_rate":"8000000"}}"#,
        )
        .unwrap();
        let m = parse(&j, Path::new("x.mp4"), 1000).unwrap();
        assert_eq!(m.duration, 60.5);
        let v = m.video.unwrap();
        assert!(v.hdr);
        assert!((v.fps - 29.97).abs() < 0.01);
        assert_eq!(m.audio.unwrap().sample_rate, 44100);
    }

    #[test]
    fn ignores_cover_art() {
        let j: Value = serde_json::from_str(
            r#"{"streams":[
                {"codec_type":"audio","codec_name":"mp3","sample_rate":"44100","channels":2},
                {"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}}
              ],"format":{"duration":"10"}}"#,
        )
        .unwrap();
        assert!(parse(&j, Path::new("x.mp3"), 1).unwrap().video.is_none());
    }
}
