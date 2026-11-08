//! Integration test: runs real FFmpeg exports for every allowed option
//! combination. Run with `cargo test -- --ignored --nocapture`.

use crate::formats::{Caps, Container};
use crate::job::*;
use crate::probe;
use std::path::{Path, PathBuf};

fn tool(name: &str) -> PathBuf {
    crate::winutil::find_on_path(name).expect("ffmpeg/ffprobe must be on PATH")
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
    let job = Job { media: src, opts: o, start, end, output: &out, video_encoder: enc, can_tonemap: caps.zscale, zones: &[] };
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
        let job = Job { media: &src, opts: &o, start: 1.0, end: 3.0, output: &out, video_encoder: enc, can_tonemap: caps.zscale, zones: &zones };
        let args = build_args(&job);
        let res = crate::winutil::hidden_command(&ffmpeg).args(&args).output().unwrap();
        assert!(res.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&res.stderr));
        let info = probe::probe(&ffprobe, &out).unwrap();
        assert!((info.duration - 2.0 / speed).abs() < 0.25, "duration {}", info.duration);
        let v = info.video.unwrap();
        assert_eq!((v.width, v.height), (640, 360), "zoom keeps the size");
    }
}
