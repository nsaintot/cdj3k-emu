//! The main LCD: its glass in a thin bezel, the guest's display stream on it,
//! and touch.
//!
//! The same part on every player; a slate supplies only where the bezel sits.

use egui::{Color32, FontId, Pos2, Rect};

use crate::app::CdjApp;

use super::{
    draw_bordered_rect_section, DoubleBorderSpec, StrokeSpec, UiScale, COL_LCD_BG, COL_SILVER,
};

/// Bezel width around the glass.
const LCD_BEZEL_REF: f32 = 4.0;
/// Corner radius of the bezel.
const LCD_ROUNDING_REF: f32 = 10.0;
/// Placeholder text while the stream is absent or popped out.
const LCD_PLACEHOLDER_FONT_SIZE: f32 = 44.0;

/// Draw the LCD with its bezel's outer edge at `bezel` (reference units).
pub(super) fn draw_main_lcd(
    app: &mut CdjApp,
    ui: &mut egui::Ui,
    p: &egui::Painter,
    layout: &UiScale,
    bezel: Rect,
) {
    let bezel_rounding = layout.sc(LCD_ROUNDING_REF);
    let bezel_rect = layout.sr(bezel.left(), bezel.top(), bezel.width(), bezel.height());
    p.rect_filled(bezel_rect, bezel_rounding, Color32::from_rgb(10, 10, 12));

    let bezel_px = layout.sc(LCD_BEZEL_REF);
    let display_rect = bezel_rect.shrink(bezel_px);
    app.bloom_excludes.push(display_rect);
    let display_rounding = (bezel_rounding - bezel_px).max(2.0);
    p.rect_filled(display_rect, display_rounding, COL_LCD_BG);

    let uv_full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if app.main_screen_popped {
        p.text(
            display_rect.center(),
            egui::Align2::CENTER_CENTER,
            "↗ external window",
            FontId::proportional(layout.sc(LCD_PLACEHOLDER_FONT_SIZE)),
            Color32::from_rgb(60, 60, 70),
        );
    } else if !app.lcds_blanked {
        if let Some(tex_id) = app.display_tex_id {
            if app.display_stream.is_connected() {
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
        if !app.display_stream.is_connected() {
            p.text(
                display_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("connecting {}…", app.display_stream.addr_str()),
                FontId::proportional(layout.sc(LCD_PLACEHOLDER_FONT_SIZE)),
                Color32::from_rgb(60, 60, 70),
            );
        }
    }

    // Touch is handled by the popout viewport when the LCD is detached.
    if !app.main_screen_popped {
        let lcd_resp = ui.interact(
            display_rect,
            ui.id().with("lcd_touch"),
            egui::Sense::click_and_drag(),
        );
        app.apply_lcd_touch(crate::app::LcdTouchCapture {
            hovered: lcd_resp.hovered(),
            scroll_y: ui.input(|i| i.raw_scroll_delta.y),
            pointer_moved: ui.input(|i| i.pointer.delta().length_sq() > 0.0),
            is_down: lcd_resp.is_pointer_button_down_on(),
            ctrl: ui.input(|i| i.modifiers.ctrl),
            right_down: ui.input(|i| {
                i.pointer.button_down(egui::PointerButton::Secondary)
                    && i.pointer
                        .hover_pos()
                        .is_some_and(|p| display_rect.contains(p))
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
