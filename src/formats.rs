//! Formats, codecs and encoders, plus the plain-English hints shown in the UI.
//!
//! The combination tables here were verified by test encodes against a real
//! FFmpeg build (see `tools/verify_combos.ps1`). Anything not listed would break.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Container {
    Mp4,
    Mkv,
    Webm,
    Mov,
    Mp3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VCodec {
    H264,
    H265,
    Av1,
    Vp9,
    ProRes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ACodec {
    Aac,
    Mp3,
    Opus,
    Flac,
    Vorbis,
    Pcm,
}

impl Container {
    pub const ALL: [Container; 5] = [Container::Mp4, Container::Mkv, Container::Webm, Container::Mov, Container::Mp3];

    pub fn label(self) -> &'static str {
        match self {
            Container::Mp4 => "MP4",
            Container::Mkv => "MKV",
            Container::Webm => "WebM",
            Container::Mov => "MOV",
            Container::Mp3 => "MP3 (audio only)",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
            Container::Mov => "mov",
            Container::Mp3 => "mp3",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Container::Mp4 => "Plays almost everywhere. Best choice if unsure.",
            Container::Mkv => "Flexible, great for archiving. Some phones and editing apps won't open it.",
            Container::Webm => "For websites and embeds.",
            Container::Mov => "Apple and editing-software friendly.",
            Container::Mp3 => "Audio only.",
        }
    }
    pub fn is_audio_only(self) -> bool {
        self == Container::Mp3
    }
    pub fn video_codecs(self) -> &'static [VCodec] {
        use VCodec::*;
        match self {
            Container::Mp4 => &[H264, H265, Av1, Vp9],
            Container::Mkv => &[H264, H265, Av1, Vp9, ProRes],
            Container::Webm => &[Vp9, Av1],
            Container::Mov => &[H264, H265, ProRes],
            Container::Mp3 => &[],
        }
    }
    pub fn audio_codecs(self) -> &'static [ACodec] {
        use ACodec::*;
        match self {
            Container::Mp4 => &[Aac, Mp3, Opus, Flac, Vorbis, Pcm],
            Container::Mkv => &[Aac, Mp3, Opus, Flac, Vorbis, Pcm],
            Container::Webm => &[Opus, Vorbis],
            Container::Mov => &[Aac, Mp3, Vorbis, Pcm],
            Container::Mp3 => &[Mp3],
        }
    }
    pub fn default_audio(self) -> ACodec {
        self.audio_codecs()[0]
    }
    /// Does copying a source stream of this ffprobe codec name into this
    /// container work (Fast mode)?
    pub fn can_copy_video(self, ffprobe_codec: &str) -> bool {
        match VCodec::from_ffprobe(ffprobe_codec) {
            Some(v) => self.video_codecs().contains(&v),
            // Unusual codecs (mpeg4, mjpeg, …): only Matroska reliably takes them.
            None => self == Container::Mkv,
        }
    }
    pub fn can_copy_audio(self, ffprobe_codec: &str) -> bool {
        match ACodec::from_ffprobe(ffprobe_codec) {
            // PCM comes in many sample formats; only MOV and MKV take them all as a copy.
            Some(ACodec::Pcm) => matches!(self, Container::Mov | Container::Mkv),
            Some(a) => self.audio_codecs().contains(&a),
            None => self == Container::Mkv,
        }
    }
    /// A pitfall for this container + codec pairing, if any.
    pub fn video_pitfall(self, v: VCodec) -> Option<&'static str> {
        match (self, v) {
            (Container::Mp4, VCodec::Vp9) => Some("VP9 in MP4 works, but Apple devices and some editors won't play it. WebM or MKV is the usual home for VP9."),
            (Container::Mp4 | Container::Mov, VCodec::H265) => Some("Tagged for Apple compatibility (hvc1), so it plays in QuickTime and on iPhones."),
            (Container::Mkv, VCodec::ProRes) => Some("ProRes in MKV works, but editing apps that want ProRes usually expect MOV."),
            _ => None,
        }
    }
    pub fn audio_pitfall(self, a: ACodec) -> Option<&'static str> {
        match (self, a) {
            (Container::Mp4, ACodec::Opus) => Some("Opus in MP4 works, but many players, phones and editors won't play the sound. AAC is the safe choice."),
            (Container::Mp4, ACodec::Flac) => Some("FLAC in MP4 works, but lots of players won't play the sound. Use AAC unless you know your player supports it."),
            (Container::Mp4, ACodec::Mp3) => Some("MP3 audio in MP4 plays in most places, but AAC is smaller for the same quality."),
            (Container::Mp4 | Container::Mov, ACodec::Vorbis) => Some("FFmpeg can write this, but it isn't standard: almost no player will play the sound. Use AAC."),
            (Container::Mp4, ACodec::Pcm) => Some("Uncompressed audio in MP4 is very new to the standard; many players won't play it. Use MOV or MKV for uncompressed audio."),
            (Container::Mov, ACodec::Mp3) => Some("Works, but AAC is the normal choice for MOV and gives smaller files."),
            (Container::Mkv, ACodec::Pcm) => Some("Uncompressed audio: perfect quality, very large."),
            (Container::Mov, ACodec::Pcm) => Some("Uncompressed audio: perfect quality, very large. Common for editing."),
            _ => None,
        }
    }
}

impl VCodec {
    pub fn label(self) -> &'static str {
        match self {
            VCodec::H264 => "H.264",
            VCodec::H265 => "H.265 (HEVC)",
            VCodec::Av1 => "AV1",
            VCodec::Vp9 => "VP9",
            VCodec::ProRes => "ProRes",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            VCodec::H264 => "Plays everywhere.",
            VCodec::H265 => "Smaller files, but older devices may not play it.",
            VCodec::Av1 => "Smallest files, slow to encode on the CPU, and older devices may not play it.",
            VCodec::Vp9 => "Good for the web, slow to encode.",
            VCodec::ProRes => "Huge files, built for editing, not sharing.",
        }
    }
    pub fn from_ffprobe(name: &str) -> Option<VCodec> {
        Some(match name {
            "h264" => VCodec::H264,
            "hevc" => VCodec::H265,
            "av1" => VCodec::Av1,
            "vp9" => VCodec::Vp9,
            "prores" => VCodec::ProRes,
            _ => return None,
        })
    }
    /// CPU encoders in order of preference.
    pub fn cpu_encoders(self) -> &'static [&'static str] {
        match self {
            VCodec::H264 => &["libx264"],
            VCodec::H265 => &["libx265"],
            VCodec::Av1 => &["libsvtav1", "libaom-av1"],
            VCodec::Vp9 => &["libvpx-vp9"],
            VCodec::ProRes => &["prores_ks"],
        }
    }
    /// GPU encoders in order of preference (NVIDIA, AMD, Intel).
    pub fn gpu_encoders(self) -> &'static [&'static str] {
        match self {
            VCodec::H264 => &["h264_nvenc", "h264_amf", "h264_qsv"],
            VCodec::H265 => &["hevc_nvenc", "hevc_amf", "hevc_qsv"],
            VCodec::Av1 => &["av1_nvenc", "av1_amf", "av1_qsv"],
            VCodec::Vp9 | VCodec::ProRes => &[],
        }
    }
    pub fn supports_bitrate(self) -> bool {
        self != VCodec::ProRes
    }
}

impl ACodec {
    pub fn label(self) -> &'static str {
        match self {
            ACodec::Aac => "AAC",
            ACodec::Mp3 => "MP3",
            ACodec::Opus => "Opus",
            ACodec::Flac => "FLAC (lossless)",
            ACodec::Vorbis => "Vorbis",
            ACodec::Pcm => "PCM (uncompressed)",
        }
    }
    pub fn encoder(self) -> &'static str {
        match self {
            ACodec::Aac => "aac",
            ACodec::Mp3 => "libmp3lame",
            ACodec::Opus => "libopus",
            ACodec::Flac => "flac",
            ACodec::Vorbis => "libvorbis",
            ACodec::Pcm => "pcm_s16le",
        }
    }
    pub fn has_bitrate(self) -> bool {
        !matches!(self, ACodec::Flac | ACodec::Pcm)
    }
    pub fn from_ffprobe(name: &str) -> Option<ACodec> {
        Some(match name {
            "aac" => ACodec::Aac,
            "mp3" => ACodec::Mp3,
            "opus" => ACodec::Opus,
            "flac" => ACodec::Flac,
            "vorbis" => ACodec::Vorbis,
            n if n.starts_with("pcm_") => ACodec::Pcm,
            _ => return None,
        })
    }
}

/// What this FFmpeg build can actually do, detected at startup.
#[derive(Clone, Debug, Default)]
pub struct Caps {
    /// Encoders FFmpeg lists.
    pub encoders: HashSet<String>,
    /// GPU encoders that passed a real test encode on this PC.
    pub working_gpu: HashSet<String>,
    /// Has the zscale filter (needed to convert HDR to standard colour).
    pub zscale: bool,
}

impl Caps {
    pub fn cpu_encoder(&self, v: VCodec) -> Option<&'static str> {
        v.cpu_encoders().iter().copied().find(|e| self.encoders.contains(*e))
    }
    pub fn gpu_encoder(&self, v: VCodec) -> Option<&'static str> {
        v.gpu_encoders().iter().copied().find(|e| self.working_gpu.contains(*e))
    }
    /// Encoder to use, honouring the GPU switch.
    pub fn pick(&self, v: VCodec, use_gpu: bool) -> Option<&'static str> {
        if use_gpu {
            if let Some(g) = self.gpu_encoder(v) {
                return Some(g);
            }
        }
        self.cpu_encoder(v)
    }
    pub fn audio_ok(&self, a: ACodec) -> bool {
        self.encoders.contains(a.encoder())
    }
    /// Why a video codec can't be used, in plain English.
    pub fn video_unavailable_reason(&self, v: VCodec) -> Option<String> {
        if self.cpu_encoder(v).is_some() || self.gpu_encoder(v).is_some() {
            None
        } else {
            Some(format!("Your FFmpeg build doesn't include an encoder for {}. A 'full' FFmpeg build would.", v.label()))
        }
    }

    /// Detect capabilities by asking FFmpeg. Slow-ish (test encodes), run off the UI thread.
    pub fn detect(ffmpeg: &std::path::Path) -> Caps {
        let mut caps = Caps::default();
        if let Ok(out) = crate::winutil::hidden_command(ffmpeg).args(["-hide_banner", "-encoders"]).output() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let mut parts = line.split_whitespace();
                if let (Some(flags), Some(name)) = (parts.next(), parts.next()) {
                    if flags.len() == 6 && (flags.starts_with('V') || flags.starts_with('A')) {
                        caps.encoders.insert(name.to_string());
                    }
                }
            }
        }
        if let Ok(out) = crate::winutil::hidden_command(ffmpeg).args(["-hide_banner", "-filters"]).output() {
            caps.zscale = String::from_utf8_lossy(&out.stdout).lines().any(|l| l.split_whitespace().nth(1) == Some("zscale"));
        }
        // A GPU encoder is only listed if FFmpeg was built with it; it only works
        // if this PC has the hardware and driver. Test each with a tiny encode.
        let candidates: Vec<&str> = [VCodec::H264, VCodec::H265, VCodec::Av1]
            .iter()
            .flat_map(|v| v.gpu_encoders().iter().copied())
            .filter(|e| caps.encoders.contains(*e))
            .collect();
        for enc in candidates {
            let ok = crate::winutil::hidden_command(ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=black:s=256x256:d=0.2", "-pix_fmt", "yuv420p", "-c:v", enc, "-f", "null", "-"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if ok {
                caps.working_gpu.insert(enc.to_string());
            }
        }
        caps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_container_has_a_default_audio() {
        for c in Container::ALL {
            assert!(!c.audio_codecs().is_empty());
            assert_eq!(c.is_audio_only(), c.video_codecs().is_empty());
        }
    }

    #[test]
    fn copy_rules() {
        assert!(Container::Mp4.can_copy_video("h264"));
        assert!(!Container::Webm.can_copy_video("hevc"));
        assert!(Container::Mkv.can_copy_video("mpeg4"));
        assert!(!Container::Mp4.can_copy_audio("pcm_s16le"));
        assert!(Container::Mov.can_copy_audio("pcm_s24le"));
    }
}
