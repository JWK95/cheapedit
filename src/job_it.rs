//! Integration test: runs real FFmpeg exports for every allowed option
//! combination. Run with `cargo test -- --ignored --nocapture`.

use crate::formats::{Caps, Container};
use crate::job::*;
use crate::probe;
use std::path::{Path, PathBuf};

/// The FFmpeg the app would use: the bundled copy (target\deps, from
/// tools\fetch-deps.ps1), or PATH if there isn't one.
fn tool(name: &str) -> PathBuf {
    let p = crate::deps::find(name).expect("run tools\\fetch-deps.ps1, or put FFmpeg on PATH");
    println!("using {}", p.display());
    p
}

fn make_source(ffmpeg: &Path, dir: &Path, name: &str, extra: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let mut cmd = crate::winutil::hidden_command(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=4", "-f", "lavfi", "-i", "sine=f=440:d=4:sample_rate=44100"]);
    cmd.args(extra);
    cmd.arg(&out);
    assert!(cmd.status().unwrap().success(), "making source {name}");
    out
}

struct Case {
    label: String,
    opts: Options,
}

fn run_case(ffmpeg: &Path, ffprobe: &Path, caps: &Caps, src: &probe::MediaInfo, dir: &Path, case: &Case, i: usize) -> Result<(), String> {
    let o = &case.opts;
    let container = o.output_container();
    let out = dir.join(format!("out{i}.{}", container.ext()));
    let _ = std::fs::remove_file(&out);
    let enc = if o.mode == CutMode::Precise && o.av != AvMode::AudioOnly { caps.pick(o.vcodec, o.use_gpu) } else { None };
    let (start, end) = (1.0, 3.0);
    let job = Job { media: src, opts: o, start, end, output: &out, video_encoder: enc, can_tonemap: caps.zscale, zones: &[], tracks: &[] };
    let args = build_args(&job);
    let res = crate::winutil::hidden_command(ffmpeg).args(&args).output().map_err(|e| e.to_string())?;
    if !res.status.success() {
        return Err(format!("ffmpeg failed: {}\n  args: {}", String::from_utf8_lossy(&res.stderr).trim(), args.join(" ")));
    }
    let info = probe::probe(ffprobe, &out)?;
    let want = output_duration(start, end, o.effective_speed());
    // Fast mode snaps to keyframes, so allow more slack there.
    let slack = if o.mode == CutMode::Fast { 2.5 } else { 0.25 };
    if (info.duration - want).abs() > slack {
        return Err(format!("duration {:.2}s, expected {:.2}s", info.duration, want));
    }
    let want_video = o.av != AvMode::AudioOnly;
    let want_audio = o.av != AvMode::RemoveAudio;
    if info.video.is_some() != want_video || info.audio.is_some() != want_audio {
        return Err(format!("streams wrong: video={} audio={}", info.video.is_some(), info.audio.is_some()));
    }
    Ok(())
}

#[test]
#[ignore]
fn every_combination_exports() {
    let (ffmpeg, ffprobe) = (tool("ffmpeg.exe"), tool("ffprobe.exe"));
    let caps = Caps::detect(&ffmpeg);
    println!("GPU encoders working: {:?}", caps.working_gpu);
    let dir = std::env::temp_dir().join("cheapedit-it");
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = make_source(&ffmpeg, &dir, "src.mp4", &["-c:v", "libx264", "-g", "30", "-pix_fmt", "yuv420p", "-c:a", "aac"]);
    let src = probe::probe(&ffprobe, &src_path).unwrap();

    let mut cases: Vec<Case> = Vec::new();
    for c in Container::ALL.iter().copied().filter(|c| !c.is_audio_only()) {
        for &v in c.video_codecs() {
            for &a in c.audio_codecs() {
                for gpu in [false, true] {
                    if gpu && caps.gpu_encoder(v).is_none() {
                        continue;
                    }
                    let opts = Options { container: c, vcodec: v, acodec: a, use_gpu: gpu, enc_speed: EncSpeed::Faster, ..Default::default() };
                    cases.push(Case { label: format!("{:?}/{:?}/{:?} gpu={gpu}", c, v, a), opts });
                }
            }
        }
        // Fast mode into a container that can't hold the source codec is blocked
        // by the pre-export checks, so only test the allowed ones.
        if c.can_copy_video(&src.video.as_ref().unwrap().codec) {
            cases.push(Case { label: format!("fast {:?}", c), opts: Options { mode: CutMode::Fast, container: c, ..Default::default() } });
            cases.push(Case { label: format!("fast mute {:?}", c), opts: Options { mode: CutMode::Fast, av: AvMode::Mute, container: c, ..Default::default() } });
        }
    }
    for speed in [0.25, 0.5, 1.5, 2.0, 4.0] {
        for pitch in [false, true] {
            cases.push(Case { label: format!("speed {speed} pitch={pitch}"), opts: Options { speed, change_pitch: pitch, ..Default::default() } });
        }
    }
    for av in AvMode::ALL {
        cases.push(Case { label: format!("{av:?}"), opts: Options { av, volume_pct: 150, ..Default::default() } });
        cases.push(Case { label: format!("fast {av:?}"), opts: Options { av, mode: CutMode::Fast, ..Default::default() } });
    }
    for q in Quality::ALL {
        cases.push(Case { label: format!("quality {q:?}"), opts: Options { quality: q, custom_kbps: 2000, ..Default::default() } });
    }
    cases.push(Case { label: "720p 24fps".into(), opts: Options { resolution: Resolution::P720, fps: FrameRate::Fps(24), ..Default::default() } });

    let mut failures = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        match run_case(&ffmpeg, &ffprobe, &caps, &src, &dir, case, i) {
            Ok(()) => println!("ok    {}", case.label),
            Err(e) => {
                println!("FAIL  {}: {e}", case.label);
                failures.push(case.label.clone());
            }
        }
    }
    assert!(failures.is_empty(), "{} failed: {failures:?}", failures.len());
}

#[test]
#[ignore]
fn hdr_source_converts_to_sdr() {
    let (ffmpeg, ffprobe) = (tool("ffmpeg.exe"), tool("ffprobe.exe"));
    let caps = Caps::detect(&ffmpeg);
    assert!(caps.zscale, "this FFmpeg has no zscale filter");
    let dir = std::env::temp_dir().join("cheapedit-it");
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = make_source(
        &ffmpeg,
        &dir,
        "hdr.mp4",
        &["-c:v", "libx265", "-pix_fmt", "yuv420p10le", "-x265-params", "log-level=error:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc", "-c:a", "aac"],
    );
    let src = probe::probe(&ffprobe, &src_path).unwrap();
    assert!(src.video.as_ref().unwrap().hdr);
    let case = Case { label: "hdr".into(), opts: Options::default() };
    run_case(&ffmpeg, &ffprobe, &caps, &src, &dir, &case, 999).unwrap();
}

#[test]
#[ignore]
fn zoom_zones_export() {
    use crate::zoom::{Resolved, ZBox, MAX_ZONES};
    let (ffmpeg, ffprobe) = (tool("ffmpeg.exe"), tool("ffprobe.exe"));
    let caps = Caps::detect(&ffmpeg);
    let dir = std::env::temp_dir().join("cheapedit-it");
    std::fs::create_dir_all(&dir).unwrap();
    // A 4 s source; the trim below is 1-3 s. Pack the full 20 zones into it,
    // mixing smooth, hard and panning zones.
    let src_path = make_source(&ffmpeg, &dir, "zsrc.mp4", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]);
    let src = probe::probe(&ffprobe, &src_path).unwrap();
    let len = 2.0 / MAX_ZONES as f64;
    let zones: Vec<Resolved> = (0..MAX_ZONES)
        .map(|i| {
            let s = 1.0 + i as f64 * len;
            let glide = if i % 3 == 0 { 0.0 } else { 0.04 };
            Resolved { start: s, end: s + len, glide, a: ZBox::centered(0.5), b: ZBox { x: 0.0, y: 0.0, s: 0.2 } }
        })
        .collect();
    for (speed, gpu) in [(1.0, false), (2.0, true)] {
        let o = Options { speed, use_gpu: gpu, ..Default::default() };
        let out = dir.join(format!("zoom{speed}.mp4"));
        let _ = std::fs::remove_file(&out);
        let enc = caps.pick(o.vcodec, o.use_gpu);
        let job = Job { media: &src, opts: &o, start: 1.0, end: 3.0, output: &out, video_encoder: enc, can_tonemap: caps.zscale, zones: &zones, tracks: &[] };
        let args = build_args(&job);
        let res = crate::winutil::hidden_command(&ffmpeg).args(&args).output().unwrap();
        assert!(res.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&res.stderr));
        let info = probe::probe(&ffprobe, &out).unwrap();
        assert!((info.duration - 2.0 / speed).abs() < 0.25, "duration {}", info.duration);
        let v = info.video.unwrap();
        assert_eq!((v.width, v.height), (640, 360), "zoom keeps the size");
    }
}

/// Discord presets land under their cap on hard-to-compress footage, with
/// both the CPU and (if present) the GPU encoder. Set CHEAPEDIT_DISCORD_SRC
/// to test on a real clip instead of generated noise.
#[test]
#[ignore]
fn discord_presets_fit() {
    let (ffmpeg, ffprobe) = (tool("ffmpeg.exe"), tool("ffprobe.exe"));
    let caps = Caps::detect(&ffmpeg);
    let dir = std::env::temp_dir().join("cheapedit-discord-it");
    let _ = std::fs::create_dir_all(&dir);
    let src = match std::env::var_os("CHEAPEDIT_DISCORD_SRC") {
        Some(p) => PathBuf::from(p),
        None => {
            let out = dir.join("noisy.mp4");
            let st = crate::winutil::hidden_command(&ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=1920x1080:r=60:d=40,noise=alls=30:allf=t", "-f", "lavfi", "-i", "sine=f=440:d=40", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "18", "-c:a", "aac"])
                .arg(&out)
                .status()
                .unwrap();
            assert!(st.success());
            out
        }
    };
    let m = probe::probe(&ffprobe, &src).unwrap();
    let v = m.video.as_ref().unwrap();
    let mut failures = Vec::new();
    for (preset, secs) in [(crate::discord::Preset::Discord10, 12.0_f64), (crate::discord::Preset::Discord10, 35.0), (crate::discord::Preset::Discord20, 30.0)] {
        let secs: f64 = secs.min(m.duration - 1.0);
        let cap = preset.cap_bytes().unwrap();
        let plan = crate::discord::plan(cap, secs, v.width, v.height, v.fps, m.audio.is_some()).unwrap();
        let o = Options { preset, ..Default::default() }.with_plan(&plan);
        for gpu in [false, true] {
            let Some(enc) = caps.pick(o.vcodec, gpu) else { continue };
            if gpu && !enc.contains('_') {
                continue;
            }
            let out = dir.join(format!("d_{}_{secs}_{enc}.mp4", preset.cap_mb().unwrap()));
            let _ = std::fs::remove_file(&out);
            let job = Job { media: &m, opts: &o, start: 0.5, end: 0.5 + secs, output: &out, video_encoder: Some(enc), can_tonemap: caps.zscale, zones: &[], tracks: &[] };
            let res = crate::winutil::hidden_command(&ffmpeg).args(build_args(&job)).output().unwrap();
            assert!(res.status.success(), "{}", String::from_utf8_lossy(&res.stderr));
            let bytes = std::fs::metadata(&out).unwrap().len();
            let line = format!(
                "{} {secs}s {enc}: {}p{} {}k+{:?}k -> {:.2} MB (plan {:.2} MB, cap {} MB)",
                preset.label(),
                plan.height,
                plan.fps,
                plan.video_kbps,
                plan.audio_kbps,
                bytes as f64 / 1e6,
                plan.bytes(secs) as f64 / 1e6,
                preset.cap_mb().unwrap()
            );
            println!("{line}");
            if bytes > cap {
                failures.push(line);
            }
        }
    }
    assert!(failures.is_empty(), "over the cap on the first pass:\n{}", failures.join("\n"));
}

/// Issue #6: a ShadowPlay-style file with game sound on track 1 and the
/// microphone on track 2. Every mix exports one audio track (or none) with
/// sound from the right tracks, in Precise and Fast mode.
#[test]
#[ignore]
fn multi_track_audio_mixes() {
    use crate::tracks::TrackMix;
    let (ffmpeg, ffprobe) = (tool("ffmpeg.exe"), tool("ffprobe.exe"));
    let dir = std::env::temp_dir().join("cheapedit-it-tracks");
    std::fs::create_dir_all(&dir).unwrap();
    // Track 1: game tone. Track 2: "mic", silent for the first 2 s then loud.
    let src_path = dir.join("two_tracks.mp4");
    let ok = crate::winutil::hidden_command(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=4"])
        .args(["-f", "lavfi", "-i", "sine=f=440:d=4:sample_rate=48000", "-f", "lavfi", "-i", r"aevalsrc=if(gte(t\,2)\,0.8*sin(2*PI*660*t)\,0):d=4:s=48000"])
        .args(["-map", "0:v", "-map", "1:a", "-map", "2:a", "-c:v", "libx264", "-g", "30", "-pix_fmt", "yuv420p", "-c:a", "aac", "-metadata:s:a:1", "title=Mic"])
        .arg(&src_path)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let src = probe::probe(&ffprobe, &src_path).unwrap();
    assert_eq!(src.audio_tracks.len(), 2);
    assert_eq!(src.audio_tracks[1].title.as_deref(), Some("Mic"));

    let off = TrackMix { on: false, ..Default::default() };
    let loud = TrackMix { volume_pct: 150, ..Default::default() };
    let cases: [(&str, [TrackMix; 2], bool); 4] = [
        ("both", [TrackMix::default(), loud], true),
        ("mic only", [off, TrackMix::default()], true),
        ("game at 150%", [loud, off], true),
        ("none", [off, off], false),
    ];
    let caps = Caps::detect(&ffmpeg);
    let mut failures = Vec::new();
    for mode in [CutMode::Precise, CutMode::Fast] {
        for (i, (label, tracks, want_audio)) in cases.iter().enumerate() {
            let o = Options { mode, enc_speed: EncSpeed::Faster, ..Default::default() };
            let out = dir.join(format!("mix{i}_{mode:?}.mp4"));
            let _ = std::fs::remove_file(&out);
            let enc = if mode == CutMode::Precise { caps.pick(o.vcodec, false) } else { None };
            let job = Job { media: &src, opts: &o, start: 0.0, end: 4.0, output: &out, video_encoder: enc, can_tonemap: false, zones: &[], tracks };
            let args = build_args(&job);
            let res = crate::winutil::hidden_command(&ffmpeg).args(&args).output().unwrap();
            if !res.status.success() {
                failures.push(format!("{label} {mode:?}: {}\n  {}", String::from_utf8_lossy(&res.stderr).trim(), args.join(" ")));
                continue;
            }
            let info = probe::probe(&ffprobe, &out).unwrap();
            if info.audio_tracks.len() != *want_audio as usize {
                failures.push(format!("{label} {mode:?}: {} audio tracks", info.audio_tracks.len()));
                continue;
            }
            if *want_audio {
                // The first 2 s only has game sound; "mic only" must be silent there.
                let loud_start = crate::waveform::tests_peak(&ffmpeg, &out, 0.2, 1.8);
                let game_on = tracks[0].on;
                if game_on != (loud_start > 0.05) {
                    failures.push(format!("{label} {mode:?}: peak {loud_start:.3} in the first 2 s"));
                }
                let loud_end = crate::waveform::tests_peak(&ffmpeg, &out, 2.2, 3.8);
                if loud_end < 0.05 {
                    failures.push(format!("{label} {mode:?}: silent in the last 2 s"));
                }
            }
            println!("ok  {label} {mode:?}");
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
