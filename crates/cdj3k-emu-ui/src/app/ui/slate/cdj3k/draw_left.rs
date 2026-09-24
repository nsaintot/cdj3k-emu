use crate::app::ui::draw_direction::{self, DirectionPlacement};
use crate::app::ui::{DoubleBorderSpec, StrokeSpec, COL_WHITE, COL_YELLOW};
use cdj3k_emu_panel::mosi_frame;
use cdj3k_emu_panel::Btn;
use egui::{Color32, FontFamily, Pos2, Rect, Stroke, Vec2};

use super::{
    draw_bordered_rect_section, layout, ButtonType, CdjApp, UiScale, COL_AMBER, COL_BLACK, COL_BTN,
    COL_BTN_CUE, COL_BTN_HOT, COL_BTN_OUTLINED_YELLOW, COL_BTN_PLAY, COL_BTN_TEXT, COL_DARK,
    COL_DARK_RED, COL_RED, COL_SILVER,
};

mod statics;

/// Transport column in reference space: **left** strip - ref X from [`layout`].
pub(super) const PERF_TRANSPORT_COL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::TRANSPORT_REF_LEFT, layout::TRANSPORT_REF_TOP),
    Pos2::new(layout::TRANSPORT_REF_RIGHT, layout::TRANSPORT_REF_BOT),
);

/// Vertical fraction within [`PERF_TRANSPORT_COL_REF`] for the "DIRECTION" label.
pub(super) const PERF_V_DIRECTION_LABEL: f32 = 0.55;

/// Where this panel puts the direction flip-switch.
pub(super) const DIRECTION_PLACE: DirectionPlacement = DirectionPlacement {
    col: PERF_TRANSPORT_COL_REF,
    center_u: 0.5,
    center_v: 0.585,
    half_w: 180.0,
    half_h: 100.0,
    label_row_pitch: 100.0 * 2.0 / 3.0,
};

/// Half horizontal spacing between the paired circle controls, as a fraction of column width.
pub(super) const PERF_PAIR_U_OFF_FRAC: f32 = 0.19;

// ── USB STOP - top of transport column ────────────────────────────────────────
/// Horizontal center fraction within PERF_TRANSPORT_COL_REF for the whole group.
pub(super) const PERF_U_USB_STOP: f32 = 0.695;
/// Vertical fractions within PERF_TRANSPORT_COL_REF.
pub(super) const PERF_V_USB_STOP_PRE_LABEL: f32 = 0.03;
pub(super) const PERF_V_USB_STOP_ICON_SCREEN: f32 = 0.017;
pub(super) const PERF_V_USB_STOP_LABEL: f32 = 0.042;
pub(super) const PERF_V_USB_STOP_BTN: f32 = 0.070;

/// Small LED-style indicator dash above the laptop icon.
pub(super) const PERF_USB_STOP_LED_W: f32 = 28.0;
pub(super) const PERF_USB_STOP_LED_H: f32 = 7.0;

/// Laptop glyph, overall width at the stand's foot.
pub(super) const PERF_USB_STOP_ICON_W: f32 = 70.0;

pub(super) const PERF_USB_STOP_LABEL_FONT_SIZE: f32 = 34.0;

pub(super) const PERF_USB_STOP_BTN_R: f32 = 42.0;
pub(super) const PERF_USB_STOP_BTN_INNER_STROKE: f32 = 4.0;
pub(super) const PERF_USB_STOP_BTN_OUTER_STROKE: f32 = 8.0;

// ── USB sign + LED cavity (left of the USB STOP button) ──────────────────────
/// Center of the USB sign (logo), in fractions of PERF_TRANSPORT_COL_REF.
pub(super) const PERF_U_USB_SIGN: f32 = 0.33;
pub(super) const PERF_V_USB_SIGN: f32 = 0.0795;
/// Center of the USB MOUNT glow rectangle.
pub(super) const PERF_U_USB_MOUNT: f32 = 0.322;
pub(super) const PERF_V_USB_MOUNT: f32 = 0.040;

/// USB trident logo dimensions (ref units).
pub(super) const PERF_USB_SIGN_W: f32 = 70.0;
pub(super) const PERF_USB_SIGN_STROKE: f32 = 3.5;
pub(super) const PERF_USB_SIGN_TAIL_DOT_R: f32 = 5.0;
pub(super) const PERF_USB_SIGN_ARROW_LEN: f32 = 12.0;
pub(super) const PERF_USB_SIGN_ARROW_HALF: f32 = 6.0;
pub(super) const PERF_USB_SIGN_FORK_OFF_Y: f32 = 12.0;
/// Fork X positions, as ref-unit offsets from the trident center (cx_ref).
/// Top fork (terminates in a filled circle).
pub(super) const PERF_USB_SIGN_TOP_FORK_X_START: f32 = -22.0;
pub(super) const PERF_USB_SIGN_TOP_FORK_X_DIAG_END: f32 = -10.0;
pub(super) const PERF_USB_SIGN_TOP_FORK_X_END: f32 = 0.0;
/// Bottom fork (terminates in a filled square).
pub(super) const PERF_USB_SIGN_BOT_FORK_X_START: f32 = -18.0;
pub(super) const PERF_USB_SIGN_BOT_FORK_X_DIAG_END: f32 = 0.0;
pub(super) const PERF_USB_SIGN_BOT_FORK_X_END: f32 = 11.0;
pub(super) const PERF_USB_SIGN_SQUARE_S: f32 = 8.0;
pub(super) const PERF_USB_SIGN_CIRCLE_R: f32 = 5.0;

/// USB MOUNT: outer glow + [`mount_rect`] = **casing** (bezel). Inside that, a nested
/// double-bordered rect = **receptacle opening**; inside that, the **tongue** bar.
pub(super) const PERF_USB_MOUNT_W: f32 = 196.0;
pub(super) const PERF_USB_MOUNT_H: f32 = 180.0;
pub(super) const PERF_USB_MOUNT_ROUNDING: f32 = 8.0;
pub(super) const PERF_USB_MOUNT_GLOW_SPREAD: f32 = 50.0;
/// Peak alpha of the halo outside the casing, before the lamp's drive level.
pub(super) const PERF_USB_MOUNT_GLOW_ALPHA: f32 = 0.04;
/// Translucent housing lit by an LED at the top and bottom wall centres: the
/// share of their light on the wall and floor, and the exposure of the
/// plastic and of the LED cores (see `led_color_hot`).
pub(super) const PERF_USB_HOUSING_WALL_LIT: f32 = 0.72;
pub(super) const PERF_USB_HOUSING_FLOOR_LIT: f32 = 0.65;
pub(super) const PERF_USB_HOUSING_EXPOSURE: f32 = 1.4;
pub(super) const PERF_USB_LED_EXPOSURE: f32 = 2.5;
/// Each LED's diffused spot: half-width and half-height of its falloff.
pub(super) const PERF_USB_LED_SPOT_RX: f32 = 135.0;
pub(super) const PERF_USB_LED_SPOT_RY: f32 = 105.0;
pub(super) const PERF_USB_MOUNT_BORDER_INNER_STROKE: f32 = 12.0;
pub(super) const PERF_USB_MOUNT_BORDER_OUTER_STROKE: f32 = 4.0;
pub(super) const PERF_USB_MOUNT_BORDER_GAP: f32 = 4.0;
/// Open tray folded against the top wall; offsets from the casing centre.
pub(super) const PERF_USB_FLAP_W: f32 = 178.0;
pub(super) const PERF_USB_FLAP_TOP_DY: f32 = -56.0;
pub(super) const PERF_USB_FLAP_BOT_DY: f32 = -44.0;
pub(super) const PERF_USB_FLAP_STROKE: f32 = 2.0;
/// Handle dome: chord along the flap's top edge, and its height above it.
pub(super) const PERF_USB_DOME_CHORD: f32 = 81.0;
pub(super) const PERF_USB_DOME_H: f32 = 12.0;
/// Receptacle frame inside casing (ref units), below the tray.
pub(super) const PERF_USB_CONNECTOR_W: f32 = 134.0;
pub(super) const PERF_USB_CONNECTOR_H: f32 = 56.0;
pub(super) const PERF_USB_CONNECTOR_V_OFF: f32 = 4.0;
pub(super) const PERF_USB_CONNECTOR_ROUNDING: f32 = 2.0;
pub(super) const PERF_USB_CONNECTOR_INNER_STROKE: f32 = 2.5;
pub(super) const PERF_USB_CONNECTOR_OUTER_STROKE: f32 = 5.0;
pub(super) const PERF_USB_CONNECTOR_BORDER_GAP: f32 = 3.0;
/// Tongue inside receptacle opening (ref units); width cap, height, top inset from cavity.
pub(super) const PERF_USB_MOUNT_TONGUE_W_FRAC: f32 = 0.88;
pub(super) const PERF_USB_MOUNT_TONGUE_H: f32 = 21.0;
pub(super) const PERF_USB_MOUNT_TONGUE_TOP_PAD: f32 = 4.0;
pub(super) const PERF_USB_MOUNT_TONGUE_ROUNDING: f32 = 2.0;

/// "5V <dc> 1A" under the USB casing: centre, font, and the drawn DC mark
/// (half-gap to the text, bar width, stroke, half-separation of the bars).
pub(super) const PERF_U_USB_RATING: f32 = 0.323;
pub(super) const PERF_V_USB_RATING: f32 = 0.0698;
pub(super) const PERF_USB_RATING_FONT_SIZE: f32 = 28.0;
pub(super) const PERF_USB_DC_GAP: f32 = 22.0;
pub(super) const PERF_USB_DC_W: f32 = 30.0;
pub(super) const PERF_USB_DC_STROKE: f32 = 3.0;
pub(super) const PERF_USB_DC_SPLIT: f32 = 4.0;

// ── SD slot - under the USB casing. Decorative: drawn with its cover closed.
/// Centre of the SD casing.
pub(super) const PERF_U_SD: f32 = 0.464;
pub(super) const PERF_V_SD: f32 = 0.130;
/// Casing silhouette; top corners barely radiused, bottom ones well rounded.
pub(super) const PERF_SD_W: f32 = 367.0;
pub(super) const PERF_SD_H: f32 = 223.0;
pub(super) const PERF_SD_TOP_R: f32 = 6.0;
pub(super) const PERF_SD_BOTTOM_R: f32 = 22.0;
/// Casing, outside in: a grey rim, the bezel, then a black wall at this inset.
pub(super) const PERF_SD_RIM_STROKE: f32 = 5.0;
pub(super) const PERF_SD_WALL_INSET: f32 = 13.0;
pub(super) const PERF_SD_WALL_STROKE: f32 = 5.0;
/// Cover: fills the cavity down to the hinge, with rounded lower corners.
pub(super) const PERF_SD_COVER_PAD: f32 = 3.0;
pub(super) const PERF_SD_COVER_BOT_DY: f32 = 51.0;
pub(super) const PERF_SD_COVER_R: f32 = 16.0;
/// Light bar below the cover: size and offset from the casing centre.
pub(super) const PERF_SD_LED_W: f32 = 231.5;
pub(super) const PERF_SD_LED_H: f32 = 22.0;
pub(super) const PERF_SD_LED_DY: f32 = 79.0;
/// "SD" legend under the casing: card glyph and label.
pub(super) const PERF_U_SD_GLYPH: f32 = 0.405;
pub(super) const PERF_V_SD_LEGEND: f32 = 0.1682;
pub(super) const PERF_SD_GLYPH_W: f32 = 46.0;
pub(super) const PERF_SD_GLYPH_H: f32 = 55.0;
pub(super) const PERF_SD_GLYPH_STROKE: f32 = 3.5;
/// Clipped corner of the card, as a fraction of its width.
pub(super) const PERF_SD_GLYPH_NOTCH_FRAC: f32 = 0.3;
pub(super) const PERF_U_SD_LABEL: f32 = 0.493;
pub(super) const PERF_SD_LABEL_FONT_SIZE: f32 = 36.0;

pub(super) const PERF_TIME_MODE_AUTO_CUE_SIZE_R: f32 = 30.0;
pub(super) const PERF_TIME_MODE_AUTO_CUE_LABEL_FONT_SIZE: f32 = 26.0;

pub(super) const PERF_SLIP_QUANTIZE_BTN_SIZE: f32 = 130.0;
pub(super) const PERF_SLIP_BTN_FONT_SIZE: f32 = 32.0;
pub(super) const PERF_QUANTIZE_BTN_FONT_SIZE: f32 = 30.0;
pub(super) const PERF_SLIP_QUANTIZE_BTN_U_OFF_FRAC: f32 = 0.19;
pub(super) const PERF_SLIP_QUANTIZE_BTN_ASPECT_RATIO: f32 = 2.2;
pub(super) const PERF_SLIP_QUANTIZE_BTN_INNER_STROKE: f32 = 1.0;
pub(super) const PERF_SLIP_QUANTIZE_BTN_OUTER_STROKE: f32 = 2.0;
pub(super) const PERF_SLIP_FILL_PAD_H: f32 = 15.0;
pub(super) const PERF_SLIP_FILL_PAD_V: f32 = 0.0;

pub(super) const PERF_LOOP_SIZE_R: f32 = 70.0;
pub(super) const PERF_RELOOP_SIZE_R: f32 = 55.0;
pub(super) const PERF_LOOP_FONT_SIZE: f32 = 150.0;
pub(super) const PERF_LOOP_LINE_THICKNESS: f32 = 4.0;
pub(super) const PERF_LOOP_LINE_1_LENGTH: f32 = 45.0;
pub(super) const PERF_LOOP_LINE_2_LENGTH: f32 = 90.0;
pub(super) const PERF_LOOP_LINE_3_LENGTH: f32 = 140.0;
pub(super) const PERF_LOOP_U_OFF_FRAC: f32 = 0.245;
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
pub(super) const PERF_BEAT_LOOP_LINE_LENGTH: f32 = 100.0;
pub(super) const PERF_BEAT_LOOP_PAIR_R: f32 = 50.0;
pub(super) const PERF_BEAT_LOOP_INNER_STROKE: f32 = 8.0;
pub(super) const PERF_BEAT_LOOP_OUTER_STROKE: f32 = 4.0;
pub(super) const PERF_BEAT_LOOP_U_OFF_FRAC: f32 = 0.24;
pub(super) const PERF_BEAT_LOOP_FONT_SIZE: f32 = 48.0;
pub(super) const PERF_BEAT_LOOP_SUBLABEL_W: f32 = 70.0;
pub(super) const PERF_BEAT_LOOP_SUBLABEL_H: f32 = 40.0;
pub(super) const PERF_BEAT_LOOP_SUBLABEL_ROUNDING: f32 = 8.0;

/// Vertical placement within [`PERF_TRANSPORT_COL_REF`] (0 = top, 1 = bottom).
pub(super) const PERF_V_TIME_MODE_AUTO_CUE_LABEL: f32 = 0.205;
pub(super) const PERF_V_TIME_MODE_AUTO_CUE: f32 = 0.207;
pub(super) const PERF_V_SLIP_QUANTIZE_BTNS: f32 = 0.254;
pub(super) const PERF_V_LOOP_IN_OUT_LABEL: f32 = 0.36;
pub(super) const PERF_V_LOOP_LINE: f32 = 0.387;
pub(super) const PERF_V_LOOP_LABEL: f32 = 0.387;
pub(super) const PERF_V_LOOP_SUBLABEL: f32 = 0.417;
pub(super) const PERF_V_BEAT_LOOP_LABEL: f32 = 0.449;
pub(super) const PERF_V_BEAT_LOOP_SUBLABEL: f32 = 0.473;
pub(super) const PERF_V_BEAT_LOOP_LINE: f32 = 0.45;
pub(super) const PERF_V_BEAT_JUMP_LABEL: f32 = 0.505;
pub(super) const PERF_V_BEAT_JUMP_BTNS: f32 = 0.506;
pub(super) const PERF_V_TRACK_SEARCH_LABEL: f32 = 0.644;
pub(super) const PERF_V_TRACK_SEARCH_CTRLS: f32 = 0.6735;
pub(super) const PERF_V_SEARCH_LABEL: f32 = 0.707;
pub(super) const PERF_V_SEARCH_CTRLS: f32 = 0.737;
pub(super) const PERF_V_CUE: f32 = 0.827;
pub(super) const PERF_V_PLAY: f32 = 0.9365;

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
pub(super) const PERF_CALL_DELETE_SEGMENT_U: f32 = 0.64; // segment start
pub(super) const PERF_CALL_DELETE_BTN_INNER_STROKE: f32 = 2.0;
pub(super) const PERF_CALL_DELETE_BTN_OUTER_STROKE: f32 = 6.0;
pub(super) const PERF_CALL_DELETE_SUBLABEL_W: f32 = 120.0;
pub(super) const PERF_CALL_DELETE_SUBLABEL_H: f32 = 40.0;
pub(super) const PERF_CALL_DELETE_SUBLABEL_ROUNDING: f32 = 8.0;
pub(super) const PERF_CALL_DELETE_SEGMENT_LENGTH: f32 = 100.0;
pub(super) const PERF_CALL_DELETE_SEGMENT_THICKNESS: f32 = 4.0;
/// Upward offset from button center to label center (ref units). Labels sit above the button top edge.
pub(super) const PERF_CALL_DELETE_LABEL_NUDGE_Y: f32 = 100.0;
/// Outer container - same width/height as a hot cue button (HOT_CUE_BTN_WIDTH_REF=180, aspect=1.8).
pub(super) const PERF_CALL_DELETE_OUTER_H: f32 = 100.0;
pub(super) const PERF_CALL_DELETE_OUTER_W: f32 = 200.0;
pub(super) const PERF_CALL_DELETE_OUTER_ROUNDING: f32 = 12.0; // HOT_CUE_BTN_HEIGHT * 0.12

pub(super) const PERF_LARGE_BTN_RADIUS: f32 = 150.0;
pub(super) const PERF_LARGE_BTN_STROKE_INNER: f32 = 20.0;
pub(super) const PERF_LARGE_BTN_STROKE_GAP: f32 = 10.0;
pub(super) const PERF_LARGE_BTN_STROKE_OUTER: f32 = 5.0;

pub(super) const PERF_CUE_FONT_SIZE: f32 = 40.0;

pub(super) const PERF_PLAY_FONT_SIZE: f32 = 38.0;
pub(super) const PERF_PLAY_LABEL_FONT_SIZE: f32 = 34.0;
pub(super) const PERF_PLAY_LABEL_GAP_Y: f32 = 210.0;

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
        let (r, g, b) = app.mosi().slot_2_rgb().unwrap_or_default();
        let lit = app.mosi().led_color(mosi_frame::LedPart::Slot, r, g, b);
        let usb_drive = mosi_frame::led_drive_factor(r, g, b).unwrap_or(0.0);

        let mount_center =
            layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_USB_MOUNT, PERF_V_USB_MOUNT);
        let mount_size = Vec2::new(layout.sc(PERF_USB_MOUNT_W), layout.sc(PERF_USB_MOUNT_H));
        let mount_rect = Rect::from_center_size(mount_center, mount_size);
        let rounding = layout.sc(PERF_USB_MOUNT_ROUNDING);

        // Soft diffuse glow halo (only when LED is lit), tinted by the USB
        // LED color. More layers + smooth falloff = wider, softer spread.
        // Alpha is gamma-lifted so whites read as bright on screen.
        if let Some(led_color) = lit {
            let max_expand = layout.sc(PERF_USB_MOUNT_GLOW_SPREAD);
            const N: usize = 12;
            for layer in 0..N {
                // t = 1.0 at the rect edge, → 0.0 at the outermost layer.
                let t = 1.0 - layer as f32 / (N - 1) as f32;
                // Quadratic ease-out for softer outer falloff.
                let falloff = t * t;
                // Each layer extends progressively farther from the rect.
                let expand = (1.0 - t) * max_expand;
                let alpha_linear: f32 = falloff * PERF_USB_MOUNT_GLOW_ALPHA * usb_drive;
                let alpha = alpha_linear.powf(1.0 / mosi_frame::LED_GAMMA);
                if alpha < 0.005 {
                    continue;
                }

                let g_rect = mount_rect.expand(expand);
                p.rect_filled(g_rect, rounding + expand, led_color.gamma_multiply(alpha));
            }
        }

        // Translucent housing lit from behind by an LED at the top and bottom
        // wall centres.
        let wall = layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE) * 0.5;
        let mosi = app.mosi();
        let hot_at =
            |exposure: f32| mosi.led_color_hot(mosi_frame::LedPart::Slot, r, g, b, exposure);
        let hot = hot_at(PERF_USB_HOUSING_EXPOSURE);
        let lit_at = |k: f32| {
            hot.map(|c| mix(COL_DARK, c, k * usb_drive))
                .unwrap_or(COL_DARK)
        };
        let floor = lit_at(PERF_USB_HOUSING_FLOOR_LIT);
        let mount_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_USB_MOUNT_BORDER_INNER_STROKE),
                color: lit_at(PERF_USB_HOUSING_WALL_LIT),
            },
            StrokeSpec {
                width: layout.sc(PERF_USB_MOUNT_BORDER_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(PERF_USB_MOUNT_BORDER_GAP),
        );
        draw_bordered_rect_section(p, mount_rect, Some(rounding), Some(floor), mount_border);

        if let Some(core) = hot_at(PERF_USB_LED_EXPOSURE) {
            let core = mix(COL_DARK, core, usb_drive);
            let spot = Vec2::new(
                layout.sc(PERF_USB_LED_SPOT_RX),
                layout.sc(PERF_USB_LED_SPOT_RY),
            );
            let clip = p.with_clip_rect(mount_rect.intersect(p.clip_rect()));
            for y in [mount_rect.top() + wall, mount_rect.bottom() - wall] {
                radial_glow(&clip, Pos2::new(mount_center.x, y), spot, core);
            }
        }

        // The open tray, folded up against the top wall.
        crate::app::ui::draw_usb_tray(
            p,
            crate::app::ui::UsbTraySpec {
                centre_x: mount_center.x,
                top: mount_center.y + layout.sc(PERF_USB_FLAP_TOP_DY),
                bottom: mount_center.y + layout.sc(PERF_USB_FLAP_BOT_DY),
                half_w: layout.sc(PERF_USB_FLAP_W) * 0.5,
                dome_chord: layout.sc(PERF_USB_DOME_CHORD),
                dome_h: layout.sc(PERF_USB_DOME_H),
                stroke: layout.sc(PERF_USB_FLAP_STROKE),
            },
        );

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

    // ── SD slot: casing, closed cover, light bar ─────────────────────────
    {
        let (r, g, b) = app.mosi().slot_1_rgb().unwrap_or_default();
        let lit = app.mosi().led_color(mosi_frame::LedPart::Slot, r, g, b);
        let drive = mosi_frame::led_drive_factor(r, g, b).unwrap_or(0.0);

        let centre = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_SD, PERF_V_SD);
        let outer = Rect::from_center_size(
            centre,
            Vec2::new(layout.sc(PERF_SD_W), layout.sc(PERF_SD_H)),
        );
        let corners = |inset: f32| egui::Rounding {
            nw: (layout.sc(PERF_SD_TOP_R) - inset).max(0.0),
            ne: (layout.sc(PERF_SD_TOP_R) - inset).max(0.0),
            sw: (layout.sc(PERF_SD_BOTTOM_R) - inset).max(0.0),
            se: (layout.sc(PERF_SD_BOTTOM_R) - inset).max(0.0),
        };
        let rim_w = layout.sc(PERF_SD_RIM_STROKE);
        p.rect_filled(outer, corners(0.0), COL_DARK);
        p.rect_stroke(
            outer.shrink(rim_w * 0.5),
            corners(rim_w * 0.5),
            Stroke::new(rim_w, COL_SILVER),
        );

        let wall_w = layout.sc(PERF_SD_WALL_STROKE);
        let wall_mid = layout.sc(PERF_SD_WALL_INSET) + wall_w * 0.5;
        p.rect_stroke(
            outer.shrink(wall_mid),
            corners(wall_mid),
            Stroke::new(wall_w, COL_BLACK),
        );
        let cavity_inset = layout.sc(PERF_SD_WALL_INSET) + wall_w;
        let cavity = outer.shrink(cavity_inset);
        p.rect_filled(cavity, corners(cavity_inset), COL_BLACK);

        let pad = layout.sc(PERF_SD_COVER_PAD);
        let cover = Rect::from_min_max(
            cavity.left_top() + Vec2::splat(pad),
            Pos2::new(
                cavity.right() - pad,
                centre.y + layout.sc(PERF_SD_COVER_BOT_DY),
            ),
        );
        let cover_r = layout.sc(PERF_SD_COVER_R);
        let cover_round = egui::Rounding {
            nw: 0.0,
            ne: 0.0,
            sw: cover_r,
            se: cover_r,
        };
        p.rect_filled(cover, cover_round, COL_BTN_HOT);
        p.rect_stroke(
            cover,
            cover_round,
            Stroke::new(layout.sc(PERF_USB_FLAP_STROKE), COL_SILVER),
        );

        // A diffuser: dim while the slot is idle, the lamp colour when driven.
        let led = Rect::from_center_size(
            centre + Vec2::new(0.0, layout.sc(PERF_SD_LED_DY)),
            Vec2::new(layout.sc(PERF_SD_LED_W), layout.sc(PERF_SD_LED_H)),
        );
        let led_col = lit.map(|c| c.gamma_multiply(drive)).unwrap_or(COL_SILVER);
        p.rect_filled(led, led.height() * 0.5, led_col);
    }

    {
        // ── USB STOP button - top of transport column ────────────────────
        let usb_stop_center =
            layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_USB_STOP, PERF_V_USB_STOP_BTN);
        let usb_stop_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_USB_STOP_BTN_INNER_STROKE),
                color: COL_BTN,
            },
            StrokeSpec {
                width: layout.sc(PERF_USB_STOP_BTN_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(10.0),
        );
        app.circle_btn(
            ui,
            layout,
            usb_stop_center,
            layout.sc(PERF_USB_STOP_BTN_R),
            Some(COL_BLACK),
            None,
            "",
            None,
            None,
            layout.sc(0.0),
            None,
            Some(usb_stop_border),
            "usb_stop",
            Btn::UsbStop,
        );
    }

    {
        let time_mode_auto_cue =
            layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.46, PERF_V_TIME_MODE_AUTO_CUE);

        app.circle_btn(
            ui,
            layout,
            time_mode_auto_cue,
            layout.sc(PERF_TIME_MODE_AUTO_CUE_SIZE_R),
            Some(COL_BLACK),
            None,
            "",
            None,
            None,
            layout.sc(0.0),
            None,
            None,
            "time_mode_auto_cue",
            Btn::TimeMode,
        );
    }

    {
        let slip_center = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.46 - PERF_SLIP_QUANTIZE_BTN_U_OFF_FRAC,
            PERF_V_SLIP_QUANTIZE_BTNS,
        );
        let quantize_center = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.46 + PERF_SLIP_QUANTIZE_BTN_U_OFF_FRAC,
            PERF_V_SLIP_QUANTIZE_BTNS,
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
            0.555 - PERF_LOOP_U_OFF_FRAC,
            PERF_V_LOOP_LINE,
        );
        let beat_out = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.555 + PERF_LOOP_U_OFF_FRAC,
            PERF_V_LOOP_LINE,
        );

        let loop_reloop = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            (0.555 + PERF_LOOP_U_OFF_FRAC) * 1.72,
            PERF_V_LOOP_LINE,
        );
        let loop_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_BEAT_LOOP_INNER_STROKE),
                color: COL_DARK,
            },
            StrokeSpec {
                width: layout.sc(PERF_BEAT_LOOP_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(5.0),
        );

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
            Some(if loop_in_led { COL_YELLOW } else { COL_SILVER }),
            Some(COL_BTN_TEXT),
            "",
            None,
            None,
            layout.sc(PERF_V_LOOP_LABEL),
            None,
            Some(loop_border),
            "loop_in",
            Btn::LoopIn,
        );
        app.circle_btn(
            ui,
            layout,
            beat_out,
            layout.sc(PERF_LOOP_SIZE_R),
            Some(if loop_out_led { COL_YELLOW } else { COL_SILVER }),
            Some(COL_BTN_TEXT),
            "",
            None,
            None,
            layout.sc(PERF_V_LOOP_LABEL),
            None,
            Some(loop_border),
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
                COL_YELLOW
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
            0.555 - PERF_BEAT_LOOP_U_OFF_FRAC,
            PERF_V_BEAT_LOOP_LABEL,
        );
        let beat_loop_2x = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.555 + PERF_BEAT_LOOP_U_OFF_FRAC,
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
                COL_YELLOW
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
                COL_YELLOW
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

        let cx_left_ref =
            PERF_TRANSPORT_COL_REF.left() + PERF_TRANSPORT_COL_REF.width() * (0.555 - 0.25);
        let cx_right_ref =
            PERF_TRANSPORT_COL_REF.left() + PERF_TRANSPORT_COL_REF.width() * (0.555 + 0.25);

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

        let beat_jump_prev_leds = app.mosi().led_bit(mosi_frame::LED_BEAT_JUMP_PREV);
        let beat_jump_next_leds = app.mosi().led_bit(mosi_frame::LED_BEAT_JUMP_NEXT);

        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            beat_jump_left,
            "◀",
            layout.sc(PERF_BEAT_JUMP_FONT_SIZE),
            Some(if beat_jump_prev_leds {
                COL_WHITE
            } else {
                COL_SILVER
            }),
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
            Some(if beat_jump_next_leds {
                COL_WHITE
            } else {
                COL_SILVER
            }),
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
            0.5 - PERF_PAIR_U_OFF_FRAC,
            PERF_V_TRACK_SEARCH_CTRLS,
        );
        let track_right = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.5 + PERF_PAIR_U_OFF_FRAC,
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

        let search_leds = app.mosi().led_bit(mosi_frame::LED_TRACK_SEARCH);

        app.circle_btn(
            ui,
            layout,
            track_left,
            layout.sc(PERF_DOUBLE_CIRCLE_BTN_R),
            None,
            None,
            "ǀ◀◀",
            Some(if search_leds { COL_AMBER } else { COL_BTN_TEXT }),
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
            Some(if search_leds { COL_AMBER } else { COL_BTN_TEXT }),
            None,
            layout.sc(PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE),
            None,
            Some(search_border),
            "trk_next",
            Btn::TrackNext,
        );

        let search_left = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.5 - PERF_PAIR_U_OFF_FRAC,
            PERF_V_SEARCH_CTRLS,
        );
        let search_right = layout.sp_in_rect(
            PERF_TRANSPORT_COL_REF,
            0.5 + PERF_PAIR_U_OFF_FRAC,
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
            Some(if search_leds { COL_AMBER } else { COL_BTN_TEXT }),
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
            Some(if search_leds { COL_AMBER } else { COL_BTN_TEXT }),
            None,
            layout.sc(PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE),
            None,
            Some(search_border),
            "src_next",
            Btn::SearchNext,
        );
    }

    {
        let cue_c = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.5, PERF_V_CUE);
        let cue_led = app.mosi().led_bit(mosi_frame::LED_CUE);
        let cue_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_LARGE_BTN_STROKE_INNER),
                color: if cue_led { COL_BTN_CUE } else { COL_BTN },
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
        let play_c = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, 0.5, PERF_V_PLAY);
        let play_led = app.mosi().led_bit(mosi_frame::LED_PLAY);
        let play_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(PERF_LARGE_BTN_STROKE_INNER),
                color: if play_led { COL_BTN_PLAY } else { COL_BTN },
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

/// `a` toward `b` by `t` (0..1), per sRGB channel.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

/// A soft elliptical spot of `col` at `centre`, opaque there and fading to
/// nothing at `radii` with a quadratic falloff.
fn radial_glow(p: &egui::Painter, centre: Pos2, radii: Vec2, col: Color32) {
    const RINGS: usize = 8;
    const SEGMENTS: usize = 40;
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, col);
    for ring in 1..=RINGS {
        let t = ring as f32 / RINGS as f32;
        let a = (1.0 - t) * (1.0 - t);
        let c = Color32::from_rgba_premultiplied(
            (col.r() as f32 * a) as u8,
            (col.g() as f32 * a) as u8,
            (col.b() as f32 * a) as u8,
            (255.0 * a) as u8,
        );
        for k in 0..SEGMENTS {
            let th = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            mesh.colored_vertex(
                centre + Vec2::new(radii.x * t * th.cos(), radii.y * t * th.sin()),
                c,
            );
        }
    }
    let at = |ring: usize, k: usize| (1 + (ring - 1) * SEGMENTS + k % SEGMENTS) as u32;
    for k in 0..SEGMENTS {
        mesh.add_triangle(0, at(1, k), at(1, k + 1));
    }
    for ring in 1..RINGS {
        for k in 0..SEGMENTS {
            mesh.add_triangle(at(ring, k), at(ring + 1, k), at(ring + 1, k + 1));
            mesh.add_triangle(at(ring, k), at(ring + 1, k + 1), at(ring, k + 1));
        }
    }
    p.add(egui::Shape::mesh(mesh));
}
