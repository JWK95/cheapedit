//! Several audio tracks in one file (ShadowPlay can record game sound and
//! the microphone separately). Each track has its own settings; the tracks
//! that are on are mixed into one for the export, and for the preview.
//!
//! Pure (no I/O), so it can be unit-tested.

/// One audio track's settings. Belongs to the open file, not saved.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TrackMix {
    /// Included in the preview and the export.
    pub on: bool,
    /// 0-200.
    pub volume_pct: u32,
    /// Preview only: hear just the soloed track(s). Never affects the export.
    pub solo: bool,
}

impl Default for TrackMix {
    fn default() -> Self {
        TrackMix { on: true, volume_pct: 100, solo: false }
    }
}

impl TrackMix {
    pub fn gain(&self) -> f64 {
        self.volume_pct as f64 / 100.0
    }
}

/// Which audio the export takes from the source.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportAudio {
    /// One track as it is (`0:a:N`), so it can still be copied in Fast mode.
    Track(usize),
    /// Several tracks (or one at a changed volume) mixed: (track, gain).
    Mix(Vec<(usize, f64)>),
    /// Every track is switched off.
    None,
}

/// `tracks` has one entry per audio track. Files with fewer than two
/// tracks always take track 0 as before.
pub fn export_audio(tracks: &[TrackMix]) -> ExportAudio {
    if tracks.len() < 2 {
        return ExportAudio::Track(0);
    }
    let on: Vec<(usize, f64)> = tracks.iter().enumerate().filter(|(_, t)| t.on).map(|(i, t)| (i, t.gain())).collect();
    match on.as_slice() {
        [] => ExportAudio::None,
        [(i, g)] if *g == 1.0 => ExportAudio::Track(*i),
        _ => ExportAudio::Mix(on),
    }
}

/// The FFmpeg `-filter_complex` graph for a mix, ending in `[aout]`, with
/// `after` (the usual speed/volume filters) applied to the mixed sound.
pub fn ffmpeg_mix_graph(mix: &[(usize, f64)], after: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut tail = String::new();
    if !after.is_empty() {
        tail = format!(",{}", after.join(","));
    }
    if let [(i, g)] = mix {
        return format!("[0:a:{i}]volume={}{tail}[aout]", num(*g));
    }
    let mut labels = String::new();
    for (k, (i, g)) in mix.iter().enumerate() {
        parts.push(format!("[0:a:{i}]volume={}[m{k}]", num(*g)));
        labels.push_str(&format!("[m{k}]"));
    }
    // normalize=0: each track keeps its own loudness instead of being divided
    // by the number of tracks.
    parts.push(format!("{labels}amix=inputs={}:duration=longest:normalize=0{tail}[aout]", mix.len()));
    parts.join(";")
}

/// What the player should do with a multi-track file.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewAudio {
    /// Play normally (a file with fewer than two tracks).
    Normal,
    /// Nothing is on (or soloed): no sound.
    Silent,
    /// mpv `lavfi-complex` graph mixing the tracks into `[ao]`.
    Graph(String),
}

pub fn preview_audio(tracks: &[TrackMix]) -> PreviewAudio {
    if tracks.len() < 2 {
        return PreviewAudio::Normal;
    }
    let soloing = tracks.iter().any(|t| t.solo);
    let heard: Vec<(usize, f64)> =
        tracks.iter().enumerate().filter(|(_, t)| if soloing { t.solo } else { t.on }).map(|(i, t)| (i, t.gain())).collect();
    if heard.is_empty() {
        return PreviewAudio::Silent;
    }
    // mpv numbers audio tracks from 1.
    if let [(i, g)] = heard.as_slice() {
        return PreviewAudio::Graph(format!("[aid{}]volume={}[ao]", i + 1, num(*g)));
    }
    let mut parts: Vec<String> = Vec::new();
    let mut labels = String::new();
    for (k, (i, g)) in heard.iter().enumerate() {
        parts.push(format!("[aid{}]volume={}[t{k}]", i + 1, num(*g)));
        labels.push_str(&format!("[t{k}]"));
    }
    parts.push(format!("{labels}amix=inputs={}:duration=longest:normalize=0[ao]", heard.len()));
    PreviewAudio::Graph(parts.join(";"))
}

fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(on: bool, v: u32, solo: bool) -> TrackMix {
        TrackMix { on, volume_pct: v, solo }
    }

    #[test]
    fn single_track_files_are_untouched() {
        assert_eq!(export_audio(&[t(true, 50, false)]), ExportAudio::Track(0));
        assert_eq!(export_audio(&[]), ExportAudio::Track(0));
        assert_eq!(preview_audio(&[t(false, 100, false)]), PreviewAudio::Normal);
    }

    #[test]
    fn export_choices() {
        assert_eq!(export_audio(&[t(true, 100, false), t(true, 100, false)]), ExportAudio::Mix(vec![(0, 1.0), (1, 1.0)]));
        // Only the mic, untouched: keep it as a plain stream (Fast mode can copy it).
        assert_eq!(export_audio(&[t(false, 100, false), t(true, 100, false)]), ExportAudio::Track(1));
        assert_eq!(export_audio(&[t(false, 100, false), t(true, 150, false)]), ExportAudio::Mix(vec![(1, 1.5)]));
        assert_eq!(export_audio(&[t(false, 100, false), t(false, 100, false)]), ExportAudio::None);
        // Solo is preview only.
        assert_eq!(export_audio(&[t(true, 100, true), t(true, 100, false)]), ExportAudio::Mix(vec![(0, 1.0), (1, 1.0)]));
    }

    #[test]
    fn ffmpeg_graphs() {
        assert_eq!(
            ffmpeg_mix_graph(&[(0, 1.0), (1, 1.5)], &[]),
            "[0:a:0]volume=1[m0];[0:a:1]volume=1.5[m1];[m0][m1]amix=inputs=2:duration=longest:normalize=0[aout]"
        );
        assert_eq!(ffmpeg_mix_graph(&[(1, 0.5)], &["atempo=2".into()]), "[0:a:1]volume=0.5,atempo=2[aout]");
    }

    #[test]
    fn preview_graphs() {
        assert_eq!(
            preview_audio(&[t(true, 100, false), t(true, 80, false)]),
            PreviewAudio::Graph("[aid1]volume=1[t0];[aid2]volume=0.8[t1];[t0][t1]amix=inputs=2:duration=longest:normalize=0[ao]".into())
        );
        // Solo wins, even over a track that's switched off.
        assert_eq!(preview_audio(&[t(true, 100, false), t(false, 100, true)]), PreviewAudio::Graph("[aid2]volume=1[ao]".into()));
        assert_eq!(preview_audio(&[t(false, 100, false), t(false, 100, false)]), PreviewAudio::Silent);
    }
}
