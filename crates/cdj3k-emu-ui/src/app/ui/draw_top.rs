//! Upper deck UI (rekordbox bar, nav, LCD, hot cues). Horizontal LCD / hot-cue band comes from [`super::layout`].

use crate::app::ui::{
    draw_bordered_rect_section, draw_cache::ShapeList, layout, DoubleBorderSpec, StrokeSpec,
    COL_AMBER, COL_BLACK, COL_BLUE, COL_BTN, COL_BTN_TEXT, COL_BTN_WHITE, COL_LCD_BG, COL_SILVER,
};
use cdj3k_emu_subucom::{miso_frame, mosi_frame};
use egui::{Color32, FontId, Pos2, Rect, Shape, Stroke};

use super::{ButtonType, CdjApp, UiScale};

/// Reference rect for the full menu-bar column (top-central panel).
const MENUBAR_COL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::TOP_CENTRAL_REF_LEFT, layout::TOP_CENTRAL_REF_TOP),
    Pos2::new(layout::TOP_CENTRAL_REF_RIGHT, layout::TOP_CENTRAL_REF_BOT),
);

const MID_CENTRAL_PANEL_REF: Rect = Rect::from_min_max(
    Pos2::new(layout::MID_CENTRAL_REF_LEFT, layout::MID_CENTRAL_REF_TOP),
    Pos2::new(layout::MID_CENTRAL_REF_RIGHT, layout::MID_CENTRAL_REF_BOT),
);

const BROWSER_UTILITY_LABEL_FONT_SIZE: f32 = 34.0;

/// Vertical placement within [`MENUBAR_COL_REF`] (0 = top, 1 = bottom).
const BROWSER_V_UTILITY_LABEL: f32 = 0.06;

// --- Nav button row ---
/// Width of a single nav button in reference units.
const NAV_BTN_WIDTH_REF: f32 = 270.0;
/// Width-to-height aspect ratio (wider than tall).
const NAV_BTN_ASPECT: f32 = 4.1;
/// Horizontal gap between consecutive nav buttons (reference units).
const NAV_BTN_GAP_REF: f32 = 75.0;
/// Vertical center of the nav row within [`MENUBAR_COL_REF`] (0 = top, 1 = bottom).
const BROWSER_V_NAV_BTNS: f32 = 0.095;
/// Font size for nav button labels.
const NAV_BTN_FONT_SIZE: f32 = 32.0;
/// Inner stroke width for the double border on nav buttons (ref units).
const NAV_BTN_INNER_STROKE: f32 = 2.0;
/// Outer stroke width for the double border on nav buttons (ref units).
const NAV_BTN_OUTER_STROKE: f32 = 1.0;

// --- LCD screen ---
/// 16:9 display aspect ratio.
const LCD_ASPECT: f32 = 1280.0 / 720.0;
/// LCD outer bezel width as a fraction of [`MENUBAR_COL_REF`] width.
const LCD_WIDTH_FRAC: f32 = 0.85;
/// Top of the LCD outer bezel within [`MENUBAR_COL_REF`] (0 = top, 1 = bottom).
const LCD_V_TOP: f32 = 0.18;
/// Bezel thickness on all sides (reference units).
const LCD_BEZEL_REF: f32 = 4.0;
/// Corner rounding for the bezel (reference units).
const LCD_ROUNDING_REF: f32 = 10.0;
/// Font size for the "connecting…" placeholder label.
const LCD_PLACEHOLDER_FONT_SIZE: f32 = 44.0;

// --- Hot cue row ---
/// Width of a single hot cue button in reference units.
const HOT_CUE_BTN_WIDTH_REF: f32 = 180.0;
/// Width-to-height aspect ratio.
const HOT_CUE_BTN_ASPECT: f32 = 1.8;
/// Horizontal gap between consecutive hot cue buttons (reference units).
const HOT_CUE_BTN_GAP_REF: f32 = 107.0;
/// Vertical center within [`MID_CENTRAL_PANEL_REF`] (0 = top, 1 = bottom).
const HOT_CUE_BTN_V_CENTER: f32 = 0.3;
/// Font size for hot cue labels.
const HOT_CUE_BTN_FONT_SIZE: f32 = 34.0;
/// Colored outline stroke width (reference units).
const HOT_CUE_BTN_INNER_STROKE: f32 = 1.0;
const HOT_CUE_BTN_OUTER_STROKE: f32 = 4.0;

// --- HOT CUE label row (above hot cue buttons) ---
/// Vertical center of the label within [`MID_CENTRAL_PANEL_REF`].
const HOT_CUE_LABEL_V: f32 = 0.08;
const HOT_CUE_LABEL_FONT_SIZE: f32 = 34.0;
/// Half-distance from panel center to where each flanking segment ends (covers label + clearance).
const HOT_CUE_LABEL_SEG_GAP_CENTER: f32 = 100.0;
/// Inset from panel left/right edge where each flanking segment starts.
const HOT_CUE_LABEL_SEG_GAP_EDGE: f32 = 20.0;
const HOT_CUE_LABEL_SEG_STROKE: f32 = 6.0;

/// Cue control row
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
/// Fraction of MENUBAR_COL_REF width for the bar's wider (top) edge. Tune freely.
const ON_AIR_BAR_TOP_WIDTH_FRAC: f32 = 0.2;
/// Bottom width as a fraction of top width - controls the inverted-trapezoid taper (0..1).
const ON_AIR_BAR_TAPER: f32 = 0.80;
/// V position of the bar's top edge within MENUBAR_COL_REF (0=panel top, 1=bottom).
const ON_AIR_BAR_V_TOP: f32 = 0.005;
/// V position of the bar's bottom edge within MENUBAR_COL_REF.
const ON_AIR_BAR_V_BOT: f32 = 0.02;
/// Radial glow spread (ref units).
const ON_AIR_BAR_GLOW_SPREAD_REF: f32 = 15.0;

/// (label, MISO btn) - accent color is read live from led_state MOSI frame.
const HOT_CUE_BTNS: &[(&str, (usize, u8))] = &[
    ("A", miso_frame::BTN_HOT_A),
    ("B", miso_frame::BTN_HOT_B),
    ("C", miso_frame::BTN_HOT_C),
    ("D", miso_frame::BTN_HOT_D),
    ("E", miso_frame::BTN_HOT_E),
    ("F", miso_frame::BTN_HOT_F),
    ("G", miso_frame::BTN_HOT_G),
    ("H", miso_frame::BTN_HOT_H),
];

/// (label, MISO btn, MOSI nav LED, lit color) - font color is computed live from led_state.
/// Medium-only lit → dimmed color, full → solid (see [`mosi_frame::StepLedMask`]).
const BROWSER_BTNS: &[(&str, (usize, u8), mosi_frame::StepLedMask, Color32)] = &[
    (
        "SOURCE",
        miso_frame::BTN_SOURCE,
        mosi_frame::LED_SOURCE,
        COL_BTN_WHITE,
    ),
    (
        "BROWSE",
        miso_frame::BTN_BROWSE,
        mosi_frame::LED_BROWSE,
        COL_BLUE,
    ),
    (
        "TAG LIST",
        miso_frame::BTN_TAG_LIST,
        mosi_frame::LED_TAG_LIST,
        COL_BLUE,
    ),
    (
        "PLAY LIST",
        miso_frame::BTN_PLAYLIST,
        mosi_frame::LED_PLAYLIST,
        COL_BLUE,
    ),
    (
        "SEARCH",
        miso_frame::BTN_SEARCH_MENU,
        mosi_frame::LED_SEARCH,
        COL_BLUE,
    ),
    (
        "MENU",
        miso_frame::BTN_MENU,
        mosi_frame::LED_MENU,
        COL_BTN_WHITE,
    ),
];

/// Collect fully-static elements of the top section into `list` (rebuilt only on resize).
///
/// Includes: "✿ rekordbox" label, "- UTILITY" label, "HOT CUE" label + flanking
/// segments, "CUE/LOOP CALL" / "DELETE" / "MEMORY" labels, and the three separator
/// lines of the cue-control row.
fn dim_color(c: Color32, factor: f32) -> Color32 {
    let scale = |v: u8| (v as f32 * factor).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(scale(c.r()), scale(c.g()), scale(c.b()))
}

fn collect_top_statics(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    list.text(
        ctx,
        layout.sp_in_rect(MENUBAR_COL_REF, 0.893, BROWSER_V_UTILITY_LABEL),
        egui::Align2::CENTER_BOTTOM,
        "— UTILITY",
        FontId::proportional(layout.sc(BROWSER_UTILITY_LABEL_FONT_SIZE)),
        COL_SILVER,
    );
    // "HOT CUE" label + flanking segments
    {
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
    // Cue-control row: labels and separator lines
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
    // Three separator line segments
    {
        let start = layout.sp_in_rect(MID_CENTRAL_PANEL_REF, 0.725, CUE_CONTROL_V);
        let end = Pos2::new(start.x + layout.sc(CUE_CONTROL_LINE_1_LENGTH), start.y);
        list.line_segment([start, end], Stroke::new(line_stroke_w, COL_BTN_TEXT));
    }
    {
        let start = layout.sp_in_rect(MID_CENTRAL_PANEL_REF, 0.8, CUE_CONTROL_V);
        let end = Pos2::new(start.x + layout.sc(CUE_CONTROL_LINE_2_LENGTH), start.y);
        list.line_segment([start, end], Stroke::new(line_stroke_w, COL_BTN_TEXT));
    }
    {
        let start = layout.sp_in_rect(MID_CENTRAL_PANEL_REF, 0.905, CUE_CONTROL_V);
        let end = Pos2::new(start.x + layout.sc(CUE_CONTROL_LINE_3_LENGTH), start.y);
        list.line_segment([start, end], Stroke::new(line_stroke_w, COL_BTN_TEXT));
    }
}

impl CdjApp {
    pub(super) fn draw_top_section(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        layout: &UiScale,
    ) {
        puffin::profile_function!();
        // ── Static labels and chrome (rebuilt only on resize) ─────────────────
        let (ox, oy, scale) = layout.cache_key();
        let ctx = ui.ctx().clone();
        let ppp = ui.ctx().pixels_per_point();
        let static_shapes = self
            .top_statics_cache
            .get_or_build(ox, oy, scale, ppp, |list| {
                collect_top_statics(list, &ctx, layout);
            });
        self.frame_shape_count += static_shapes.len() as u64;
        p.extend(static_shapes.iter().cloned());

        // --- ON AIR LED bar - inverted trapezoid above the nav button row ---
        {
            let f = &self.led_state.frame;
            if let Some(bar_color) = mosi_frame::led_color(
                f[mosi_frame::ON_AIR_LED_BASE],
                f[mosi_frame::ON_AIR_LED_BASE + 1],
                f[mosi_frame::ON_AIR_LED_BASE + 2],
            ) {
                let panel_cx = (MENUBAR_COL_REF.left() + MENUBAR_COL_REF.right()) * 0.5;
                let bar_top_y = MENUBAR_COL_REF.top() + MENUBAR_COL_REF.height() * ON_AIR_BAR_V_TOP;
                let bar_bot_y = MENUBAR_COL_REF.top() + MENUBAR_COL_REF.height() * ON_AIR_BAR_V_BOT;
                let top_hw = MENUBAR_COL_REF.width() * ON_AIR_BAR_TOP_WIDTH_FRAC * 0.5;
                let bot_hw = top_hw * ON_AIR_BAR_TAPER;

                let tl = layout.sp(panel_cx - top_hw, bar_top_y);
                let tr = layout.sp(panel_cx + top_hw, bar_top_y);
                let br = layout.sp(panel_cx + bot_hw, bar_bot_y);
                let bl = layout.sp(panel_cx - bot_hw, bar_bot_y);

                // Glow layers (outermost → innermost, then main fill on top).
                let max_expand = layout.sc(ON_AIR_BAR_GLOW_SPREAD_REF);
                for layer in 0..4usize {
                    let t = 1.0 - layer as f32 / 3.0; // 1.0 → 0.0
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

        // --- Nav button row, centered on the midpoint of MENUBAR_COL_REF ---
        {
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
                let color = match mosi_frame::led_step(&self.led_state.frame, *led) {
                    mosi_frame::StepLed::Full => *color,
                    mosi_frame::StepLed::Medium => dim_color(*color, 0.7),
                    mosi_frame::StepLed::Off => COL_BTN_TEXT,
                };
                self.btn(
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

        // --- LCD screen, centered on MENUBAR_COL_REF ---
        {
            let lcd_w = MENUBAR_COL_REF.width() * LCD_WIDTH_FRAC;
            let lcd_h = lcd_w / LCD_ASPECT;
            let center_x = (MENUBAR_COL_REF.left() + MENUBAR_COL_REF.right()) * 0.5;
            let lcd_left = center_x - lcd_w * 0.5;
            let lcd_top = MENUBAR_COL_REF.top() + MENUBAR_COL_REF.height() * LCD_V_TOP;

            let bezel_rounding = layout.sc(LCD_ROUNDING_REF);
            let bezel_rect = layout.sr(lcd_left, lcd_top, lcd_w, lcd_h);
            p.rect_filled(bezel_rect, bezel_rounding, Color32::from_rgb(10, 10, 12));

            let bezel_px = layout.sc(LCD_BEZEL_REF);
            let display_rect = bezel_rect.shrink(bezel_px);
            self.bloom_excludes.push(display_rect);
            let display_rounding = (bezel_rounding - bezel_px).max(2.0);
            p.rect_filled(display_rect, display_rounding, COL_LCD_BG);

            let uv_full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            if self.main_screen_popped {
                p.text(
                    display_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "↗ external window",
                    FontId::proportional(layout.sc(LCD_PLACEHOLDER_FONT_SIZE)),
                    Color32::from_rgb(60, 60, 70),
                );
            } else if !self.lcds_blanked {
                if let Some(tex_id) = self.display_tex_id {
                    if self.display_stream.is_connected() {
                        p.image(tex_id, display_rect, uv_full, Color32::WHITE);
                    } else {
                        p.image(
                            tex_id,
                            display_rect,
                            uv_full,
                            Color32::from_rgba_unmultiplied(255, 255, 255, 60),
                        );
                    }
                }
                if !self.display_stream.is_connected() {
                    p.text(
                        display_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("connecting {}…", self.display_stream.addr_str()),
                        FontId::proportional(layout.sc(LCD_PLACEHOLDER_FONT_SIZE)),
                        Color32::from_rgb(60, 60, 70),
                    );
                }
            }

            // Touch is handled by the popout viewport when the LCD is detached.
            if !self.main_screen_popped {
                let lcd_resp = ui.interact(
                    display_rect,
                    ui.id().with("lcd_touch"),
                    egui::Sense::click_and_drag(),
                );
                self.apply_lcd_touch(crate::app::LcdTouchCapture {
                    hovered: lcd_resp.hovered(),
                    scroll_y: ui.input(|i| i.raw_scroll_delta.y),
                    pointer_moved: ui.input(|i| i.pointer.delta().length_sq() > 0.0),
                    is_down: lcd_resp.is_pointer_button_down_on(),
                    ctrl: ui.input(|i| i.modifiers.ctrl),
                    right_down: ui.input(|i| {
                        i.pointer.button_down(egui::PointerButton::Secondary)
                            && i.pointer
                                .hover_pos()
                                .map_or(false, |p| display_rect.contains(p))
                    }),
                    interact_pos: lcd_resp.interact_pointer_pos(),
                    display_rect,
                });
            }

            draw_bordered_rect_section(
                p,
                display_rect,
                None,
                None,
                DoubleBorderSpec::from_strokes_with_gap(
                    StrokeSpec {
                        width: layout.sc(LCD_BEZEL_REF),
                        color: COL_SILVER,
                    },
                    StrokeSpec {
                        width: layout.sc(LCD_BEZEL_REF),
                        color: COL_SILVER,
                    },
                    layout.sc(1.0),
                ),
            );
        }

        // --- Hot cue row, centered on MID_CENTRAL_PANEL_REF ---
        {
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

            // "HOT CUE" label + flanking segments are in top_statics_cache (drawn above).

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
                let base = mosi_frame::HOT_CUE_BASE[i];
                let f = &self.led_state.frame;
                let accent =
                    mosi_frame::led_color(f[base], f[base + 1], f[base + 2]).unwrap_or(COL_SILVER);
                self.btn(
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
        {
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
            let call_led = {
                let f = &self.led_state.frame;
                f[mosi_frame::LED_TRACK_SEARCH.0] & mosi_frame::LED_TRACK_SEARCH.1 != 0
            };
            let call_border = DoubleBorderSpec::from_strokes_with_gap(
                StrokeSpec {
                    width: layout.sc(CUE_CONTROL_CALL_BTNS_INNER_STROKE),
                    color: COL_SILVER,
                },
                StrokeSpec {
                    width: layout.sc(CUE_CONTROL_CALL_BTNS_OUTER_STROKE),
                    color: COL_BTN,
                },
                layout.sc(5.0),
            );
            self.circle_btn(
                ui,
                layout,
                call_prev_btn,
                layout.sc(CUE_CONTROL_CALL_BTNS_SIZE),
                Some(COL_BLACK),
                None,
                "◀",
                Some(if call_led { COL_AMBER } else { COL_BTN_TEXT }),
                None,
                layout.sc(CUE_CONTROL_CALL_BTNS_FONT_SIZE),
                None,
                Some(call_border),
                "call_prev",
                miso_frame::BTN_CALL_PREV,
            );
            self.circle_btn(
                ui,
                layout,
                call_next_btn,
                layout.sc(CUE_CONTROL_CALL_BTNS_SIZE),
                Some(COL_BLACK),
                None,
                "▶",
                Some(if call_led { COL_AMBER } else { COL_BTN_TEXT }),
                None,
                layout.sc(CUE_CONTROL_CALL_BTNS_FONT_SIZE),
                None,
                Some(call_border),
                "call_next",
                miso_frame::BTN_CALL_NEXT,
            );

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
            self.circle_btn(
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
                miso_frame::BTN_DELETE,
            );

            self.circle_btn(
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
                miso_frame::BTN_MEMORY,
            );
        }
    }
}
