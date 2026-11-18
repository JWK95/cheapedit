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

    /// Track zone `i` from `from` (original time), starting with the target
    /// in rectangle `b` (frame fractions x, y, w, h).
    pub(super) fn start_track(&mut self, i: usize, from: f64, b: (f64, f64, f64, f64)) {
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
            let res = tracker::track(&ffmpeg, &src, w, h, fps, from, end, b, gpu, &p, &c);
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
            lasso_mode: self.lasso_mode,
            lasso_error: self.lasso_error.clone(),
            sel_zone: self.sel_zone,
        }
    }

    pub(super) fn apply_track_act(&mut self, i: usize, act: TrackAct) {
        match act {
            TrackAct::Cancel => self.cancel_track(),
            TrackAct::Track => {
                let Some((s, _)) = self.zones[i].times() else { return };
                let r = self.zones[i].start_rect(None);
                self.zones[i].track_points.clear();
                self.zones[i].locks.clear();
                self.fix_zone = None;
                self.lasso_mode = false;
                self.start_track(i, s, r);
            }
            TrackAct::Fix => {
                let pos = self.position();
                self.fix_zone = Some(i);
                self.sel_zone = Some(i);
                self.lasso_mode = false;
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
                let from = if k > 0 { z.locks[k - 1].t } else { s };
                let r = z.start_rect(k.checked_sub(1));
                z.track_points.retain(|p| p.t < from - 1e-6);
                z.locks.retain(|l| l.t <= from + 1e-6);
                self.start_track(i, from, r);
            }
            TrackAct::DrawLoop => {
                // Clicking it again while drawing stops.
                if self.lasso_mode && self.sel_zone == Some(i) {
                    self.stop_lasso();
                } else {
                    self.goto_zone_box(i, 0, false);
                    self.stop_lasso();
                    self.lasso_mode = true;
                }
            }
            TrackAct::ClearLoop => {
                let z = &mut self.zones[i];
                z.lasso.clear();
                z.track_points.clear();
                z.locks.clear();
                self.track_note = Some("Loop removed. Click Track to follow the whole box again.".into());
            }
            TrackAct::FixLoop => {
                let on = !self.lasso_mode;
                self.stop_lasso();
                self.lasso_mode = on;
            }
        }
    }

    /// Drawing a loop by hand inside box `b`. Returns the finished loop when
    /// the mouse is let go. A loop that can't be used says why (on the video
    /// and on the card) and stays ready to try again; Esc stops.
    fn lasso_on_video(&mut self, ui: &mut egui::Ui, resp: &egui::Response, vr: egui::Rect, b: ZBox, col: Color32) -> Option<Vec<[f64; 2]>> {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.stop_lasso();
            return None;
        }
        let painter = ui.painter_at(vr);
        let to_frac = |p: egui::Pos2| (((p.x - vr.left()) / vr.width()) as f64, ((p.y - vr.top()) / vr.height()) as f64);
        let to_screen = |x: f64, y: f64| egui::pos2(vr.left() + x as f32 * vr.width(), vr.top() + y as f32 * vr.height());
        let r = egui::Rect::from_min_max(to_screen(b.x, b.y), to_screen(b.x + b.s, b.y + b.s));
        painter.rect_stroke(r, 0.0, Stroke::new(1.5, col.gamma_multiply(0.7)), egui::StrokeKind::Inside);
        if self.lasso_stroke.len() > 1 {
            let pts: Vec<egui::Pos2> = self.lasso_stroke.iter().map(|&(x, y)| to_screen(x.clamp(b.x, b.x + b.s), y.clamp(b.y, b.y + b.s))).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(2.0, col)));
        } else {
            let next = banner(&painter, vr, vr.top() + 8.0, "Draw a loop around the thing to follow, inside the box. Esc to stop.", Color32::WHITE);
            if let Some(e) = &self.lasso_error {
                banner(&painter, vr, next, &format!("Try again: {e}"), theme::WARN);
            }
        }
        if resp.hover_pos().is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.clicked() {
            self.lasso_error = zoom::make_loop(&[], b).err().map(String::from);
        }
        if resp.drag_started() {
            self.lasso_stroke.clear();
            self.lasso_error = None;
        }
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.lasso_stroke.push(to_frac(p));
        }
        if resp.drag_stopped() {
            let stroke = std::mem::take(&mut self.lasso_stroke);
            match zoom::make_loop(&stroke, b) {
                Ok(l) => {
                    self.stop_lasso();
                    return Some(l);
                }
                Err(e) => {
                    self.log.info(format!("Loop not used: {e}"));
                    self.lasso_error = Some(e.into());
                }
            }
        }
        None
    }

    /// Leave loop drawing (done, or Esc).
    pub(super) fn stop_lasso(&mut self) {
        self.lasso_mode = false;
        self.lasso_error = None;
        self.lasso_stroke.clear();
    }

    /// Drawing zone `i`'s loop inside its box, at the zone start.
    pub(super) fn start_loop_on_video(&mut self, ui: &mut egui::Ui, resp: &egui::Response, vr: egui::Rect, i: usize) {
        let b = self.zones[i].a;
        if let Some(l) = self.lasso_on_video(ui, resp, vr, b, zone_color(i)) {
            let z = &mut self.zones[i];
            z.lasso = l;
            z.track_points.clear();
            z.locks.clear();
            self.track_note = Some("Loop drawn. Click Track to follow it.".into());
        }
    }

    /// Fixing: the box on the video can be dragged (moved, not resized) so
    /// the loop (or the box, with no loop) sits on the target; letting go
    /// starts tracking again from this frame. Redraw loop draws a new loop
    /// in the box instead, for a target that changed shape.
    pub(super) fn fix_on_video(&mut self, ui: &mut egui::Ui, resp: &egui::Response, vr: egui::Rect, i: usize) {
        let Some(b) = self.fix_box else { return };
        let col = Color32::from_rgb(0xff, 0xd0, 0x40);
        if self.lasso_mode {
            if let Some(l) = self.lasso_on_video(ui, resp, vr, b, col) {
                self.finish_fix(i, l);
            }
            return;
        }
        let lasso = self.zones[i].loop_at(self.position()).to_vec();
        let r = egui::Rect::from_min_size(egui::pos2(vr.left() + b.x as f32 * vr.width(), vr.top() + b.y as f32 * vr.height()), egui::vec2(b.s as f32 * vr.width(), b.s as f32 * vr.height()));
        let painter = ui.painter_at(vr);
        let (msg, box_col) = if lasso.is_empty() {
            painter.rect_filled(r, 0.0, col.gamma_multiply(0.1));
            ("drag onto the target", col)
        } else {
            paint_loop(&painter, vr, &lasso, b, col);
            ("drag the loop onto the target", col.gamma_multiply(0.5))
        };
        painter.rect_stroke(r, 0.0, Stroke::new(2.0, box_col), egui::StrokeKind::Inside);
        painter.text(r.left_top() + egui::vec2(6.0, 5.0), egui::Align2::LEFT_TOP, format!("Zone {} - {msg}", i + 1), egui::FontId::proportional(12.0), col);
        let to_frac = |p: egui::Pos2| (((p.x - vr.left()) / vr.width()) as f64, ((p.y - vr.top()) / vr.height()) as f64);
        if resp.hover_pos().is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
        if resp.drag_started() {
            if let Some(p) = resp.interact_pointer_pos() {
                let f = to_frac(p);
                // Grab anywhere: if not grabbed, the loop's (or box's) centre jumps under the pointer.
                let (lx, ly, lw, lh) = zoom::loop_rect(&lasso, b);
                self.fix_grab = Some(if r.contains(p) { (f.0 - b.x, f.1 - b.y) } else { (lx + lw / 2.0 - b.x, ly + lh / 2.0 - b.y) });
            }
        }
        if resp.dragged() {
            if let (Some(off), Some(p)) = (self.fix_grab, resp.interact_pointer_pos()) {
                let f = to_frac(p);
                self.fix_box = Some(ZBox { x: f.0 - off.0, y: f.1 - off.1, s: b.s }.clamped());
            }
        }
        if resp.drag_stopped() && self.fix_grab.take().is_some() {
            self.finish_fix(i, Vec::new());
        }
    }

    /// The fix was placed: lock it (with a new loop, or empty to keep the
    /// one before) and track on from here.
    pub(super) fn finish_fix(&mut self, i: usize, lasso: Vec<[f64; 2]>) {
        let t = self.position();
        let (Some(b), Some(z)) = (self.fix_box, self.zones.get_mut(i)) else { return };
        z.locks.retain(|l| l.t < t - 1e-6);
        z.locks.push(zoom::Lock { t, b, lasso });
        z.track_points.retain(|p| p.t < t - 1e-6);
        let r = z.start_rect(Some(z.locks.len() - 1));
        self.fix_zone = None;
        self.fix_box = None;
        self.lasso_mode = false;
        self.start_track(i, t, r);
    }
}

/// A loop drawn in box `b`, and the rectangle the tracker follows.
pub(super) fn paint_loop(painter: &egui::Painter, vr: egui::Rect, lasso: &[[f64; 2]], b: ZBox, col: Color32) {
    let to_screen = |x: f64, y: f64| egui::pos2(vr.left() + x as f32 * vr.width(), vr.top() + y as f32 * vr.height());
    let pts: Vec<egui::Pos2> = lasso.iter().map(|p| to_screen(b.x + p[0] * b.s, b.y + p[1] * b.s)).collect();
    if pts.len() > 2 {
        painter.add(egui::Shape::closed_line(pts, Stroke::new(2.0, col)));
    }
    let (x, y, w, h) = zoom::loop_rect(lasso, b);
    let r = egui::Rect::from_min_max(to_screen(x, y), to_screen(x + w, y + h));
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, col.gamma_multiply(0.5)), egui::StrokeKind::Outside);
}

/// A message across the video, `y` below its top.
/// A message across the video with its top at `top`, wrapped to fit.
/// Returns where the next one can go.
fn banner(painter: &egui::Painter, vr: egui::Rect, top: f32, msg: &str, col: Color32) -> f32 {
    let galley = painter.layout(msg.to_string(), egui::FontId::proportional(14.0), col, (vr.width() - 40.0).max(100.0));
    let size = galley.size() + egui::vec2(16.0, 8.0);
    let bg = egui::Rect::from_min_size(egui::pos2(vr.center().x - size.x / 2.0, top), size);
    painter.rect_filled(bg, 4.0, Color32::from_black_alpha(200));
    painter.galley(bg.min + egui::vec2(8.0, 4.0), galley, col);
    bg.bottom() + 6.0
}

pub(super) struct TrackCtx {
    tracking: Option<usize>,
    progress: f32,
    paused: bool,
    pos: f64,
    fix_zone: Option<usize>,
    lasso_mode: bool,
    lasso_error: Option<String>,
    sel_zone: Option<usize>,
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
    // The optional loop around just the thing to follow.
    let fixing = c.fix_zone == Some(i);
    let drawing_loop = c.lasso_mode && c.sel_zone == Some(i);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new("Loop (optional)").small().color(theme::MUTED));
        if fixing {
            if ui
                .add(egui::Button::selectable(drawing_loop, "Redraw loop"))
                .on_hover_text("The target changed shape? Draw a new loop around it inside the yellow box. Tracking restarts from here.")
                .clicked()
            {
                act = Some(TrackAct::FixLoop);
            }
        } else {
            let label = if drawing_loop { "Stop drawing" } else if z.lasso.is_empty() { "Draw loop" } else { "Redraw loop" };
            if ui
                .add_enabled(z.a_drawn && !busy_other, egui::Button::selectable(drawing_loop, label))
                .on_hover_text("Draw around just the thing to follow, inside the box. It tracks better, and the zoom keeps your framing around it.")
                .on_disabled_hover_text(if z.a_drawn { "Another zone is being tracked" } else { "Draw the box first" })
                .clicked()
            {
                act = Some(TrackAct::DrawLoop);
            }
            if !z.lasso.is_empty() && ui.add_enabled(!busy_other, egui::Button::new("Clear loop")).on_hover_text("Follow the whole box instead").clicked() {
                act = Some(TrackAct::ClearLoop);
            }
        }
    });
    if drawing_loop {
        match &c.lasso_error {
            Some(e) => {
                let kept = if z.lasso.is_empty() || fixing { "" } else { " Your old loop is kept until a new one works." };
                theme::warn(ui, format!("Loop not used: {e}{kept}"));
            }
            None => theme::hint(ui, "Drag on the video to draw the loop. Esc to stop."),
        }
    }
    ui.horizontal_wrapped(|ui| {
        let label = if z.tracked() { "Track again" } else { "Track" };
        if ui
            .add_enabled(z.a_drawn && !busy_other, egui::Button::new(label))
            .on_hover_text("Follow what's in the box to the end of the zone")
            .on_disabled_hover_text(if z.a_drawn { "Another zone is being tracked" } else { "Draw the box around the thing to follow first" })
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
        theme::hint(ui, "Draw the box around the thing to follow, then click Track. The zoom stays the box's size; only its position follows.");
    } else {
        theme::hint(ui, "Click Track to follow what's in the box.");
    }
    if fixing {
        theme::hint(
            ui,
            if drawing_loop {
                "Draw a loop around the target inside the yellow box. Tracking restarts from here when you let go."
            } else if z.loop_at(c.pos).is_empty() {
                "Drag the box onto the target on the video. Tracking restarts from here when you let go."
            } else {
                "Drag so the loop sits on the target; the framing moves with it. Tracking restarts from here when you let go."
            },
        );
    }
    act
}

pub(super) enum TrackAct {
    Track,
    Cancel,
    Fix,
    Smooth(f64),
    RemoveLock(usize),
    DrawLoop,
    ClearLoop,
    FixLoop,
}
