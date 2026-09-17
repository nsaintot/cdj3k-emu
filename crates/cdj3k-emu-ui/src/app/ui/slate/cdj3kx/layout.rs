//! Section zones of the CDJ-3000X reference canvas (`super::REF_W` x `super::REF_H`).
//!

use egui::{Pos2, Rect};

/// Jog assembly zone, handed to the shared `ui::draw_jog` renderer.
pub(super) fn jog_rect() -> Rect {
    Rect::from_min_max(
        Pos2::new(JOG_REF_LEFT, JOG_REF_TOP),
        Pos2::new(JOG_REF_RIGHT, JOG_REF_BOT),
    )
}

/// Transport column (Cue / Play / search)
pub const TRANSPORT_REF_SIZE_H: f32 = 4080.0;
pub const TRANSPORT_REF_SIZE_W: f32 = 510.0;
pub const TRANSPORT_REF_TOP: f32 = 579.0;
pub const TRANSPORT_REF_BOT: f32 = TRANSPORT_REF_TOP + TRANSPORT_REF_SIZE_H;
pub const TRANSPORT_REF_LEFT: f32 = 10.0;
pub const TRANSPORT_REF_RIGHT: f32 = TRANSPORT_REF_LEFT + TRANSPORT_REF_SIZE_W;

// TOP CENTER (MENU, LCD)
pub const TOP_CENTRAL_AFTER_TRANSPORT_GAP: f32 = -108.0;
pub const TOP_CENTRAL_REF_SIZE_H: f32 = 1773.0;
pub const TOP_CENTRAL_REF_SIZE_W: f32 = 2508.0;
pub const TOP_CENTRAL_REF_TOP: f32 = 19.0;
pub const TOP_CENTRAL_REF_BOT: f32 = TOP_CENTRAL_REF_TOP + TOP_CENTRAL_REF_SIZE_H;
pub const TOP_CENTRAL_REF_LEFT: f32 = TRANSPORT_REF_RIGHT + TOP_CENTRAL_AFTER_TRANSPORT_GAP;
pub const TOP_CENTRAL_REF_RIGHT: f32 = TOP_CENTRAL_REF_LEFT + TOP_CENTRAL_REF_SIZE_W;

// MID CENTER (HOT CUE + HELPER ROW)
pub const MID_CENTRAL_AFTER_TOP_CENTRAL_GAP: f32 = 5.0;
pub const MID_CENTRAL_AFTER_TRANSPORT_GAP: f32 = 20.0;
pub const MID_CENTRAL_REF_SIZE_H: f32 = 460.0;
pub const MID_CENTRAL_REF_SIZE_W: f32 =
    TOP_CENTRAL_REF_SIZE_W + (MID_CENTRAL_AFTER_TRANSPORT_GAP * 2.0);
pub const MID_CENTRAL_REF_TOP: f32 = TOP_CENTRAL_REF_BOT + MID_CENTRAL_AFTER_TOP_CENTRAL_GAP;
pub const MID_CENTRAL_REF_BOT: f32 = MID_CENTRAL_REF_TOP + MID_CENTRAL_REF_SIZE_H;
pub const MID_CENTRAL_REF_LEFT: f32 = TOP_CENTRAL_REF_LEFT - MID_CENTRAL_AFTER_TRANSPORT_GAP;
pub const MID_CENTRAL_REF_RIGHT: f32 = MID_CENTRAL_REF_LEFT + MID_CENTRAL_REF_SIZE_W;

// JOG
// Centred on ref (1666, 3280) - the panel's own centre line, and the centre
// the drawing's outer ring fits to within a quarter of a drawing pixel.
pub const JOG_AFTER_MID_CENTRAL_GAP: f32 = -47.0;
pub const JOG_AFTER_TRANSPORT_GAP: f32 = 76.0;
pub const JOG_REF_SIZE_H: f32 = 2140.0;
pub const JOG_REF_SIZE_W: f32 = 2140.0;
pub const JOG_REF_TOP: f32 = MID_CENTRAL_REF_BOT + JOG_AFTER_MID_CENTRAL_GAP;
pub const JOG_REF_BOT: f32 = JOG_REF_TOP + JOG_REF_SIZE_H;
pub const JOG_REF_LEFT: f32 = TRANSPORT_REF_RIGHT + JOG_AFTER_TRANSPORT_GAP;
pub const JOG_REF_RIGHT: f32 = JOG_REF_LEFT + JOG_REF_SIZE_W;

// NAVIGATION
/// Centre of the nav pod - the rotary and its four buttons - within the modes
/// column, shared with the decorative line that runs around it.
pub const NAV_POD_U: f32 = 0.444;
pub const NAV_POD_V: f32 = 0.135;

// MODES
pub const MODE_AFTER_CENTRAL_GAP: f32 = 0.0;
pub const MODE_AFTER_JOG_GAP: f32 = 69.0;
pub const MODES_REF_SIZE_H: f32 = TRANSPORT_REF_SIZE_H;
pub const MODES_REF_SIZE_W: f32 = 520.0;
pub const MODES_REF_TOP: f32 = TRANSPORT_REF_TOP + MODE_AFTER_CENTRAL_GAP;
pub const MODES_REF_BOT: f32 = MODES_REF_TOP + MODES_REF_SIZE_H;
pub const MODES_REF_LEFT: f32 = JOG_REF_RIGHT + MODE_AFTER_JOG_GAP;
pub const MODES_REF_RIGHT: f32 = MODES_REF_LEFT + MODES_REF_SIZE_W;

// BOTTOM DECORATIVE
