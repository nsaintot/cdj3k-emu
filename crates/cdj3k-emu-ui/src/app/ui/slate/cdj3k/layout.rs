//! Section zones of the CDJ-3000 reference canvas (`super::REF_W` x `super::REF_H`).
//!

use egui::{Pos2, Rect};

/// Jog assembly zone, handed to the shared `ui::draw_jog` renderer.
pub(super) const fn jog_rect() -> Rect {
    Rect::from_min_max(
        Pos2::new(JOG_REF_LEFT, JOG_REF_TOP),
        Pos2::new(JOG_REF_RIGHT, JOG_REF_BOT),
    )
}

/// Transport column (CUE, PLAY and the search buttons).
pub const TRANSPORT_REF_SIZE_H: f32 = 4080.0;
pub const TRANSPORT_REF_SIZE_W: f32 = 510.0;
pub const TRANSPORT_REF_TOP: f32 = 280.0;
pub const TRANSPORT_REF_BOT: f32 = TRANSPORT_REF_TOP + TRANSPORT_REF_SIZE_H;
pub const TRANSPORT_REF_LEFT: f32 = 10.0;
pub const TRANSPORT_REF_RIGHT: f32 = TRANSPORT_REF_LEFT + TRANSPORT_REF_SIZE_W;

// Top centre: the menu row and the LCD
pub const TOP_CENTRAL_AFTER_TRANSPORT_GAP: f32 = -40.0;
pub const TOP_CENTRAL_REF_SIZE_H: f32 = 1400.0;
pub const TOP_CENTRAL_REF_SIZE_W: f32 = 2220.0;
pub const TOP_CENTRAL_REF_TOP: f32 = 80.0;
pub const TOP_CENTRAL_REF_BOT: f32 = TOP_CENTRAL_REF_TOP + TOP_CENTRAL_REF_SIZE_H;
pub const TOP_CENTRAL_REF_LEFT: f32 = TRANSPORT_REF_RIGHT + TOP_CENTRAL_AFTER_TRANSPORT_GAP;
pub const TOP_CENTRAL_REF_RIGHT: f32 = TOP_CENTRAL_REF_LEFT + TOP_CENTRAL_REF_SIZE_W;

// Mid centre: the hot cue row and the helper row
pub const MID_CENTRAL_AFTER_TOP_CENTRAL_GAP: f32 = 5.0;
pub const MID_CENTRAL_AFTER_TRANSPORT_GAP: f32 = 20.0;
pub const MID_CENTRAL_REF_SIZE_H: f32 = 460.0;
pub const MID_CENTRAL_REF_SIZE_W: f32 =
    TOP_CENTRAL_REF_SIZE_W + (MID_CENTRAL_AFTER_TRANSPORT_GAP * 2.0);
pub const MID_CENTRAL_REF_TOP: f32 = TOP_CENTRAL_REF_BOT + MID_CENTRAL_AFTER_TOP_CENTRAL_GAP;
pub const MID_CENTRAL_REF_BOT: f32 = MID_CENTRAL_REF_TOP + MID_CENTRAL_REF_SIZE_H;
pub const MID_CENTRAL_REF_LEFT: f32 = TOP_CENTRAL_REF_LEFT - MID_CENTRAL_AFTER_TRANSPORT_GAP;
pub const MID_CENTRAL_REF_RIGHT: f32 = MID_CENTRAL_REF_LEFT + MID_CENTRAL_REF_SIZE_W;

// Jog
pub const JOG_AFTER_MID_CENTRAL_GAP: f32 = -30.0;
pub const JOG_REF_SIZE_H: f32 = 2140.0;
pub const JOG_REF_SIZE_W: f32 = 2140.0;
pub const JOG_REF_TOP: f32 = MID_CENTRAL_REF_BOT + JOG_AFTER_MID_CENTRAL_GAP;
pub const JOG_REF_BOT: f32 = JOG_REF_TOP + JOG_REF_SIZE_H;
pub const JOG_REF_LEFT: f32 = TRANSPORT_REF_RIGHT;
pub const JOG_REF_RIGHT: f32 = JOG_REF_LEFT + JOG_REF_SIZE_W;

// Navigation
/// Centre of the nav pod (the rotary and its four buttons) within the modes
/// column. The decorative line that runs around the pod uses it too.
pub const NAV_POD_U: f32 = 0.36038;
pub const NAV_POD_V: f32 = 0.137745;

// Modes column
pub const MODE_AFTER_CENTRAL_GAP: f32 = 0.0;
pub const MODE_AFTER_JOG_GAP: f32 = -5.0;
pub const MODES_REF_SIZE_H: f32 = TRANSPORT_REF_SIZE_H;
pub const MODES_REF_SIZE_W: f32 = 520.0;
pub const MODES_REF_TOP: f32 = TRANSPORT_REF_TOP + MODE_AFTER_CENTRAL_GAP;
pub const MODES_REF_BOT: f32 = MODES_REF_TOP + MODES_REF_SIZE_H;
pub const MODES_REF_LEFT: f32 = JOG_REF_RIGHT + MODE_AFTER_JOG_GAP;
pub const MODES_REF_RIGHT: f32 = MODES_REF_LEFT + MODES_REF_SIZE_W;

// LCD
/// The screen's glass is 1903 x 1030 reference units at y 357..1387. The panel
/// that draws the screen and the chassis that frames it share this rectangle,
/// because the recess walls meet the glass.
///
/// The aspect is the drawing's, which is a 16:9 panel seen tilted, so it is
/// wider than 1280/720; the guest framebuffer stretches to fill it.
pub const LCD_GLASS_WIDTH_FRAC: f32 = 0.8572;
pub const LCD_GLASS_ASPECT: f32 = 1903.0 / 1030.0;
pub const LCD_GLASS_V_TOP: f32 = 0.1979;
