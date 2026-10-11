// Bloom post-processing pipeline.
//
// One PaintCallback at Order::Debug, the last GL operation of the frame, runs
// five passes:
//   1. `glBlitFramebuffer` copies the current egui frame from the screen into
//      `copy_fbo`.
//   2. The threshold pass writes `copy_tex` into `blur_fbo[0]` at half
//      resolution. It keeps only bright, saturated pixels and masks out the LCD
//      rects, except where a control stands over them.
//   3. The horizontal blur writes `blur_fbo[0]` into `blur_fbo[1]`.
//   4. The vertical blur writes `blur_fbo[1]` back into `blur_fbo[0]`.
//   5. The composite adds `blur_tex[0]` times the strength onto the screen,
//      under the same mask, so no halo lands on a screen.
//
// The halo around an LED (about 0.03 at 2 px) is far below BLOOM_THRESHOLD, so
// halo pixels never bloom again on the next frame.

use egui::Rect;
use glow::HasContext;
use std::sync::{Arc, Mutex};

mod shaders;

use shaders::{BLUR_FRAG, COMPOSITE_FRAG, THRESHOLD_FRAG, VERT};

// ── Tuning constants ───────────────────────────────────────────────────────────

/// Minimum brightness, the largest of R, G and B, for a pixel to enter the
/// bloom. It must stay below the dimmest lit LED in the palette, COL_GREEN
/// (200/255 = 0.784).
pub const BLOOM_THRESHOLD: f32 = 0.55;

/// Transition band above BLOOM_THRESHOLD: a pixel's weight ramps from zero to
/// full over [BLOOM_THRESHOLD, BLOOM_THRESHOLD * (1 + BLOOM_THRESHOLD_KNEE)].
const BLOOM_THRESHOLD_KNEE: f32 = 0.20;

/// Minimum HSV saturation for a pixel to bloom. It excludes grey and white UI
/// chrome (COL_BTN_TEXT has a saturation of about 0.05, COL_SILVER about 0.09).
/// The shader lets pure white bloom anyway.
pub const BLOOM_SAT_MIN: f32 = 0.10;

/// Saturation transition band: a pixel's weight ramps from zero to full over
/// [BLOOM_SAT_MIN, BLOOM_SAT_MIN + BLOOM_SAT_KNEE].
const BLOOM_SAT_KNEE: f32 = 0.15;

/// How much of the blurred layer is added to the scene.
pub const BLOOM_STRENGTH: f32 = 0.9;

/// Blur step size in half-resolution texels; 1.0 is one texel per tap.
pub const BLOOM_RADIUS: f32 = 1.0;

/// Maximum number of LCD exclusion rectangles.
const MAX_EXCLUDE_ZONES: usize = 4;
/// Must match the `u_keep_*` array sizes in the shaders.
const MAX_KEEP_ZONES: usize = 2;
/// Must match the `u_chrome` array size in the shaders.
const MAX_CHROME_ZONES: usize = 8;

/// A control drawn over a masked screen rect. Inside the control's area, it
/// blooms and receives bloom as usual. The area is everything within `core_r`
/// of `centre`, plus everything within `outer_r` that lies in a vertical band
/// `half_w` wide or in a sector `atan(tan_a)` either side of vertical. Values
/// are in logical points.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BloomKeep {
    pub centre: egui::Pos2,
    pub core_r: f32,
    pub outer_r: f32,
    pub half_w: f32,
    pub tan_a: f32,
}

/// Where bloom is suppressed: the screens, minus the controls drawn over them.
#[derive(Clone, Debug, Default)]
pub struct BloomMask {
    pub excludes: Vec<Rect>,
    pub keeps: Vec<BloomKeep>,
    /// The application's own chrome, the menu strip and its panels. It neither
    /// blooms nor receives a halo, whatever is drawn under it.
    pub chrome: Vec<Rect>,
}

// ── Pipeline ───────────────────────────────────────────────────────────────────

pub struct BloomPipeline {
    threshold_prog: glow::Program,
    blur_prog: glow::Program,
    composite_prog: glow::Program,
    quad_vao: glow::VertexArray,
    // A full-resolution copy of the screen, read by the threshold pass.
    copy_tex: Option<glow::Texture>,
    copy_fbo: Option<glow::Framebuffer>,
    // Two half-resolution targets that the blur passes alternate between.
    blur_tex: Option<[glow::Texture; 2]>,
    blur_fbo: Option<[glow::Framebuffer; 2]>,
    size: [i32; 2],
    /// Hash of the inputs that produced the current `blur_tex[0]`. When `run()`
    /// gets a matching key, it skips the blit, threshold and blur passes and
    /// composites the cached blur. A resize clears it.
    cache_key: Option<u64>,
}

impl BloomPipeline {
    pub fn new(gl: &glow::Context) -> Self {
        unsafe {
            let threshold_prog = compile_program(gl, VERT, THRESHOLD_FRAG);
            let blur_prog = compile_program(gl, VERT, BLUR_FRAG);
            let composite_prog = compile_program(gl, VERT, COMPOSITE_FRAG);
            let quad_vao = gl
                .create_vertex_array()
                .expect("could not create the bloom vertex array");
            Self {
                threshold_prog,
                blur_prog,
                composite_prog,
                quad_vao,
                copy_tex: None,
                copy_fbo: None,
                blur_tex: None,
                blur_fbo: None,
                size: [0, 0],
                cache_key: None,
            }
        }
    }

    /// Run the bloom pass. Call from the Order::Debug PaintCallback.
    ///
    /// `w` and `h` are in physical pixels, and `ppp` is pixels per point (the
    /// DPI scale). `mask` marks where bloom neither starts nor lands, in logical
    /// points with Y down. `scene_key` hashes every input that affects the
    /// pixels that bloom, such as the LED state, button hover and jog ring
    /// brightness. When the key matches the cached one, the blit, threshold and
    /// blur passes are skipped and the retained `blur_tex[0]` is composited.
    /// This makes the bloom callback more than five times faster on idle
    /// frames.
    ///
    /// The composite always runs, because egui clears the framebuffer every
    /// frame.
    pub fn run(
        &mut self,
        gl: &glow::Context,
        w: i32,
        h: i32,
        ppp: f32,
        mask: &BloomMask,
        scene_key: u64,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.resize_if_needed(gl, w, h);

        // Combine the caller's scene hash with the layout parameters that also
        // invalidate the cached blur: the window size, the DPI and the mask
        // rects. A resize already clears `cache_key`, but `ppp` and the mask
        // can change without a resize.
        let full_key = compose_key(scene_key, w, h, ppp, mask);
        let blur_is_cached = self.cache_key == Some(full_key);

        unsafe {
            if !blur_is_cached {
                self.blur_scene(gl, w, h, ppp, mask);
                self.cache_key = Some(full_key);
            } else {
                gl.bind_vertex_array(Some(self.quad_vao));
                gl.active_texture(glow::TEXTURE0);
            }
            self.composite(gl, w, h, ppp, mask);
            restore_gl_state(gl);
        }
    }

    /// Passes 1-4: copies the screen, thresholds it to half resolution and
    /// blurs it both ways, leaving the result in `blur_tex[0]`.
    unsafe fn blur_scene(&self, gl: &glow::Context, w: i32, h: i32, ppp: f32, mask: &BloomMask) {
        let hw = (w / 2).max(1);
        let hh = (h / 2).max(1);
        let copy_tex = self.copy_tex.unwrap();
        let copy_fbo = self.copy_fbo.unwrap();
        let blur_tex = self.blur_tex.unwrap();
        let blur_fbo = self.blur_fbo.unwrap();

        // ── 1. Blit the screen into copy_fbo ─────────────────────────────────
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(copy_fbo));
        gl.blit_framebuffer(
            0,
            0,
            w,
            h,
            0,
            0,
            w,
            h,
            glow::COLOR_BUFFER_BIT,
            glow::NEAREST,
        );

        gl.bind_vertex_array(Some(self.quad_vao));

        // ── 2. Threshold copy_tex into blur_fbo[0] at half resolution ────────
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(blur_fbo[0]));
        gl.viewport(0, 0, hw, hh);
        gl.use_program(Some(self.threshold_prog));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(copy_tex));
        set_1i(gl, self.threshold_prog, "u_scene", 0);
        set_1f(gl, self.threshold_prog, "u_threshold", BLOOM_THRESHOLD);
        set_1f(
            gl,
            self.threshold_prog,
            "u_threshold_knee",
            BLOOM_THRESHOLD_KNEE,
        );
        set_1f(gl, self.threshold_prog, "u_sat_min", BLOOM_SAT_MIN);
        set_1f(gl, self.threshold_prog, "u_sat_knee", BLOOM_SAT_KNEE);
        set_mask(gl, self.threshold_prog, mask, ppp, w, h);
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

        // ── 3. Blur blur_fbo[0] horizontally into blur_fbo[1] ────────────────
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(blur_fbo[1]));
        gl.use_program(Some(self.blur_prog));
        gl.bind_texture(glow::TEXTURE_2D, Some(blur_tex[0]));
        set_1i(gl, self.blur_prog, "u_tex", 0);
        set_2f(gl, self.blur_prog, "u_dir", BLOOM_RADIUS / hw as f32, 0.0);
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

        // ── 4. Blur blur_fbo[1] vertically into blur_fbo[0] ──────────────────
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(blur_fbo[0]));
        gl.bind_texture(glow::TEXTURE_2D, Some(blur_tex[1]));
        set_2f(gl, self.blur_prog, "u_dir", 0.0, BLOOM_RADIUS / hh as f32);
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
    }

    /// Pass 5: adds the blur onto the screen, under the mask.
    unsafe fn composite(&self, gl: &glow::Context, w: i32, h: i32, ppp: f32, mask: &BloomMask) {
        let blur_tex = self.blur_tex.unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.viewport(0, 0, w, h);
        gl.disable(glow::SCISSOR_TEST);
        gl.enable(glow::BLEND);
        gl.blend_func_separate(
            glow::ONE,
            glow::ONE, // RGB: additive
            glow::ZERO,
            glow::ONE, // Alpha: keep the destination's alpha, which egui writes as 1.0
        );
        gl.use_program(Some(self.composite_prog));
        gl.bind_texture(glow::TEXTURE_2D, Some(blur_tex[0]));
        set_1i(gl, self.composite_prog, "u_bloom", 0);
        set_1f(gl, self.composite_prog, "u_strength", BLOOM_STRENGTH);
        set_mask(gl, self.composite_prog, mask, ppp, w, h);
        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.threshold_prog);
            gl.delete_program(self.blur_prog);
            gl.delete_program(self.composite_prog);
            gl.delete_vertex_array(self.quad_vao);
            if let Some(t) = self.copy_tex {
                gl.delete_texture(t);
            }
            if let Some(f) = self.copy_fbo {
                gl.delete_framebuffer(f);
            }
            if let Some([t0, t1]) = self.blur_tex {
                gl.delete_texture(t0);
                gl.delete_texture(t1);
            }
            if let Some([f0, f1]) = self.blur_fbo {
                gl.delete_framebuffer(f0);
                gl.delete_framebuffer(f1);
            }
        }
    }

    fn resize_if_needed(&mut self, gl: &glow::Context, w: i32, h: i32) {
        if self.size == [w, h] {
            return;
        }
        unsafe {
            if let Some(t) = self.copy_tex.take() {
                gl.delete_texture(t);
            }
            if let Some(f) = self.copy_fbo.take() {
                gl.delete_framebuffer(f);
            }
            if let Some([t0, t1]) = self.blur_tex.take() {
                gl.delete_texture(t0);
                gl.delete_texture(t1);
            }
            if let Some([f0, f1]) = self.blur_fbo.take() {
                gl.delete_framebuffer(f0);
                gl.delete_framebuffer(f1);
            }
            let hw = (w / 2).max(1);
            let hh = (h / 2).max(1);
            let (ct, cf) = make_fbo(gl, w, h);
            let (bt0, bf0) = make_fbo(gl, hw, hh);
            let (bt1, bf1) = make_fbo(gl, hw, hh);
            self.copy_tex = Some(ct);
            self.copy_fbo = Some(cf);
            self.blur_tex = Some([bt0, bt1]);
            self.blur_fbo = Some([bf0, bf1]);
            self.size = [w, h];
            // The cached blur has the previous viewport's size; clear the key
            // so the next frame builds it again at the new size.
            self.cache_key = None;
        }
    }
}

/// Hand egui back the GL state it expects.
unsafe fn restore_gl_state(gl: &glow::Context) {
    gl.bind_texture(glow::TEXTURE_2D, None);
    gl.blend_func_separate(
        glow::ONE,
        glow::ONE_MINUS_SRC_ALPHA,
        glow::ONE_MINUS_DST_ALPHA,
        glow::ONE,
    );
    gl.use_program(None);
    gl.bind_vertex_array(None);
    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
}

/// Upload `mask`, given in egui logical points with Y down, to `prog`'s mask
/// uniforms. Rects are sent as UV coordinates and keeps in physical pixels. The
/// program must be in use.
unsafe fn set_mask(
    gl: &glow::Context,
    prog: glow::Program,
    mask: &BloomMask,
    ppp: f32,
    w: i32,
    h: i32,
) {
    let loc = |name: &str| gl.get_uniform_location(prog, name);
    set_1i(
        gl,
        prog,
        "u_exclude_n",
        mask.excludes.len().min(MAX_EXCLUDE_ZONES) as i32,
    );
    for (i, rect) in mask.excludes.iter().take(MAX_EXCLUDE_ZONES).enumerate() {
        let [x0, y0, x1, y1] = uv_rect(rect, ppp, w, h);
        gl.uniform_4_f32(loc(&format!("u_exclude[{i}]")).as_ref(), x0, y0, x1, y1);
    }
    set_1i(
        gl,
        prog,
        "u_chrome_n",
        mask.chrome.len().min(MAX_CHROME_ZONES) as i32,
    );
    for (i, rect) in mask.chrome.iter().take(MAX_CHROME_ZONES).enumerate() {
        let [x0, y0, x1, y1] = uv_rect(rect, ppp, w, h);
        gl.uniform_4_f32(loc(&format!("u_chrome[{i}]")).as_ref(), x0, y0, x1, y1);
    }
    set_2f(gl, prog, "u_screen_px", w as f32, h as f32);
    set_1i(
        gl,
        prog,
        "u_keep_n",
        mask.keeps.len().min(MAX_KEEP_ZONES) as i32,
    );
    for (i, k) in mask.keeps.iter().take(MAX_KEEP_ZONES).enumerate() {
        gl.uniform_4_f32(
            loc(&format!("u_keep_a[{i}]")).as_ref(),
            k.centre.x * ppp,
            h as f32 - k.centre.y * ppp,
            k.core_r * ppp,
            k.outer_r * ppp,
        );
        gl.uniform_2_f32(
            loc(&format!("u_keep_b[{i}]")).as_ref(),
            k.half_w * ppp,
            k.tan_a,
        );
    }
}

/// Converts a rect in points to (x0, y0, x1, y1) in UV space, with Y flipped
/// for GL.
fn uv_rect(rect: &Rect, ppp: f32, w: i32, h: i32) -> [f32; 4] {
    [
        rect.min.x * ppp / w as f32,
        1.0 - rect.max.y * ppp / h as f32,
        rect.max.x * ppp / w as f32,
        1.0 - rect.min.y * ppp / h as f32,
    ]
}

/// Combine the caller's scene hash with the layout parameters that also
/// invalidate the cached blur. `DefaultHasher` (SipHash-1-3) takes a few
/// hundred nanoseconds, which is negligible next to the bloom passes.
fn compose_key(scene_key: u64, w: i32, h: i32, ppp: f32, mask: &BloomMask) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h_ = std::collections::hash_map::DefaultHasher::new();
    scene_key.hash(&mut h_);
    w.hash(&mut h_);
    h.hash(&mut h_);
    // `ppp` and the rect coordinates are floats, so hash their bit patterns.
    (ppp.to_bits()).hash(&mut h_);
    for r in mask
        .excludes
        .iter()
        .take(MAX_EXCLUDE_ZONES)
        .chain(mask.chrome.iter().take(MAX_CHROME_ZONES))
    {
        r.min.x.to_bits().hash(&mut h_);
        r.min.y.to_bits().hash(&mut h_);
        r.max.x.to_bits().hash(&mut h_);
        r.max.y.to_bits().hash(&mut h_);
    }
    for k in mask.keeps.iter().take(MAX_KEEP_ZONES) {
        for v in [
            k.centre.x, k.centre.y, k.core_r, k.outer_r, k.half_w, k.tan_a,
        ] {
            v.to_bits().hash(&mut h_);
        }
    }
    h_.finish()
}

// ── GL helpers ─────────────────────────────────────────────────────────────────

unsafe fn make_fbo(gl: &glow::Context, w: i32, h: i32) -> (glow::Texture, glow::Framebuffer) {
    let tex = gl
        .create_texture()
        .expect("could not create a bloom texture");
    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
    gl.tex_image_2d(
        glow::TEXTURE_2D,
        0,
        glow::RGBA as i32,
        w,
        h,
        0,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelUnpackData::Slice(None),
    );
    for (param, val) in [
        (glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32),
        (glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32),
        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32),
        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32),
    ] {
        gl.tex_parameter_i32(glow::TEXTURE_2D, param, val);
    }
    let fbo = gl
        .create_framebuffer()
        .expect("could not create a bloom framebuffer");
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
    (tex, fbo)
}

unsafe fn compile_program(gl: &glow::Context, vert: &str, frag: &str) -> glow::Program {
    let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
    gl.shader_source(vs, vert);
    gl.compile_shader(vs);
    assert!(
        gl.get_shader_compile_status(vs),
        "the bloom vertex shader did not compile: {}",
        gl.get_shader_info_log(vs)
    );

    let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
    gl.shader_source(fs, frag);
    gl.compile_shader(fs);
    assert!(
        gl.get_shader_compile_status(fs),
        "the bloom fragment shader did not compile: {}",
        gl.get_shader_info_log(fs)
    );

    let prog = gl.create_program().unwrap();
    gl.attach_shader(prog, vs);
    gl.attach_shader(prog, fs);
    gl.link_program(prog);
    assert!(
        gl.get_program_link_status(prog),
        "the bloom shader program did not link: {}",
        gl.get_program_info_log(prog)
    );
    gl.detach_shader(prog, vs);
    gl.delete_shader(vs);
    gl.detach_shader(prog, fs);
    gl.delete_shader(fs);
    prog
}

fn set_1i(gl: &glow::Context, prog: glow::Program, name: &str, v: i32) {
    unsafe {
        if let Some(l) = gl.get_uniform_location(prog, name) {
            gl.uniform_1_i32(Some(&l), v);
        }
    }
}

fn set_1f(gl: &glow::Context, prog: glow::Program, name: &str, v: f32) {
    unsafe {
        if let Some(l) = gl.get_uniform_location(prog, name) {
            gl.uniform_1_f32(Some(&l), v);
        }
    }
}

fn set_2f(gl: &glow::Context, prog: glow::Program, name: &str, x: f32, y: f32) {
    unsafe {
        if let Some(l) = gl.get_uniform_location(prog, name) {
            gl.uniform_2_f32(Some(&l), x, y);
        }
    }
}

pub type SharedBloom = Arc<Mutex<BloomPipeline>>;
