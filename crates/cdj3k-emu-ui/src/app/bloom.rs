// Bloom post-processing pipeline.
//
// Single PaintCallback at Order::Debug (last GL op of the frame):
//   1. glBlitFramebuffer: screen → copy_fbo  (GPU blit of current egui frame)
//   2. Threshold: copy_tex → blur_fbo[0]  (half-res, bright + saturated pixels only,
//                                           LCD rects masked out)
//   3. H-blur: blur_fbo[0] → blur_fbo[1]
//   4. V-blur: blur_fbo[1] → blur_fbo[0]
//   5. Additive composite: blur_tex[0]*strength → screen
//
// Feedback is structurally bounded: the blur halo around an LED (~0.03 at 2 px)
// is far below BLOOM_THRESHOLD, so halo pixels can never re-bloom themselves.

use egui::Rect;
use glow::HasContext;
use std::sync::{Arc, Mutex};

// ── Tuning constants ───────────────────────────────────────────────────────────

/// Minimum brightness (max of R/G/B) for a pixel to enter the bloom.
/// Dimmest lit LED in the palette is COL_BTN_GREEN (200/255 = 0.784),
/// so this must be below that.
pub const BLOOM_THRESHOLD: f32 = 0.55;

/// Transition band above BLOOM_THRESHOLD: pixels ramp from 0 → full weight
/// over [BLOOM_THRESHOLD, BLOOM_THRESHOLD * (1 + BLOOM_THRESHOLD_KNEE)].
const BLOOM_THRESHOLD_KNEE: f32 = 0.20;

/// Minimum HSV saturation for a pixel to bloom.
/// Excludes grey/white UI chrome (COL_BTN_TEXT sat ≈ 0.05, COL_SILVER sat ≈ 0.09),
/// except pure white, which is explicitly allowed in shader logic.
pub const BLOOM_SAT_MIN: f32 = 0.10;

/// Saturation transition band: pixels ramp from 0 → full weight
/// over [BLOOM_SAT_MIN, BLOOM_SAT_MIN + BLOOM_SAT_KNEE].
const BLOOM_SAT_KNEE: f32 = 0.15;

/// How much of the blurred layer is added to the scene.
pub const BLOOM_STRENGTH: f32 = 0.9;

/// Blur step size in half-res texel units (1.0 = one texel per tap).
pub const BLOOM_RADIUS: f32 = 1.0;

/// Maximum number of LCD exclusion rectangles.
const MAX_EXCLUDE_ZONES: usize = 4;

// ── Shaders ────────────────────────────────────────────────────────────────────

const VERT: &str = r#"
#version 330 core
out vec2 v_uv;
void main() {
    float x = float((gl_VertexID & 1) * 2) - 1.0;
    float y = float((gl_VertexID >> 1) * 2) - 1.0;
    v_uv = vec2(x, y) * 0.5 + 0.5;
    gl_Position = vec4(x, y, 0.0, 1.0);
}
"#;

// MAX_EXCLUDE_ZONES must match the Rust constant above.
const THRESHOLD_FRAG: &str = r#"
#version 330 core
in vec2 v_uv;
out vec4 out_color;
uniform sampler2D u_scene;
uniform float u_threshold;
uniform float u_threshold_knee; // upper edge = threshold * (1 + knee)
uniform float u_sat_min;
uniform float u_sat_knee;
uniform int  u_exclude_n;
uniform vec4 u_exclude[4]; // (x0,y0,x1,y1) in UV space, Y=0 at bottom
void main() {
    for (int i = 0; i < u_exclude_n; i++) {
        vec4 r = u_exclude[i];
        if (v_uv.x >= r.x && v_uv.x <= r.z && v_uv.y >= r.y && v_uv.y <= r.w) {
            out_color = vec4(0.0);
            return;
        }
    }
    vec3  scene    = texture(u_scene, v_uv).rgb;
    float hi       = max(scene.r, max(scene.g, scene.b));
    float lo       = min(scene.r, min(scene.g, scene.b));
    float sat      = (hi > 0.001) ? (hi - lo) / hi : 0.0;
    float bright_w = smoothstep(u_threshold, u_threshold * (1.0 + u_threshold_knee), hi);
    float sat_w    = smoothstep(u_sat_min, u_sat_min + u_sat_knee, sat);
    // Allow pure white LEDs to bloom while still filtering neutral UI chrome.
    float white_w  = (scene.r >= 0.999 && scene.g >= 0.999 && scene.b >= 0.999) ? 1.0 : 0.0;
    sat_w = max(sat_w, white_w);
    out_color = vec4(scene * (bright_w * sat_w), 1.0);
}
"#;

// 9-tap separable Gaussian kernel (σ ≈ 1.0, taps at offsets 0..4).
// Weights are analytically derived and sum to 1.0.
const BLUR_FRAG: &str = r#"
#version 330 core
in vec2 v_uv;
out vec4 out_color;
uniform sampler2D u_tex;
uniform vec2 u_dir; // (step_x, 0) for H-pass, (0, step_y) for V-pass
const float W[5] = float[5](
    0.2270270, // offset 0
    0.1945946, // offset ±1
    0.1216216, // offset ±2
    0.0540541, // offset ±3
    0.0162162  // offset ±4
);
void main() {
    vec4 c = texture(u_tex, v_uv) * W[0];
    for (int i = 1; i < 5; i++) {
        c += texture(u_tex, v_uv + u_dir * float(i)) * W[i];
        c += texture(u_tex, v_uv - u_dir * float(i)) * W[i];
    }
    out_color = c;
}
"#;

// Bloom-only overlay - drawn additively on top of egui's framebuffer.
const COMPOSITE_FRAG: &str = r#"
#version 330 core
in vec2 v_uv;
out vec4 out_color;
uniform sampler2D u_bloom;
uniform float u_strength;
void main() {
    out_color = vec4(texture(u_bloom, v_uv).rgb * u_strength, 0.0);
}
"#;

// ── Pipeline ───────────────────────────────────────────────────────────────────

pub struct BloomPipeline {
    threshold_prog: glow::Program,
    blur_prog: glow::Program,
    composite_prog: glow::Program,
    quad_vao: glow::VertexArray,
    // Full-res copy of the screen (input to threshold).
    copy_tex: Option<glow::Texture>,
    copy_fbo: Option<glow::Framebuffer>,
    // Half-res ping-pong blur.
    blur_tex: Option<[glow::Texture; 2]>,
    blur_fbo: Option<[glow::Framebuffer; 2]>,
    size: [i32; 2],
    /// Hash of the inputs that produced the current `blur_tex[0]`. When `run()`
    /// is called with a matching key, the blit + threshold + blur passes are
    /// skipped and the cached blur is composited directly. Reset on resize.
    cache_key: Option<u64>,
}

impl BloomPipeline {
    pub fn new(gl: &glow::Context) -> Self {
        unsafe {
            let threshold_prog = compile_program(gl, VERT, THRESHOLD_FRAG);
            let blur_prog = compile_program(gl, VERT, BLUR_FRAG);
            let composite_prog = compile_program(gl, VERT, COMPOSITE_FRAG);
            let quad_vao = gl.create_vertex_array().expect("bloom vao");
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
    /// `w`/`h` are physical pixels. `ppp` is pixels-per-point (DPI scale).
    /// `exclude` is egui screen-space rects (logical points, Y-down) to suppress.
    /// `scene_key` is a hash of every input that affects bloom-relevant pixels
    /// (LED state, button hover, jog ring brightness, …). When the key matches
    /// the cached one, the blit + threshold + blur passes are skipped and the
    /// retained `blur_tex[0]` is composited directly - a >5× speedup of the
    /// bloom callback on idle frames where nothing visual has changed.
    ///
    /// The composite always runs because egui clears the framebuffer every
    /// frame, so the additive overlay must be re-applied even when blur is
    /// reused.
    pub fn run(
        &mut self,
        gl: &glow::Context,
        w: i32,
        h: i32,
        ppp: f32,
        exclude: &[Rect],
        scene_key: u64,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.resize_if_needed(gl, w, h);

        let hw = (w / 2).max(1);
        let hh = (h / 2).max(1);
        let copy_tex = self.copy_tex.unwrap();
        let copy_fbo = self.copy_fbo.unwrap();
        let blur_tex = self.blur_tex.unwrap();
        let blur_fbo = self.blur_fbo.unwrap();

        // Combine the caller-supplied scene hash with the layout-affecting
        // parameters that also invalidate the cached blur (window size, DPI,
        // exclusion rects). Resize already nukes `cache_key`, but ppp/exclude
        // can change without triggering a resize.
        let full_key = compose_key(scene_key, w, h, ppp, exclude);
        let blur_is_cached = self.cache_key == Some(full_key);

        unsafe {
            if !blur_is_cached {
                // ── 1. GPU blit: screen → copy_fbo ───────────────────────────
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

                // ── 2. Threshold: copy_tex → blur_fbo[0] (half-res) ──────────
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
                let n = exclude.len().min(MAX_EXCLUDE_ZONES) as i32;
                set_1i(gl, self.threshold_prog, "u_exclude_n", n);
                for (i, rect) in exclude.iter().take(MAX_EXCLUDE_ZONES).enumerate() {
                    // rect is in egui logical points; convert to UV (Y flipped for GL).
                    let x0 = rect.min.x * ppp / w as f32;
                    let y0 = 1.0 - rect.max.y * ppp / h as f32;
                    let x1 = rect.max.x * ppp / w as f32;
                    let y1 = 1.0 - rect.min.y * ppp / h as f32;
                    if let Some(loc) =
                        gl.get_uniform_location(self.threshold_prog, &format!("u_exclude[{i}]"))
                    {
                        gl.uniform_4_f32(Some(&loc), x0, y0, x1, y1);
                    }
                }
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

                // ── 3. H-blur: blur_fbo[0] → blur_fbo[1] ─────────────────────
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(blur_fbo[1]));
                gl.use_program(Some(self.blur_prog));
                gl.bind_texture(glow::TEXTURE_2D, Some(blur_tex[0]));
                set_1i(gl, self.blur_prog, "u_tex", 0);
                set_2f(gl, self.blur_prog, "u_dir", BLOOM_RADIUS / hw as f32, 0.0);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

                // ── 4. V-blur: blur_fbo[1] → blur_fbo[0] ─────────────────────
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(blur_fbo[0]));
                gl.bind_texture(glow::TEXTURE_2D, Some(blur_tex[1]));
                set_2f(gl, self.blur_prog, "u_dir", 0.0, BLOOM_RADIUS / hh as f32);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

                self.cache_key = Some(full_key);
            } else {
                gl.bind_vertex_array(Some(self.quad_vao));
                gl.active_texture(glow::TEXTURE0);
            }

            // ── 5. Additive bloom → screen ────────────────────────────────────
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, w, h);
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func_separate(
                glow::ONE,
                glow::ONE, // RGB: additive
                glow::ZERO,
                glow::ONE, // Alpha: preserve dst (egui writes 1.0)
            );
            gl.use_program(Some(self.composite_prog));
            gl.bind_texture(glow::TEXTURE_2D, Some(blur_tex[0]));
            set_1i(gl, self.composite_prog, "u_bloom", 0);
            set_1f(gl, self.composite_prog, "u_strength", BLOOM_STRENGTH);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

            // ── Restore GL state ──────────────────────────────────────────────
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
            // Cached blur is sized for the previous viewport - invalidate it
            // so the next frame regenerates against the new dimensions.
            self.cache_key = None;
        }
    }
}

/// Combine the caller's scene hash with the layout-affecting parameters that
/// also invalidate the cached blur. Uses `DefaultHasher` (SipHash-1-3) - a few
/// hundred ns; negligible vs. the bloom passes themselves.
fn compose_key(scene_key: u64, w: i32, h: i32, ppp: f32, exclude: &[Rect]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h_ = std::collections::hash_map::DefaultHasher::new();
    scene_key.hash(&mut h_);
    w.hash(&mut h_);
    h.hash(&mut h_);
    // ppp / Rect coords are floats - quantize to fixed-point so identical
    // logical layouts hash identically.
    (ppp.to_bits()).hash(&mut h_);
    for r in exclude.iter().take(MAX_EXCLUDE_ZONES) {
        r.min.x.to_bits().hash(&mut h_);
        r.min.y.to_bits().hash(&mut h_);
        r.max.x.to_bits().hash(&mut h_);
        r.max.y.to_bits().hash(&mut h_);
    }
    h_.finish()
}

// ── GL helpers ─────────────────────────────────────────────────────────────────

unsafe fn make_fbo(gl: &glow::Context, w: i32, h: i32) -> (glow::Texture, glow::Framebuffer) {
    let tex = gl.create_texture().expect("bloom tex");
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
        None,
    );
    for (param, val) in [
        (glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32),
        (glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32),
        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32),
        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32),
    ] {
        gl.tex_parameter_i32(glow::TEXTURE_2D, param, val);
    }
    let fbo = gl.create_framebuffer().expect("bloom fbo");
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
        "bloom vert: {}",
        gl.get_shader_info_log(vs)
    );

    let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
    gl.shader_source(fs, frag);
    gl.compile_shader(fs);
    assert!(
        gl.get_shader_compile_status(fs),
        "bloom frag: {}",
        gl.get_shader_info_log(fs)
    );

    let prog = gl.create_program().unwrap();
    gl.attach_shader(prog, vs);
    gl.attach_shader(prog, fs);
    gl.link_program(prog);
    assert!(
        gl.get_program_link_status(prog),
        "bloom link: {}",
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
