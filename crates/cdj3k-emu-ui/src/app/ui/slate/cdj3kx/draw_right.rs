use cdj3k_emu_panel::{Btn, Lamp};
use egui::{FontFamily, FontId, Pos2, Rect, Vec2};

use crate::app::ui::{
    DoubleBorderSpec, StrokeSpec, COL_BLUE, COL_BTN_OUTLINED_YELLOW, COL_BTN_TEXT, COL_BTN_WHITE,
    COL_DARK, COL_GREEN, COL_SILVER,
};

use crate::app::ui::draw_vinyl_speed::{self, VinylSpeedPlacement};

use super::{layout, ButtonType, CdjApp, UiScale, LEGEND_TAB_H, LEGEND_TAB_W};

pub(super) mod nav_rotary;
mod statics;
mod tempo;

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
    // ── Static labels and chrome (rebuilt only on resize) ─────────────────
    let ppp = ui.ctx().pixels_per_point();
    let ctx = ui.ctx().clone();
    app.frame_shape_count += app.caches.right_statics.paint(p, layout, ppp, |list| {
        statics::collect_right_statics(list, &ctx, layout);
    });

    nav_rotary::draw_nav_rotary(app, ui, p, layout);
    draw_vinyl_speed::draw_vinyl_speed_adj(app, ui, p, layout, VINYL_SPEED_PLACE);
    draw_jog_mode(app, ui, p, layout);
    draw_sync(app, ui, layout);
    tempo::draw_tempo_range(app, ui, layout);
    tempo::draw_master_tempo(app, ui, layout);
    tempo::draw_tempo_slider(app, ui, p, layout);
}

/// Draws the JOG MODE button and the VINYL and CDJ legends, each lit while its
/// mode is on.
fn draw_jog_mode(app: &mut CdjApp, ui: &mut egui::Ui, p: &egui::Painter, layout: &UiScale) {
    let (combo_ref, jog_ref) = jog_mode_boxes();
    let label_x = combo_ref.center().x + JOG_MODE_LABEL_OFF_X;
    let jog_rect = screen(layout, jog_ref);

    let vinyl_led = app.mosi().lit(Lamp::JogModeVinyl);
    let cdj_led = app.mosi().lit(Lamp::JogModeCdj);

    // The VINYL legend is blue while its lamp is lit.
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
    // The CDJ legend is green while its lamp is lit.
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

/// Draws BEAT SYNC and MASTER side by side, with KEY SYNC centred below them.
fn draw_sync(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
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

    let beat_sync_led = app.mosi().lit(Lamp::BeatSync);
    let master_led = app.mosi().lit(Lamp::Master);
    let key_sync_led = app.mosi().lit(Lamp::KeySync);

    let master_nudge = Vec2::new(0.0, layout.sc(SYNC_MASTER_BTN_LABEL_NUDGE_Y));
    let condensed = FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_CONDENSED.into());
    let lit = |on: bool, color| if on { color } else { COL_BTN_TEXT };
    let rows = [
        (
            bs_rect,
            "BEAT\nSYNC",
            SYNC_FONT_SIZE,
            lit(beat_sync_led, COL_BTN_WHITE),
            None,
            FontFamily::Proportional,
            "beat_sync",
            Btn::BeatSync,
        ),
        (
            master_rect,
            "MASTER",
            SYNC_MASTER_FONT_SIZE,
            lit(master_led, COL_BTN_OUTLINED_YELLOW),
            Some(master_nudge),
            condensed,
            "master",
            Btn::Master,
        ),
        (
            ks_rect,
            "KEY SYNC",
            SYNC_FONT_SIZE,
            lit(key_sync_led, COL_BLUE),
            None,
            FontFamily::Proportional,
            "key_sync",
            Btn::KeySync,
        ),
    ];
    for (rect, label, font_size, color, nudge, family, id, btn) in rows {
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            rect,
            label,
            layout.sc(font_size),
            Some(color),
            None,
            None,
            None,
            nudge,
            Some(sync_border),
            family,
            id,
            btn,
        );
    }
}
