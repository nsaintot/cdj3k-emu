//! TEMPO range, MASTER TEMPO, the tempo slider and TEMPO RESET.

use cdj3k_emu_panel::{Btn, Lamp};
use egui::{Pos2, Rect, Sense, Stroke, Vec2};

use super::{CdjApp, UiScale, MODES_COL_REF};
use crate::app::ui::{
    DoubleBorderSpec, StrokeSpec, COL_BLACK, COL_BTN, COL_BTN_TEXT, COL_DARK, COL_GREEN, COL_RED,
    COL_SILVER, COL_WHITE,
};

/// Centre of the knob's ring, which the drawing prints at ref y 2612.
pub(super) const TEMPO_RANGE_V: f32 = 0.571627;
pub(super) const TEMPO_RANGE_LABEL_V: f32 = 0.546;
pub(super) const TEMPO_RANGE_SUBLABEL_V: f32 = 0.554142;
pub(super) const TEMPO_RANGE_LABEL_FONT_SIZE: f32 = 32.0;
pub(super) const TEMPO_RANGE_SUBLABEL_FONT_SIZE: f32 = 30.0;
pub(super) const TEMPO_RANGE_BTN_R: f32 = 28.0;
pub(super) const TEMPO_RANGE_INNER_STROKE: f32 = 4.0;
pub(super) const TEMPO_RANGE_OUTER_STROKE: f32 = 8.0;

/// Centre of the knob's ring, which the drawing prints at ref y 2841.
pub(super) const MASTER_TEMPO_V: f32 = 0.62777;
pub(super) const MASTER_TEMPO_LABEL_V: f32 = 0.605456;
pub(super) const MASTER_TEMPO_LABEL_FONT_SIZE: f32 = 32.0;
pub(super) const MASTER_TEMPO_BTN_R: f32 = 28.0;
pub(super) const MASTER_TEMPO_BTN_FONT_SIZE: f32 = 100.0;
pub(super) const MASTER_TEMPO_INNER_STROKE: f32 = 4.0;
pub(super) const MASTER_TEMPO_OUTER_STROKE: f32 = 8.0;

// ── Tempo slider ──────────────────────────────────────────────────────────────
/// Vertical range within MODES_COL_REF, from 0 at the top to 1 at the bottom. The drawing
/// prints the scale's first and last segment at ref y 3092 and 4048; with the
/// travel on those, [`TEMPO_BACKDROP_PAD_Y`] lands the backdrop on 2952..4188,
/// which is exactly where the drawing puts the slider's surround.
pub(super) const TEMPO_SLIDER_V_TOP: f32 = 0.68922;
pub(super) const TEMPO_SLIDER_V_BOT: f32 = 0.92353;
/// Centre of the TEMPO wordmark, which the drawing prints at ref y 4114.
pub(super) const TEMPO_SLIDER_LABEL_V: f32 = 0.93971;
pub(super) const TEMPO_SLIDER_LABEL_FONT_SIZE: f32 = 34.0;

/// Number of tick positions: the − sign, a gap, 22 dots, a gap, the 0, a gap,
/// 22 dots, a gap and the + sign. Each gap takes one position.
pub(super) const TEMPO_TICK_COUNT: usize = 51;

/// The horizontal position as a fraction of the MODES column, from 0 at
/// `col.left()` to 1 at `col.right()`.
pub(super) const TEMPO_TICK_X_U: f32 = 0.225;

/// Bounds of the slider content (tick marks, segments, knob and track bar) as
/// fractions of the column width.
pub(super) const TEMPO_SLIDER_LEFT_U: f32 = 0.285;
pub(super) const TEMPO_SLIDER_RIGHT_U: f32 = 0.725;

/// How far the backdrop extends beyond the slider content on each side, in
/// reference units.
pub(super) const TEMPO_BACKDROP_PAD_X: f32 = 5.0;
pub(super) const TEMPO_BACKDROP_PAD_Y: f32 = 140.0;

pub(super) const TEMPO_BACKDROP_ROUNDING: f32 = 36.0;
pub(super) const TEMPO_BACKDROP_INNER_STROKE: f32 = 5.0;
pub(super) const TEMPO_BACKDROP_GAP: f32 = 0.0;
pub(super) const TEMPO_BACKDROP_OUTER_STROKE: f32 = 3.0;

/// Segment width as a fraction of the backdrop inner width.
pub(super) const TEMPO_SEG_NORMAL_W: f32 = 0.72;
pub(super) const TEMPO_SEG_MAJOR_W: f32 = 0.78;
pub(super) const TEMPO_SEG_NORMAL_STROKE: f32 = 2.0;
pub(super) const TEMPO_SEG_MAJOR_STROKE: f32 = 5.0;

/// Width and gap of the two track bars, which mark the range of the physical
/// potentiometer.
pub(super) const TEMPO_TRACK_BAR_W: f32 = 5.0;
pub(super) const TEMPO_TRACK_BAR_GAP: f32 = 12.0;
/// The track bars' inset from the backdrop's top and bottom, in reference
/// units.
pub(super) const TEMPO_TRACK_BAR_INSET: f32 = 110.0;

/// A dragged value within this distance of 0.5 snaps to 0.5.
pub(super) const TEMPO_CENTER_SNAP_ZONE: f32 = 0.02;

/// Knob dimensions in reference units.
pub(super) const TEMPO_KNOB_H: f32 = 200.0;
pub(super) const TEMPO_KNOB_W: f32 = 180.0;
pub(super) const TEMPO_KNOB_INNER_W_FRAC: f32 = 1.0;
pub(super) const TEMPO_KNOB_INNER_H_FRAC: f32 = 0.50;
pub(super) const TEMPO_KNOB_ROUNDING: f32 = 2.0;
pub(super) const TEMPO_KNOB_LINE_STROKE: f32 = 10.0;
pub(super) const TEMPO_KNOB_LINE_W_FRAC: f32 = 0.80;

pub(super) const TEMPO_TICK_CHAR_FONT_SIZE: f32 = 42.0;
pub(super) const TEMPO_TICK_DOT_R: f32 = 2.0;

// ── Tempo reset button ────────────────────────────────────────────────────────
/// Horizontal center of the TEMPO RESET button as a fraction of the MODES column width.
/// The value puts the center 39 ref units left of the column's left edge.
pub(super) const TEMPO_RESET_BTN_X_U: f32 = -0.075;
pub(super) const TEMPO_RESET_BTN_R: f32 = 28.0;
pub(super) const TEMPO_RESET_BTN_INNER_STROKE: f32 = 10.0;
pub(super) const TEMPO_RESET_BTN_OUTER_STROKE: f32 = 5.0;
pub(super) const TEMPO_RESET_BTN_BORDER_GAP: f32 = 27.0;
pub(super) const TEMPO_RESET_LED_GAP_L: f32 = 60.0;
pub(super) const TEMPO_RESET_LED_GAP_R: f32 = 20.0;
pub(super) const TEMPO_RESET_LED_STROKE: f32 = 12.0;
pub(super) const TEMPO_RESET_LABEL_FONT_SIZE: f32 = 28.0;
pub(super) const TEMPO_RESET_LABEL_GAP: f32 = 50.0;

/// Returns the slider's content area (tick and segment positions, knob, drag
/// mapping) and its backdrop, which is the content area padded on all sides.
/// Both are in screen space.
pub(super) fn tempo_slider_rects(layout: &UiScale) -> (Rect, Rect) {
    let col = MODES_COL_REF;
    let col_w = col.width();
    let col_h = col.height();

    // Slider content bounds in reference space.
    let slider_top = col.top() + col_h * TEMPO_SLIDER_V_TOP;
    let slider_bot = col.top() + col_h * TEMPO_SLIDER_V_BOT;
    let slider_left = col.left() + col_w * TEMPO_SLIDER_LEFT_U;
    let slider_right = col.left() + col_w * TEMPO_SLIDER_RIGHT_U;

    let slider = Rect::from_min_max(
        layout.sp(slider_left, slider_top),
        layout.sp(slider_right, slider_bot),
    );

    let backdrop = Rect::from_min_max(
        layout.sp(
            slider_left - TEMPO_BACKDROP_PAD_X,
            slider_top - TEMPO_BACKDROP_PAD_Y,
        ),
        layout.sp(
            slider_right + TEMPO_BACKDROP_PAD_X,
            slider_bot + TEMPO_BACKDROP_PAD_Y,
        ),
    );
    (slider, backdrop)
}

pub(super) fn draw_tempo_range(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let col = MODES_COL_REF;
    let btn_center = layout.sp_in_rect(col, 0.5, TEMPO_RANGE_V);
    let mt_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(TEMPO_RANGE_INNER_STROKE),
            color: COL_DARK,
        },
        StrokeSpec {
            width: layout.sc(TEMPO_RANGE_OUTER_STROKE),
            color: COL_SILVER,
        },
        layout.sc(5.0),
    );

    app.circle_btn(
        ui,
        layout,
        btn_center,
        layout.sc(TEMPO_RANGE_BTN_R),
        Some(COL_BLACK),
        None,
        "",
        None,
        Some(COL_RED),
        layout.sc(TEMPO_RANGE_LABEL_FONT_SIZE),
        None,
        Some(mt_border),
        "tempo",
        Btn::TempoRange,
    );
}

pub(super) fn draw_master_tempo(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let col = MODES_COL_REF;
    let btn_center = layout.sp_in_rect(col, 0.5, MASTER_TEMPO_V);
    let mt_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(MASTER_TEMPO_INNER_STROKE),
            color: COL_BTN,
        },
        StrokeSpec {
            width: layout.sc(MASTER_TEMPO_OUTER_STROKE),
            color: COL_SILVER,
        },
        layout.sc(5.0),
    );

    let master_tempo_led = app.mosi().lit(Lamp::MasterTempo);

    app.circle_btn(
        ui,
        layout,
        btn_center,
        layout.sc(MASTER_TEMPO_BTN_R),
        Some(COL_BLACK),
        None,
        "•",
        Some(if master_tempo_led {
            COL_RED
        } else {
            COL_BTN_TEXT
        }),
        None,
        layout.sc(MASTER_TEMPO_BTN_FONT_SIZE),
        None,
        Some(mt_border),
        "master_tempo",
        Btn::MasterTempo,
    );
}

/// The fader: its knob, the drag that moves it, and TEMPO RESET beside it.
pub(super) fn draw_tempo_slider(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
) {
    let (slider, backdrop) = tempo_slider_rects(layout);
    paint_tempo_knob(p, layout, slider, app.tempo);
    tempo_drag(app, ui, slider, backdrop);
    draw_tempo_reset(app, ui, p, layout, slider);
}

/// Paints the knob over the cached backdrop and segments, sized relative to the
/// slider content area.
fn paint_tempo_knob(p: &egui::Painter, layout: &UiScale, slider: Rect, tempo: f32) {
    let track_cx = slider.center().x;
    let knob_cy = slider.top() + slider.height() * tempo;
    let knob_h = layout.sc(TEMPO_KNOB_H);
    let knob_outer_w = layout.sc(TEMPO_KNOB_W);
    let knob_inner_w = layout.sc(TEMPO_KNOB_W) * TEMPO_KNOB_INNER_W_FRAC;
    let knob_inner_h = knob_h * TEMPO_KNOB_INNER_H_FRAC;
    let knob_rounding = layout.sc(TEMPO_KNOB_ROUNDING);

    let knob_outer = Rect::from_center_size(
        Pos2::new(track_cx, knob_cy),
        Vec2::new(knob_outer_w, knob_h),
    );
    let knob_inner = Rect::from_center_size(
        Pos2::new(track_cx, knob_cy),
        Vec2::new(knob_inner_w, knob_inner_h),
    );

    p.rect_filled(knob_outer, knob_rounding, COL_SILVER);
    p.rect_stroke(
        knob_outer,
        knob_rounding,
        Stroke::new(layout.sc(2.0), COL_SILVER),
        egui::StrokeKind::Middle,
    );
    p.rect_filled(knob_inner, knob_rounding * 0.5, COL_BTN);
    // The horizontal line across the knob's center.
    let line_half_w = slider.width() * TEMPO_KNOB_LINE_W_FRAC * 0.5;
    p.line_segment(
        [
            Pos2::new(track_cx - line_half_w, knob_cy),
            Pos2::new(track_cx + line_half_w, knob_cy),
        ],
        Stroke::new(layout.sc(TEMPO_KNOB_LINE_STROKE), COL_WHITE),
    );
}

/// Handles dragging. The backdrop is the hit area, and the tempo maps to the
/// range of the slider content.
fn tempo_drag(app: &mut CdjApp, ui: &mut egui::Ui, slider: Rect, backdrop: Rect) {
    let drag = ui.interact(backdrop, ui.id().with("tempo_slider"), Sense::drag());
    if drag.dragged() {
        if let Some(ptr) = drag.interact_pointer_pos() {
            let t = ((ptr.y - slider.top()) / slider.height()).clamp(0.0, 1.0);
            let t = if (t - 0.5).abs() < TEMPO_CENTER_SNAP_ZONE {
                0.5
            } else {
                t
            };
            let prev = app.tempo;
            app.tempo = t;
            if (app.tempo - prev).abs() > 1e-4 {
                app.inject_tempo();
            }
        }
    }
}

/// Draws the TEMPO RESET button, level with the "0" tick at the center of the
/// slider, and its LED.
fn draw_tempo_reset(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
    slider: Rect,
) {
    let col = MODES_COL_REF;
    let col_w = col.width();
    let reset_cx = layout
        .sp(col.left() + col_w * TEMPO_RESET_BTN_X_U, col.top())
        .x;
    let zero_y = slider.top() + slider.height() * 0.5;
    let reset_r = layout.sc(TEMPO_RESET_BTN_R);

    let reset_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(TEMPO_RESET_BTN_INNER_STROKE),
            color: COL_SILVER,
        },
        StrokeSpec {
            width: layout.sc(TEMPO_RESET_BTN_OUTER_STROKE),
            color: COL_SILVER,
        },
        layout.sc(TEMPO_RESET_BTN_BORDER_GAP),
    );

    app.circle_btn(
        ui,
        layout,
        Pos2::new(reset_cx, zero_y),
        reset_r,
        Some(COL_BLACK),
        None,
        "",
        None,
        None,
        0.0,
        None,
        Some(reset_border),
        "tempo_reset",
        Btn::TempoReset,
    );

    // Draw the LED between the button's right edge and the tick column, level
    // with the 0 tick.
    let tick_x_screen = layout.sp(col.left() + col_w * TEMPO_TICK_X_U, col.top()).x;
    let led_x0 = reset_cx + reset_r + layout.sc(TEMPO_RESET_LED_GAP_L);
    let led_x1 = tick_x_screen - layout.sc(TEMPO_RESET_LED_GAP_R);
    if led_x1 > led_x0 {
        let tempo_reset_led = app.mosi().lit(Lamp::TempoReset);

        p.line_segment(
            [Pos2::new(led_x0, zero_y), Pos2::new(led_x1, zero_y)],
            Stroke::new(
                layout.sc(TEMPO_RESET_LED_STROKE),
                if tempo_reset_led {
                    COL_GREEN
                } else {
                    COL_SILVER
                },
            ),
        );
    }
}
