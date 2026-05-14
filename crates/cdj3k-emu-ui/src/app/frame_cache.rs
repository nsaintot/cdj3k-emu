//! Whole-framebuffer cache: stores the final post-bloom screen contents in a
//! GPU texture so the next "nothing-changed" frame can blit it back instead
//! of running the entire CPU draw + bloom pipeline.
//!
//! Intended call order on an unchanged frame:
//!   1. `restore(gl, w, h)` → `glBlitFramebuffer(cache → screen)`
//!   2. (nothing else)
//!
//! On a "real" frame:
//!   1. egui renders normally
//!   2. bloom runs
//!   3. `capture(gl, w, h)` → `glBlitFramebuffer(screen → cache)`
//!
//! The cache is a single `RGBA8` texture sized to the current viewport
//! (physical pixels). Resize invalidates `has_data` so the next frame is
//! forced through the full render path even if the scene key matches.

use glow::HasContext as _;
use std::sync::{Arc, Mutex};

pub struct FrameCache {
    tex: Option<glow::Texture>,
    fbo: Option<glow::Framebuffer>,
    size: [i32; 2],
    /// `true` once at least one `capture` succeeded at the current `size`.
    /// Reset on resize (the cached texture is the wrong dimensions).
    has_data: bool,
}

impl FrameCache {
    pub fn new() -> Self {
        Self {
            tex: None,
            fbo: None,
            size: [0, 0],
            has_data: false,
        }
    }

    fn resize_if_needed(&mut self, gl: &glow::Context, w: i32, h: i32) {
        if self.size == [w, h] && self.tex.is_some() {
            return;
        }
        unsafe {
            if let Some(t) = self.tex.take() {
                gl.delete_texture(t);
            }
            if let Some(f) = self.fbo.take() {
                gl.delete_framebuffer(f);
            }
            let tex = gl.create_texture().expect("frame_cache tex");
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w,
                h,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                None,
            );
            for (param, val) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, val);
            }
            let fbo = gl.create_framebuffer().expect("frame_cache fbo");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            self.tex = Some(tex);
            self.fbo = Some(fbo);
            self.size = [w, h];
            self.has_data = false;
        }
    }

    /// Copy the current default-framebuffer (screen) contents into the cache.
    /// Call after bloom finishes.
    pub fn capture(&mut self, gl: &glow::Context, w: i32, h: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.resize_if_needed(gl, w, h);
        let fbo = match self.fbo {
            Some(f) => f,
            None => return,
        };
        unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(fbo));
            gl.blit_framebuffer(
                0, 0, w, h,
                0, 0, w, h,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        }
        self.has_data = true;
    }

    /// Copy the cached framebuffer back to the default framebuffer (screen).
    /// Call this exactly once on a "skip-frame" before the buffer swap.
    /// Returns `false` if there's no usable cache (e.g. first frame, or after
    /// a resize) so the caller can fall back to the full render path.
    pub fn restore(&self, gl: &glow::Context, w: i32, h: i32) -> bool {
        if !self.has_data || self.size != [w, h] || self.fbo.is_none() {
            return false;
        }
        let fbo = self.fbo.unwrap();
        unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
            gl.blit_framebuffer(
                0, 0, w, h,
                0, 0, w, h,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
        }
        true
    }

    pub fn has_data(&self) -> bool {
        self.has_data
    }

    #[allow(dead_code)]
    pub fn invalidate(&mut self) {
        self.has_data = false;
    }
}

pub type SharedFrameCache = Arc<Mutex<FrameCache>>;
