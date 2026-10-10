//! Allocates, updates and blanks the GL textures of the main display (sized
//! by [`Model::main_lcd`](cdj3k_emu_panel::Model::main_lcd)) and of the
//! 320×240 jog LCD.

use cdj3k_emu_streams::jog_stream::{JogFrame, JOG_FB_H, JOG_FB_W};
use cdj3k_emu_streams::main_stream::{DisplayDirty, SHM_PIXELS_OFFSET};
use egui::Color32;
use glow::HasContext as _;

use super::CdjApp;

/// Size of an XRGB or RGBA pixel, in bytes.
const PX_BYTES: usize = 4;

impl CdjApp {
    /// After a model switch changes the main framebuffer size, re-specify the
    /// storage of the texture that the painter already knows by its id.
    fn ensure_display_storage(&mut self, gl: &glow::Context) {
        if !self.textures.main_stale {
            return;
        }
        if let Some(tex) = self.textures.main_gl {
            let (w, h) = self.model.main_lcd();
            unsafe { specify_lcd_storage(gl, tex, w as i32, h as i32) };
        }
        self.textures.main_stale = false;
    }

    /// Clear both LCD GL textures so the popout windows go black after QEMU
    /// exits. The next paint after `lcd_textures_need_blank` is set calls it
    /// once.
    pub(super) fn blank_lcd_textures(&mut self, gl: &glow::Context) {
        self.ensure_display_storage(gl);
        unsafe {
            if let Some(tex) = self.textures.main_gl {
                let (w, h) = self.model.main_lcd();
                let zeros = vec![0u8; (w * h) as usize * PX_BYTES];
                upload_full_zero(gl, tex, w as i32, h as i32, &zeros);
            }
            if let Some(tex) = self.textures.jog_gl {
                let zeros = vec![0u8; (JOG_FB_W * JOG_FB_H) * PX_BYTES];
                upload_full_zero(gl, tex, JOG_FB_W as i32, JOG_FB_H as i32, &zeros);
            }
        }
    }

    /// Allocate the main LCD texture if needed and upload a dirty sub-rect.
    /// Returns `true` if the texture behind `textures.main_id` is usable.
    pub(super) fn upload_display_dirty(
        &mut self,
        frame: &mut eframe::Frame,
        dirty: &DisplayDirty,
    ) -> bool {
        let Some(gl) = frame.gl().cloned() else {
            return false;
        };

        self.ensure_display_storage(&gl);
        if self.textures.main_gl.is_none() {
            // TEXTURE_SWIZZLE_A = ONE forces the sampled alpha to 1.0, so egui's
            // premultiplied blend works although QEMU stores 0 in the X byte.
            let (w, h) = self.model.main_lcd();
            let tex =
                unsafe { allocate_lcd_texture(&gl, w as i32, h as i32, LcdSwizzle::AlphaOnly) };
            let tex_id = frame.register_native_glow_texture(tex);
            self.textures.main_gl = Some(tex);
            self.textures.main_id = Some(tex_id);
        }

        // Upload without copying. The shm pixels are already RGBA8888, because
        // QEMU converts XRGB to RGBA in shm_gfx_update, and UNPACK_ROW_LENGTH
        // lets the upload read rows straight from the mmap, even for dirty
        // sub-rects.
        if let Some(tex) = self.textures.main_gl {
            let offset = SHM_PIXELS_OFFSET
                + dirty.y as usize * dirty.stride as usize
                + dirty.x as usize * PX_BYTES;
            // The smallest slice that covers the rect: whole rows, except the
            // last row, which needs only w*4 bytes.
            let len = (dirty.h as usize - 1) * dirty.stride as usize + dirty.w as usize * PX_BYTES;
            let pixels = &dirty.mmap[offset..offset + len];
            unsafe {
                upload_sub_image(
                    &gl,
                    tex,
                    dirty.stride as i32 / PX_BYTES as i32,
                    dirty.x as i32,
                    dirty.y as i32,
                    dirty.w as i32,
                    dirty.h as i32,
                    pixels,
                );
            }
        }
        true
    }

    /// Allocate the jog LCD texture, with its channel swizzles, if needed, and
    /// upload the dirty sub-rect from the latest [`JogFrame`].
    pub(super) fn upload_jog_dirty(&mut self, frame: &mut eframe::Frame, jf: &JogFrame) -> bool {
        let Some(gl) = frame.gl().cloned() else {
            return false;
        };

        if self.textures.jog_gl.is_none() {
            // The swizzles make the shader read red from wire byte 2, blue from
            // wire byte 0, and alpha as 1.0. The GPU swaps the bytes, so the
            // stream thread needs no per-pixel pass.
            let tex = unsafe {
                allocate_lcd_texture(
                    &gl,
                    JOG_FB_W as i32,
                    JOG_FB_H as i32,
                    LcdSwizzle::BgrToRgbAlphaOne,
                )
            };
            let tex_id = frame.register_native_glow_texture(tex);
            self.textures.jog_gl = Some(tex);
            self.textures.jog_id = Some(tex_id);
        }

        if let Some(tex) = self.textures.jog_gl {
            // The jog stream always publishes the full 320×240 canvas in its
            // `image` Arc. A partial update writes its rect at (jf.x, jf.y) in
            // the same buffer, so UNPACK_ROW_LENGTH = JOG_FB_W gives the right
            // stride.
            //
            // SAFETY: `Color32` is `#[repr(C)] [u8; 4]`, so `Vec<Color32>`
            // and a `&[u8]` four times as long have the same layout. The
            // reborrow lasts only inside this block, and the Arc keeps the
            // storage alive.
            let pixels = &jf.image.pixels;
            let bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    pixels.as_ptr() as *const u8,
                    pixels.len() * std::mem::size_of::<Color32>(),
                )
            };
            let row_stride_bytes = JOG_FB_W * PX_BYTES;
            let offset = (jf.y as usize) * row_stride_bytes + (jf.x as usize) * PX_BYTES;
            let len = (jf.h as usize - 1) * row_stride_bytes + (jf.w as usize) * PX_BYTES;
            let slice = &bytes[offset..offset + len];
            unsafe {
                upload_sub_image(
                    &gl,
                    tex,
                    JOG_FB_W as i32,
                    jf.x as i32,
                    jf.y as i32,
                    jf.w as i32,
                    jf.h as i32,
                    slice,
                );
            }
        }
        true
    }
}

#[derive(Clone, Copy)]
enum LcdSwizzle {
    /// Forces the sampled alpha to 1.0 and leaves RGB unchanged.
    AlphaOnly,
    /// Reads the BGRX wire bytes as RGB and forces the sampled alpha to 1.0.
    BgrToRgbAlphaOne,
}

/// Allocate an RGBA8 LCD texture with linear sampling, clamp wrap, and the
/// requested channel swizzle. The format is plain RGBA8, because egui's shader
/// treats what it samples as gamma-encoded, and the deck's framebuffer bytes
/// already are.
unsafe fn allocate_lcd_texture(
    gl: &glow::Context,
    w: i32,
    h: i32,
    swizzle: LcdSwizzle,
) -> glow::Texture {
    let tex = gl
        .create_texture()
        .expect("could not create an LCD texture");
    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_MIN_FILTER,
        glow::LINEAR as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_MAG_FILTER,
        glow::LINEAR as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_S,
        glow::CLAMP_TO_EDGE as i32,
    );
    gl.tex_parameter_i32(
        glow::TEXTURE_2D,
        glow::TEXTURE_WRAP_T,
        glow::CLAMP_TO_EDGE as i32,
    );
    match swizzle {
        LcdSwizzle::AlphaOnly => {
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_SWIZZLE_A, glow::ONE as i32);
        }
        LcdSwizzle::BgrToRgbAlphaOne => {
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_SWIZZLE_R, glow::BLUE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_SWIZZLE_B, glow::RED as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_SWIZZLE_A, glow::ONE as i32);
        }
    }
    specify_lcd_storage(gl, tex, w, h);
    tex
}

/// Allocate, or allocate again, `tex`'s level-0 storage as an uninitialised
/// `w`×`h` RGBA8 image.
unsafe fn specify_lcd_storage(gl: &glow::Context, tex: glow::Texture, w: i32, h: i32) {
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
        glow::PixelUnpackData::Slice(None),
    );
    gl.bind_texture(glow::TEXTURE_2D, None);
}

unsafe fn upload_sub_image(
    gl: &glow::Context,
    tex: glow::Texture,
    row_length_px: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    pixels: &[u8],
) {
    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
    gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, row_length_px);
    gl.tex_sub_image_2d(
        glow::TEXTURE_2D,
        0,
        x,
        y,
        w,
        h,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelUnpackData::Slice(Some(pixels)),
    );
    gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
    gl.bind_texture(glow::TEXTURE_2D, None);
}

unsafe fn upload_full_zero(gl: &glow::Context, tex: glow::Texture, w: i32, h: i32, zeros: &[u8]) {
    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
    gl.tex_sub_image_2d(
        glow::TEXTURE_2D,
        0,
        0,
        0,
        w,
        h,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelUnpackData::Slice(Some(zeros)),
    );
    gl.bind_texture(glow::TEXTURE_2D, None);
}
