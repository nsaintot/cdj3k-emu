//! CDJ-3000 slate: reference canvas, section zones and the section renderers.
//!
//! The jog assembly is the shared `ui::draw_jog`, placed at this slate's jog
//! zone. The section files reach the shared toolkit through `super::` (the
//! glob import below).

mod draw_chassis;
mod draw_left;
mod draw_right;
mod draw_top;
mod layout;

use super::card::{Bay, Card, CardGlyph};
use crate::app::ui::slate::{draw_flat_panel, riser_glyph, FlatPanel, RiserZones, Slate};
#[allow(unused_imports)]
use crate::app::ui::*;
use crate::app::CdjApp;
use egui::{Color32, Pos2, Rect, Vec2};

pub(in crate::app) const SLATE: Slate = Slate {
    ref_canvas: (REF_W, REF_H),
    accent: Color32::from_rgb(56, 132, 255),
    card: Card {
        subtitle: "9-inch touch display",
        era: "2020",
        glyph: CardGlyph::Riser(riser_glyph(RiserZones {
            canvas: (REF_W, REF_H),
            riser: draw_top::MENUBAR_COL_REF,
            screen: Rect::from_min_max(
                Pos2::new(GLASS_LEFT, GLASS_TOP),
                Pos2::new(GLASS_LEFT + GLASS_W, GLASS_TOP + GLASS_H),
            ),
            bezel: Vec2::new(0.0362, 0.0225),
            recessed: true,
            pads: Rect::from_min_max(
                Pos2::new(PADS_LEFT, PADS_MID_Y - PADS_H * 0.5),
                Pos2::new(PADS_LEFT + PADS_W, PADS_MID_Y + PADS_H * 0.5),
            ),
            pad_gap: draw_top::HOT_CUE_BTN_GAP_REF / PAD_PITCH,
            transport_col: draw_left::PERF_TRANSPORT_COL_REF,
            modes_col: draw_right::MODES_COL_REF,
            bay: Bay::UsbAndSd,
            jog: layout::jog_rect(),
            pod: Pos2::new(POD_X, POD_Y),
            pod_half_w: 150.0,
            pod_ring_r: POD_RING_R,
        })),
    },
    draw_panel,
};

/// Reference canvas (unscaled layout units) every zone in [`layout`] is
/// expressed in; the window is aspect-locked to `REF_W : REF_H`.
pub(in crate::app) const REF_W: f32 = 3185.0;
pub(in crate::app) const REF_H: f32 = 4360.0;

// ── The picker's miniature ────────────────────────────────────────────────────
// Every fraction the card draws is computed from the zones the panel draws
// from, so the card always matches the panel.

/// The screen's glass, placed where the panel places it: centred in the
/// top-central block, with a width that is a fraction of the block and a
/// height set by the glass's aspect ratio.
const GLASS_W: f32 = layout::TOP_CENTRAL_REF_SIZE_W * layout::LCD_GLASS_WIDTH_FRAC;
const GLASS_H: f32 = GLASS_W / layout::LCD_GLASS_ASPECT;
const GLASS_LEFT: f32 =
    (layout::TOP_CENTRAL_REF_LEFT + layout::TOP_CENTRAL_REF_RIGHT) * 0.5 - GLASS_W * 0.5;
const GLASS_TOP: f32 =
    layout::TOP_CENTRAL_REF_TOP + layout::TOP_CENTRAL_REF_SIZE_H * layout::LCD_GLASS_V_TOP;

/// The hot-cue strip: eight keys at the panel's own width and pitch, centred
/// in the mid-central block.
const PAD_PITCH: f32 = draw_top::HOT_CUE_BTN_WIDTH_REF + draw_top::HOT_CUE_BTN_GAP_REF;
const PADS_W: f32 = 8.0 * draw_top::HOT_CUE_BTN_WIDTH_REF + 7.0 * draw_top::HOT_CUE_BTN_GAP_REF;
const PADS_H: f32 = draw_top::HOT_CUE_BTN_WIDTH_REF / draw_top::HOT_CUE_BTN_ASPECT;
const PADS_LEFT: f32 =
    (layout::MID_CENTRAL_REF_LEFT + layout::MID_CENTRAL_REF_RIGHT) * 0.5 - PADS_W * 0.5;
const PADS_MID_Y: f32 =
    layout::MID_CENTRAL_REF_TOP + layout::MID_CENTRAL_REF_SIZE_H * draw_top::HOT_CUE_BTN_V_CENTER;

/// The browse pod's bezel ring: the knob, the gap around it, then the bezel.
const POD_RING_R: f32 = draw_vinyl_speed::VINYL_SPEED_KNOB_RADIUS_REF
    + draw_right::nav_rotary::NAV_BEZEL_INNER_GAP_REF
    + draw_right::nav_rotary::NAV_BEZEL_WIDTH_REF;

/// The browse pod's centre, where the modes column puts it.
const POD_X: f32 = layout::MODES_REF_LEFT + layout::NAV_POD_U * layout::MODES_REF_SIZE_W;
const POD_Y: f32 = layout::MODES_REF_TOP + layout::NAV_POD_V * layout::MODES_REF_SIZE_H;

/// The dash that prefixes a drawn legend (INST. DOUBLES). It is drawn as a
/// shape so that it has the same weight as the legend at panel scale.
pub(super) const LEGEND_TAB_W: f32 = 22.0;
pub(super) const LEGEND_TAB_H: f32 = 6.0;

/// The jog assembly chrome of this slate.
const JOG_CHROME: JogChrome = JogChrome {
    adjust_label: "JOG ADJUST",
    nav_arrows: true,
};

/// How this slate's panel is composed.
const PANEL: FlatPanel = FlatPanel {
    canvas: (REF_W, REF_H),
    chassis: draw_chassis::collect_chassis,
    lcd_overlay: draw_chassis::collect_chassis_lcd_overlay,
    top: draw_top::draw_top_section,
    left: draw_left::draw_left_section,
    jog: (layout::jog_rect, JOG_CHROME),
    right: draw_right::draw_right_sections,
};

fn draw_panel(app: &mut CdjApp, ui: &mut egui::Ui) {
    draw_flat_panel(app, ui, &PANEL);
}
