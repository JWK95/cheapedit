//! Runs one FFmpeg export in the background with progress and cancel.

use crate::log::{Kind, Log};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// 0.0 – 1.0
    pub fraction: f32,
    /// Estimated seconds left.
    pub eta: Option<f64>,
    pub outcome: Option<Outcome>,
}

pub struct Export {
    pub output: PathBuf,
    pub state: Arc<Mutex<Progress>>,
    child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl Export {
    /// Start FFmpeg. `total` is the expected output length in seconds.
    pub fn start(ffmpeg: &Path, args: Vec<String>, output: PathBuf, total: f64, log: Log, ctx: eframe::egui::Context) -> Result<Export, String> {
        log.push(Kind::Command, format!("{} {}", quote(&ffmpeg.to_string_lossy()), args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")));
        let mut child = crate::winutil::hidden_command(ffmpeg)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Couldn't start FFmpeg: {e}"))?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let state = Arc::new(Mutex::new(Progress::default()));
        let child = Arc::new(Mutex::new(Some(child)));
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // stderr: warnings and errors, kept for the log and the error message.
        let errors = Arc::new(Mutex::new(Vec::<String>::new()));
        let err_thread = {
            let errors = errors.clone();
            let log = log.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        log.push(Kind::Output, line.clone());
                        errors.lock().unwrap().push(line);
                    }
                }
            })
        };

        {
            let (state, child, cancelled, output) = (state.clone(), child.clone(), cancelled.clone(), output.clone());
            std::thread::spawn(move || {
                let began = Instant::now();
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    // "-progress" writes key=value lines; out_time_us is microseconds written so far.
                    if let Some(v) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")) {
                        if let Ok(us) = v.trim().parse::<f64>() {
                            let f = if total > 0.0 { (us / 1e6 / total).clamp(0.0, 1.0) } else { 0.0 };
                            let el = began.elapsed().as_secs_f64();
                            let mut s = state.lock().unwrap();
                            s.fraction = f as f32;
                            s.eta = (f > 0.02).then(|| el * (1.0 - f) / f);
                            drop(s);
                            ctx.request_repaint();
                        }
                    }
                }
                let _ = err_thread.join();
                let status = child.lock().unwrap().take().and_then(|mut c| c.wait().ok());
                let outcome = if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = std::fs::remove_file(&output);
                    log.info("Export cancelled; partial file deleted.");
                    Outcome::Cancelled
                } else if status.map(|s| s.success()).unwrap_or(false) {
                    log.info(format!("Saved {}", output.display()));
                    Outcome::Done
                } else {
                    let lines = errors.lock().unwrap().clone();
                    let msg = explain(&lines);
                    // Never leave a broken half-file behind. (`-n` means it can't be someone else's file.)
                    let _ = std::fs::remove_file(&output);
                    log.error(&msg);
                    Outcome::Failed(msg)
                };
                let mut s = state.lock().unwrap();
                if outcome == Outcome::Done {
                    s.fraction = 1.0;
                }
                s.eta = None;
                s.outcome = Some(outcome);
                drop(s);
                ctx.request_repaint();
            });
        }
        Ok(Export { output, state, child, cancelled })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }

    pub fn progress(&self) -> Progress {
        self.state.lock().unwrap().clone()
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        // If the app closes mid-export, don't leave FFmpeg running.
        if self.progress().outcome.is_none() {
            self.cancel();
        }
    }
}

fn quote(s: &str) -> String {
    if s.contains([' ', '"', ',', ';']) { format!("\"{}\"", s.replace('"', "\\\"")) } else { s.to_string() }
}

/// Turn FFmpeg's error output into one plain-English sentence.
pub fn explain(lines: &[String]) -> String {
    let all = lines.join("\n").to_lowercase();
    if all.contains("no space left") {
        "Your disk filled up during the export. Free some space and try again.".into()
    } else if all.contains("permission denied") {
        "Windows won't let cheapedit write to that folder. Choose a different output folder.".into()
    } else if all.contains("already exists") {
        "A file with that name appeared while exporting. Export again and cheapedit will pick a new name.".into()
    } else if (all.contains("nvenc") || all.contains("amf")) && (all.contains("error") || all.contains("cannot") || all.contains("failed")) {
        "The graphics card encoder couldn't start — it may be busy, or your graphics driver may need updating. Turn off 'Use GPU' in Advanced and try again.".into()
    } else if all.contains("invalid data found") || all.contains("corrupt") {
        "The source file looks damaged, so FFmpeg couldn't finish reading it.".into()
    } else if all.contains("could not find tag") || all.contains("not currently supported in container") || all.contains("only vp8 or vp9") {
        "That format can't hold this combination of codecs. Pick a different format or codec.".into()
    } else {
        "FFmpeg stopped with an error. Open the log to see exactly what it said.".into()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn explains() {
        let e = |s: &str| super::explain(&[s.to_string()]);
        assert!(e("av_interleaved_write_frame(): No space left on device").contains("disk"));
        assert!(e("[h264_nvenc @ 0x] OpenEncodeSessionEx failed: out of memory (10): (no details)\nError while opening encoder").contains("Use GPU"));
        assert!(e("something odd").contains("log"));
    }
}
