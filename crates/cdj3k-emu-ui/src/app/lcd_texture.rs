//! GL texture allocation, sub-image upload, and blank-on-exit for the
//! main 1280×720 display and the 320×240 jog LCD framebuffers.

use cdj3k_emu_streams::jog_stream::{JogFrame, JOG_FB_H, JOG_FB_W};
use cdj3k_emu_streams::main_stream::{self, DisplayDirty};
use egui::Color32;
use glow::HasContext as _;

use super::CdjApp;

/// XRGB / RGBA pixel size in bytes.
const PX_BYTES: usize = 4;

impl CdjApp {
    /// Zero both LCD GL textures so popout windows go black after QEMU exit.
    /// Cleared once by the next paint after `lcd_textures_need_blank` is set.
    pub(super) fn blank_lcd_textures(&mut self, gl: &glow::Context) {
        unsafe {
            if let Some(tex) = self.display_gl_tex {
                let zeros = vec![0u8; main_stream::LCD_W * main_stream::LCD_H * PX_BYTES];
                upload_full_zero(
                    gl,
                    tex,
                    main_stream::LCD_W as i32,
                    main_stream::LCD_H as i32,
                    &zeros,
                );
            }
            if let Some(tex) = self.jog_gl_tex {
                let zeros = vec![0u8; (JOG_FB_W * JOG_FB_H) as usize * PX_BYTES];
                upload_full_zero(gl, tex, JOG_FB_W as i32, JOG_FB_H as i32, &zeros);
            }
        }
    }

    /// Lazy-allocate the main LCD texture and upload a dirty sub-rect.
    /// Returns `true` if `display_tex_id` is now usable.
    pub(super) fn upload_display_dirty(
        &mut self,
        frame: &mut eframe::Frame,
        dirty: &DisplayDirty,
    ) -> bool {
        let Some(gl) = frame.gl().cloned() else {
            return false;
        };

        if self.display_gl_tex.is_none() {
            // TEXTURE_SWIZZLE_A = ONE forces sampled alpha to 1.0 so egui's
            // premultiplied blend works even though QEMU stores 0 in the X byte.
            let tex = unsafe {
                allocate_lcd_texture(
                    &gl,
                    main_stream::LCD_W as i32,
                    main_stream::LCD_H as i32,
                    LcdSwizzle::AlphaOnly,
                )
            };
            let tex_id = frame.register_native_glow_texture(tex);
            self.display_gl_tex = Some(tex);
            self.display_tex_id = Some(tex_id);
        }

        // Zero-copy upload: shm pixels are RGBA8888 (QEMU converts XRGB→RGBA
        // in shm_gfx_update); UNPACK_ROW_LENGTH lets us point directly into
        // the mmap without any row copy, even for dirty sub-rects.
        if let Some(tex) = self.display_gl_tex {
            let offset = main_stream::SHM_PIXELS_OFFSET
                + dirty.y as usize * dirty.stride as usize
                + dirty.x as usize * PX_BYTES;
            // Minimum slice: full rows except last (which only needs w*4 bytes).
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

    /// Lazy-allocate the jog LCD texture (with channel swizzles) and upload
    /// the dirty sub-rect from the latest [`JogFrame`].
    pub(super) fn upload_jog_dirty(&mut self, frame: &mut eframe::Frame, jf: &JogFrame) -> bool {
        let Some(gl) = frame.gl().cloned() else {
            return false;
        };

        if self.jog_gl_tex.is_none() {
            // Channel swizzles: shader.r ← wire byte 2, shader.b ← wire byte 0,
            // shader.a ← 1.0. Lets the shader handle the byte-swap instead of
            // a per-pixel CPU pass on the stream thread.
            let tex = unsafe {
                allocate_lcd_texture(
                    &gl,
                    JOG_FB_W as i32,
                    JOG_FB_H as i32,
                    LcdSwizzle::BgrToRgbAlphaOne,
                )
            };
            let tex_id = frame.register_native_glow_texture(tex);
            self.jog_gl_tex = Some(tex);
            self.jog_tex_id = Some(tex_id);
        }

        if let Some(tex) = self.jog_gl_tex {
            // The jog stream always publishes the full 320×240 canvas in its
            // `image` Arc, even for partial-rect packets (those write the
            // subrect at (jf.x, jf.y) in the same buffer). Use
            // UNPACK_ROW_LENGTH = JOG_FB_W to read at the correct stride.
            //
            // SAFETY: `Color32` is `#[repr(C)] [u8; 4]`, so `Vec<Color32>`
            // and a `&[u8]` of 4× the length share layout. The reborrow lasts
            // only inside this block; the underlying Arc keeps storage alive.
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
    /// Force sampled alpha to 1.0; leave RGB untouched.
    AlphaOnly,
    /// BGR(X) wire bytes → shader RGB; sampled alpha = 1.0.
    BgrToRgbAlphaOne,
}

/// Allocate an SRGB8_ALPHA8 LCD texture with linear sampling, clamp wrap, and
/// the requested channel swizzle.
unsafe fn allocate_lcd_texture(
    gl: &glow::Context,
    w: i32,
    h: i32,
    swizzle: LcdSwizzle,
) -> glow::Texture {
    let tex = gl.create_texture().expect("create LCD texture");
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
    gl.tex_image_2d(
        glow::TEXTURE_2D,
        0,
        glow::SRGB8_ALPHA8 as i32,
        w,
        h,
        0,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        None,
    );
    gl.bind_texture(glow::TEXTURE_2D, None);
    tex
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
        glow::PixelUnpackData::Slice(pixels),
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
        glow::PixelUnpackData::Slice(zeros),
    );
    gl.bind_texture(glow::TEXTURE_2D, None);
}
