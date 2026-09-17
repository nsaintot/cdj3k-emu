use crate::app::ui::draw_direction::{self, DirectionPlacement};
use crate::app::ui::{DoubleBorderSpec, StrokeSpec, COL_AMBER, COL_WHITE};
use cdj3k_emu_panel::mosi_frame;
use cdj3k_emu_panel::Btn;
use egui::{FontFamily, Pos2, Rect, Stroke, Vec2};

use super::{
    draw_bordered_rect_section, layout, ButtonType, CdjApp, UiScale, COL_BLACK, COL_BTN,
    COL_BTN_HOT, COL_BTN_OUTLINED_YELLOW, COL_BTN_TEXT, COL_DARK, COL_DARK_RED, COL_RED,
    COL_SILVER, LEGEND_TAB_H, LEGEND_TAB_W,
};

mod statics;

/// Transport column in reference space: **left** strip - ref X from [`layout`].
pub(super) const PERF_TRANSPORT_COL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::TRANSPORT_REF_LEFT, layout::TRANSPORT_REF_TOP),
    Pos2::new(layout::TRANSPORT_REF_RIGHT, layout::TRANSPORT_REF_BOT),
);

/// Where this panel puts the direction flip-switch.
pub(super) const DIRECTION_PLACE: DirectionPlacement = DirectionPlacement {
    col: PERF_TRANSPORT_COL_REF,
    center_u: 0.529,
    center_v: 0.585,
    half_w: 180.0,
    half_h: 107.0,
    label_row_pitch: 57.5,
};

/// Midline each paired row is centred on, as a fraction of column width.
pub(super) const PERF_PAIR_U_MID: f32 = 0.525;
/// The pair's caption sits a touch right of that midline.
pub(super) const PERF_PAIR_LABEL_U: f32 = 0.528;
pub(super) const PERF_LOOP_U_MID: f32 = 0.568;
pub(super) const PERF_BEAT_LOOP_U_MID: f32 = 0.569;
/// CUE and PLAY/PAUSE share the paired rows' midline, not the column's.
pub(super) const PERF_U_CUE: f32 = 0.525;
pub(super) const PERF_U_PLAY: f32 = 0.525;
/// Half horizontal spacing between the paired circle controls, as a fraction of column width.
pub(super) const PERF_PAIR_U_OFF_FRAC: f32 = 0.189;

// ── USB 1 slot - top of transport column ─────────────────────────────────────
// The CDJ-3000X stacks the slot: name, rating, casing, light band, then a notch tab
// with "STOP" and a slim STOP button. No PC-link glyph, no trident logo.
/// Centre line the whole group shares.
pub(super) const PERF_U_USB_STOP_BTN: f32 = 0.331;
/// The button sits left of that centre line.
pub(super) const PERF_USB_STOP_BTN_OFF_X: f32 = -7.0;
/// V of each element within PERF_TRANSPORT_COL_REF.
pub(super) const PERF_V_USB_NAME: f32 = 0.014;
pub(super) const PERF_V_USB_RATING: f32 = 0.023;
pub(super) const PERF_V_USB_STOP_LABEL: f32 = 0.088;
pub(super) const PERF_V_USB_STOP_BTN: f32 = 0.098;

pub(super) const PERF_USB_NAME_FONT_SIZE: f32 = 36.0;
pub(super) const PERF_USB_RATING_FONT_SIZE: f32 = 24.0;
/// Drawn DC mark between "5 V" and "1 A": half-gap to the text, bar width,
/// stroke, and the half-separation of the solid bar from the broken one.
pub(super) const PERF_USB_DC_GAP: f32 = 21.0;
pub(super) const PERF_USB_DC_W: f32 = 20.0;
pub(super) const PERF_USB_DC_STROKE: f32 = 2.5;
pub(super) const PERF_USB_DC_SPLIT: f32 = 4.0;
pub(super) const PERF_USB_STOP_LABEL_FONT_SIZE: f32 = 30.0;

/// Placement of the [`LEGEND_TAB_W`]-wide dash drawn to the left of "STOP".
pub(super) const PERF_USB_STOP_TAB_OFF_X: f32 = 54.0;
pub(super) const PERF_USB_STOP_LABEL_OFF_X: f32 = -34.0;

/// Host-link computer glyph, right of the USB 1 legend.
pub(super) const PERF_U_USB_LINK_GLYPH: f32 = 0.655;
pub(super) const PERF_V_USB_LINK_GLYPH: f32 = 0.021;
pub(super) const PERF_USB_LINK_GLYPH_W: f32 = 64.0;

/// STOP is a slim bar here, not the CDJ-3000's round eject button.
pub(super) const PERF_USB_STOP_BTN_W: f32 = 84.0;
pub(super) const PERF_USB_STOP_BTN_H: f32 = 20.0;
/// Border gap; the outer radius is this plus the corner rounding.
pub(super) const PERF_USB_STOP_BTN_ROUNDING: f32 = 1.0;
pub(super) const PERF_USB_STOP_BTN_INNER_STROKE: f32 = 1.5;
pub(super) const PERF_USB_STOP_BTN_OUTER_STROKE: f32 = 2.0;

/// Center of the USB MOUNT casing.
pub(super) const PERF_U_USB_MOUNT: f32 = 0.331;
pub(super) const PERF_V_USB_MOUNT: f32 = 0.055;

/// USB MOUNT: outer glow + [`mount_rect`] = **casing** (bezel). Inside that, a nested
/// double-bordered rect = **receptacle opening**; inside that, the **tongue** bar.
pub(super) const PERF_USB_MOUNT_W: f32 = 200.0;
pub(super) const PERF_USB_MOUNT_H: f32 = 200.0;
/// Only the casing's bottom corners are radiused; the top two stay square.
pub(super) const PERF_USB_MOUNT_BOTTOM_R: f32 = 12.0;
/// Light strip along the bottom of the casing: height, width as a fraction of
/// the casing, and how far above the casing's bottom edge it sits.
/// Light band: sits flush on the cavity floor, spanning its full width.
pub(super) const PERF_USB_STRIP_H: f32 = 13.0;
/// Casing, outside in: the panel's black, a thicker grey, then black.
pub(super) const PERF_USB_MOUNT_BORDER_INNER_STROKE: f32 = 3.0;
pub(super) const PERF_USB_MOUNT_BORDER_OUTER_STROKE: f32 = 6.0;
pub(super) const PERF_USB_MOUNT_BORDER_GAP: f32 = 2.0;
/// Receptacle frame inside casing (ref units), centered in the mount cavity.
/// The receptacle sits high in the casing, leaving room for the band below.
pub(super) const PERF_USB_CONNECTOR_V_OFF: f32 = -26.0;
/// Port housing: the opened trap around the receptacle. Its sides and its
/// top edge - the trap's handle - run to the casing's inner border; the bottom
/// edge sits the same distance below the port glyph as the handle sits above it.
pub(super) const PERF_USB_PORT_HOUSING_STROKE: f32 = 3.0;
pub(super) const PERF_USB_PORT_HOUSING_TOP_STROKE: f32 = 9.0;
pub(super) const PERF_USB_CONNECTOR_W: f32 = 104.0;
pub(super) const PERF_USB_CONNECTOR_H: f32 = 42.0;
pub(super) const PERF_USB_CONNECTOR_ROUNDING: f32 = 2.0;
pub(super) const PERF_USB_CONNECTOR_INNER_STROKE: f32 = 2.5;
pub(super) const PERF_USB_CONNECTOR_OUTER_STROKE: f32 = 5.0;
pub(super) const PERF_USB_CONNECTOR_BORDER_GAP: f32 = 3.0;
/// Tongue inside receptacle opening (ref units); width cap, height, top inset from cavity.
pub(super) const PERF_USB_MOUNT_TONGUE_W_FRAC: f32 = 0.88;
pub(super) const PERF_USB_MOUNT_TONGUE_H: f32 = 10.0;
pub(super) const PERF_USB_MOUNT_TONGUE_TOP_PAD: f32 = 6.0;
pub(super) const PERF_USB_MOUNT_TONGUE_ROUNDING: f32 = 2.0;

pub(super) const PERF_SLIP_QUANTIZE_BTN_SIZE: f32 = 130.0;
pub(super) const PERF_SLIP_BTN_FONT_SIZE: f32 = 32.0;
pub(super) const PERF_QUANTIZE_BTN_FONT_SIZE: f32 = 30.0;
/// The CDJ-3000X stacks QUANTIZE over SLIP on the column's centre line, so the pair is
/// offset in V rather than U. Half the centre-to-centre spacing, as a fraction
/// of the column height.
pub(super) const PERF_SLIP_QUANTIZE_BTN_V_OFF_FRAC: f32 = 0.017;
pub(super) const PERF_SLIP_QUANTIZE_BTN_ASPECT_RATIO: f32 = 2.2;
pub(super) const PERF_SLIP_QUANTIZE_BTN_INNER_STROKE: f32 = 1.0;
pub(super) const PERF_SLIP_QUANTIZE_BTN_OUTER_STROKE: f32 = 2.0;
pub(super) const PERF_SLIP_FILL_PAD_H: f32 = 15.0;
pub(super) const PERF_SLIP_FILL_PAD_V: f32 = 0.0;

pub(super) const PERF_LOOP_SIZE_R: f32 = 70.0;
pub(super) const PERF_RELOOP_SIZE_R: f32 = 55.0;
pub(super) const PERF_LOOP_FONT_SIZE: f32 = 150.0;
pub(super) const PERF_LOOP_LINE_THICKNESS: f32 = 4.0;
pub(super) const PERF_LOOP_LINE_1_LENGTH: f32 = 47.0;
pub(super) const PERF_LOOP_LINE_2_LENGTH: f32 = 97.0;
pub(super) const PERF_LOOP_LINE_3_LENGTH: f32 = 174.0;
pub(super) const PERF_LOOP_U_OFF_FRAC: f32 = 0.242;
pub(super) const PERF_LOOP_SUBLABEL_ROUNDING: f32 = 8.0;
pub(super) const PERF_LOOP_SUBLABEL_IN_W: f32 = 180.0;
pub(super) const PERF_LOOP_SUBLABEL_OUT_W: f32 = 210.0;
pub(super) const PERF_LOOP_SUBLABEL_H: f32 = 40.0;

pub(super) const PERF_DOUBLE_CIRCLE_RING_R: f32 = 92.0;
pub(super) const PERF_DOUBLE_CIRCLE_RING_STROKE: f32 = 4.0;
pub(super) const PERF_DOUBLE_CIRCLE_BTN_R: f32 = 60.0;
pub(super) const PERF_DOUBLE_CIRCLE_BTN_INNER_STROKE: f32 = 8.0;
pub(super) const PERF_DOUBLE_CIRCLE_BTN_OUTER_STROKE: f32 = 4.0;
pub(super) const PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE: f32 = 34.0;

pub(super) const PERF_LABEL_FONT_SIZE: f32 = 30.0;

pub(super) const PERF_BEAT_LOOP_LINE_THICKNESS: f32 = 4.0;
pub(super) const PERF_BEAT_LOOP_LINE_LENGTH: f32 = 112.0;
pub(super) const PERF_BEAT_LOOP_PAIR_R: f32 = 50.0;
pub(super) const PERF_BEAT_LOOP_INNER_STROKE: f32 = 8.0;
pub(super) const PERF_BEAT_LOOP_OUTER_STROKE: f32 = 4.0;
/// IN/CUE and OUT lamp ring, straddling the black face's edge. The gap closes
/// by the same amount the ring widens, so the button's outer edge stays put.
/// Narrower than the CUE and PLAY rims: the ring is the whole lit surface and
/// the bloom scales with it, and the labels sit close above these two.
pub(super) const PERF_LOOP_LAMP_STROKE: f32 = 10.0;
pub(super) const PERF_LOOP_LAMP_GAP: f32 = 5.0;
pub(super) const PERF_BEAT_LOOP_U_OFF_FRAC: f32 = 0.241;
pub(super) const PERF_BEAT_LOOP_FONT_SIZE: f32 = 48.0;
pub(super) const PERF_BEAT_LOOP_SUBLABEL_W: f32 = 70.0;
pub(super) const PERF_BEAT_LOOP_SUBLABEL_H: f32 = 40.0;
pub(super) const PERF_BEAT_LOOP_SUBLABEL_ROUNDING: f32 = 8.0;

/// Vertical placement within [`PERF_TRANSPORT_COL_REF`] (0 = top, 1 = bottom).
pub(super) const PERF_V_SLIP_QUANTIZE_BTNS: f32 = 0.265;
pub(super) const PERF_V_LOOP_IN_OUT_LABEL: f32 = 0.36;
pub(super) const PERF_V_LOOP_LINE: f32 = 0.386;
pub(super) const PERF_V_LOOP_LABEL: f32 = 0.387;
pub(super) const PERF_V_LOOP_SUBLABEL: f32 = 0.417;
pub(super) const PERF_V_BEAT_LOOP_LABEL: f32 = 0.449;
pub(super) const PERF_V_BEAT_LOOP_SUBLABEL: f32 = 0.473;
pub(super) const PERF_V_BEAT_LOOP_LINE: f32 = 0.45;
pub(super) const PERF_V_BEAT_JUMP_LABEL: f32 = 0.505;
/// Midline and half spacing of the BEAT JUMP pair.
pub(super) const PERF_BEAT_JUMP_U_MID: f32 = 0.573;
pub(super) const PERF_BEAT_JUMP_U_OFF_FRAC: f32 = 0.241;
pub(super) const PERF_V_BEAT_JUMP_BTNS: f32 = 0.505;
pub(super) const PERF_V_TRACK_SEARCH_LABEL: f32 = 0.644;
pub(super) const PERF_V_TRACK_SEARCH_CTRLS: f32 = 0.672;
pub(super) const PERF_V_SEARCH_LABEL: f32 = 0.707;
pub(super) const PERF_V_SEARCH_CTRLS: f32 = 0.735;
pub(super) const PERF_V_CUE: f32 = 0.826;
pub(super) const PERF_V_PLAY: f32 = 0.934;

pub(super) const PERF_BEAT_JUMP_BTN_ASPECT_RATIO: f32 = 1.2;
pub(super) const PERF_BEAT_JUMP_BTN_SIZE: f32 = 130.0;
pub(super) const PERF_BEAT_JUMP_FONT_SIZE: f32 = 34.0;
pub(super) const PERF_BEAT_JUMP_BTN_INNER_STROKE: f32 = 1.0;
pub(super) const PERF_BEAT_JUMP_BTN_OUTER_STROKE: f32 = 4.0;

// ── CALL/DELETE button (same Y as hot cue row) ────────────────────────────────
/// Vertical fraction within PERF_TRANSPORT_COL_REF that aligns with the hot cue row center.
/// = (MID_CENTRAL_REF_TOP + MID_CENTRAL_REF_SIZE_H * HOT_CUE_BTN_V_CENTER - TRANSPORT_REF_TOP)
///   / TRANSPORT_REF_SIZE_H  ≈ (1505 + 460*0.3 - 300) / 4080 ≈ 0.329
pub(super) const PERF_V_CALL_DELETE: f32 = 0.329;
pub(super) const PERF_CALL_DELETE_BTN_SIZE: f32 = 45.0; // square side length in ref units
pub(super) const PERF_CALL_DELETE_BTN_U: f32 = 0.35; // button horizontal center (fraction of col width)
pub(super) const PERF_LABEL_DELETE_FONT_SIZE: f32 = 28.0;
pub(super) const PERF_CALL_DELETE_LEFT_LABEL_U: f32 = 0.22; // "CALL/" label center
pub(super) const PERF_CALL_DELETE_RIGHT_LABEL_U: f32 = 0.45; // "DELETE" outlined sublabel center
pub(super) const PERF_CALL_DELETE_SEGMENT_U: f32 = 0.633; // segment start
pub(super) const PERF_CALL_DELETE_BTN_INNER_STROKE: f32 = 2.0;
pub(super) const PERF_CALL_DELETE_BTN_OUTER_STROKE: f32 = 6.0;
pub(super) const PERF_CALL_DELETE_SUBLABEL_W: f32 = 120.0;
pub(super) const PERF_CALL_DELETE_SUBLABEL_H: f32 = 40.0;
pub(super) const PERF_CALL_DELETE_SUBLABEL_ROUNDING: f32 = 8.0;
pub(super) const PERF_CALL_DELETE_SEGMENT_LENGTH: f32 = 174.0;
pub(super) const PERF_CALL_DELETE_SEGMENT_THICKNESS: f32 = 4.0;
/// Upward offset from button center to label center (ref units). Labels sit above the button top edge.
pub(super) const PERF_CALL_DELETE_LABEL_NUDGE_Y: f32 = 100.0;
/// Outer container - same width/height as a hot cue button (HOT_CUE_BTN_WIDTH_REF=180, aspect=1.8).
pub(super) const PERF_CALL_DELETE_OUTER_H: f32 = 100.0;
pub(super) const PERF_CALL_DELETE_OUTER_W: f32 = 200.0;
pub(super) const PERF_CALL_DELETE_OUTER_ROUNDING: f32 = 12.0; // HOT_CUE_BTN_HEIGHT * 0.12

pub(super) const PERF_LARGE_BTN_RADIUS: f32 = 150.0;
/// CUE and PLAY are the same physical buttons as the CDJ-3000's, and both
/// canvases carry 0.1033 mm per reference unit, so their rim lamp is the same
/// width on either slate.
pub(super) const PERF_LARGE_BTN_STROKE_INNER: f32 = 20.0;
pub(super) const PERF_LARGE_BTN_STROKE_GAP: f32 = 10.0;
pub(super) const PERF_LARGE_BTN_STROKE_OUTER: f32 = 5.0;

pub(super) const PERF_CUE_FONT_SIZE: f32 = 40.0;

pub(super) const PERF_PLAY_FONT_SIZE: f32 = 38.0;

/// Collect fully-static elements of the left (transport) section.
///
/// Includes: "BEAT JUMP" label, "TRACK SEARCH" and
/// "SEARCH" labels, the two back-capsule borders behind those paired circle buttons,
/// and the "PLAY" sublabel.

pub(super) fn draw_left_section(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
) {
    puffin::profile_function!();
    // ── Static labels and chrome (rebuilt only on resize) ─────────────────
    let (ox, oy, scale) = layout.cache_key();
    let ppp = ui.ctx().pixels_per_point();
    let ctx = ui.ctx().clone();
    let static_shapes = app
        .left_statics_cache
        .get_or_build(ox, oy, scale, ppp, |list| {
            statics::collect_left_statics(list, &ctx, layout);
        });
    app.frame_shape_count += static_shapes.len() as u64;
    p.extend(static_shapes.iter().cloned());

    // ── USB MOUNT glow rectangle (LED-tinted, USB-A cavity graphic) ─────
    {
        let (r, g, b) = app.mosi().slot_1_rgb().unwrap_or_default();
        let lit = mosi_frame::led_color(r, g, b);
        let usb_drive = mosi_frame::led_drive_factor(r, g, b).unwrap_or(0.0);

        let mount_center =
            layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_USB_MOUNT, PERF_V_USB_MOUNT);
        let mount_size = Vec2::new(layout.sc(PERF_USB_MOUNT_W), layout.sc(PERF_USB_MOUNT_H));
        let mount_rect = Rect::from_center_size(mount_center, mount_size);

        // Casing, outside in: black, a thicker grey, black, then the cavity.
        // The outermost black is the panel the casing sits on; the grey and the
        // inner black are the spec's two strokes. Only the bottom corners are
        // radiused, so these are painted with a per-corner rounding instead of
        // draw_bordered_rect_section's single value.
        let mount_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE),
                color: COL_BLACK,
            },
            StrokeSpec {
                width: layout.sc(PERF_USB_MOUNT_BORDER_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(PERF_USB_MOUNT_BORDER_GAP),
        );
        let br = layout.sc(PERF_USB_MOUNT_BOTTOM_R);
        let round = egui::Rounding {
            nw: 0.0,
            ne: 0.0,
            sw: br,
            se: br,
        };
        let round_outer = egui::Rounding {
            nw: 0.0,
            ne: 0.0,
            sw: br + mount_border.gap,
            se: br + mount_border.gap,
        };
        p.rect_filled(mount_rect, round, COL_DARK);
        p.rect_stroke(
            mount_rect.expand(mount_border.gap),
            round_outer,
            mount_border.outer.stroke(),
        );
        p.rect_stroke(mount_rect, round, mount_border.inner.stroke());

        // Light band: flush on the cavity floor, full width, no gap to the border.
        {
            let inset = layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE);
            let band_h = layout.sc(PERF_USB_STRIP_H);
            let band = Rect::from_min_max(
                Pos2::new(
                    mount_rect.left() + inset,
                    mount_rect.bottom() - inset - band_h,
                ),
                Pos2::new(mount_rect.right() - inset, mount_rect.bottom() - inset),
            );
            // A diffuser: dim while the slot is idle, the lamp colour when driven.
            let strip_col = lit
                .map(|c| c.gamma_multiply(usb_drive))
                .unwrap_or(COL_SILVER.gamma_multiply(0.30));
            p.rect_filled(
                band,
                egui::Rounding {
                    nw: 0.0,
                    ne: 0.0,
                    sw: br * 0.55,
                    se: br * 0.55,
                },
                strip_col,
            );
        }

        // Receptacle opening: its own double border (not the casing border above).
        let connector_size = Vec2::new(
            layout.sc(PERF_USB_CONNECTOR_W),
            layout.sc(PERF_USB_CONNECTOR_H),
        );
        let connector_rect = Rect::from_center_size(
            mount_center + Vec2::new(0.0, layout.sc(PERF_USB_CONNECTOR_V_OFF)),
            connector_size,
        );
        let connector_r = layout.sc(PERF_USB_CONNECTOR_ROUNDING);
        let connector_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_USB_CONNECTOR_INNER_STROKE),
                color: COL_SILVER,
            },
            StrokeSpec {
                width: layout.sc(PERF_USB_CONNECTOR_OUTER_STROKE),
                color: COL_BTN_TEXT,
            },
            layout.sc(PERF_USB_CONNECTOR_BORDER_GAP),
        );
        // Port housing: three of its edges run to the casing's inner border, so
        // the trap and the casing read as one piece. Drawn under the
        // receptacle. Strokes are centred on their edge, so each edge is placed
        // half its own width outside the face it is meant to present.
        {
            let inset = layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE);
            let side_w = layout.sc(PERF_USB_PORT_HOUSING_STROKE);
            let top_w = layout.sc(PERF_USB_PORT_HOUSING_TOP_STROKE);
            let top_y = mount_rect.top() + inset + top_w * 0.5;
            let clearance = connector_rect.top() - (top_y + top_w * 0.5);
            let housing = Rect::from_min_max(
                Pos2::new(mount_rect.left() + inset, top_y),
                Pos2::new(
                    mount_rect.right() - inset,
                    connector_rect.bottom() + clearance + side_w * 0.5,
                ),
            );
            p.rect_stroke(housing, 0.0, Stroke::new(side_w, COL_BTN_HOT));
            p.line_segment(
                [housing.left_top(), housing.right_top()],
                Stroke::new(top_w, COL_BTN_HOT),
            );
        }

        draw_bordered_rect_section(
            p,
            connector_rect,
            Some(connector_r),
            Some(COL_DARK),
            connector_border,
        );

        // Tongue: inside receptacle cavity, top-centered.
        let cavity_inset = layout.sc(PERF_USB_CONNECTOR_INNER_STROKE) * 0.5 + layout.sc(3.0);
        let cavity = connector_rect.shrink(cavity_inset);
        let tongue_top = cavity.top() + layout.sc(PERF_USB_MOUNT_TONGUE_TOP_PAD);
        let max_tongue_h = (cavity.bottom() - tongue_top).max(1.0);
        let tongue_h = layout
            .sc(PERF_USB_MOUNT_TONGUE_H)
            .min(cavity.height() * 0.45)
            .min(max_tongue_h);
        let tongue_w = (cavity.width() * PERF_USB_MOUNT_TONGUE_W_FRAC).min(cavity.width());
        let tongue_rect = Rect::from_min_size(
            Pos2::new(cavity.center().x - 0.5 * tongue_w, tongue_top),
            Vec2::new(tongue_w, tongue_h),
        );
        let tongue_r = layout.sc(PERF_USB_MOUNT_TONGUE_ROUNDING);
        p.rect_filled(tongue_rect, tongue_r, COL_SILVER);
        p.rect_stroke(
            tongue_rect,
            tongue_r,
            Stroke::new(layout.sc(1.5), COL_DARK.gamma_multiply(0.65)),
        );

        // Reserve interactive area for future open-mount click handler.
        let _resp = ui.interact(
            mount_rect,
            ui.id().with("usb_mount_open"),
            egui::Sense::click(),
        );
    }

    {
        // ── USB STOP: a slim bar under the slot's light band ─────────────
        let usb_stop_center = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_U_USB_STOP_BTN,
            PERF_V_USB_STOP_BTN,
        ) + Vec2::new(layout.sc(PERF_USB_STOP_BTN_OFF_X), 0.0);
        let usb_stop_rect = Rect::from_center_size(
            usb_stop_center,
            Vec2::new(
                layout.sc(PERF_USB_STOP_BTN_W),
                layout.sc(PERF_USB_STOP_BTN_H),
            ),
        );
        let usb_stop_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_USB_STOP_BTN_INNER_STROKE),
                color: COL_BTN,
            },
            StrokeSpec {
                width: layout.sc(PERF_USB_STOP_BTN_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(PERF_USB_STOP_BTN_ROUNDING),
        );
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            usb_stop_rect,
            "",
            layout.sc(0.0),
            None,
            Some(COL_BTN),
            None,
            None,
            None,
            Some(usb_stop_border),
            FontFamily::Proportional,
            "usb_stop",
            Btn::UsbStop,
        );
    }

    {
        let quantize_center = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_U_USB_MOUNT,
            PERF_V_SLIP_QUANTIZE_BTNS - PERF_SLIP_QUANTIZE_BTN_V_OFF_FRAC,
        );
        let slip_center = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_U_USB_MOUNT,
            PERF_V_SLIP_QUANTIZE_BTNS + PERF_SLIP_QUANTIZE_BTN_V_OFF_FRAC,
        );
        let slip_btn_size = Vec2::new(
            layout.sc(PERF_SLIP_QUANTIZE_BTN_SIZE),
            layout.sc(PERF_SLIP_QUANTIZE_BTN_SIZE / PERF_SLIP_QUANTIZE_BTN_ASPECT_RATIO),
        );
        let slip = Rect::from_center_size(slip_center, slip_btn_size);
        let quantize = Rect::from_center_size(quantize_center, slip_btn_size);

        let slip_step = app.mosi().step_led(mosi_frame::LED_SLIP);
        let quantize_step = app.mosi().step_led(mosi_frame::LED_QUANTIZE);

        let slip_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_SLIP_QUANTIZE_BTN_INNER_STROKE),
                color: COL_DARK,
            },
            StrokeSpec {
                width: layout.sc(PERF_SLIP_QUANTIZE_BTN_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(2.0),
        );

        let slip_nudge = Vec2::new(0.0, layout.sc(-5.0));
        app.btn(
            ui,
            layout,
            ButtonType::InsetFill {
                pad_h: layout.sc(PERF_SLIP_FILL_PAD_H),
                pad_v: layout.sc(PERF_SLIP_FILL_PAD_V),
            },
            slip,
            "SLIP",
            layout.sc(PERF_SLIP_BTN_FONT_SIZE),
            Some(COL_BLACK),
            Some(match slip_step {
                mosi_frame::StepLed::Full => COL_RED,
                mosi_frame::StepLed::Medium => COL_DARK_RED,
                mosi_frame::StepLed::Off => COL_SILVER,
            }),
            Some(COL_WHITE),
            None,
            None,
            Some(slip_border),
            FontFamily::Proportional,
            "slip",
            Btn::Slip,
        );
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            quantize,
            "QUANTIZE",
            layout.sc(PERF_QUANTIZE_BTN_FONT_SIZE),
            Some(match quantize_step {
                mosi_frame::StepLed::Full => COL_RED,
                mosi_frame::StepLed::Medium => COL_DARK_RED,
                mosi_frame::StepLed::Off => COL_SILVER,
            }),
            Some(COL_BLACK),
            None,
            Some(COL_WHITE),
            Some(slip_nudge),
            Some(slip_border),
            FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_CONDENSED.into()),
            "quantize",
            Btn::Quantize,
        );
    }

    {
        // ── CALL/DELETE button ─────────────────────────────────────────────
        // Aligned vertically with the hot cue row in the mid-central panel.
        // Static chrome (outer container, labels, segment) lives in statics.rs.
        let col = PERF_TRANSPORT_COL_REF;
        let cy_ref = col.top() + col.height() * PERF_V_CALL_DELETE;
        let btn_size = PERF_CALL_DELETE_BTN_SIZE;
        let btn_cx_ref = col.left() + col.width() * PERF_CALL_DELETE_BTN_U;

        // Inner square black button with hot-cue-style double border.
        let btn_rect = layout.ar2rect(
            btn_cx_ref - btn_size * 0.5,
            cy_ref - btn_size * 0.5,
            1.0,
            btn_size,
        );
        let call_delete_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_CALL_DELETE_BTN_INNER_STROKE),
                color: COL_BTN,
            },
            StrokeSpec {
                width: layout.sc(PERF_CALL_DELETE_BTN_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(2.0),
        );
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            btn_rect,
            "",
            layout.sc(PERF_LABEL_FONT_SIZE),
            None,            // accent hidden on black bg
            Some(COL_BLACK), // bg fill
            None,            // touchdown = COL_BTN_HOT
            None,
            None,
            Some(call_delete_border),
            FontFamily::Proportional,
            "call_delete",
            Btn::CallDelete,
        );
    }

    {
        let beat_in = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_LOOP_U_MID - PERF_LOOP_U_OFF_FRAC,
            PERF_V_LOOP_LINE,
        );
        let beat_out = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_LOOP_U_MID + PERF_LOOP_U_OFF_FRAC,
            PERF_V_LOOP_LINE,
        );

        let loop_reloop = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            (PERF_LOOP_U_MID + PERF_LOOP_U_OFF_FRAC) * 1.72,
            PERF_V_LOOP_LINE,
        );
        // IN/CUE and OUT keep a black face: the lamp is the rim around it,
        // which the inner stroke straddles.
        let loop_border = |lit: bool| {
            DoubleBorderSpec::from_strokes_with_gap(
                StrokeSpec {
                    width: layout.sc(PERF_LOOP_LAMP_STROKE),
                    color: if lit { COL_AMBER } else { COL_DARK },
                },
                StrokeSpec {
                    width: layout.sc(PERF_BEAT_LOOP_OUTER_STROKE),
                    color: COL_SILVER,
                },
                layout.sc(PERF_LOOP_LAMP_GAP),
            )
        };

        let loop_reloop_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_BEAT_LOOP_INNER_STROKE),
                color: COL_DARK,
            },
            StrokeSpec {
                width: layout.sc(PERF_BEAT_LOOP_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(0.0),
        );

        let loop_in_led = app.mosi().led_bit(mosi_frame::LED_LOOP_IN);
        let loop_out_led = app.mosi().led_bit(mosi_frame::LED_LOOP_OUT);
        let loop_reloop_led = app.mosi().led_bit(mosi_frame::LED_RELOOP);

        app.circle_btn(
            ui,
            layout,
            beat_in,
            layout.sc(PERF_LOOP_SIZE_R),
            Some(COL_BLACK),
            Some(COL_BTN_HOT),
            "",
            None,
            None,
            layout.sc(PERF_V_LOOP_LABEL),
            None,
            Some(loop_border(loop_in_led)),
            "loop_in",
            Btn::LoopIn,
        );
        app.circle_btn(
            ui,
            layout,
            beat_out,
            layout.sc(PERF_LOOP_SIZE_R),
            Some(COL_BLACK),
            Some(COL_BTN_HOT),
            "",
            None,
            None,
            layout.sc(PERF_V_LOOP_LABEL),
            None,
            Some(loop_border(loop_out_led)),
            "loop_out",
            Btn::LoopOut,
        );
        app.circle_btn(
            ui,
            layout,
            loop_reloop,
            layout.sc(PERF_RELOOP_SIZE_R),
            Some(COL_BLACK),
            None,
            "•",
            Some(if loop_reloop_led {
                COL_AMBER
            } else {
                COL_BTN_TEXT
            }),
            None,
            layout.sc(PERF_LOOP_FONT_SIZE),
            None,
            Some(loop_reloop_border),
            "loop_reloop",
            Btn::Reloop,
        );
    }
    {
        let beat_loop_half = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_BEAT_LOOP_U_MID - PERF_BEAT_LOOP_U_OFF_FRAC,
            PERF_V_BEAT_LOOP_LABEL,
        );
        let beat_loop_2x = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_BEAT_LOOP_U_MID + PERF_BEAT_LOOP_U_OFF_FRAC,
            PERF_V_BEAT_LOOP_LABEL,
        );
        let beat_loop_4_led = app.mosi().led_bit(mosi_frame::LED_BEAT_JUMP_4);
        let beat_loop_8_led = app.mosi().led_bit(mosi_frame::LED_BEAT_JUMP_8);

        let beat_loop_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_BEAT_LOOP_INNER_STROKE),
                color: COL_DARK,
            },
            StrokeSpec {
                width: layout.sc(PERF_BEAT_LOOP_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(0.0),
        );

        app.circle_btn(
            ui,
            layout,
            beat_loop_half,
            layout.sc(PERF_BEAT_LOOP_PAIR_R),
            Some(COL_BLACK),
            None,
            "4",
            Some(if beat_loop_4_led {
                COL_AMBER
            } else {
                COL_BTN_TEXT
            }),
            Some(COL_BTN_OUTLINED_YELLOW),
            layout.sc(PERF_BEAT_LOOP_FONT_SIZE),
            None,
            Some(beat_loop_border),
            "beat_loop_half",
            Btn::BeatloopHalf,
        );
        app.circle_btn(
            ui,
            layout,
            beat_loop_2x,
            layout.sc(PERF_BEAT_LOOP_PAIR_R),
            Some(COL_BLACK),
            None,
            "8",
            Some(if beat_loop_8_led {
                COL_AMBER
            } else {
                COL_BTN_TEXT
            }),
            Some(COL_BTN_OUTLINED_YELLOW),
            layout.sc(PERF_BEAT_LOOP_FONT_SIZE),
            None,
            Some(beat_loop_border),
            "beat_loop_2x",
            Btn::Beatloop2x,
        );
    }

    {
        // Reference-space layout (never pass screen `Pos2` from `sp_in_rect` into `sr` / `ar2rect`).
        let cy_ref =
            PERF_TRANSPORT_COL_REF.top() + PERF_TRANSPORT_COL_REF.height() * PERF_V_BEAT_JUMP_BTNS;
        let w_ref = PERF_BEAT_JUMP_BTN_SIZE;
        let h_ref = PERF_BEAT_JUMP_BTN_SIZE / PERF_BEAT_JUMP_BTN_ASPECT_RATIO;

        let cx_left_ref = PERF_TRANSPORT_COL_REF.left()
            + PERF_TRANSPORT_COL_REF.width() * (PERF_BEAT_JUMP_U_MID - PERF_BEAT_JUMP_U_OFF_FRAC);
        let cx_right_ref = PERF_TRANSPORT_COL_REF.left()
            + PERF_TRANSPORT_COL_REF.width() * (PERF_BEAT_JUMP_U_MID + PERF_BEAT_JUMP_U_OFF_FRAC);

        let beat_jump_left = layout.ar2rect(
            cx_left_ref - w_ref * 0.5,
            cy_ref - h_ref * 0.5,
            PERF_BEAT_JUMP_BTN_ASPECT_RATIO,
            PERF_BEAT_JUMP_BTN_SIZE,
        );
        let beat_jump_right = layout.ar2rect(
            cx_right_ref - w_ref * 0.5,
            cy_ref - h_ref * 0.5,
            PERF_BEAT_JUMP_BTN_ASPECT_RATIO,
            PERF_BEAT_JUMP_BTN_SIZE,
        );

        let beat_jump_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_BEAT_JUMP_BTN_INNER_STROKE),
                color: COL_BLACK,
            },
            StrokeSpec {
                width: layout.sc(PERF_BEAT_JUMP_BTN_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(2.0),
        );

        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            beat_jump_left,
            "◀",
            layout.sc(PERF_BEAT_JUMP_FONT_SIZE),
            Some(COL_SILVER),
            Some(COL_BLACK),
            None,
            None,
            None,
            Some(beat_jump_border),
            FontFamily::Proportional,
            "cueloop_prev",
            Btn::BeatjumpPrev,
        );
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            beat_jump_right,
            "▶",
            layout.sc(PERF_BEAT_JUMP_FONT_SIZE),
            Some(COL_SILVER),
            Some(COL_BLACK),
            None,
            None,
            None,
            Some(beat_jump_border),
            FontFamily::Proportional,
            "cueloop_next",
            Btn::BeatjumpNext,
        );
    }
    draw_direction::draw_direction_switch(app, ui, p, layout, DIRECTION_PLACE);

    {
        let track_left = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_PAIR_U_MID - PERF_PAIR_U_OFF_FRAC,
            PERF_V_TRACK_SEARCH_CTRLS,
        );
        let track_right = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_PAIR_U_MID + PERF_PAIR_U_OFF_FRAC,
            PERF_V_TRACK_SEARCH_CTRLS,
        );

        let search_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_DOUBLE_CIRCLE_BTN_INNER_STROKE),
                color: COL_SILVER,
            },
            StrokeSpec {
                width: layout.sc(PERF_DOUBLE_CIRCLE_BTN_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(5.0),
        );

        app.circle_btn(
            ui,
            layout,
            track_left,
            layout.sc(PERF_DOUBLE_CIRCLE_BTN_R),
            None,
            None,
            "ǀ◀◀",
            Some(COL_BTN_TEXT),
            None,
            layout.sc(PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE),
            None,
            Some(search_border),
            "trk_prev",
            Btn::TrackPrev,
        );
        app.circle_btn(
            ui,
            layout,
            track_right,
            layout.sc(PERF_DOUBLE_CIRCLE_BTN_R),
            None,
            None,
            "▶▶ǀ",
            Some(COL_BTN_TEXT),
            None,
            layout.sc(PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE),
            None,
            Some(search_border),
            "trk_next",
            Btn::TrackNext,
        );

        let search_left = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_PAIR_U_MID - PERF_PAIR_U_OFF_FRAC,
            PERF_V_SEARCH_CTRLS,
        );
        let search_right = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            PERF_PAIR_U_MID + PERF_PAIR_U_OFF_FRAC,
            PERF_V_SEARCH_CTRLS,
        );
        app.circle_btn(
            ui,
            layout,
            search_left,
            layout.sc(PERF_DOUBLE_CIRCLE_BTN_R),
            None,
            None,
            "◀◀",
            Some(COL_BTN_TEXT),
            None,
            layout.sc(PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE),
            None,
            Some(search_border),
            "src_prev",
            Btn::SearchPrev,
        );
        app.circle_btn(
            ui,
            layout,
            search_right,
            layout.sc(PERF_DOUBLE_CIRCLE_BTN_R),
            None,
            None,
            "▶▶",
            Some(COL_BTN_TEXT),
            None,
            layout.sc(PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE),
            None,
            Some(search_border),
            "src_next",
            Btn::SearchNext,
        );
    }

    {
        let cue_c = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_CUE, PERF_V_CUE);
        // CUE is an RGB lamp on the CDJ-3000X, not the CDJ-3000's single bit: take
        // the rim's colour from the deck and fall back to the unlit face.
        let cue_lamp = app
            .mosi()
            .cue_rgb()
            .and_then(|(r, g, b)| mosi_frame::led_color(r, g, b));
        let cue_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_LARGE_BTN_STROKE_INNER),
                color: cue_lamp.unwrap_or(COL_BTN),
            },
            StrokeSpec {
                width: layout.sc(PERF_LARGE_BTN_STROKE_OUTER),
                color: COL_SILVER,
            },
            layout.sc(PERF_LARGE_BTN_STROKE_GAP),
        );
        app.circle_btn(
            ui,
            layout,
            cue_c,
            layout.sc(PERF_LARGE_BTN_RADIUS),
            None,
            None,
            "CUE",
            None,
            None,
            layout.sc(PERF_CUE_FONT_SIZE),
            None,
            Some(cue_border),
            "cue_large",
            Btn::Cue,
        );
    }
    {
        let play_c = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_PLAY, PERF_V_PLAY);
        // PLAY is an RGB lamp on the CDJ-3000X - see the CUE rim above.
        let play_lamp = app
            .mosi()
            .play_rgb()
            .and_then(|(r, g, b)| mosi_frame::led_color(r, g, b));
        let play_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_LARGE_BTN_STROKE_INNER),
                color: play_lamp.unwrap_or(COL_BTN),
            },
            StrokeSpec {
                width: layout.sc(PERF_LARGE_BTN_STROKE_OUTER),
                color: COL_SILVER,
            },
            layout.sc(PERF_LARGE_BTN_STROKE_GAP),
        );
        app.circle_btn(
            ui,
            layout,
            play_c,
            layout.sc(PERF_LARGE_BTN_RADIUS),
            None,
            None,
            "▶ / ⏸",
            None,
            None,
            layout.sc(PERF_PLAY_FONT_SIZE),
            None,
            Some(play_border),
            "play_large",
            Btn::Play,
        );
    }
}
