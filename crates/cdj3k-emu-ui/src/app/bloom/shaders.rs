//! The bloom passes' GLSL.

pub(super) const VERT: &str = r#"
#version 330 core
out vec2 v_uv;
void main() {
    float x = float((gl_VertexID & 1) * 2) - 1.0;
    float y = float((gl_VertexID >> 1) * 2) - 1.0;
    v_uv = vec2(x, y) * 0.5 + 0.5;
    gl_Position = vec4(x, y, 0.0, 1.0);
}
"#;

/// The mask uniforms and the `kept` / `masked` tests the threshold and the
/// composite share. The array sizes match `MAX_EXCLUDE_ZONES`,
/// `MAX_KEEP_ZONES` and `MAX_CHROME_ZONES`.
macro_rules! mask_glsl {
    () => {
        r#"uniform int  u_exclude_n;
uniform vec4 u_exclude[4]; // (x0,y0,x1,y1) in UV space, Y=0 at bottom
uniform int  u_keep_n;
uniform vec4 u_keep_a[2]; // (cx, cy, core_r, outer_r) in pixels, Y=0 at bottom
uniform vec2 u_keep_b[2]; // (half_w, tan_a)
uniform vec2 u_screen_px;
uniform int  u_chrome_n;
uniform vec4 u_chrome[8]; // (x0,y0,x1,y1) in UV space, Y=0 at bottom
bool kept(vec2 uv) {
    vec2 p = uv * u_screen_px;
    for (int i = 0; i < u_keep_n; i++) {
        vec2 d = p - u_keep_a[i].xy;
        float r = length(d);
        if (r <= u_keep_a[i].z) return true;
        if (r <= u_keep_a[i].w
            && (abs(d.x) <= u_keep_b[i].x || abs(d.x) <= abs(d.y) * u_keep_b[i].y)) return true;
    }
    return false;
}
bool masked(vec2 uv) {
    for (int i = 0; i < u_chrome_n; i++) {
        vec4 r = u_chrome[i];
        if (uv.x >= r.x && uv.x <= r.z && uv.y >= r.y && uv.y <= r.w) return true;
    }
    for (int i = 0; i < u_exclude_n; i++) {
        vec4 r = u_exclude[i];
        if (uv.x >= r.x && uv.x <= r.z && uv.y >= r.y && uv.y <= r.w) return !kept(uv);
    }
    return false;
}
"#
    };
}

pub(super) const THRESHOLD_FRAG: &str = concat!(
    r#"
#version 330 core
in vec2 v_uv;
out vec4 out_color;
uniform sampler2D u_scene;
uniform float u_threshold;
uniform float u_threshold_knee; // The upper edge is threshold * (1 + knee).
uniform float u_sat_min;
uniform float u_sat_knee;
"#,
    mask_glsl!(),
    r#"void main() {
    if (masked(v_uv)) {
        out_color = vec4(0.0);
        return;
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
"#
);

// A 9-tap separable blur at offsets 0 to ±4. The weights are binomial (row 12
// of Pascal's triangle, truncated), close to a Gaussian with σ ≈ 1.7, and sum
// to 1.0.
pub(super) const BLUR_FRAG: &str = r#"
#version 330 core
in vec2 v_uv;
out vec4 out_color;
uniform sampler2D u_tex;
uniform vec2 u_dir; // (step_x, 0) for the horizontal pass, (0, step_y) for the vertical pass
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

// The bloom overlay, drawn additively over egui's framebuffer.
pub(super) const COMPOSITE_FRAG: &str = concat!(
    r#"
#version 330 core
in vec2 v_uv;
out vec4 out_color;
uniform sampler2D u_bloom;
uniform float u_strength;
"#,
    mask_glsl!(),
    r#"void main() {
    if (masked(v_uv)) {
        out_color = vec4(0.0);
        return;
    }
    out_color = vec4(texture(u_bloom, v_uv).rgb * u_strength, 0.0);
}
"#
);
