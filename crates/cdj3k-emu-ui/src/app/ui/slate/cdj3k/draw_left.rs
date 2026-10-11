use crate::app::ui::draw_direction::{self, DirectionPlacement};
use crate::app::ui::{DoubleBorderSpec, StrokeSpec, COL_WHITE};
use cdj3k_emu_panel::Btn;
use cdj3k_emu_panel::{mosi_frame, Lamp};
use egui::{Color32, FontFamily, Pos2, Rect, Vec2};

use super::{
    layout, ButtonType, CdjApp, UiScale, COL_BLACK, COL_BTN, COL_DARK, COL_DARK_RED, COL_RED,
    COL_SILVER,
};

mod loops;
mod media;
mod statics;
mod transport;

use loops::*;
use media::*;
use transport::*;

/// The transport column on the left, in reference space; its x range comes
/// from [`layout`].
pub(super) const PERF_TRANSPORT_COL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::TRANSPORT_REF_LEFT, layout::TRANSPORT_REF_TOP),
    Pos2::new(layout::TRANSPORT_REF_RIGHT, layout::TRANSPORT_REF_BOT),
);

/// Vertical fraction within [`PERF_TRANSPORT_COL_REF`] for the "DIRECTION" label.
pub(super) const PERF_V_DIRECTION_LABEL: f32 = 0.55;

/// Where this panel places the direction switch.
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

pub(super) const PERF_TIME_MODE_AUTO_CUE_SIZE_R: f32 = 30.0;
pub(super) const PERF_TIME_MODE_AUTO_CUE_LABEL_FONT_SIZE: f32 = 26.0;

pub(super) const PERF_V_TIME_MODE_AUTO_CUE_LABEL: f32 = 0.205;
pub(super) const PERF_V_TIME_MODE_AUTO_CUE: f32 = 0.207;

pub(super) const PERF_SLIP_QUANTIZE_BTN_SIZE: f32 = 130.0;
pub(super) const PERF_SLIP_BTN_FONT_SIZE: f32 = 32.0;
pub(super) const PERF_QUANTIZE_BTN_FONT_SIZE: f32 = 30.0;
pub(super) const PERF_SLIP_QUANTIZE_BTN_U_OFF_FRAC: f32 = 0.19;
pub(super) const PERF_SLIP_QUANTIZE_BTN_ASPECT_RATIO: f32 = 2.2;
pub(super) const PERF_SLIP_QUANTIZE_BTN_INNER_STROKE: f32 = 1.0;
pub(super) const PERF_SLIP_QUANTIZE_BTN_OUTER_STROKE: f32 = 2.0;
pub(super) const PERF_SLIP_FILL_PAD_H: f32 = 15.0;
pub(super) const PERF_SLIP_FILL_PAD_V: f32 = 0.0;

pub(super) const PERF_V_SLIP_QUANTIZE_BTNS: f32 = 0.254;

pub(super) const PERF_LABEL_FONT_SIZE: f32 = 30.0;

// ── CALL/DELETE button (same Y as hot cue row) ────────────────────────────────
/// The vertical fraction within PERF_TRANSPORT_COL_REF that lines up with the
/// hot cue row's center. It is (MID_CENTRAL_REF_TOP + MID_CENTRAL_REF_SIZE_H *
/// HOT_CUE_BTN_V_CENTER - TRANSPORT_REF_TOP) / TRANSPORT_REF_SIZE_H, which is
/// (1485 + 460 * 0.3 - 280) / 4080 ≈ 0.329.
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
/// The outer container is as tall as a hot cue button, 100 units (180 / 1.8),
/// and 20 units wider.
pub(super) const PERF_CALL_DELETE_OUTER_H: f32 = 100.0;
pub(super) const PERF_CALL_DELETE_OUTER_W: f32 = 200.0;
/// The rounding of the CALL/DELETE outline, which matches the hot cue
/// buttons: 12% of their 100-unit height.
pub(super) const PERF_CALL_DELETE_OUTER_ROUNDING: f32 = 12.0;

pub(super) fn draw_left_section(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
) {
    puffin::profile_function!();
    // ── Static labels and chrome (rebuilt only on resize) ─────────────────
    let ppp = ui.ctx().pixels_per_point();
    let ctx = ui.ctx().clone();
    app.frame_shape_count += app.caches.left_statics.paint(p, layout, ppp, |list| {
        statics::collect_left_statics(list, &ctx, layout);
    });

    draw_usb_mount(app, ui, p, layout);
    draw_sd_slot(app, p, layout);
    draw_usb_stop(app, ui, layout);
    draw_time_mode(app, ui, layout);
    draw_slip_quantize(app, ui, layout);
    draw_call_delete(app, ui, layout);
    draw_loop(app, ui, layout);
    draw_beat_loop(app, ui, layout);
    draw_beat_jump(app, ui, layout);
    draw_direction::draw_direction_switch(app, ui, p, layout, DIRECTION_PLACE);
    draw_search(app, ui, layout);
    draw_cue_play(app, ui, layout);
}

fn draw_time_mode(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
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

/// A SLIP / QUANTIZE legend's colour at the lamp's step.
fn step_color(step: mosi_frame::StepLed) -> Color32 {
    match step {
        mosi_frame::StepLed::Full => COL_RED,
        mosi_frame::StepLed::Medium => COL_DARK_RED,
        mosi_frame::StepLed::Off => COL_SILVER,
    }
}

fn draw_slip_quantize(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
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

    let slip_step = app.mosi().step(Lamp::Slip);
    let quantize_step = app.mosi().step(Lamp::Quantize);

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
        Some(step_color(slip_step)),
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
        Some(step_color(quantize_step)),
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

/// Draws the CALL/DELETE button, aligned vertically with the hot cue row in
/// the mid-central panel. Its static chrome (the outer container, labels and
/// segment) is drawn in statics.rs.
fn draw_call_delete(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let col = PERF_TRANSPORT_COL_REF;
    let cy_ref = col.top() + col.height() * PERF_V_CALL_DELETE;
    let btn_size = PERF_CALL_DELETE_BTN_SIZE;
    let btn_cx_ref = col.left() + col.width() * PERF_CALL_DELETE_BTN_U;

    // The inner button is a black square with a double border like the hot
    // cue buttons'.
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
        None,            // No accent: it would not show on the black fill.
        Some(COL_BLACK), // The background fill.
        None,            // None keeps the default pressed fill, COL_BTN_HOT.
        None,
        None,
        Some(call_delete_border),
        FontFamily::Proportional,
        "call_delete",
        Btn::CallDelete,
    );
}
