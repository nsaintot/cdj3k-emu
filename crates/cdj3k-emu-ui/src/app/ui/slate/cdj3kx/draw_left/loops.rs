//! The loop block: IN/CUE, OUT and RELOOP/EXIT, BEAT LOOP and BEAT JUMP.

use cdj3k_emu_panel::{Btn, Lamp};
use egui::FontFamily;

use super::super::{
    ButtonType, CdjApp, UiScale, COL_BLACK, COL_BTN_HOT, COL_BTN_OUTLINED_YELLOW, COL_BTN_TEXT,
    COL_DARK, COL_SILVER,
};
use super::PERF_TRANSPORT_COL_REF;
use crate::app::ui::{DoubleBorderSpec, StrokeSpec, COL_AMBER};

pub(super) const PERF_LOOP_U_MID: f32 = 0.568;
pub(super) const PERF_BEAT_LOOP_U_MID: f32 = 0.569;

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

pub(super) const PERF_BEAT_LOOP_LINE_THICKNESS: f32 = 4.0;
pub(super) const PERF_BEAT_LOOP_LINE_LENGTH: f32 = 112.0;
pub(super) const PERF_BEAT_LOOP_PAIR_R: f32 = 50.0;
pub(super) const PERF_BEAT_LOOP_INNER_STROKE: f32 = 8.0;
pub(super) const PERF_BEAT_LOOP_OUTER_STROKE: f32 = 4.0;
/// The stroke width of the IN/CUE and OUT lamp ring, which straddles the black
/// face's edge. The gap closes by the same amount the ring widens, so the
/// button's outer edge stays put. The ring is narrower than the CUE and PLAY
/// rims, because it is the whole lit surface and the bloom scales with it, and
/// because the labels sit close above these two buttons.
pub(super) const PERF_LOOP_LAMP_STROKE: f32 = 10.0;
pub(super) const PERF_LOOP_LAMP_GAP: f32 = 5.0;
pub(super) const PERF_BEAT_LOOP_U_OFF_FRAC: f32 = 0.241;
pub(super) const PERF_BEAT_LOOP_FONT_SIZE: f32 = 48.0;
pub(super) const PERF_BEAT_LOOP_SUBLABEL_W: f32 = 70.0;
pub(super) const PERF_BEAT_LOOP_SUBLABEL_H: f32 = 40.0;
pub(super) const PERF_BEAT_LOOP_SUBLABEL_ROUNDING: f32 = 8.0;

/// Vertical placement within [`PERF_TRANSPORT_COL_REF`], from 0 at the top to 1 at the bottom.
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

pub(super) const PERF_BEAT_JUMP_BTN_ASPECT_RATIO: f32 = 1.2;
pub(super) const PERF_BEAT_JUMP_BTN_SIZE: f32 = 130.0;
pub(super) const PERF_BEAT_JUMP_FONT_SIZE: f32 = 34.0;
pub(super) const PERF_BEAT_JUMP_BTN_INNER_STROKE: f32 = 1.0;
pub(super) const PERF_BEAT_JUMP_BTN_OUTER_STROKE: f32 = 4.0;

/// IN/CUE, OUT and RELOOP/EXIT.
pub(super) fn draw_loop(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
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

    let loop_in_led = app.mosi().lit(Lamp::LoopIn);
    let loop_out_led = app.mosi().lit(Lamp::LoopOut);

    for (center, lit, id, btn) in [
        (beat_in, loop_in_led, "loop_in", Btn::LoopIn),
        (beat_out, loop_out_led, "loop_out", Btn::LoopOut),
    ] {
        app.circle_btn(
            ui,
            layout,
            center,
            layout.sc(PERF_LOOP_SIZE_R),
            Some(COL_BLACK),
            Some(COL_BTN_HOT),
            "",
            None,
            None,
            layout.sc(PERF_V_LOOP_LABEL),
            None,
            Some(loop_border(lit)),
            id,
            btn,
        );
    }
    draw_reloop(app, ui, layout);
}

/// RELOOP/EXIT.
fn draw_reloop(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let loop_reloop = layout.sp_in_rect(
        PERF_TRANSPORT_COL_REF,
        (PERF_LOOP_U_MID + PERF_LOOP_U_OFF_FRAC) * 1.72,
        PERF_V_LOOP_LINE,
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

    let loop_reloop_led = app.mosi().lit(Lamp::Reloop);
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

/// Draws the two BEAT LOOP buttons, labeled 4 and 8 (`Btn::BeatloopHalf` and
/// `Btn::Beatloop2x`).
pub(super) fn draw_beat_loop(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
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
    let beat_loop_4_led = app.mosi().lit(Lamp::BeatJump4);
    let beat_loop_8_led = app.mosi().lit(Lamp::BeatJump8);

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

    for (center, label, lit, id, btn) in [
        (
            beat_loop_half,
            "4",
            beat_loop_4_led,
            "beat_loop_half",
            Btn::BeatloopHalf,
        ),
        (
            beat_loop_2x,
            "8",
            beat_loop_8_led,
            "beat_loop_2x",
            Btn::Beatloop2x,
        ),
    ] {
        app.circle_btn(
            ui,
            layout,
            center,
            layout.sc(PERF_BEAT_LOOP_PAIR_R),
            Some(COL_BLACK),
            None,
            label,
            Some(if lit { COL_AMBER } else { COL_BTN_TEXT }),
            Some(COL_BTN_OUTLINED_YELLOW),
            layout.sc(PERF_BEAT_LOOP_FONT_SIZE),
            None,
            Some(beat_loop_border),
            id,
            btn,
        );
    }
}

/// BEAT JUMP ◀ and ▶.
pub(super) fn draw_beat_jump(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    // Lay the buttons out in reference space: `sr` and `ar2rect` take reference
    // coordinates, so never pass them a screen `Pos2` from `sp_in_rect`.
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

    for (rect, label, id, btn) in [
        (beat_jump_left, "◀", "cueloop_prev", Btn::BeatjumpPrev),
        (beat_jump_right, "▶", "cueloop_next", Btn::BeatjumpNext),
    ] {
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            rect,
            label,
            layout.sc(PERF_BEAT_JUMP_FONT_SIZE),
            Some(COL_SILVER),
            Some(COL_BLACK),
            None,
            None,
            None,
            Some(beat_jump_border),
            FontFamily::Proportional,
            id,
            btn,
        );
    }
}
