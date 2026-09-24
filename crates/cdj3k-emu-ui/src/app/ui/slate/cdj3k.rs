//! CDJ-3000 slate: reference canvas, section zones and the section renderers.
//!
//! The jog assembly is the shared `ui::draw_jog`, placed at this slate's jog
//! zone. The section files reach the shared toolkit through `super::` (the
//! glob import below), exactly as they did when they lived at the `ui` root.

mod draw_chassis;
mod draw_left;
mod draw_right;
mod draw_top;
mod layout;

use crate::app::picker::{Bay, Card, Glyph};
use crate::app::ui::slate::Slate;
#[allow(unused_imports)]
use crate::app::ui::*;
use crate::app::CdjApp;
use egui::{Color32, Pos2, Rect, Vec2};

pub(in crate::app) const SLATE: Slate = Slate {
    ref_canvas: (REF_W, REF_H),
    card: Card {
        subtitle: "9-inch touch display",
        era: "2020",
        accent: Color32::from_rgb(56, 132, 255),
        glyph: Glyph {
            canvas: (REF_W, REF_H),
            body_top: layout::TRANSPORT_REF_TOP / REF_H,
            riser: (
                layout::TOP_CENTRAL_REF_LEFT / REF_W,
                layout::TOP_CENTRAL_REF_RIGHT / REF_W,
            ),
            screen: Rect::from_min_max(
                Pos2::new(GLASS_LEFT / REF_W, GLASS_TOP / REF_H),
                Pos2::new(
                    (GLASS_LEFT + GLASS_W) / REF_W,
                    (GLASS_TOP + GLASS_H) / REF_H,
                ),
            ),
            bezel: Vec2::new(0.0362, 0.0225),
            transport_col: Rect::from_min_max(
                Pos2::new(
                    layout::TRANSPORT_REF_LEFT / REF_W,
                    layout::TRANSPORT_REF_TOP / REF_H,
                ),
                Pos2::new(
                    layout::TRANSPORT_REF_RIGHT / REF_W,
                    layout::TRANSPORT_REF_BOT / REF_H,
                ),
            ),
            modes_col: Rect::from_min_max(
                Pos2::new(
                    layout::MODES_REF_LEFT / REF_W,
                    layout::MODES_REF_TOP / REF_H,
                ),
                Pos2::new(
                    layout::MODES_REF_RIGHT / REF_W,
                    layout::MODES_REF_BOT / REF_H,
                ),
            ),
            bay: Bay::UsbAndSd,
            recessed: true,
            pads_x: (PADS_LEFT / REF_W, (PADS_LEFT + PADS_W) / REF_W),
            pads_y: (
                (PADS_MID_Y - PADS_H * 0.5) / REF_H,
                (PADS_MID_Y + PADS_H * 0.5) / REF_H,
            ),
            pad_gap: draw_top::HOT_CUE_BTN_GAP_REF / PAD_PITCH,
            jog: (
                (layout::JOG_REF_LEFT + layout::JOG_REF_RIGHT) * 0.5 / REF_W,
                (layout::JOG_REF_TOP + layout::JOG_REF_BOT) * 0.5 / REF_H,
                draw_jog::JOG_OUTER_2_STROKE_RADIUS / REF_W,
            ),
            jog_hub: draw_jog::JOG_INNER_LCD_C0_RADIUS / draw_jog::JOG_OUTER_2_STROKE_RADIUS,
            pod: Pos2::new(POD_X / REF_W, POD_Y / REF_H),
            pod_half_w: 150.0,
            pod_ring_r: POD_RING_R,
        },
    },
    draw_panel,
};

/// Reference canvas (unscaled layout units) every zone in [`layout`] is
/// expressed in; the window is aspect-locked to `REF_W : REF_H`.
pub(in crate::app) const REF_W: f32 = 3185.0;
pub(in crate::app) const REF_H: f32 = 4360.0;

// ── The picker's miniature ────────────────────────────────────────────────────
// Every fraction the card draws is computed from the zones the panel itself
// draws from, so the two cannot drift apart. Nothing here is eyeballed.

/// The screen's glass, placed as the panel places it: centred in the
/// top-central block, its width a fraction of that block and its height its
/// own aspect.
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

/// The dash that prefixes a drawn legend (INST. DOUBLES). Drawn rather than
/// typed so it reads at the same weight at panel scale.
pub(super) const LEGEND_TAB_W: f32 = 22.0;
pub(super) const LEGEND_TAB_H: f32 = 6.0;

/// Jog assembly chrome of this slate.
const JOG_CHROME: JogChrome = JogChrome {
    adjust_label: "JOG ADJUST",
    nav_arrows: true,
};

/// Draw the whole panel: chassis, LCD overlay, then every section.
fn draw_panel(app: &mut CdjApp, ui: &mut egui::Ui) {
    let layout = UiScale::fit(ui.clip_rect().shrink(2.0), REF_W, REF_H);
    let (ox, oy, scale) = layout.cache_key();
    let p = ui.painter().clone();
    let ppp = ui.ctx().pixels_per_point();

    // Chassis - fully static, rebuilt only on resize.
    let bg_shapes = app
        .chassis_bg_cache
        .get_or_build(ox, oy, scale, ppp, |list| {
            draw_chassis::collect_chassis(list, &layout);
        });
    app.frame_shape_count += bg_shapes.len() as u64;
    p.extend(bg_shapes.iter().cloned());
    // LCD overlay on top of all sections - fully static, rebuilt only on resize.
    let overlay_shapes = app
        .chassis_lcd_overlay_cache
        .get_or_build(ox, oy, scale, ppp, |list| {
            draw_chassis::collect_chassis_lcd_overlay(list, &layout);
        });
    app.frame_shape_count += overlay_shapes.len() as u64;
    p.extend(overlay_shapes.iter().cloned());

    {
        puffin::profile_scope!("draw_top");
        draw_top::draw_top_section(app, ui, &p, &layout);
    }
    {
        puffin::profile_scope!("draw_left");
        draw_left::draw_left_section(app, ui, &p, &layout);
    }
    {
        puffin::profile_scope!("draw_jog");
        app.draw_jog_wheel_section(ui, &p, &layout, layout::jog_rect(), JOG_CHROME);
    }
    {
        puffin::profile_scope!("draw_right");
        draw_right::draw_right_sections(app, ui, &p, &layout);
    }

    debug_align_squares(&p, &layout, REF_W, REF_H);
}
