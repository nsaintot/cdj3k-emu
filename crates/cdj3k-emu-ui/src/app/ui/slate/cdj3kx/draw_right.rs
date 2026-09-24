use cdj3k_emu_panel::Btn;
use egui::{FontFamily, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::app::ui::{
    DoubleBorderSpec, StrokeSpec, COL_BLACK, COL_BLUE, COL_BTN, COL_BTN_OUTLINED_YELLOW,
    COL_BTN_TEXT, COL_BTN_WHITE, COL_DARK, COL_GREEN, COL_RED, COL_SILVER, COL_WHITE,
};
use cdj3k_emu_panel::mosi_frame;

use crate::app::ui::draw_vinyl_speed::{self, VinylSpeedPlacement};

use super::{layout, ButtonType, CdjApp, UiScale, LEGEND_TAB_H, LEGEND_TAB_W};

pub(super) mod nav_rotary;
mod statics;

pub(super) const MODES_COL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::MODES_REF_LEFT, layout::MODES_REF_TOP),
    Pos2::new(layout::MODES_REF_RIGHT, layout::MODES_REF_BOT),
);

/// Where this panel puts the VINYL SPEED ADJUST knob.
pub(super) const VINYL_SPEED_PLACE: VinylSpeedPlacement = VinylSpeedPlacement {
    col: MODES_COL_REF,
    center_u: 0.506,
    center_v: 0.329,
};

// ── JOG MODE combo ────────────────────────────────────────────────────────────
/// Centre of the combo housing within the modes column.
pub(super) const JOG_MODE_U: f32 = 0.513;
pub(super) const JOG_MODE_V: f32 = 0.387;
/// Housing frame, on the drawing's line centres.
pub(super) const JOG_MODE_COMBO_W: f32 = 356.0;
pub(super) const JOG_MODE_COMBO_H: f32 = 135.0;
pub(super) const JOG_MODE_BORDER_ROUNDING: f32 = 25.0;
/// JOG MODE button, right of the VINYL / CDJ legends.
pub(super) const JOG_MODE_BTN_OFF_X: f32 = 96.0;
pub(super) const JOG_MODE_BTN_W: f32 = 140.0;
pub(super) const JOG_MODE_BTN_H: f32 = 96.0;
pub(super) const JOG_MODE_FONT_SIZE: f32 = 32.5;
pub(super) const JOG_MODE_INNER_STROKE: f32 = 1.0;
pub(super) const JOG_MODE_OUTER_STROKE: f32 = 2.0;
/// VINYL and CDJ legends, offset from the housing centre.
pub(super) const JOG_MODE_LABEL_OFF_X: f32 = -79.3;
pub(super) const JOG_MODE_VINYL_OFF_Y: f32 = -32.8;
pub(super) const JOG_MODE_CDJ_OFF_Y: f32 = 28.4;
pub(super) const JOG_MODE_LABEL_FONT_SIZE: f32 = 36.0;

// ── BEAT SYNC / MASTER / KEY SYNC ─────────────────────────────────────────────
/// Centre line the whole sync section shares with the JOG MODE combo above it.
pub(super) const SYNC_U: f32 = 0.513;
pub(super) const SYNC_V: f32 = 0.455;
/// Housing frame, on the drawing's line centres.
pub(super) const SYNC_SECTION_V: f32 = 0.472;
pub(super) const SYNC_SECTION_W: f32 = 356.0;
pub(super) const SYNC_SECTION_H: f32 = 315.0;
pub(super) const SYNC_BORDER_ROUNDING: f32 = 25.0;
/// "INST. DOUBLES": left edge of the legend, and the gap back to the centre of
/// the [`LEGEND_TAB_W`] dash that prefixes it.
pub(super) const SYNC_INST_DOUBLES_U: f32 = 0.269;
pub(super) const SYNC_INST_DOUBLES_TAB_GAP: f32 = 21.0;
pub(super) const SYNC_INST_DOUBLES_V: f32 = 0.425;
pub(super) const SYNC_BTN_WIDTH: f32 = 141.0;
pub(super) const SYNC_BTN_ASPECT: f32 = 1.205;
pub(super) const SYNC_BTN_GAP: f32 = 52.0;
pub(super) const KEY_SYNC_BTN_WIDTH: f32 = 199.0;
pub(super) const KEY_SYNC_BTN_ASPECT_RATIO: f32 = 2.76;
pub(super) const KEY_SYNC_V_OFFSET: f32 = 0.0157;
pub(super) const SYNC_FONT_SIZE: f32 = 28.0;
pub(super) const SYNC_MASTER_BTN_LABEL_NUDGE_Y: f32 = -5.0;
pub(super) const SYNC_MASTER_FONT_SIZE: f32 = 34.0;
pub(super) const SYNC_INNER_STROKE: f32 = 1.0;
pub(super) const SYNC_OUTER_STROKE: f32 = 2.0;
pub(super) const SYNC_BORDER_GAP: f32 = 2.0;
pub(super) const SYNC_INST_DOUBLES_FONT_SIZE: f32 = 32.0;

/// Reference-space boxes of the JOG MODE combo: the housing frame, then the
/// button. Shared with [`statics`] so the frame cannot drift off its button.
pub(super) fn jog_mode_boxes() -> (Rect, Rect) {
    let col = MODES_COL_REF;
    let center = Pos2::new(
        col.left() + col.width() * JOG_MODE_U,
        col.top() + col.height() * JOG_MODE_V,
    );
    (
        Rect::from_center_size(center, Vec2::new(JOG_MODE_COMBO_W, JOG_MODE_COMBO_H)),
        Rect::from_center_size(
            Pos2::new(center.x + JOG_MODE_BTN_OFF_X, center.y),
            Vec2::new(JOG_MODE_BTN_W, JOG_MODE_BTN_H),
        ),
    )
}

/// Reference-space boxes of the sync section: the housing frame, then BEAT
/// SYNC, MASTER and KEY SYNC. Shared with [`statics`].
pub(super) fn sync_boxes() -> (Rect, Rect, Rect, Rect) {
    let col = MODES_COL_REF;
    let center_x = col.left() + col.width() * SYNC_U;
    let cy_ref = col.top() + col.height() * SYNC_V;
    let btn_h = SYNC_BTN_WIDTH / SYNC_BTN_ASPECT;
    let btn_size = Vec2::new(SYNC_BTN_WIDTH, btn_h);
    let ks_h = KEY_SYNC_BTN_WIDTH / KEY_SYNC_BTN_ASPECT_RATIO;
    (
        Rect::from_center_size(
            Pos2::new(center_x, col.top() + col.height() * SYNC_SECTION_V),
            Vec2::new(SYNC_SECTION_W, SYNC_SECTION_H),
        ),
        Rect::from_min_size(
            Pos2::new(
                center_x - SYNC_BTN_GAP * 0.5 - SYNC_BTN_WIDTH,
                cy_ref - btn_h * 0.5,
            ),
            btn_size,
        ),
        Rect::from_min_size(
            Pos2::new(center_x + SYNC_BTN_GAP * 0.5, cy_ref - btn_h * 0.5),
            btn_size,
        ),
        Rect::from_min_size(
            Pos2::new(
                center_x - KEY_SYNC_BTN_WIDTH * 0.5,
                cy_ref + btn_h * 0.5 + col.height() * KEY_SYNC_V_OFFSET,
            ),
            Vec2::new(KEY_SYNC_BTN_WIDTH, ks_h),
        ),
    )
}

/// A reference-space box in screen coordinates.
#[inline]
fn screen(layout: &UiScale, r: Rect) -> Rect {
    layout.sr(r.left(), r.top(), r.width(), r.height())
}

/// TEMPO range and MASTER TEMPO share a centre line.
pub(super) const TEMPO_KNOB_U: f32 = 0.510;
pub(super) const TEMPO_RANGE_V: f32 = 0.572;
pub(super) const TEMPO_RANGE_LABEL_V: f32 = 0.546;
pub(super) const TEMPO_RANGE_SUBLABEL_V: f32 = 0.555;
pub(super) const TEMPO_RANGE_LABEL_FONT_SIZE: f32 = 32.0;
pub(super) const TEMPO_RANGE_SUBLABEL_FONT_SIZE: f32 = 30.0;
pub(super) const TEMPO_RANGE_BTN_R: f32 = 28.0;
pub(super) const TEMPO_RANGE_INNER_STROKE: f32 = 4.0;
pub(super) const TEMPO_RANGE_OUTER_STROKE: f32 = 8.0;

pub(super) const MASTER_TEMPO_V: f32 = 0.628;
pub(super) const MASTER_TEMPO_LABEL_V: f32 = 0.607;
pub(super) const MASTER_TEMPO_LABEL_FONT_SIZE: f32 = 32.0;
pub(super) const MASTER_TEMPO_BTN_R: f32 = 28.0;
pub(super) const MASTER_TEMPO_BTN_FONT_SIZE: f32 = 100.0;
pub(super) const MASTER_TEMPO_INNER_STROKE: f32 = 4.0;
pub(super) const MASTER_TEMPO_OUTER_STROKE: f32 = 8.0;

// ── Tempo slider ──────────────────────────────────────────────────────────────
/// Vertical range within MODES_COL_REF (0 = top, 1 = bottom).
pub(super) const TEMPO_SLIDER_V_TOP: f32 = 0.692;
pub(super) const TEMPO_SLIDER_V_BOT: f32 = 0.928;

/// Number of tick positions: − [gap] 22 dots [gap] 0 [gap] 22 dots [gap] +
pub(super) const TEMPO_TICK_COUNT: usize = 51;

/// Horizontal fraction of the MODES column (0 = col.left(), 1 = col.right()).
pub(super) const TEMPO_TICK_X_U: f32 = 0.225;

/// Slider *content* bounds (tick marks, segments, knob, track bar) as a fraction of col width.
pub(super) const TEMPO_SLIDER_LEFT_U: f32 = 0.285;
pub(super) const TEMPO_SLIDER_RIGHT_U: f32 = 0.725;

/// Backdrop expands this many reference units beyond the slider content on each side.
pub(super) const TEMPO_BACKDROP_PAD_X: f32 = 5.0;
pub(super) const TEMPO_BACKDROP_PAD_Y: f32 = 140.0;

pub(super) const TEMPO_BACKDROP_ROUNDING: f32 = 36.0;
pub(super) const TEMPO_BACKDROP_INNER_STROKE: f32 = 5.0;
pub(super) const TEMPO_BACKDROP_GAP: f32 = 0.0;
pub(super) const TEMPO_BACKDROP_OUTER_STROKE: f32 = 3.0;

/// Double track bar (physical potentiometer range).
pub(super) const TEMPO_TRACK_BAR_W: f32 = 5.0;
pub(super) const TEMPO_TRACK_BAR_GAP: f32 = 12.0;
/// Inset from backdrop top/bottom where the track bar starts/ends (ref units).
pub(super) const TEMPO_TRACK_BAR_INSET: f32 = 110.0;

/// Snap-to-center zone: if the dragged value is within this distance of 0.5, snap to 0.5.
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
// Center at 43 ref from col.left; outer ring radius ≈ 35 → left margin = 8, fits in 117 available.
pub(super) const TEMPO_RESET_BTN_X_U: f32 = -0.065;
pub(super) const TEMPO_RESET_BTN_R: f32 = 28.0;
pub(super) const TEMPO_RESET_BTN_INNER_STROKE: f32 = 10.0;
pub(super) const TEMPO_RESET_BTN_OUTER_STROKE: f32 = 5.0;
pub(super) const TEMPO_RESET_BTN_BORDER_GAP: f32 = 27.0;
pub(super) const TEMPO_RESET_LED_GAP_L: f32 = 60.0;
pub(super) const TEMPO_RESET_LED_GAP_R: f32 = 20.0;
pub(super) const TEMPO_RESET_LED_STROKE: f32 = 12.0;
pub(super) const TEMPO_RESET_LABEL_FONT_SIZE: f32 = 28.0;
pub(super) const TEMPO_RESET_LABEL_GAP: f32 = 68.0;

// ── Bottom label ──────────────────────────────────────────────────────────
pub(super) const BOTTOM_LABEL_SERIE_V: f32 = 0.974;
/// Centre line of the tempo slider: (TEMPO_SLIDER_LEFT_U + TEMPO_SLIDER_RIGHT_U) / 2.
pub(super) const BOTTOM_LABEL_SERIE_X: f32 = 0.505;
pub(super) const BOTTOM_LABEL_SERIE_FONT_SIZE: f32 = 60.0;

pub(super) fn draw_right_sections(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
) {
    puffin::profile_function!();
    let col = MODES_COL_REF;

    // ── Static labels and chrome (rebuilt only on resize) ─────────────────
    let (ox, oy, scale) = layout.cache_key();
    let ppp = ui.ctx().pixels_per_point();
    let ctx = ui.ctx().clone();
    let static_shapes = app
        .right_statics_cache
        .get_or_build(ox, oy, scale, ppp, |list| {
            statics::collect_right_statics(list, &ctx, layout);
        });
    app.frame_shape_count += static_shapes.len() as u64;
    p.extend(static_shapes.iter().cloned());

    nav_rotary::draw_nav_rotary(app, ui, p, layout);
    draw_vinyl_speed::draw_vinyl_speed_adj(app, ui, p, layout, VINYL_SPEED_PLACE);

    {
        let (combo_ref, jog_ref) = jog_mode_boxes();
        let label_x = combo_ref.center().x + JOG_MODE_LABEL_OFF_X;
        let jog_rect = screen(layout, jog_ref);

        let vinyl_led = app.mosi().led_bit(mosi_frame::LED_JOG_MODE_VINYL);
        let cdj_led = app.mosi().led_bit(mosi_frame::LED_JOG_MODE_CDJ);

        // VINYL label (yellow, illuminated when active)
        p.text(
            layout.sp(label_x, combo_ref.center().y + JOG_MODE_VINYL_OFF_Y),
            egui::Align2::CENTER_CENTER,
            "VINYL",
            FontId::new(
                layout.sc(JOG_MODE_LABEL_FONT_SIZE),
                FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into()),
            ),
            if vinyl_led { COL_BLUE } else { COL_DARK },
        );
        // CDJ label
        p.text(
            layout.sp(label_x, combo_ref.center().y + JOG_MODE_CDJ_OFF_Y),
            egui::Align2::CENTER_CENTER,
            "CDJ",
            FontId::new(
                layout.sc(JOG_MODE_LABEL_FONT_SIZE),
                FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into()),
            ),
            if cdj_led { COL_GREEN } else { COL_DARK },
        );

        // JOG MODE button
        let jog_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(JOG_MODE_INNER_STROKE),
                color: COL_DARK,
            },
            StrokeSpec {
                width: layout.sc(JOG_MODE_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(2.0),
        );
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            jog_rect,
            " JOG\nMODE",
            layout.sc(JOG_MODE_FONT_SIZE),
            None,
            None,
            None,
            None,
            None,
            Some(jog_border),
            FontFamily::Proportional,
            "jog_mode",
            Btn::JogMode,
        );
    }

    {
        // BEAT SYNC (left) and MASTER (right) on one row, KEY SYNC centred below.
        let (_, bs_ref, master_ref, ks_ref) = sync_boxes();
        let bs_rect = screen(layout, bs_ref);
        let master_rect = screen(layout, master_ref);
        let ks_rect = screen(layout, ks_ref);

        let sync_border = DoubleBorderSpec::from_strokes_with_gap(
            StrokeSpec {
                width: layout.sc(SYNC_INNER_STROKE),
                color: COL_DARK,
            },
            StrokeSpec {
                width: layout.sc(SYNC_OUTER_STROKE),
                color: COL_SILVER,
            },
            layout.sc(SYNC_BORDER_GAP),
        );

        let beat_sync_led = app.mosi().led_bit(mosi_frame::LED_BEAT_SYNC);
        let master_led = app.mosi().led_bit(mosi_frame::LED_MASTER);
        let key_sync_led = app.mosi().led_bit(mosi_frame::LED_KEY_SYNC);

        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            bs_rect,
            "BEAT\nSYNC",
            layout.sc(SYNC_FONT_SIZE),
            Some(if beat_sync_led {
                COL_BTN_WHITE
            } else {
                COL_BTN_TEXT
            }),
            None,
            None,
            None,
            None,
            Some(sync_border),
            FontFamily::Proportional,
            "beat_sync",
            Btn::BeatSync,
        );
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            master_rect,
            "MASTER",
            layout.sc(SYNC_MASTER_FONT_SIZE),
            Some(if master_led {
                COL_BTN_OUTLINED_YELLOW
            } else {
                COL_BTN_TEXT
            }),
            None,
            None,
            None,
            Some(Vec2::new(0.0, layout.sc(SYNC_MASTER_BTN_LABEL_NUDGE_Y))),
            Some(sync_border),
            FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_CONDENSED.into()),
            "master",
            Btn::Master,
        );
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            ks_rect,
            "KEY SYNC",
            layout.sc(SYNC_FONT_SIZE),
            Some(if key_sync_led { COL_BLUE } else { COL_BTN_TEXT }),
            None,
            None,
            None,
            None,
            Some(sync_border),
            FontFamily::Proportional,
            "key_sync",
            Btn::KeySync,
        );
    }

    {
        let btn_center = layout.sp_in_rect(col, TEMPO_KNOB_U, TEMPO_RANGE_V);
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

    {
        let btn_center = layout.sp_in_rect(col, TEMPO_KNOB_U, MASTER_TEMPO_V);
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

        let master_tempo_led = app.mosi().led_bit(mosi_frame::LED_MASTER_TEMPO);

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

    {
        let col = MODES_COL_REF;
        let col_w = col.width();
        let col_h = col.height();

        // Slider content bounds in reference space.
        let slider_top = col.top() + col_h * TEMPO_SLIDER_V_TOP;
        let slider_bot = col.top() + col_h * TEMPO_SLIDER_V_BOT;
        let slider_left = col.left() + col_w * TEMPO_SLIDER_LEFT_U;
        let slider_right = col.left() + col_w * TEMPO_SLIDER_RIGHT_U;

        // Slider content area in screen space (tick/segment positions, knob, drag mapping).
        let slider = Rect::from_min_max(
            layout.sp(slider_left, slider_top),
            layout.sp(slider_right, slider_bot),
        );

        // Backdrop = slider content + padding on all sides.
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

        // Knob - drawn on top of cached backdrop/segments, sized relative to slider content area.
        let track_cx = slider.center().x;
        let knob_cy = slider.top() + slider.height() * app.tempo;
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

        // Outer rect (color A).
        p.rect_filled(knob_outer, knob_rounding, COL_SILVER);
        p.rect_stroke(
            knob_outer,
            knob_rounding,
            Stroke::new(layout.sc(2.0), COL_SILVER),
        );
        // Inner rect (color B).
        p.rect_filled(knob_inner, knob_rounding * 0.5, COL_BTN);
        // Center horizontal line.
        let line_half_w = slider.width() * TEMPO_KNOB_LINE_W_FRAC * 0.5;
        p.line_segment(
            [
                Pos2::new(track_cx - line_half_w, knob_cy),
                Pos2::new(track_cx + line_half_w, knob_cy),
            ],
            Stroke::new(layout.sc(TEMPO_KNOB_LINE_STROKE), COL_WHITE),
        );

        // Drag interaction: backdrop is the hit area; tempo maps to slider content range.
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

        // ── Tempo reset button ────────────────────────────────────────────────
        // Vertically aligned with the "0" tick (center of slider).
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

        // LED indicator - between button right edge and tick column, at the 0 position.
        let tick_x_screen = layout.sp(col.left() + col_w * TEMPO_TICK_X_U, col.top()).x;
        let led_x0 = reset_cx + reset_r + layout.sc(TEMPO_RESET_LED_GAP_L);
        let led_x1 = tick_x_screen - layout.sc(TEMPO_RESET_LED_GAP_R);
        if led_x1 > led_x0 {
            let tempo_reset_led = app.mosi().led_bit(mosi_frame::LED_TEMPO_RESET);

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
}
