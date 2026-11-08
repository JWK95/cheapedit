//! Checks run before every export, so problems are explained in plain
//! English instead of FFmpeg failing.

use crate::formats::{Caps, Container};
use crate::job::{AvMode, CutMode, Options};
use crate::probe::MediaInfo;
use crate::timecode;
use crate::zoom::{self, Zone, ZoneFix};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    UseEndOfVideo,
    SwitchToPrecise,
    ChooseFolder,
    SwitchToMkv,
    Zone(ZoneFix),
}

impl Fix {
    pub fn label(self) -> String {
        match self {
            Fix::UseEndOfVideo => "Use end of video".into(),
            Fix::SwitchToPrecise => "Switch to Precise".into(),
            Fix::ChooseFolder => "Choose folder".into(),
            Fix::SwitchToMkv => "Switch to MKV".into(),
            Fix::Zone(z) => z.label(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub message: String,
    pub fixes: Vec<Fix>,
}

fn p(message: impl Into<String>, fix: Option<Fix>) -> Problem {
    Problem { message: message.into(), fixes: fix.into_iter().collect() }
}

pub struct Input<'a> {
    pub media: Option<&'a MediaInfo>,
    pub start_text: &'a str,
    pub end_text: &'a str,
    pub until_end: bool,
    pub opts: &'a Options,
    pub output_dir: Option<&'a Path>,
    pub caps: Option<&'a Caps>,
    /// (estimated output bytes, free bytes on the output drive)
    pub space: Option<(u64, u64)>,
    pub dir_writable: bool,
    pub zones: &'a [Zone],
}

/// The trim range in seconds, if the boxes make sense.
pub fn trim_range(media: &MediaInfo, start_text: &str, end_text: &str, until_end: bool) -> Result<(f64, f64), Problem> {
    let start = timecode::parse(start_text).map_err(|e| p(format!("Start: {e}."), None))?;
    let end = if until_end { media.duration } else { timecode::parse(end_text).map_err(|e| p(format!("End: {e}."), None))? };
    if start >= media.duration {
        return Err(p(format!("Start is past the end — this video is only {} long.", timecode::format_short(media.duration)), None));
    }
    if end > media.duration + 0.0005 {
        return Err(p(format!("This video is only {} long.", timecode::format(media.duration)), Some(Fix::UseEndOfVideo)));
    }
    if start >= end {
        return Err(p("Start must be before end.", None));
    }
    Ok((start, end))
}

pub fn check(i: &Input) -> Vec<Problem> {
    let mut out = Vec::new();
    let Some(media) = i.media else {
        return vec![p("Open a video first.", None)];
    };
    if let Err(e) = trim_range(media, i.start_text, i.end_text, i.until_end) {
        out.push(e);
    }

    match i.output_dir {
        None => out.push(p("Choose an output folder — it's where your new clip will be saved.", Some(Fix::ChooseFolder))),
        Some(d) if !d.is_dir() => out.push(p("The output folder doesn't exist any more.", Some(Fix::ChooseFolder))),
        Some(_) if !i.dir_writable => out.push(p("cheapedit can't save into that folder (it's read-only or protected).", Some(Fix::ChooseFolder))),
        _ => {}
    }

    let o = i.opts;
    let container = o.output_container();
    if o.mode == CutMode::Fast && o.av != AvMode::AudioOnly {
        if let Some(v) = &media.video {
            if !container.can_copy_video(&v.codec) {
                let fix = if container != Container::Mkv && Container::Mkv.can_copy_video(&v.codec) { Some(Fix::SwitchToMkv) } else { Some(Fix::SwitchToPrecise) };
                out.push(p(
                    format!(
                        "{} can't hold this video's {} without re-encoding. Switch to Precise, or pick a format that can (MKV holds almost anything).",
                        container.label(),
                        v.codec.to_uppercase()
                    ),
                    fix,
                ));
            }
        }
    }

    match i.caps {
        None => out.push(p("Still checking which encoders your FFmpeg has…", None)),
        Some(caps) => {
            let encodes_video = o.mode == CutMode::Precise && o.av != AvMode::AudioOnly && media.video.is_some();
            if encodes_video {
                if let Some(reason) = caps.video_unavailable_reason(o.vcodec) {
                    out.push(p(reason, None));
                }
            }
            let acodec = if o.av == AvMode::AudioOnly { crate::formats::ACodec::Mp3 } else { o.acodec };
            let encodes_audio = media.audio.is_some() && o.av != AvMode::RemoveAudio && (o.mode == CutMode::Precise || o.av == AvMode::AudioOnly);
            if encodes_audio && !caps.audio_ok(acodec) {
                out.push(p(format!("Your FFmpeg build has no {} encoder.", acodec.label()), None));
            }
        }
    }

    // Zoom zones (ignored for audio-only exports, which have no picture).
    if !i.zones.is_empty() && o.av != AvMode::AudioOnly && media.video.is_some() {
        if o.mode == CutMode::Fast {
            out.push(p("Zoom zones need Precise mode — Fast mode copies the video untouched.", Some(Fix::SwitchToPrecise)));
        } else {
            let trim = trim_range(media, i.start_text, i.end_text, i.until_end).ok();
            for z in zoom::check(i.zones, trim) {
                out.push(Problem { message: z.message, fixes: z.fixes.into_iter().map(Fix::Zone).collect() });
            }
        }
    }

    if let Some((need, free)) = i.space {
        // Leave headroom: estimates are rough and Windows needs space too.
        let need = need + need / 5 + 50_000_000;
        if need > free {
            out.push(p(format!("Not enough free space: this export needs about {}, and the drive has {} free.", bytes(need), bytes(free)), None));
        }
    }
    out
}

pub fn bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 { format!("{:.1} GB", b / 1e9) } else if b >= 1e6 { format!("{:.0} MB", b / 1e6) } else { format!("{:.0} KB", b / 1e3) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{AudioInfo, VideoInfo};

    fn media(codec: &str) -> MediaInfo {
        MediaInfo {
            path: "x.mp4".into(),
            duration: 60.0,
            size_bytes: 1,
            bitrate: None,
            video: Some(VideoInfo { codec: codec.into(), width: 1920, height: 1080, fps: 30.0, hdr: false }),
            audio: Some(AudioInfo { codec: "aac".into(), sample_rate: 48000, channels: 2 }),
        }
    }

    fn run(m: &MediaInfo, s: &str, e: &str, o: &Options) -> Vec<Problem> {
        let caps = Caps { encoders: ["libx264", "aac", "libmp3lame"].iter().map(|s| s.to_string()).collect(), ..Default::default() };
        check(&Input {
            media: Some(m),
            start_text: s,
            end_text: e,
            until_end: false,
            opts: o,
            output_dir: Some(Path::new("C:\\")),
            caps: Some(&caps),
            space: Some((1_000, 10_000_000_000)),
            dir_writable: true,
            zones: &[],
        })
    }

    #[test]
    fn good_input_passes() {
        assert!(run(&media("h264"), "25", "45", &Options::default()).is_empty());
    }

    #[test]
    fn trim_problems() {
        let m = media("h264");
        assert_eq!(run(&m, "45", "25", &Options::default())[0].message, "Start must be before end.");
        assert_eq!(run(&m, "25", "1:30", &Options::default())[0].fixes, vec![Fix::UseEndOfVideo]);
    }

    #[test]
    fn fast_mode_webm_blocked() {
        let o = Options { mode: CutMode::Fast, container: Container::Webm, ..Default::default() };
        let probs = run(&media("hevc"), "1", "2", &o);
        assert_eq!(probs.len(), 1);
        assert_eq!(probs[0].fixes, vec![Fix::SwitchToMkv]);
    }

    #[test]
    fn missing_encoder_explained() {
        let o = Options { vcodec: crate::formats::VCodec::Vp9, container: Container::Webm, acodec: crate::formats::ACodec::Opus, ..Default::default() };
        let probs = run(&media("h264"), "1", "2", &o);
        assert_eq!(probs.len(), 2); // no VP9 and no Opus encoder in this fake build
    }
}
