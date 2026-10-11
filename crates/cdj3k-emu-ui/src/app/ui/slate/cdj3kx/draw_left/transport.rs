//! TRACK SEARCH, SEARCH, CUE and PLAY/PAUSE, at the foot of the transport column.

use cdj3k_emu_panel::{mosi_frame, Btn, Lamp};
use egui::{Color32, Pos2};

use super::super::{CdjApp, UiScale, COL_BTN, COL_BTN_TEXT, COL_SILVER};
use super::{PERF_PAIR_U_MID, PERF_PAIR_U_OFF_FRAC, PERF_TRANSPORT_COL_REF};
use crate::app::ui::{DoubleBorderSpec, StrokeSpec};

/// CUE and PLAY/PAUSE share the paired rows' midline, not the column's.
pub(super) const PERF_U_CUE: f32 = 0.525;
pub(super) const PERF_U_PLAY: f32 = 0.525;

pub(super) const PERF_DOUBLE_CIRCLE_RING_R: f32 = 92.0;
pub(super) const PERF_DOUBLE_CIRCLE_RING_STROKE: f32 = 4.0;
pub(super) const PERF_DOUBLE_CIRCLE_BTN_R: f32 = 60.0;
pub(super) const PERF_DOUBLE_CIRCLE_BTN_INNER_STROKE: f32 = 8.0;
pub(super) const PERF_DOUBLE_CIRCLE_BTN_OUTER_STROKE: f32 = 4.0;
pub(super) const PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE: f32 = 34.0;

/// Vertical placement within [`PERF_TRANSPORT_COL_REF`], from 0 at the top to 1 at the bottom.
pub(super) const PERF_V_TRACK_SEARCH_LABEL: f32 = 0.644;
pub(super) const PERF_V_TRACK_SEARCH_CTRLS: f32 = 0.672;
pub(super) const PERF_V_SEARCH_LABEL: f32 = 0.707;
pub(super) const PERF_V_SEARCH_CTRLS: f32 = 0.735;
pub(super) const PERF_V_CUE: f32 = 0.826;
pub(super) const PERF_V_PLAY: f32 = 0.934;

pub(super) const PERF_LARGE_BTN_RADIUS: f32 = 150.0;
/// CUE and PLAY are the same physical buttons as the CDJ-3000's, and both
/// canvases carry 0.1033 mm per reference unit, so their rim lamp is the same
/// width on either slate.
pub(super) const PERF_LARGE_BTN_STROKE_INNER: f32 = 20.0;
pub(super) const PERF_LARGE_BTN_STROKE_GAP: f32 = 10.0;
pub(super) const PERF_LARGE_BTN_STROKE_OUTER: f32 = 5.0;

pub(super) const PERF_CUE_FONT_SIZE: f32 = 40.0;

pub(super) const PERF_PLAY_FONT_SIZE: f32 = 38.0;

/// Draws the TRACK SEARCH and SEARCH buttons, two pairs of circle buttons.
pub(super) fn draw_search(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let pair = |v: f32| {
        (
            layout.sp_in_rect(
                PERF_TRANSPORT_COL_REF,
                PERF_PAIR_U_MID - PERF_PAIR_U_OFF_FRAC,
                v,
            ),
            layout.sp_in_rect(
                PERF_TRANSPORT_COL_REF,
                PERF_PAIR_U_MID + PERF_PAIR_U_OFF_FRAC,
                v,
            ),
        )
    };
    let (track_left, track_right) = pair(PERF_V_TRACK_SEARCH_CTRLS);
    let (search_left, search_right) = pair(PERF_V_SEARCH_CTRLS);

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

    for (center, label, id, btn) in [
        (track_left, "ǀ◀◀", "trk_prev", Btn::TrackPrev),
        (track_right, "▶▶ǀ", "trk_next", Btn::TrackNext),
        (search_left, "◀◀", "src_prev", Btn::SearchPrev),
        (search_right, "▶▶", "src_next", Btn::SearchNext),
    ] {
        app.circle_btn(
            ui,
            layout,
            center,
            layout.sc(PERF_DOUBLE_CIRCLE_BTN_R),
            None,
            None,
            label,
            Some(COL_BTN_TEXT),
            None,
            layout.sc(PERF_DOUBLE_CIRCLE_LABEL_FONT_SIZE),
            None,
            Some(search_border),
            id,
            btn,
        );
    }
}

/// Draws CUE and PLAY/PAUSE. On the CDJ-3000X both are RGB lamps: the rim takes its
/// colour from the deck and falls back to COL_BTN when the lamp is off.
pub(super) fn draw_cue_play(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let cue_c = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_CUE, PERF_V_CUE);
    let cue_lamp = app
        .mosi()
        .rgb(Lamp::Cue)
        .and_then(|(r, g, b)| app.mosi().led_color(mosi_frame::LedPart::CueRim, r, g, b));
    let cue_rim = cue_lamp.unwrap_or(COL_BTN);
    large_btn(
        app,
        ui,
        layout,
        cue_c,
        cue_rim,
        "CUE",
        PERF_CUE_FONT_SIZE,
        "cue_large",
        Btn::Cue,
    );

    let play_c = layout.sp_in_rect(PERF_TRANSPORT_COL_REF, PERF_U_PLAY, PERF_V_PLAY);
    let play_lamp = app
        .mosi()
        .rgb(Lamp::Play)
        .and_then(|(r, g, b)| app.mosi().led_color(mosi_frame::LedPart::PlayRim, r, g, b));
    let play_rim = play_lamp.unwrap_or(COL_BTN);
    large_btn(
        app,
        ui,
        layout,
        play_c,
        play_rim,
        "▶ / ⏸",
        PERF_PLAY_FONT_SIZE,
        "play_large",
        Btn::Play,
    );
}

fn large_btn(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    layout: &UiScale,
    center: Pos2,
    rim: Color32,
    label: &str,
    font_size: f32,
    id: &str,
    btn: Btn,
) {
    let border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(PERF_LARGE_BTN_STROKE_INNER),
            color: rim,
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
        center,
        layout.sc(PERF_LARGE_BTN_RADIUS),
        None,
        None,
        label,
        None,
        None,
        layout.sc(font_size),
        None,
        Some(border),
        id,
        btn,
    );
}
