//! The panel UI: a shared control toolkit and one slate per model.
//!
//! The shared toolkit lives at this root: the colour palette, [`UiScale`]
//! (which maps the reference canvas to the screen), the button and rotary
//! primitives ([`draw_primitives`]), the shape caches ([`draw_cache`]) and the
//! jog wheel assembly ([`draw_jog`]). Each [`slate`] builds the full panel of
//! one model from the toolkit.

pub(super) mod draw_cache;
pub(super) mod draw_direction;
mod draw_jog;
pub(super) mod draw_lcd;
mod draw_primitives;
pub(super) mod draw_vinyl_speed;
pub(super) mod slate;
pub(super) mod tilt;

pub(super) use draw_jog::JogChrome;

use egui::{Color32, Pos2, Rect, Vec2};

use super::CdjApp;

/// The number of detents in one turn of the navigation rotary.
pub(in crate::app) const NAV_DETENT_COUNT: f32 = 18.0;
/// Pixels of smooth scroll per navigation rotary detent, for both the LCD and
/// the rotary.
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
    collect_button, collect_circle_button, collect_computer_glyph, collect_usb_trident,
    draw_bordered_rect_section, draw_rotary_control, draw_rotary_control_collect, draw_usb_tray,
    paint_double_circle_ring, paint_double_circle_ring_collect, rotary_arc_fraction,
    scroll_detents, ArcDecorSpec, ArcNotchSpec, ButtonType, DoubleBorderSpec, RotaryControlSpec,
    RotaryGearSpec, RotaryIndicatorSpec, RotaryTickSpec, StrokeSpec, UsbTraySpec,
};

pub(super) struct UiScale {
    ox: f32,
    oy: f32,
    scale: f32,
}

impl UiScale {
    /// Fits a `ref_w` x `ref_h` reference canvas into `avail`, centred and
    /// letterboxed.
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

/// Returns whether `CDJ3K_DEBUG_ALIGN=1` is set. On a debug build it draws a
/// red square at each corner of the reference canvas, which shows a
/// letterboxing or aspect error at a glance. The variable is read once,
/// because the panel asks on every frame.
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

    /// Returns the screen point at the fractions (`horizontal_pct`,
    /// `vertical_pct`), each in 0..1, of `region`. `region` is in unscaled
    /// reference space, like the arguments of `sp` and `sr`. For example,
    /// (0.5, 0.0) is the top centre and (1.0, 1.0) is the bottom-right corner.
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

impl CdjApp {
    pub(super) fn draw_ui(&mut self, ui: &mut egui::Ui) {
        puffin::profile_function!();
        self.frame_shape_count = 0;
        (slate::for_model(self.model).draw_panel)(self, ui);
    }
}
