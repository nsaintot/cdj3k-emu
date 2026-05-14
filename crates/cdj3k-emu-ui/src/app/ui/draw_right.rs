use egui::{FontFamily, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::app::ui::{
    layout, DoubleBorderSpec, StrokeSpec, COL_BLACK, COL_BLUE, COL_BTN, COL_BTN_OUTLINED_YELLOW,
    COL_BTN_TEXT, COL_BTN_WHITE, COL_DARK, COL_GREEN, COL_RED, COL_SILVER, COL_WHITE,
};
use cdj3k_emu_subucom::{miso_frame, mosi_frame};

use super::{ButtonType, CdjApp, UiScale};

mod nav_rotary;
mod statics;
mod vinyl_speed;

pub(super) const MODES_COL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::MODES_REF_LEFT, layout::MODES_REF_TOP),
    Pos2::new(layout::MODES_REF_RIGHT, layout::MODES_REF_BOT),
);

pub(super) const JOG_MODE_V: f32 = 0.387;
pub(super) const JOG_MODE_BTN_WIDTH: f32 = 120.0;
pub(super) const JOG_MODE_BTN_ASPECT_RATIO: f32 = 1.10;
pub(super) const JOG_MODE_MODE_ASPECT_RATIO: f32 = 1.1;
pub(super) const JOG_MODE_BTN_GAP_L: f32 = 100.0;
pub(super) const JOG_MODE_BTN_GAP_R: f32 = 10.0;
pub(super) const JOG_MODE_FONT_SIZE: f32 = 28.0;
pub(super) const JOG_MODE_LABEL_FONT_SIZE: f32 = 36.0;
pub(super) const JOG_MODE_BORDER_ROUNDING: f32 = 36.0;
pub(super) const JOG_MODE_BORDER_PAD: f32 = 30.0;
pub(super) const JOG_MODE_BORDER_PAD_L: f32 = 50.0;
pub(super) const JOG_MODE_BORDER_PAD_R: f32 = 30.0;
pub(super) const JOG_MODE_INNER_STROKE: f32 = 1.0;
pub(super) const JOG_MODE_OUTER_STROKE: f32 = 2.0;

pub(super) const SYNC_V: f32 = 0.455;
pub(super) const SYNC_INST_DOUBLES_V: f32 = 0.425;
pub(super) const SYNC_BTN_WIDTH: f32 = 140.0;
pub(super) const SYNC_BTN_ASPECT: f32 = 1.25;
pub(super) const SYNC_BTN_GAP: f32 = 50.0;
pub(super) const KEY_SYNC_BTN_WIDTH: f32 = 190.0;
pub(super) const KEY_SYNC_BTN_ASPECT_RATIO: f32 = 3.2;
pub(super) const KEY_SYNC_V_OFFSET: f32 = 0.018;
pub(super) const SYNC_FONT_SIZE: f32 = 28.0;
pub(super) const SYNC_MASTER_BTN_LABEL_NUDGE_Y: f32 = -5.0;
pub(super) const SYNC_MASTER_FONT_SIZE: f32 = 34.0;
pub(super) const SYNC_INNER_STROKE: f32 = 1.0;
pub(super) const SYNC_OUTER_STROKE: f32 = 2.0;
pub(super) const SYNC_BORDER_ROUNDING: f32 = 36.0;
pub(super) const SYNC_BORDER_PAD_X: f32 = 25.0;
pub(super) const SYNC_BORDER_PAD_Y: f32 = 40.0;
pub(super) const SYNC_INST_DOUBLES_FONT_SIZE: f32 = 32.0;

pub(super) const TEMPO_RANGE_V: f32 = 0.573;
pub(super) const TEMPO_RANGE_LABEL_V: f32 = 0.546;
pub(super) const TEMPO_RANGE_SUBLABEL_V: f32 = 0.555;
pub(super) const TEMPO_RANGE_LABEL_FONT_SIZE: f32 = 32.0;
pub(super) const TEMPO_RANGE_SUBLABEL_FONT_SIZE: f32 = 30.0;
pub(super) const TEMPO_RANGE_BTN_R: f32 = 28.0;
pub(super) const TEMPO_RANGE_INNER_STROKE: f32 = 4.0;
pub(super) const TEMPO_RANGE_OUTER_STROKE: f32 = 8.0;

pub(super) const MASTER_TEMPO_V: f32 = 0.63;
pub(super) const MASTER_TEMPO_LABEL_V: f32 = 0.607;
pub(super) const MASTER_TEMPO_LABEL_FONT_SIZE: f32 = 32.0;
pub(super) const MASTER_TEMPO_BTN_R: f32 = 30.0;
pub(super) const MASTER_TEMPO_BTN_FONT_SIZE: f32 = 100.0;
pub(super) const MASTER_TEMPO_INNER_STROKE: f32 = 4.0;
pub(super) const MASTER_TEMPO_OUTER_STROKE: f32 = 8.0;

// ── Tempo slider ──────────────────────────────────────────────────────────────
/// Vertical range within MODES_COL_REF (0 = top, 1 = bottom).
pub(super) const TEMPO_SLIDER_V_TOP: f32 = 0.6915;
pub(super) const TEMPO_SLIDER_V_BOT: f32 = 0.928;
pub(super) const TEMPO_SLIDER_LABEL_V: f32 = 0.945;
pub(super) const TEMPO_SLIDER_LABEL_FONT_SIZE: f32 = 34.0;

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

/// Segment width as a fraction of the backdrop inner width.
pub(super) const TEMPO_SEG_NORMAL_W: f32 = 0.72;
pub(super) const TEMPO_SEG_MAJOR_W: f32 = 0.78;
pub(super) const TEMPO_SEG_NORMAL_STROKE: f32 = 2.0;
pub(super) const TEMPO_SEG_MAJOR_STROKE: f32 = 5.0;

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

// ── Bottom label ──────────────────────────────────────────────────────────
pub(super) const BOTTOM_LABEL_TYPE_V: f32 = 0.973;
pub(super) const BOTTOM_LABEL_TYPE_X: f32 = 0.05;
pub(super) const BOTTOM_LABEL_TYPE_FONT_SIZE: f32 = 34.0;
pub(super) const BOTTOM_LABEL_SERIE_V: f32 = 0.974;
pub(super) const BOTTOM_LABEL_SERIE_X: f32 = 0.6;
pub(super) const BOTTOM_LABEL_SERIE_FONT_SIZE: f32 = 60.0;

// Navigation rotary + encoder constants now live in `nav_rotary.rs`.
// Re-export the two crate-public tuning knobs so external modules
// (`app/ui.rs`, `app/lcd_touch.rs`) keep their existing import paths.
pub(crate) use nav_rotary::{NAV_DETENT_COUNT, NAV_SCROLL_PX_PER_TICK};

impl CdjApp {
    pub(super) fn draw_right_sections(
        &mut self,
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
        let static_shapes = self
            .right_statics_cache
            .get_or_build(ox, oy, scale, ppp, |list| {
                statics::collect_right_statics(list, &ctx, layout);
            });
        self.frame_shape_count += static_shapes.len() as u64;
        p.extend(static_shapes.iter().cloned());

        self.draw_nav_rotary(ui, p, layout);
        self.draw_vinyl_speed_adj(ui, p, layout);

        {
            let center_x = col.left() + col.width() * 0.55;
            let cy_ref = col.top() + col.height() * JOG_MODE_V;
            let btn_h = JOG_MODE_BTN_WIDTH / JOG_MODE_MODE_ASPECT_RATIO;

            // Left button area: VINYL / CDJ labels
            let vinyl_btn_left = center_x - JOG_MODE_BTN_WIDTH - JOG_MODE_BTN_GAP_L * 0.45;
            let vinyl_btn_top = cy_ref - btn_h * 0.5;
            let vinyl_rect = layout.ar2rect(
                vinyl_btn_left,
                vinyl_btn_top,
                JOG_MODE_MODE_ASPECT_RATIO,
                JOG_MODE_BTN_WIDTH,
            );

            // Right button: JOG MODE
            let jog_btn_left = center_x + JOG_MODE_BTN_GAP_R * 0.5;
            let jog_rect = layout.ar2rect(
                jog_btn_left,
                vinyl_btn_top,
                JOG_MODE_BTN_ASPECT_RATIO,
                JOG_MODE_BTN_WIDTH,
            );

            let vinyl_led = self
                .led_state
                .mosi()
                .led_bit(mosi_frame::LED_JOG_MODE_VINYL);
            let cdj_led = self.led_state.mosi().led_bit(mosi_frame::LED_JOG_MODE_CDJ);

            // VINYL label (yellow, illuminated when active)
            p.text(
                Pos2::new(
                    vinyl_rect.center().x,
                    vinyl_rect.top() + vinyl_rect.height() * 0.20,
                ),
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
                Pos2::new(
                    vinyl_rect.center().x,
                    vinyl_rect.top() + vinyl_rect.height() * 0.72,
                ),
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
            self.btn(
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
                miso_frame::BTN_JOG_MODE,
            );
        }

        {
            let center_x = col.left() + col.width() * 0.5;
            let cy_ref = col.top() + col.height() * SYNC_V;

            // BEAT SYNC (left) and MASTER (right) on the same row
            let sync_btn_h = SYNC_BTN_WIDTH / SYNC_BTN_ASPECT;
            let bs_left = center_x - SYNC_BTN_WIDTH - SYNC_BTN_GAP * 0.5;
            let bs_top = cy_ref - sync_btn_h * 0.5;
            let bs_rect = layout.ar2rect(bs_left, bs_top, SYNC_BTN_ASPECT, SYNC_BTN_WIDTH);

            let master_left = center_x + SYNC_BTN_GAP * 0.5;
            let master_rect = layout.ar2rect(master_left, bs_top, SYNC_BTN_ASPECT, SYNC_BTN_WIDTH);

            // KEY SYNC centered below
            let ks_cy = cy_ref + sync_btn_h * 0.5 + col.height() * KEY_SYNC_V_OFFSET;
            let ks_left = center_x - KEY_SYNC_BTN_WIDTH * 0.5;
            let ks_rect = layout.ar2rect(
                ks_left,
                ks_cy,
                KEY_SYNC_BTN_ASPECT_RATIO,
                KEY_SYNC_BTN_WIDTH,
            );

            let sync_border = DoubleBorderSpec::from_strokes_with_gap(
                StrokeSpec {
                    width: layout.sc(SYNC_INNER_STROKE),
                    color: COL_DARK,
                },
                StrokeSpec {
                    width: layout.sc(SYNC_OUTER_STROKE),
                    color: COL_SILVER,
                },
                layout.sc(2.0),
            );

            let beat_sync_led = self.led_state.mosi().led_bit(mosi_frame::LED_BEAT_SYNC);
            let master_led = self.led_state.mosi().led_bit(mosi_frame::LED_MASTER);
            let key_sync_led = self.led_state.mosi().led_bit(mosi_frame::LED_KEY_SYNC);

            self.btn(
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
                miso_frame::BTN_BEAT_SYNC,
            );
            self.btn(
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
                miso_frame::BTN_MASTER,
            );
            self.btn(
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
                miso_frame::BTN_KEY_SYNC,
            );
        }

        {
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

            self.circle_btn(
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
                miso_frame::BTN_TEMPO_RANGE,
            );
        }

        {
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

            let master_tempo_led = self.led_state.mosi().led_bit(mosi_frame::LED_MASTER_TEMPO);

            self.circle_btn(
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
                miso_frame::BTN_MASTER_TEMPO,
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
            let knob_cy = slider.top() + slider.height() * self.tempo;
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
                    let prev = self.tempo;
                    self.tempo = t;
                    if (self.tempo - prev).abs() > 1e-4 {
                        self.inject_tempo();
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

            self.circle_btn(
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
                miso_frame::BTN_TEMPO_RESET,
            );

            // LED indicator - between button right edge and tick column, at the 0 position.
            let tick_x_screen = layout.sp(col.left() + col_w * TEMPO_TICK_X_U, col.top()).x;
            let led_x0 = reset_cx + reset_r + layout.sc(TEMPO_RESET_LED_GAP_L);
            let led_x1 = tick_x_screen - layout.sc(TEMPO_RESET_LED_GAP_R);
            if led_x1 > led_x0 {
                let tempo_reset_led = self.led_state.mosi().led_bit(mosi_frame::LED_TEMPO_RESET);

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
}
