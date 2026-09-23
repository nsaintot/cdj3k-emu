//! Panel UI: a shared control toolkit + per-model slates.
//!
//! The SHARED toolkit lives at this root: the colour palette, [`UiScale`]
//! (reference-canvas to screen mapping), the button/rotary primitives
//! ([`draw_primitives`]), the shape caches ([`draw_cache`]) and the jog-wheel
//! assembly ([`draw_jog`]). Each [`slate`] composes the toolkit into the full
//! panel of one model.

pub(super) mod draw_cache;
pub(super) mod draw_direction;
mod draw_jog;
mod draw_primitives;
pub(super) mod draw_vinyl_speed;
pub(super) mod slate;

pub(super) use draw_jog::JogChrome;

use egui::{Color32, Pos2, Rect, Vec2};

use super::CdjApp;

/// Navigation rotary: detent positions per full revolution.
pub(in crate::app) const NAV_DETENT_COUNT: f32 = 18.0;
/// Navigation rotary: pixels of smooth scroll per detent (LCD scroll + rotary).
pub(in crate::app) const NAV_SCROLL_PX_PER_TICK: f32 = 50.0;

pub(super) const COL_BLACK: Color32 = Color32::from_rgb(20, 20, 26);
pub(super) const COL_DARK: Color32 = Color32::from_rgb(30, 30, 36);
pub(super) const COL_BTN: Color32 = Color32::from_rgb(48, 48, 55);
pub(super) const COL_BLUE: Color32 = Color32::from_rgb(0, 102, 255);
pub(super) const COL_GREEN: Color32 = Color32::from_rgb(140, 200, 0);
pub(super) const COL_AMBER: Color32 = Color32::from_rgb(249, 182, 42);
pub(super) const COL_SILVER: Color32 = Color32::from_rgb(100, 100, 110);
pub(super) const COL_YELLOW: Color32 = Color32::from_rgb(255, 240, 44);
pub(super) const COL_RED: Color32 = Color32::from_rgb(238, 69, 85);
pub(super) const COL_DARK_RED: Color32 = Color32::from_rgb(130, 20, 20);
pub(super) const COL_WHITE: Color32 = Color32::from_rgb(222, 222, 222);
pub(super) const COL_BTN_OUTLINED_YELLOW: Color32 = Color32::from_rgb(255, 237, 120);
pub(super) const COL_BTN_OUTLINED_WHITE: Color32 = Color32::from_rgb(242, 242, 242);
pub(super) const COL_BTN_CUE: Color32 = Color32::from_rgb(255, 150, 0);
pub(super) const COL_BTN_PLAY: Color32 = Color32::from_rgb(0, 200, 100);
pub(super) const COL_BTN_HOT: Color32 = Color32::from_rgb(70, 70, 80);
pub(super) const COL_BTN_TEXT: Color32 = Color32::from_rgb(200, 200, 210);
pub(super) const COL_BTN_WHITE: Color32 = Color32::from_rgb(255, 255, 255);
pub(super) const COL_LCD_BG: Color32 = Color32::from_rgb(0, 0, 0);
pub(super) const COL_JOG_BODY: Color32 = Color32::from_rgb(35, 35, 40);

#[cfg(debug_assertions)]
const DEBUG_ALIGN_SQUARE_SIZE_PX: f32 = 87.0;
#[cfg(debug_assertions)]
const DEBUG_ALIGN_SQUARE_COLOR: Color32 = Color32::RED;

pub(super) fn panel_bg() -> Color32 {
    COL_BLACK
}

pub(super) use draw_primitives::{
    collect_arc_quad_button, collect_back_double_circle_border, collect_bordered_rect_section,
    collect_button, collect_circle_button, collect_computer_glyph, draw_bordered_rect_section,
    draw_rotary_control, draw_rotary_control_collect, draw_usb_tray, paint_double_circle_ring,
    paint_double_circle_ring_collect, ArcDecorSpec, ArcNotchSpec, ButtonType, DoubleBorderSpec,
    RotaryControlSpec, RotaryGearSpec, RotaryIndicatorSpec, RotaryTickSpec, StrokeSpec,
    UsbTraySpec,
};

pub(super) struct UiScale {
    ox: f32,
    oy: f32,
    scale: f32,
}

impl UiScale {
    /// Letterbox a `ref_w` x `ref_h` reference canvas into `avail`, centred.
    pub(super) fn fit(avail: Rect, ref_w: f32, ref_h: f32) -> Self {
        let scale = (avail.width() / ref_w).min(avail.height() / ref_h);
        let ox = avail.left() + (avail.width() - ref_w * scale) * 0.5;
        let oy = avail.top() + (avail.height() - ref_h * scale) * 0.5;
        Self { ox, oy, scale }
    }

    /// Returns the `(ox, oy, scale)` triple used as a cache invalidation key.
    pub(super) fn cache_key(&self) -> (f32, f32, f32) {
        (self.ox, self.oy, self.scale)
    }
}

/// `CDJ3K_DEBUG_ALIGN=1` on a debug build: a red square at each corner of the
/// reference canvas, so a letterboxing or aspect regression is visible at a
/// glance. Read once - the panel asks on every frame.
#[cfg(debug_assertions)]
fn debug_align_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("CDJ3K_DEBUG_ALIGN").is_some_and(|v| v != "0" && !v.is_empty())
    })
}

#[cfg(debug_assertions)]
pub(super) fn debug_align_squares(p: &egui::Painter, layout: &UiScale, ref_w: f32, ref_h: f32) {
    if !debug_align_enabled() {
        return;
    }
    let s = DEBUG_ALIGN_SQUARE_SIZE_PX;
    for (x, y) in [
        (0.0, 0.0),
        (ref_w - s, 0.0),
        (0.0, ref_h - s),
        (ref_w - s, ref_h - s),
    ] {
        p.rect_filled(layout.sr(x, y, s, s), 0.0, DEBUG_ALIGN_SQUARE_COLOR);
    }
}

#[cfg(not(debug_assertions))]
pub(super) fn debug_align_squares(_p: &egui::Painter, _layout: &UiScale, _ref_w: f32, _ref_h: f32) {
}

impl UiScale {
    pub(super) fn sp(&self, x: f32, y: f32) -> Pos2 {
        Pos2::new(self.ox + x * self.scale, self.oy + y * self.scale)
    }

    /// Point inside `region` using normalized coordinates `u`, `v` in 0..1
    /// (`region` is in unscaled layout-reference space, same as `sp` / `sr`).
    /// Returns a point inside `region` using normalized coordinates (`horizontal_pct`, `vertical_pct`) in 0..1.
    /// E.g., (0.5, 0.0) is the top-center, (1.0, 1.0) is the bottom-right.
    pub(super) fn sp_in_rect(&self, region: Rect, horizontal_pct: f32, vertical_pct: f32) -> Pos2 {
        let x = region.left() + region.width() * horizontal_pct;
        let y = region.top() + region.height() * vertical_pct;
        self.sp(x, y)
    }

    pub(super) fn sc(&self, value: f32) -> f32 {
        value * self.scale
    }

    pub(super) fn sr(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.sp(x, y), Vec2::new(w * self.scale, h * self.scale))
    }

    pub(super) fn ar2rect(&self, left: f32, top: f32, aspect_ratio: f32, size: f32) -> Rect {
        if aspect_ratio >= 1.0 {
            self.sr(left, top, size, size / aspect_ratio)
        } else {
            self.sr(left, top, size * aspect_ratio, size)
        }
    }
}

/// Reference-space widths for [`CdjApp::btn`] when the caller does not pass `border_override`.
const DEFAULT_RECT_BTN_BORDER_INNER_REF: f32 = 3.0;
const DEFAULT_RECT_BTN_BORDER_OUTER_REF: f32 = 2.0;
const DEFAULT_RECT_BTN_BORDER_GAP_REF: f32 = 2.0;

/// Reference-space widths for [`CdjApp::circle_btn`] when `btn_border` is `None`.
const DEFAULT_CIRCLE_BTN_BORDER_INNER_REF: f32 = 2.0;
const DEFAULT_CIRCLE_BTN_BORDER_OUTER_REF: f32 = 2.0;
const DEFAULT_CIRCLE_BTN_BORDER_GAP_REF: f32 = 2.0;

#[inline]
pub(super) fn default_rect_btn_border(layout: &UiScale) -> DoubleBorderSpec {
    DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(DEFAULT_RECT_BTN_BORDER_INNER_REF),
            color: COL_SILVER,
        },
        StrokeSpec {
            width: layout.sc(DEFAULT_RECT_BTN_BORDER_OUTER_REF),
            color: COL_DARK,
        },
        layout.sc(DEFAULT_RECT_BTN_BORDER_GAP_REF),
    )
}

#[inline]
pub(super) fn default_circle_btn_border(layout: &UiScale) -> DoubleBorderSpec {
    DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(DEFAULT_CIRCLE_BTN_BORDER_INNER_REF),
            color: COL_SILVER,
        },
        StrokeSpec {
            width: layout.sc(DEFAULT_CIRCLE_BTN_BORDER_OUTER_REF),
            color: COL_DARK,
        },
        layout.sc(DEFAULT_CIRCLE_BTN_BORDER_GAP_REF),
    )
}

/// Snapshot of the parts of [`CdjApp`] state that the debug viewport reads.
///
/// The debug viewport runs as a *deferred* viewport with its own update cycle,
/// so we cannot borrow `&CdjApp` from inside its closure (different thread of
/// execution as far as egui is concerned). Each main `update()` writes a fresh
/// snapshot into a shared `Arc<Mutex<DebugSnapshot>>`; the deferred closure
/// reads from it. This decouples the debug window's repaint cadence from the
/// main window - its FPS counter measures *its own* render rate.
#[derive(Clone)]
pub struct DebugSnapshot {
    pub status: String,
    /// Main viewport fps (so the debug pane can show both its own and the
    /// main viewport's frame rate independently).
    pub main_fps: f32,
    pub main_shape_count: u64,
    pub jog_dbg_lines: [String; 3],
    pub lcd_touch: Option<(u16, u16)>,
    pub last_miso: [u8; cdj3k_emu_panel::miso_frame::MISO_SIZE],
    pub led_frame: [u8; 64],
}

impl Default for DebugSnapshot {
    fn default() -> Self {
        Self {
            status: String::new(),
            main_fps: 60.0,
            main_shape_count: 0,
            jog_dbg_lines: [String::new(), String::new(), String::new()],
            lcd_touch: None,
            last_miso: [0u8; cdj3k_emu_panel::miso_frame::MISO_SIZE],
            led_frame: [0u8; 64],
        }
    }
}

/// Render the debug pane from a snapshot. `local_fps` is the fps measured
/// inside the debug viewport's own update cycle (independent of `main_fps`).
pub(super) fn draw_debug_content(ui: &mut egui::Ui, snap: &DebugSnapshot, local_fps: f32) {
    let mono = egui::FontId::monospace(11.0);

    // ---- Status / FPS / shape count ----
    ui.label(egui::RichText::new(&snap.status).font(mono.clone()));
    ui.label(
        egui::RichText::new(format!(
            "fps: {:.1} (debug)  {:.1} (main)  shapes/f: {}",
            local_fps, snap.main_fps, snap.main_shape_count
        ))
        .font(mono.clone()),
    );
    ui.separator();

    // ---- Jog kinematics ----
    ui.label(egui::RichText::new("jog").font(mono.clone()).strong());
    for line in &snap.jog_dbg_lines {
        ui.label(egui::RichText::new(line).font(mono.clone()));
    }
    ui.separator();

    // ---- LCD touch ----
    ui.label(egui::RichText::new("lcd touch").font(mono.clone()).strong());
    let touch_str = match snap.lcd_touch {
        Some((x, y)) => format!("x={:4}  y={:4}", x & !cdj3k_emu_panel::TOUCH_DOWN, y),
        None => "none".to_string(),
    };
    ui.label(egui::RichText::new(touch_str).font(mono.clone()));
    ui.separator();

    // ---- MISO frame (64 bytes, 4 rows of 16) ----
    ui.label(
        egui::RichText::new("miso (ctrl)")
            .font(mono.clone())
            .strong(),
    );
    for row in 0..4 {
        let start = row * 16;
        let hex: String = snap.last_miso[start..start + 16]
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ");
        ui.label(egui::RichText::new(format!("b{:02}  {}", start, hex)).font(mono.clone()));
    }
    ui.separator();

    // ---- MOSI LED frame (64 bytes, 4 rows of 16) ----
    ui.label(
        egui::RichText::new("mosi (led)")
            .font(mono.clone())
            .strong(),
    );
    let led = &snap.led_frame;
    for row in 0..4 {
        let start = row * 16;
        let hex: String = led[start..start + 16]
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ");
        ui.label(egui::RichText::new(format!("b{:02}  {}", start, hex)).font(mono.clone()));
    }

    // ---- MOSI decoded ----
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("mosi decoded")
            .font(mono.clone())
            .strong(),
    );
    let bitfield: String = (2..=11)
        .map(|i| format!("b{:02}:{:08b}", i, led[i]))
        .collect::<Vec<_>>()
        .join("  ");
    ui.label(egui::RichText::new(bitfield).font(mono.clone()));
    let pads: String = (0..8usize)
        .map(|p| {
            let base = 12 + p * 3;
            format!(
                "{}:{:02x}/{:02x}/{:02x}",
                (b'A' + p as u8) as char,
                led[base],
                led[base + 1],
                led[base + 2]
            )
        })
        .collect::<Vec<_>>()
        .join("  ");
    ui.label(egui::RichText::new(pads).font(mono.clone()));
    let inds = format!(
        "SD:{:02x}/{:02x}/{:02x}  USB:{:02x}/{:02x}/{:02x}  ONAIR:{:02x}/{:02x}/{:02x}",
        led[36], led[37], led[38], led[39], led[40], led[41], led[42], led[43], led[44],
    );
    ui.label(egui::RichText::new(inds).font(mono));
}

impl CdjApp {
    /// Builds a fresh [`DebugSnapshot`] from the current app state.
    pub(super) fn debug_snapshot(&self) -> DebugSnapshot {
        DebugSnapshot {
            status: self.status.clone(),
            main_fps: self.fps_smooth,
            main_shape_count: self.frame_shape_count,
            jog_dbg_lines: self.jog_dbg_lines.clone(),
            lcd_touch: self.lcd_touch,
            last_miso: self.last_miso,
            led_frame: self.led_state.frame,
        }
    }
}

impl CdjApp {
    pub(super) fn draw_ui(&mut self, ui: &mut egui::Ui) {
        puffin::profile_function!();
        self.frame_shape_count = 0;
        (slate::for_model(self.model).draw_panel)(self, ui);
    }
}
