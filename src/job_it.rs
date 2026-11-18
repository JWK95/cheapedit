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
    /// Speed zones, on the source's clock (the trim is 1-3 s).
    speed: Vec<crate::speed::Resolved>,
}

impl Case {
    fn new(label: impl Into<String>, opts: Options) -> Case {
        Case { label: label.into(), opts, speed: Vec::new() }
    }
}

fn run_case(ffmpeg: &Path, ffprobe: &Path, caps: &Caps, src: &probe::MediaInfo, dir: &Path, case: &Case, i: usize) -> Result<(), String> {
    let o = &case.opts;
    let container = o.output_container();
    let out = dir.join(format!("out{i}.{}", container.ext()));
    let _ = std::fs::remove_file(&out);
    let enc = if o.mode == CutMode::Precise && o.av != AvMode::AudioOnly { caps.pick(o.vcodec, o.use_gpu) } else { None };
    let (start, end) = (1.0, 3.0);
    let job = Job { media: src, opts: o, start, end, output: &out, video_encoder: enc, can_tonemap: caps.zscale, zones: &[], tracks: &[], speed_zones: &case.speed, overlays: &[], montage: None };
    let args = build_args(&job);
    let res = crate::winutil::hidden_command(ffmpeg).args(&args).output().map_err(|e| e.to_string())?;
    if !res.status.success() {
        return Err(format!("ffmpeg failed: {}\n  args: {}", String::from_utf8_lossy(&res.stderr).trim(), args.join(" ")));
    }
    let info = probe::probe(ffprobe, &out)?;
    let want = output_duration(start, end, if o.mode == CutMode::Fast { &[] } else { &case.speed });
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
                    cases.push(Case::new(format!("{:?}/{:?}/{:?} gpu={gpu}", c, v, a), opts));
                }
            }
        }
        // Fast mode into a container that can't hold the source codec is blocked
        // by the pre-export checks, so only test the allowed ones.
        if c.can_copy_video(&src.video.as_ref().unwrap().codec) {
            cases.push(Case::new(format!("fast {:?}", c), Options { mode: CutMode::Fast, container: c, ..Default::default() }));
            cases.push(Case::new(format!("fast mute {:?}", c), Options { mode: CutMode::Fast, av: AvMode::Mute, container: c, ..Default::default() }));
        }
    }
    for speed in [0.25, 0.5, 1.5, 2.0, 4.0] {
        for pitch in [false, true] {
            let mut c = Case::new(format!("speed {speed} pitch={pitch}"), Options { change_pitch: pitch, ..Default::default() });
            c.speed = vec![crate::speed::Resolved { start: 1.0, end: 3.0, speed, smooth: false, mute: false }];
            cases.push(c);
        }
    }
    // Speed zones inside the trim: slow (smooth, muted), a normal gap, fast.
    let mut c = Case::new("speed zones", Options::default());
    c.speed = vec![
        crate::speed::Resolved { start: 1.2, end: 1.8, speed: 0.5, smooth: true, mute: true },
        crate::speed::Resolved { start: 2.2, end: 3.0, speed: 2.0, smooth: false, mute: false },
    ];
    cases.push(c);
    for av in AvMode::ALL {
        cases.push(Case::new(format!("{av:?}"), Options { av, volume_pct: 150, ..Default::default() }));
        cases.push(Case::new(format!("fast {av:?}"), Options { av, mode: CutMode::Fast, ..Default::default() }));
    }
    for q in Quality::ALL {
        cases.push(Case::new(format!("quality {q:?}"), Options { quality: q, custom_kbps: 2000, ..Default::default() }));
    }
    cases.push(Case::new("720p 24fps", Options { resolution: Resolution::P720, fps: FrameRate::Fps(24), ..Default::default() }));

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
    let case = Case::new("hdr", Options::default());
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
            Resolved { start: s, end: s + len, glide, a: ZBox::centered(0.5), b: ZBox { x: 0.0, y: 0.0, s: 0.2 }, path: None }
        })
        .collect();
    for (speed, gpu) in [(1.0, false), (2.0, true)] {
        let o = Options { use_gpu: gpu, ..Default::default() };
        let sz = [crate::speed::Resolved { start: 1.0, end: 3.0, speed, smooth: false, mute: false }];
        let out = dir.join(format!("zoom{speed}.mp4"));
        let _ = std::fs::remove_file(&out);
        let enc = caps.pick(o.vcodec, o.use_gpu);
        let job = Job { media: &src, opts: &o, start: 1.0, end: 3.0, output: &out, video_encoder: enc, can_tonemap: caps.zscale, zones: &zones, tracks: &[], speed_zones: &sz, overlays: &[], montage: None };
        let args = build_args(&job);
        let res = crate::winutil::hidden_command(&ffmpeg).args(&args).output().unwrap();
        assert!(res.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&res.stderr));
        let info = probe::probe(&ffprobe, &out).unwrap();
        assert!((info.duration - 2.0 / speed).abs() < 0.25, "duration {}", info.duration);
        let v = info.video.unwrap();
        assert_eq!((v.width, v.height), (640, 360), "zoom keeps the size");
    }
}

/// Overlays of every kind, with speed zones, in one real export: an image,
/// a looping GIF, a picture-in-picture video with its sound (following the
/// clip speed), and moving text with an outline, background and fades.
#[test]
#[ignore]
fn overlays_export() {
    use crate::overlay::{Kind, Overlay, Pos};
    use crate::speed::Resolved;
    let (ffmpeg, ffprobe) = (tool("ffmpeg.exe"), tool("ffprobe.exe"));
    let caps = Caps::detect(&ffmpeg);
    let dir = std::env::temp_dir().join("cheapedit-it-overlays");
    std::fs::create_dir_all(&dir).unwrap();
    let make = |name: &str, args: &[&str]| {
        let out = dir.join(name);
        let ok = crate::winutil::hidden_command(&ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(&out).status().unwrap().success();
        assert!(ok, "making {name}");
        out
    };
    let src_path = make_source(&ffmpeg, &dir, "src.mp4", &["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]);
    let png = make("logo.png", &["-f", "lavfi", "-i", "color=c=red@0.5:s=200x100,format=rgba", "-frames:v", "1"]);
    let gif = make("anim.gif", &["-f", "lavfi", "-i", "testsrc=s=160x120:r=10:d=1"]);
    let webp = make("anim.webp", &["-f", "lavfi", "-i", "testsrc=s=120x120:r=10:d=0.5", "-c:v", "libwebp_anim", "-loop", "0"]);
    let cam = make("cam.mp4", &["-f", "lavfi", "-i", "testsrc2=s=320x180:r=30:d=3", "-f", "lavfi", "-i", "sine=f=880:d=3", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]);
    let text_file = dir.join("t0.txt");
    std::fs::write(&text_file, "Hello: it's \"cheapedit\", 100%\nline two").unwrap();
    let src = probe::probe(&ffprobe, &src_path).unwrap();
    let fa = 360.0 / 640.0;

    let image = Overlay::media(Kind::Image, png, 0.5, fa, false, 1.0, 2.5);
    let mut anim = Overlay::media(Kind::Animated, gif, 0.75, fa, false, 1.5, 3.0);
    anim.fade_in = 0.3;
    anim.opacity = 70;
    let mut video = Overlay::media(Kind::Video, cam, 0.5625, fa, true, 1.2, 2.8);
    video.audio_on = true;
    video.follow_speed = true;
    let mut text = Overlay::text(1.0, 3.0);
    text.text_file = Some(text_file);
    let font = std::path::PathBuf::from(r"C:\Windows\Fonts\arial.ttf");
    text.text.font_file = font.is_file().then_some(font);
    text.text.background = true;
    text.text.shadow = true;
    text.fade_out = 0.5;
    text.pan = true;
    text.b_drawn = true;
    text.b = Pos { x: 0.5, y: 0.6 };
    // An animated WebP (what KLIPY gives), looping for longer than it lasts.
    let looped = Overlay::media(Kind::Animated, webp, 1.0, fa, false, 1.0, 3.0);
    let overlays = [image, anim, video, text, looped];
    let speed = [Resolved { start: 1.5, end: 2.0, speed: 0.5, smooth: false, mute: false }, Resolved { start: 2.5, end: 3.0, speed: 2.0, smooth: false, mute: false }];

    for (label, sz) in [("plain", &[][..]), ("with speed zones", &speed[..])] {
        let o = Options { enc_speed: EncSpeed::Faster, ..Default::default() };
        let out = dir.join(format!("ov_{}.mp4", sz.len()));
        let _ = std::fs::remove_file(&out);
        let enc = caps.pick(o.vcodec, false);
        let job = Job { media: &src, opts: &o, start: 1.0, end: 3.0, output: &out, video_encoder: enc, can_tonemap: false, zones: &[], tracks: &[], speed_zones: sz, overlays: &overlays, montage: None };
        let args = build_args(&job);
        let res = crate::winutil::hidden_command(&ffmpeg).args(&args).output().unwrap();
        assert!(res.status.success(), "{label}: {}\n  {}", String::from_utf8_lossy(&res.stderr), args.join(" "));
        let info = probe::probe(&ffprobe, &out).unwrap();
        let want = output_duration(1.0, 3.0, sz);
        assert!((info.duration - want).abs() < 0.25, "{label}: duration {} want {want}", info.duration);
        assert_eq!(info.video.as_ref().map(|v| (v.width, v.height)), Some((640, 360)));
        assert_eq!(info.audio_tracks.len(), 1, "{label}: one mixed audio track");
        println!("ok  overlays {label}");
    }
}

/// A montage: three different clips (1280x720 30 fps with sound, a tall
/// 360x640 25 fps clip with no sound, and a slowed-down third), each fitted
/// to one frame, then joined without re-encoding.
#[test]
#[ignore]
fn montage_joins_clips() {
    use crate::speed::Resolved;
    let (ffmpeg, ffprobe) = (tool("ffmpeg.exe"), tool("ffprobe.exe"));
    let caps = Caps::detect(&ffmpeg);
    let dir = std::env::temp_dir().join("cheapedit-it-montage");
    std::fs::create_dir_all(&dir).unwrap();
    let make = |name: &str, args: &[&str]| {
        let out = dir.join(name);
        let ok = crate::winutil::hidden_command(&ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).arg(&out).status().unwrap().success();
        assert!(ok, "making {name}");
        probe::probe(&ffprobe, &out).unwrap()
    };
    let wide = make("wide.mp4", &["-f", "lavfi", "-i", "testsrc2=s=1280x720:r=30:d=3", "-f", "lavfi", "-i", "sine=f=440:d=3:sample_rate=44100", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-ac", "1"]);
    let tall = make("tall.mp4", &["-f", "lavfi", "-i", "testsrc=s=360x640:r=25:d=3", "-c:v", "libx264", "-pix_fmt", "yuv420p"]);
    let frame = Frame { w: 1280, h: 720, fps: 30.0 };
    let o = Options { enc_speed: EncSpeed::Faster, ..Default::default() };
    let enc = caps.pick(o.vcodec, false);
    let slow = [Resolved { start: 0.5, end: 1.5, speed: 0.5, smooth: false, mute: false }];
    let parts: [(&probe::MediaInfo, f64, f64, &[Resolved]); 3] = [(&wide, 0.0, 2.0, &[]), (&tall, 0.5, 2.5, &[]), (&wide, 0.5, 1.5, &slow)];
    let mut files = Vec::new();
    let mut want = 0.0;
    for (i, (m, s, e, sz)) in parts.iter().enumerate() {
        let out = dir.join(format!("part{i}.mp4"));
        let _ = std::fs::remove_file(&out);
        let job = Job { media: m, opts: &o, start: *s, end: *e, output: &out, video_encoder: enc, can_tonemap: false, zones: &[], tracks: &[], speed_zones: sz, overlays: &[], montage: Some(frame) };
        let args = build_args(&job);
        let res = crate::winutil::hidden_command(&ffmpeg).args(&args).output().unwrap();
        assert!(res.status.success(), "part {i}: {}\n  {}", String::from_utf8_lossy(&res.stderr), args.join(" "));
        want += output_duration(*s, *e, sz);
        files.push(out);
    }
    let list = dir.join("list.txt");
    std::fs::write(&list, crate::montage::concat_list(&files)).unwrap();
    let out = dir.join("montage.mp4");
    let _ = std::fs::remove_file(&out);
    let res = crate::winutil::hidden_command(&ffmpeg).args(crate::montage::join_args(&list, &out, true)).output().unwrap();
    assert!(res.status.success(), "join: {}", String::from_utf8_lossy(&res.stderr));
    let info = probe::probe(&ffprobe, &out).unwrap();
    assert!((info.duration - want).abs() < 0.3, "duration {} want {want}", info.duration);
    let v = info.video.unwrap();
    assert_eq!((v.width, v.height), (1280, 720));
    assert!((v.fps - 30.0).abs() < 0.01, "fps {}", v.fps);
    assert_eq!(info.audio_tracks.len(), 1);
    assert_eq!((info.audio_tracks[0].sample_rate, info.audio_tracks[0].channels), (48000, 2));
    // The silent middle clip's audio is there and the last clip still has sound.
    assert!(crate::waveform::tests_peak(&ffmpeg, &out, want - 1.5, want - 0.2) > 0.05);
}

/// A tracked zoom zone, end to end: track a red square moving across the
/// frame, then export with the zoom following it. A frame from the end of
/// the export should be mostly the square's red.
#[test]
#[ignore]
fn tracked_zone_exports() {
    use crate::zoom::{Zone, ZBox};
    use std::sync::atomic::{AtomicBool, AtomicU32};
    let (ffmpeg, ffprobe) = (tool("ffmpeg.exe"), tool("ffprobe.exe"));
    let caps = Caps::detect(&ffmpeg);
    let dir = std::env::temp_dir().join("cheapedit-it-track");
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("square.mp4");
    let ok = crate::winutil::hidden_command(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30:d=4,hue=s=0,eq=brightness=-0.3", "-f", "lavfi", "-i", "color=c=0xFF2020:s=80x80:r=30:d=4"])
        .args(["-filter_complex", "[0:v][1:v]overlay=x='40+120*t':y=140:shortest=1", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&src_path)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let src = probe::probe(&ffprobe, &src_path).unwrap();
    // The zoom box: a square box around the target, a quarter of the frame.
    let s = 0.25;
    let a = ZBox { x: 80.0 / 640.0 - s / 2.0, y: 180.0 / 360.0 - s / 2.0, s };
    let (p, c) = (AtomicU32::new(0), AtomicBool::new(false));
    let run = crate::tracker::track(&ffmpeg, &src_path, 640, 360, 30.0, 0.0, 4.0, (a.x, a.y, a.s, a.s), false, &p, &c).unwrap();
    let mut z = Zone::new(0.0, 4.0);
    z.a = a;
    z.a_drawn = true;
    z.mode = crate::zoom::Mode::Precise;
    z.follow = true;
    z.track_points = run.points;
    let zones = [z.resolve().unwrap()];
    assert!(zones[0].path.as_ref().is_some_and(|p| p.len() >= 2));
    let o = Options { enc_speed: EncSpeed::Faster, ..Default::default() };
    let out = dir.join("tracked.mp4");
    let _ = std::fs::remove_file(&out);
    let enc = caps.pick(o.vcodec, false);
    let job = Job { media: &src, opts: &o, start: 0.0, end: 4.0, output: &out, video_encoder: enc, can_tonemap: false, zones: &zones, tracks: &[], speed_zones: &[], overlays: &[], montage: None };
    let args = build_args(&job);
    let res = crate::winutil::hidden_command(&ffmpeg).args(&args).output().unwrap();
    assert!(res.status.success(), "{}\n  {}", String::from_utf8_lossy(&res.stderr), args.join(" "));
    // At 3 s (well past the glide) the square should fill the middle of the picture.
    let raw = crate::winutil::hidden_command(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-ss", "3", "-i"])
        .arg(&out)
        .args(["-frames:v", "1", "-vf", "crop=iw/4:ih/4:iw*3/8:ih*3/8,scale=1:1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    let px = raw.stdout;
    assert_eq!(px.len(), 3);
    println!("centre colour at 3 s: {px:?}");
    assert!(px[0] > 150 && px[1] < 90 && px[2] < 90, "the centre should be the red square: {px:?}");
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
            let job = Job { media: &m, opts: &o, start: 0.5, end: 0.5 + secs, output: &out, video_encoder: Some(enc), can_tonemap: caps.zscale, zones: &[], tracks: &[], speed_zones: &[], overlays: &[], montage: None };
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
            let job = Job { media: &src, opts: &o, start: 0.0, end: 4.0, output: &out, video_encoder: enc, can_tonemap: false, zones: &[], tracks, speed_zones: &[], overlays: &[], montage: None };
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
