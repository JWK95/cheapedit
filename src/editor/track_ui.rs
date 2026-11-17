//! Tracked zoom zones: running the tracker in the background, fixing it
//! where it went wrong, and the zone card's Track controls.

use super::*;
use crate::tracker;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// A tracking run for one zone.
pub(super) struct TrackJob {
    pub zone: usize,
    /// Original time it starts from (the zone start, or a fix).
    pub from: f64,
    pub progress: Arc<AtomicU32>,
    pub cancel: Arc<AtomicBool>,
    pub result: Arc<Mutex<Option<Result<tracker::Run, String>>>>,
}

impl Editor {
    pub(super) fn tracking_zone(&self) -> Option<usize> {
        self.track_job.as_ref().map(|j| j.zone)
    }

    /// Track zone `i` from `from` (original time), starting with the target in `b`.
    pub(super) fn start_track(&mut self, i: usize, from: f64, b: ZBox) {
        if self.track_job.is_some() {
            return;
        }
        if let Err(e) = tracker::available() {
            self.track_note = Some(e);
            return;
        }
        let (Some(m), Some((_, end))) = (self.media.as_ref(), self.zones.get(i).and_then(|z| z.times())) else { return };
        let Some(v) = m.video.as_ref() else { return };
        let (src, w, h, fps) = (m.path.clone(), v.width, v.height, v.fps);
        let job = TrackJob { zone: i, from, progress: Arc::new(AtomicU32::new(0)), cancel: Arc::new(AtomicBool::new(false)), result: Arc::new(Mutex::new(None)) };
        let (p, c, r, ffmpeg, ctx, gpu) = (job.progress.clone(), job.cancel.clone(), job.result.clone(), self.ffmpeg.clone(), self.egui_ctx.clone(), self.settings.tracker_gpu);
        self.log.info(format!("Tracking zone {} from {} ({})", i + 1, timecode::format_short(from), if gpu { "GPU" } else { "CPU" }));
        std::thread::spawn(move || {
            let res = tracker::track(&ffmpeg, &src, w, h, fps, from, end, (b.x, b.y, b.s, b.s), gpu, &p, &c);
            *r.lock().unwrap() = Some(res);
            ctx.request_repaint();
        });
        self.track_note = None;
        self.track_job = Some(job);
    }

    /// A finished run's points go into the zone (replacing what was there from `from` on).
    pub(super) fn poll_track(&mut self) {
        let Some(job) = &self.track_job else { return };
        let done = job.result.lock().unwrap().take();
        let Some(res) = done else {
            self.egui_ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
        let (i, from) = (job.zone, job.from);
        self.track_job = None;
        match res {
            Ok(run) => {
                let device = if run.gpu_used { "graphics card" } else { "CPU" };
                self.tracker_last = Some(match &run.gpu_note {
                    Some(n) => format!("Last run: {n}"),
                    None => format!("Last run used the {device}."),
                });
                if let Some(z) = self.zones.get_mut(i) {
                    z.track_points.retain(|p| p.t < from - 1e-6);
                    z.track_points.extend(run.points.iter().copied());
                    z.locks.retain(|l| l.t <= from + 1e-6);
                }
                let lost = if run.lost > 0 { format!(", lost it for {} frame{}", run.lost, if run.lost == 1 { "" } else { "s" }) } else { String::new() };
                let msg = format!("Tracked {} frames on the {device}{lost}.", run.points.len());
                self.log.info(format!("Zone {}: {msg}", i + 1));
                self.track_note = Some(msg);
            }
            Err(e) => {
                self.log.error(format!("Tracking: {e}"));
                self.track_note = Some(e);
            }
        }
    }

    pub(super) fn cancel_track(&mut self) {
        if let Some(j) = &self.track_job {
            j.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// What the Track controls need to know, taken before the zone cards are drawn.
    pub(super) fn track_ctx(&self) -> TrackCtx {
        TrackCtx {
            tracking: self.tracking_zone(),
            progress: self.track_job.as_ref().map_or(0.0, |j| j.progress.load(Ordering::SeqCst) as f32 / 1000.0),
            paused: self.paused(),
            pos: self.position(),
            fix_zone: self.fix_zone,
        }
    }

    pub(super) fn apply_track_act(&mut self, i: usize, act: TrackAct) {
        match act {
            TrackAct::Cancel => self.cancel_track(),
            TrackAct::Track => {
                let Some((s, _)) = self.zones[i].times() else { return };
                let b = self.zones[i].a;
                self.zones[i].track_points.clear();
                self.zones[i].locks.clear();
                self.fix_zone = None;
                self.start_track(i, s, b);
            }
            TrackAct::Fix => {
                let pos = self.position();
                self.fix_zone = Some(i);
                self.sel_zone = Some(i);
                self.view = View::Original;
                self.fix_box = self.zones[i].resolve().and_then(|r| r.path.map(|p| zoom::box_at(&p, pos)));
            }
            TrackAct::Smooth(v) => self.zones[i].smoothness = v,
            TrackAct::RemoveLock(k) => {
                let z = &mut self.zones[i];
                if k >= z.locks.len() {
                    return;
                }
                z.locks.remove(k);
                // Track again from the fix before it (or the start). Fixes are in time order.
                let (s, _) = z.times().unwrap_or((0.0, 0.0));
                let (from, b) = if k > 0 { (z.locks[k - 1].t, z.locks[k - 1].b) } else { (s, z.a) };
                z.track_points.retain(|p| p.t < from - 1e-6);
                z.locks.retain(|l| l.t <= from + 1e-6);
                self.start_track(i, from, b);
            }
        }
    }

    /// Fixing: the box on the video can be dragged (moved, not resized) onto
    /// the target; letting go starts tracking again from this frame.
    pub(super) fn fix_on_video(&mut self, ui: &mut egui::Ui, resp: &egui::Response, vr: egui::Rect, i: usize) {
        let Some(b) = self.fix_box else { return };
        let col = Color32::from_rgb(0xff, 0xd0, 0x40);
        let r = egui::Rect::from_min_size(egui::pos2(vr.left() + b.x as f32 * vr.width(), vr.top() + b.y as f32 * vr.height()), egui::vec2(b.s as f32 * vr.width(), b.s as f32 * vr.height()));
        let painter = ui.painter_at(vr);
        painter.rect_filled(r, 0.0, col.gamma_multiply(0.1));
        painter.rect_stroke(r, 0.0, Stroke::new(2.0, col), egui::StrokeKind::Inside);
        painter.text(r.left_top() + egui::vec2(6.0, 5.0), egui::Align2::LEFT_TOP, format!("Zone {} - drag onto the target", i + 1), egui::FontId::proportional(12.0), col);
        let to_frac = |p: egui::Pos2| (((p.x - vr.left()) / vr.width()) as f64, ((p.y - vr.top()) / vr.height()) as f64);
        if resp.hover_pos().is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                let f = to_frac(p);
                // Grab anywhere: the box jumps so its centre is under the pointer if it wasn't grabbed.
                self.fix_grab = Some(if r.contains(p) { (f.0 - b.x, f.1 - b.y) } else { (b.s / 2.0, b.s / 2.0) });
            }
        }
        if resp.dragged() {
            if let (Some(off), Some(p)) = (self.fix_grab, resp.interact_pointer_pos()) {
                let f = to_frac(p);
                self.fix_box = Some(ZBox { x: f.0 - off.0, y: f.1 - off.1, s: b.s }.clamped());
            }
        }
        if resp.drag_stopped() && self.fix_grab.take().is_some() {
            self.finish_fix(i);
        }
    }

    /// The fix box being dragged was let go: lock it and track on from here.
    pub(super) fn finish_fix(&mut self, i: usize) {
        let t = self.position();
        let (Some(b), Some(z)) = (self.fix_box, self.zones.get_mut(i)) else { return };
        z.locks.retain(|l| l.t < t - 1e-6);
        z.locks.push(zoom::Lock { t, b });
        z.track_points.retain(|p| p.t < t - 1e-6);
        self.fix_zone = None;
        self.fix_box = None;
        self.start_track(i, t, b);
    }
}

pub(super) struct TrackCtx {
    tracking: Option<usize>,
    progress: f32,
    paused: bool,
    pos: f64,
    fix_zone: Option<usize>,
}

/// The Track controls on zone `i`'s card.
pub(super) fn track_controls(ui: &mut egui::Ui, i: usize, z: &Zone, c: &TrackCtx) -> Option<TrackAct> {
    let mut act = None;
    let busy_here = c.tracking == Some(i);
    let busy_other = c.tracking.is_some() && !busy_here;
    let (s, e) = z.times().unwrap_or((0.0, 0.0));
    if busy_here {
        ui.horizontal(|ui| {
            ui.add(egui::ProgressBar::new(c.progress).show_percentage().desired_width(ui.available_width() - 70.0));
            if ui.button("Cancel").clicked() {
                act = Some(TrackAct::Cancel);
            }
        });
        return act;
    }
    ui.horizontal_wrapped(|ui| {
        let label = if z.tracked() { "Track again" } else { "Track" };
        if ui
            .add_enabled(z.a_drawn && !busy_other, egui::Button::new(label))
            .on_hover_text("Follow what's in the start box to the end of the zone")
            .on_disabled_hover_text(if z.a_drawn { "Another zone is being tracked" } else { "Draw the start box around the thing to follow first" })
            .clicked()
        {
            act = Some(TrackAct::Track);
        }
        let can_fix = z.tracked() && c.paused && c.pos > s + 0.05 && c.pos < e - 0.05 && !busy_other;
        if ui
            .add_enabled(can_fix, egui::Button::selectable(c.fix_zone == Some(i), "Fix at playhead"))
            .on_hover_text("Lost it? Pause where it went wrong, click this, and drag the box back onto the target. Tracking carries on from there.")
            .on_disabled_hover_text("Track first, then pause inside the zone where it went wrong")
            .clicked()
        {
            act = Some(TrackAct::Fix);
        }
    });
    if z.tracked() {
        let mut sm = z.smoothness;
        ui.horizontal(|ui| {
            ui.label("Smoothness");
            if ui.add(egui::Slider::new(&mut sm, 0.0..=1.0).show_value(false)).on_hover_text("Left: follows every move. Right: a slow, steady camera.").changed() {
                act = Some(TrackAct::Smooth(sm));
            }
        });
        for (k, l) in z.locks.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Fixed at {}", timecode::format_short(l.t))).small().color(theme::MUTED));
                if ui.small_button("Remove").on_hover_text("Forget this fix and track again from the one before it").clicked() {
                    act = Some(TrackAct::RemoveLock(k));
                }
            });
        }
    } else if !z.a_drawn {
        theme::hint(ui, "Draw the start box around the thing to follow, then click Track. The zoom stays the box's size; only its position follows.");
    } else {
        theme::hint(ui, "Click Track to follow what's in the start box.");
    }
    if c.fix_zone == Some(i) {
        theme::hint(ui, "Drag the box onto the target on the video. Tracking restarts from here when you let go.");
    }
    act
}

pub(super) enum TrackAct {
    Track,
    Cancel,
    Fix,
    Smooth(f64),
    RemoveLock(usize),
}
