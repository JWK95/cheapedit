//! Draws libmpv's video into an OpenGL texture that egui shows like an image.

use crate::mpv::{Api, Player, Renderer};
use eframe::egui;
use eframe::glow::{self, HasContext};
use std::sync::Arc;

struct Target {
    tex: glow::Texture,
    fbo: glow::Framebuffer,
    id: egui::TextureId,
    size: [i32; 2],
}

pub struct VideoView {
    // Field order matters: the renderer must be freed before the player.
    renderer: Option<Renderer>,
    pub player: Player,
    target: Option<Target>,
    pub error: Option<String>,
    /// Draw this many more frames even if mpv doesn't flag a new one.
    /// mpv doesn't always flag the first frame of a file loaded while paused,
    /// which left the previous video on screen.
    pub force_frames: u8,
}

impl VideoView {
    pub fn new(api: Arc<Api>) -> Result<VideoView, String> {
        Ok(VideoView { renderer: None, player: Player::new(api)?, target: None, error: None, force_frames: 0 })
    }

    /// Paint the video filling `rect`. Must be called from `App::ui`.
    pub fn paint(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, rect: egui::Rect) {
        ui.painter().rect_filled(rect, 4.0, egui::Color32::BLACK);
        let Some(gl) = frame.gl().cloned() else {
            self.error = Some("cheapedit needs OpenGL to show video.".into());
            return;
        };
        if self.renderer.is_none() && self.error.is_none() {
            let ctx = ui.ctx().clone();
            match self.player.create_renderer(Box::new(move || ctx.request_repaint())) {
                Ok(r) => self.renderer = Some(r),
                Err(e) => self.error = Some(e),
            }
        }
        let Some(renderer) = &self.renderer else { return };

        let ppp = ui.ctx().pixels_per_point();
        let size = [(rect.width() * ppp).round().max(2.0) as i32, (rect.height() * ppp).round().max(2.0) as i32];
        let mut resized = false;
        unsafe {
            match &mut self.target {
                None => {
                    let tex = gl.create_texture().expect("GL texture");
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                    alloc(&gl, size);
                    let fbo = gl.create_framebuffer().expect("GL framebuffer");
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                    gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
                    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                    gl.bind_texture(glow::TEXTURE_2D, None);
                    // egui takes ownership of the texture; we keep using the same GL name and
                    // only reallocate its storage on resize, so the egui id stays valid.
                    let id = frame.register_native_glow_texture(tex);
                    self.target = Some(Target { tex, fbo, id, size });
                    resized = true;
                }
                Some(t) if t.size != size => {
                    gl.bind_texture(glow::TEXTURE_2D, Some(t.tex));
                    alloc(&gl, size);
                    gl.bind_texture(glow::TEXTURE_2D, None);
                    t.size = size;
                    resized = true;
                }
                _ => {}
            }
        }
        let t = self.target.as_ref().unwrap();
        let forced = self.force_frames > 0;
        if renderer.wants_frame() || resized || forced {
            renderer.render(t.fbo.0.get(), size[0], size[1]);
            unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, None) };
        }
        if forced {
            self.force_frames -= 1;
            ui.ctx().request_repaint();
        }
        ui.painter().image(t.id, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }

    /// Free GL objects. Call from `App::on_exit` while the GL context is current.
    pub fn destroy_gl(&mut self, gl: Option<&glow::Context>) {
        self.renderer = None;
        if let (Some(gl), Some(t)) = (gl, self.target.take()) {
            unsafe { gl.delete_framebuffer(t.fbo) };
        }
    }
}

unsafe fn alloc(gl: &glow::Context, size: [i32; 2]) {
    unsafe {
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            size[0],
            size[1],
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
    }
}
