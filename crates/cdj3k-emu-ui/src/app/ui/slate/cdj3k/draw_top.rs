//! Draws the upper deck: the ON AIR bar, the nav buttons, the LCD and the hot
//! cues. The horizontal band that holds the LCD and the hot cues comes
//! from [`super::layout`].

use crate::app::ui::{
    draw_cache::ShapeList, DoubleBorderSpec, StrokeSpec, COL_AMBER, COL_BLACK, COL_BLUE, COL_BTN,
    COL_BTN_TEXT, COL_BTN_WHITE, COL_SILVER,
};
use cdj3k_emu_panel::Btn;
use cdj3k_emu_panel::{mosi_frame, Lamp};
use egui::{Color32, FontId, Pos2, Rect, Shape, Stroke};

use super::{layout, ButtonType, CdjApp, UiScale};

/// Reference rect for the full menu-bar column (top-central panel).
pub(super) const MENUBAR_COL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::TOP_CENTRAL_REF_LEFT, layout::TOP_CENTRAL_REF_TOP),
    Pos2::new(layout::TOP_CENTRAL_REF_RIGHT, layout::TOP_CENTRAL_REF_BOT),
);

const MID_CENTRAL_PANEL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::MID_CENTRAL_REF_LEFT, layout::MID_CENTRAL_REF_TOP),
    Pos2::new(layout::MID_CENTRAL_REF_RIGHT, layout::MID_CENTRAL_REF_BOT),
);

const BROWSER_UTILITY_LABEL_FONT_SIZE: f32 = 34.0;

/// Vertical placement within [`MENUBAR_COL_REF`], from 0 at the top to 1 at the bottom.
const BROWSER_V_UTILITY_LABEL: f32 = 0.06;

// --- Nav button row ---
/// Width of a single nav button in reference units.
const NAV_BTN_WIDTH_REF: f32 = 270.0;
/// Width-to-height aspect ratio (wider than tall).
const NAV_BTN_ASPECT: f32 = 4.1;
/// Horizontal gap between consecutive nav buttons (reference units).
const NAV_BTN_GAP_REF: f32 = 75.0;
/// Vertical center of the nav row within [`MENUBAR_COL_REF`], from 0 at the top to 1 at the bottom.
/// It puts the row's centre at ref y 223, midway between the buttons' outer
/// edges at 184 and 262.
const BROWSER_V_NAV_BTNS: f32 = 0.10214;
/// Font size for nav button labels.
const NAV_BTN_FONT_SIZE: f32 = 32.0;
/// Inner stroke width for the double border on nav buttons (ref units).
const NAV_BTN_INNER_STROKE: f32 = 2.0;
/// Outer stroke width for the double border on nav buttons (ref units).
const NAV_BTN_OUTER_STROKE: f32 = 1.0;

// --- LCD screen ---
// The glass itself lives in `layout`, because the chassis frames the same
// rectangle and its recess walls have to land on it.
use layout::{
    LCD_GLASS_ASPECT as LCD_ASPECT, LCD_GLASS_V_TOP as LCD_V_TOP,
    LCD_GLASS_WIDTH_FRAC as LCD_WIDTH_FRAC,
};

// --- Hot cue row ---
/// Width of a single hot cue button in reference units.
pub(in crate::app) const HOT_CUE_BTN_WIDTH_REF: f32 = 180.0;
/// Width-to-height aspect ratio.
pub(in crate::app) const HOT_CUE_BTN_ASPECT: f32 = 1.8;
/// Horizontal gap between consecutive hot cue buttons (reference units).
pub(in crate::app) const HOT_CUE_BTN_GAP_REF: f32 = 107.0;
/// Vertical center within [`MID_CENTRAL_PANEL_REF`], from 0 at the top to 1 at the bottom.
pub(in crate::app) const HOT_CUE_BTN_V_CENTER: f32 = 0.3;
/// Font size for hot cue labels.
const HOT_CUE_BTN_FONT_SIZE: f32 = 34.0;
/// Colored outline stroke width (reference units).
const HOT_CUE_BTN_INNER_STROKE: f32 = 1.0;
const HOT_CUE_BTN_OUTER_STROKE: f32 = 4.0;

// --- HOT CUE label row (above hot cue buttons) ---
/// Vertical center of the label within [`MID_CENTRAL_PANEL_REF`].
const HOT_CUE_LABEL_V: f32 = 0.08;
const HOT_CUE_LABEL_FONT_SIZE: f32 = 34.0;
/// Distance from the panel center to the inner end of each flanking segment;
/// it covers the label and its clearance.
const HOT_CUE_LABEL_SEG_GAP_CENTER: f32 = 100.0;
/// Inset from panel left/right edge where each flanking segment starts.
const HOT_CUE_LABEL_SEG_GAP_EDGE: f32 = 20.0;
const HOT_CUE_LABEL_SEG_STROKE: f32 = 6.0;

/// Vertical position of the cue-control row within [`MID_CENTRAL_PANEL_REF`].
const CUE_CONTROL_V: f32 = 0.81;
const CUE_CONTROL_LABEL_FONT_SIZE: f32 = 32.0;
const CUE_CONTROL_LINE_1_LENGTH: f32 = 50.0;
const CUE_CONTROL_LINE_2_LENGTH: f32 = 60.0;
const CUE_CONTROL_LINE_3_LENGTH: f32 = 20.0;
const CUE_CONTROL_LINE_THICKNESS: f32 = 4.0;
const CUE_CONTROL_CALL_BTNS_SIZE: f32 = 28.0;
const CUE_CONTROL_CALL_BTNS_GAP: f32 = 0.038;
const CUE_CONTROL_CALL_BTNS_FONT_SIZE: f32 = 34.0;
const CUE_CONTROL_CALL_BTNS_INNER_STROKE: f32 = 8.0;
const CUE_CONTROL_CALL_BTNS_OUTER_STROKE: f32 = 4.0;
const CUE_CONTROL_CALL_LABEL_V: f32 = 0.6;

const CUE_CONTROL_DEL_BTN_SIZE: f32 = 40.0;
const CUE_CONTROL_DEL_BTNS_INNER_STROKE: f32 = 8.0;
const CUE_CONTROL_DEL_BTNS_OUTER_STROKE: f32 = 4.0;
const CUE_CONTROL_MEM_BTN_SIZE: f32 = 25.0;
const CUE_CONTROL_DELMEM_BTN_GAP: f32 = 0.038;
const CUE_CONTROL_DELMEM_LABEL_V: f32 = 0.56;

// ── ON AIR LED bar ────────────────────────────────────────────────────────────
/// Width of the bar's wider (top) edge as a fraction of the MENUBAR_COL_REF
/// width.
const ON_AIR_BAR_TOP_WIDTH_FRAC: f32 = 0.2;
/// Bottom width as a fraction of the top width (0..1), which sets the taper of
/// the inverted trapezoid.
const ON_AIR_BAR_TAPER: f32 = 0.80;
/// Vertical position of the bar's top edge within MENUBAR_COL_REF, from 0 at
/// the panel top to 1 at the bottom.
const ON_AIR_BAR_V_TOP: f32 = 0.005;
/// Vertical position of the bar's bottom edge within MENUBAR_COL_REF.
const ON_AIR_BAR_V_BOT: f32 = 0.02;
/// Radial glow spread (ref units).
const ON_AIR_BAR_GLOW_SPREAD_REF: f32 = 15.0;

/// Each entry is (label, MISO button). The accent color is read live from the
/// LED state in the MOSI frame.
const HOT_CUE_BTNS: &[(&str, Btn)] = &[
    ("A", Btn::HotA),
    ("B", Btn::HotB),
    ("C", Btn::HotC),
    ("D", Btn::HotD),
    ("E", Btn::HotE),
    ("F", Btn::HotF),
    ("G", Btn::HotG),
    ("H", Btn::HotH),
];

/// Each entry is (label, button, lamp, lit color), and the font color follows
/// the lamp: a lamp at medium shows a dimmed color and a lamp at full shows the
/// solid color (see [`StepLed`](mosi_frame::StepLed)).
const BROWSER_BTNS: &[(&str, Btn, Lamp, Color32)] = &[
    ("SOURCE", Btn::Source, Lamp::Source, COL_BTN_WHITE),
    ("BROWSE", Btn::Browse, Lamp::Browse, COL_BLUE),
    ("TAG LIST", Btn::TagList, Lamp::TagList, COL_BLUE),
    ("PLAY LIST", Btn::Playlist, Lamp::Playlist, COL_BLUE),
    ("SEARCH", Btn::SearchMenu, Lamp::Search, COL_BLUE),
    ("MENU", Btn::Menu, Lamp::Menu, COL_BTN_WHITE),
];

/// Returns `c` with its channels scaled by `factor`.
fn dim_color(c: Color32, factor: f32) -> Color32 {
    let scale = |v: u8| (v as f32 * factor).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(scale(c.r()), scale(c.g()), scale(c.b()))
}

/// Collects the static elements of the top section into `list`, which is
/// rebuilt only on a resize: the "— UTILITY" label, the "HOT CUE" legend and the
/// cue-control row's legends and separators.
fn collect_top_statics(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    list.text(
        ctx,
        layout.sp_in_rect(MENUBAR_COL_REF, 0.893, BROWSER_V_UTILITY_LABEL),
        egui::Align2::CENTER_BOTTOM,
        "— UTILITY",
        FontId::proportional(layout.sc(BROWSER_UTILITY_LABEL_FONT_SIZE)),
        COL_SILVER,
    );
    collect_hot_cue_legend(list, ctx, layout);
    collect_cue_control_legends(list, ctx, layout);
}

/// Collects the "HOT CUE" label and its flanking segments.
fn collect_hot_cue_legend(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    let label_pos = layout.sp_in_rect(MID_CENTRAL_PANEL_REF, 0.5, HOT_CUE_LABEL_V);
    list.text(
        ctx,
        label_pos,
        egui::Align2::CENTER_CENTER,
        "HOT CUE",
        FontId::proportional(layout.sc(HOT_CUE_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    let seg_stroke = Stroke::new(layout.sc(HOT_CUE_LABEL_SEG_STROKE), COL_BTN_TEXT);
    let y = label_pos.y;
    let x_left_start = layout
        .sp(
            MID_CENTRAL_PANEL_REF.left() + HOT_CUE_LABEL_SEG_GAP_EDGE,
            0.0,
        )
        .x;
    let x_left_end = label_pos.x - layout.sc(HOT_CUE_LABEL_SEG_GAP_CENTER);
    let x_right_start = label_pos.x + layout.sc(HOT_CUE_LABEL_SEG_GAP_CENTER);
    let x_right_end = layout
        .sp(
            MID_CENTRAL_PANEL_REF.right() - HOT_CUE_LABEL_SEG_GAP_EDGE,
            0.0,
        )
        .x;
    if x_left_end > x_left_start {
        list.line_segment(
            [Pos2::new(x_left_start, y), Pos2::new(x_left_end, y)],
            seg_stroke,
        );
    }
    if x_right_end > x_right_start {
        list.line_segment(
            [Pos2::new(x_right_start, y), Pos2::new(x_right_end, y)],
            seg_stroke,
        );
    }
}

/// Collects the cue-control row: the "CUE/LOOP CALL", "DELETE" and "MEMORY"
/// labels and the three separator lines.
fn collect_cue_control_legends(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    list.text(
        ctx,
        layout.sp_in_rect(MID_CENTRAL_PANEL_REF, 0.735, CUE_CONTROL_CALL_LABEL_V),
        egui::Align2::CENTER_CENTER,
        "CUE/LOOP\n    CALL",
        FontId::proportional(layout.sc(CUE_CONTROL_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    list.text(
        ctx,
        layout.sp_in_rect(MID_CENTRAL_PANEL_REF, 0.865, CUE_CONTROL_DELMEM_LABEL_V),
        egui::Align2::CENTER_CENTER,
        "DELETE",
        FontId::proportional(layout.sc(CUE_CONTROL_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    list.text(
        ctx,
        layout.sp_in_rect(MID_CENTRAL_PANEL_REF, 0.945, CUE_CONTROL_DELMEM_LABEL_V),
        egui::Align2::CENTER_CENTER,
        "MEMORY",
        FontId::proportional(layout.sc(CUE_CONTROL_LABEL_FONT_SIZE)),
        COL_BTN_TEXT,
    );
    let line_stroke_w = layout.sc(CUE_CONTROL_LINE_THICKNESS);
    for (u, length) in [
        (0.725, CUE_CONTROL_LINE_1_LENGTH),
        (0.8, CUE_CONTROL_LINE_2_LENGTH),
        (0.905, CUE_CONTROL_LINE_3_LENGTH),
    ] {
        let start = layout.sp_in_rect(MID_CENTRAL_PANEL_REF, u, CUE_CONTROL_V);
        let end = Pos2::new(start.x + layout.sc(length), start.y);
        list.line_segment([start, end], Stroke::new(line_stroke_w, COL_BTN_TEXT));
    }
}

pub(super) fn draw_top_section(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
) {
    puffin::profile_function!();
    // ── Static labels and chrome (rebuilt only on resize) ─────────────────
    let ctx = ui.ctx().clone();
    let ppp = ui.ctx().pixels_per_point();
    app.frame_shape_count += app.caches.top_statics.paint(p, layout, ppp, |list| {
        collect_top_statics(list, &ctx, layout);
    });

    draw_on_air_bar(app, p, layout);
    draw_browser_buttons(app, ui, layout);
    draw_lcd(app, ui, p, layout);
    draw_hot_cues(app, ui, layout);
    draw_cue_call(app, ui, layout);
    draw_delete_memory(app, ui, layout);
}

/// Draws the ON AIR LED bar, an inverted trapezoid above the nav button row.
fn draw_on_air_bar(app: &CdjApp, p: &egui::Painter, layout: &UiScale) {
    let (r, g, b) = app.mosi().rgb(Lamp::OnAir).unwrap_or_default();
    if let Some(bar_color) = app.mosi().led_color(mosi_frame::LedPart::OnAir, r, g, b) {
        let panel_cx = (MENUBAR_COL_REF.left() + MENUBAR_COL_REF.right()) * 0.5;
        let bar_top_y = MENUBAR_COL_REF.top() + MENUBAR_COL_REF.height() * ON_AIR_BAR_V_TOP;
        let bar_bot_y = MENUBAR_COL_REF.top() + MENUBAR_COL_REF.height() * ON_AIR_BAR_V_BOT;
        let top_hw = MENUBAR_COL_REF.width() * ON_AIR_BAR_TOP_WIDTH_FRAC * 0.5;
        let bot_hw = top_hw * ON_AIR_BAR_TAPER;

        let tl = layout.sp(panel_cx - top_hw, bar_top_y);
        let tr = layout.sp(panel_cx + top_hw, bar_top_y);
        let br = layout.sp(panel_cx + bot_hw, bar_bot_y);
        let bl = layout.sp(panel_cx - bot_hw, bar_bot_y);

        // Paint the glow layers from the outermost to the innermost, then the
        // main fill on top.
        let max_expand = layout.sc(ON_AIR_BAR_GLOW_SPREAD_REF);
        for layer in 0..4usize {
            let t = 1.0 - layer as f32 / 3.0; // `t` falls from 1.0 to 0.0.
            let expand = t * max_expand;
            let glow_alpha = t * 0.28;
            if glow_alpha < 0.005 {
                continue;
            }
            let g_tl = Pos2::new(tl.x - expand, tl.y - expand);
            let g_tr = Pos2::new(tr.x + expand, tr.y - expand);
            let g_br = Pos2::new(br.x + expand, br.y + expand);
            let g_bl = Pos2::new(bl.x - expand, bl.y + expand);
            p.add(Shape::convex_polygon(
                vec![g_tl, g_tr, g_br, g_bl],
                bar_color.gamma_multiply(glow_alpha),
                Stroke::NONE,
            ));
        }
        // Main fill
        p.add(Shape::convex_polygon(
            vec![tl, tr, br, bl],
            bar_color,
            Stroke::NONE,
        ));
    }
}

/// Draws the nav button row, centered on the midpoint of MENUBAR_COL_REF.
fn draw_browser_buttons(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let n = BROWSER_BTNS.len() as f32;
    let btn_w = NAV_BTN_WIDTH_REF;
    let btn_h = NAV_BTN_WIDTH_REF / NAV_BTN_ASPECT;
    let gap = NAV_BTN_GAP_REF;
    let total_w = n * btn_w + (n - 1.0) * gap;
    let center_x = (MENUBAR_COL_REF.left() + MENUBAR_COL_REF.right()) * 0.5;
    let first_left = center_x - total_w * 0.5;
    let btn_top =
        MENUBAR_COL_REF.top() + MENUBAR_COL_REF.height() * BROWSER_V_NAV_BTNS - btn_h * 0.5;

    let nav_border = DoubleBorderSpec::from_strokes(
        StrokeSpec {
            width: layout.sc(NAV_BTN_INNER_STROKE),
            color: COL_SILVER,
        },
        StrokeSpec {
            width: layout.sc(NAV_BTN_OUTER_STROKE),
            color: COL_BLACK,
        },
    );

    for (i, (label, btn, led, color)) in BROWSER_BTNS.iter().enumerate() {
        let btn_left = first_left + i as f32 * (btn_w + gap);
        let rect = layout.ar2rect(btn_left, btn_top, NAV_BTN_ASPECT, btn_w);
        let color = match app.mosi().step(*led) {
            mosi_frame::StepLed::Full => *color,
            mosi_frame::StepLed::Medium => dim_color(*color, 0.7),
            mosi_frame::StepLed::Off => COL_BTN_TEXT,
        };
        app.btn(
            ui,
            layout,
            ButtonType::Basic,
            rect,
            label,
            layout.sc(NAV_BTN_FONT_SIZE),
            Some(color),
            Some(COL_BLACK),
            None,
            None,
            None,
            Some(nav_border),
            egui::FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into()),
            (*label, "nav"),
            *btn,
        );
    }
}

/// Draws the LCD screen, centered on MENUBAR_COL_REF.
fn draw_lcd(app: &mut CdjApp, ui: &mut egui::Ui, p: &egui::Painter, layout: &UiScale) {
    // View > Extend Screen grows the panel until it fills the band between
    // the horizontal decoration lines that frame it, taken from the chassis
    // so the two cannot drift apart. The width follows from the aspect ratio.
    let extended = cdj3k_emu_platform::menu_state::lock().screen_extended;
    let (lcd_w, lcd_h, lcd_top) = if extended {
        let (band_top, band_bot) = super::draw_chassis::lcd_decor_band();
        let h = band_bot - band_top;
        (h * LCD_ASPECT, h, band_top)
    } else {
        let w = MENUBAR_COL_REF.width() * LCD_WIDTH_FRAC;
        (
            w,
            w / LCD_ASPECT,
            MENUBAR_COL_REF.top() + MENUBAR_COL_REF.height() * LCD_V_TOP,
        )
    };
    let center_x = (MENUBAR_COL_REF.left() + MENUBAR_COL_REF.right()) * 0.5;
    let lcd_left = center_x - lcd_w * 0.5;

    let bezel = Rect::from_min_size(Pos2::new(lcd_left, lcd_top), egui::vec2(lcd_w, lcd_h));
    crate::app::ui::draw_lcd::draw_main_lcd(app, ui, p, layout, bezel);
}

/// Draws the hot cue row, centered on MID_CENTRAL_PANEL_REF.
fn draw_hot_cues(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let n = HOT_CUE_BTNS.len() as f32;
    let btn_w = HOT_CUE_BTN_WIDTH_REF;
    let btn_h = btn_w / HOT_CUE_BTN_ASPECT;
    let gap = HOT_CUE_BTN_GAP_REF;
    let total_w = n * btn_w + (n - 1.0) * gap;
    let center_x = (MID_CENTRAL_PANEL_REF.left() + MID_CENTRAL_PANEL_REF.right()) * 0.5;
    let first_left = center_x - total_w * 0.5;
    let btn_top = MID_CENTRAL_PANEL_REF.top()
        + MID_CENTRAL_PANEL_REF.height() * HOT_CUE_BTN_V_CENTER
        - btn_h * 0.5;

    let hotcue_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(HOT_CUE_BTN_INNER_STROKE),
            color: COL_BLACK,
        },
        StrokeSpec {
            width: layout.sc(HOT_CUE_BTN_OUTER_STROKE),
            color: COL_SILVER,
        },
        layout.sc(2.0),
    );

    for (i, (label, btn)) in HOT_CUE_BTNS.iter().enumerate() {
        let btn_left = first_left + i as f32 * (btn_w + gap);
        let rect = layout.ar2rect(btn_left, btn_top, HOT_CUE_BTN_ASPECT, btn_w);
        let (r, g, b) = app.mosi().rgb(Lamp::HOT_CUES[i]).unwrap_or_default();
        let accent = app
            .mosi()
            .led_color(mosi_frame::LedPart::Pad, r, g, b)
            .unwrap_or(COL_SILVER);
        app.btn(
            ui,
            layout,
            ButtonType::HotCue,
            rect,
            label,
            layout.sc(HOT_CUE_BTN_FONT_SIZE),
            Some(accent),
            None,
            None,
            None,
            None,
            Some(hotcue_border),
            egui::FontFamily::Proportional,
            (*label, "hotcue"),
            *btn,
        );
    }
}

/// Draws the CUE/LOOP CALL ◀ and ▶ buttons.
fn draw_cue_call(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let call_prev_btn = layout.sp_in_rect(
        MID_CENTRAL_PANEL_REF,
        0.735 - CUE_CONTROL_CALL_BTNS_GAP,
        CUE_CONTROL_V,
    );
    let call_next_btn = layout.sp_in_rect(
        MID_CENTRAL_PANEL_REF,
        0.735 + CUE_CONTROL_CALL_BTNS_GAP,
        CUE_CONTROL_V,
    );
    let call_led = app.mosi().lit(Lamp::TrackSearch);
    let call_border = cue_call_border(layout);
    for (center, label, id, btn) in [
        (call_prev_btn, "◀", "call_prev", Btn::CallPrev),
        (call_next_btn, "▶", "call_next", Btn::CallNext),
    ] {
        app.circle_btn(
            ui,
            layout,
            center,
            layout.sc(CUE_CONTROL_CALL_BTNS_SIZE),
            Some(COL_BLACK),
            None,
            label,
            Some(if call_led { COL_AMBER } else { COL_BTN_TEXT }),
            None,
            layout.sc(CUE_CONTROL_CALL_BTNS_FONT_SIZE),
            None,
            Some(call_border),
            id,
            btn,
        );
    }
}

/// Draws the DELETE and MEMORY buttons.
fn draw_delete_memory(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let call_border = cue_call_border(layout);
    let delete_btn = layout.sp_in_rect(
        MID_CENTRAL_PANEL_REF,
        0.904 - CUE_CONTROL_DELMEM_BTN_GAP,
        CUE_CONTROL_V,
    );

    let memory_btn = layout.sp_in_rect(
        MID_CENTRAL_PANEL_REF,
        0.904 + CUE_CONTROL_DELMEM_BTN_GAP,
        CUE_CONTROL_V,
    );

    let delete_border = DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(CUE_CONTROL_DEL_BTNS_INNER_STROKE),
            color: COL_SILVER,
        },
        StrokeSpec {
            width: layout.sc(CUE_CONTROL_DEL_BTNS_OUTER_STROKE),
            color: COL_BTN,
        },
        layout.sc(20.0),
    );
    app.circle_btn(
        ui,
        layout,
        delete_btn,
        layout.sc(CUE_CONTROL_DEL_BTN_SIZE),
        Some(COL_BLACK),
        None,
        "",
        None,
        None,
        layout.sc(CUE_CONTROL_CALL_BTNS_FONT_SIZE),
        None,
        Some(delete_border),
        "delete",
        Btn::Delete,
    );

    app.circle_btn(
        ui,
        layout,
        memory_btn,
        layout.sc(CUE_CONTROL_MEM_BTN_SIZE),
        Some(COL_BLACK),
        None,
        "",
        None,
        None,
        layout.sc(CUE_CONTROL_CALL_BTNS_FONT_SIZE),
        None,
        Some(call_border),
        "memory",
        Btn::Memory,
    );
}

/// Returns the border of the CUE/LOOP CALL buttons and of MEMORY.
fn cue_call_border(layout: &UiScale) -> DoubleBorderSpec {
    DoubleBorderSpec::from_strokes_with_gap(
        StrokeSpec {
            width: layout.sc(CUE_CONTROL_CALL_BTNS_INNER_STROKE),
            color: COL_SILVER,
        },
        StrokeSpec {
            width: layout.sc(CUE_CONTROL_CALL_BTNS_OUTER_STROKE),
            color: COL_BTN,
        },
        layout.sc(5.0),
    )
}
