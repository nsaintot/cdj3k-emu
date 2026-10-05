//! The row under the screen: DELETE and its lamp, the eight HOT CUE keys,
//! and the BACK / TAG TRACK pair.
//!
//! Positions are drawing pixels, converted by [`px`]; captions are fitted to
//! the ink boxes the drawing prints them in.

use cdj3k_emu_panel::{Btn, Lamp};
use egui::{Color32, FontFamily, Pos2, Rect, Stroke, Vec2};

use crate::app::ui::draw_cache::ShapeList;
use crate::app::ui::{COL_BTN_CUE, COL_BTN_TEXT, COL_DARK, COL_SILVER};
use crate::app::CdjApp;

use super::key::{self, Outline};
use super::layout::px;
use super::UiScale;

// ── DELETE ────────────────────────────────────────────────────────────────────
const DELETE_CENTER: (f32, f32) = (124.8, 1050.9);
/// Outer edge of the round key; the profile insets from it.
const DELETE_R: f32 = 27.5;
const DELETE_CAPTION: (f32, f32, f32, f32) = (85.0, 994.0, 167.0, 1009.0);

/// The power lamp left of DELETE: a ring around a dot, red while the deck
/// sleeps.
const DELETE_LAMP_CENTER: (f32, f32) = (62.5, 1050.9);
const DELETE_LAMP_R: f32 = 4.5;
const COL_STANDBY: Color32 = Color32::from_rgb(235, 40, 30);

// ── HOT CUE ───────────────────────────────────────────────────────────────────
/// Outer edge of key A; the others follow at [`HOT_CUE_PITCH`].
const HOT_CUE_A: (f32, f32, f32, f32) = (247.0, 1016.0, 349.5, 1086.0);
const HOT_CUE_PITCH: f32 = 119.0;
const HOT_CUE_R: f32 = 9.0;
/// The letter's ink box, from the key's left and top edge.
const HOT_CUE_LETTER_OFF: (f32, f32) = (19.0, 18.0);
const HOT_CUE_LETTER_H: f32 = 14.0;
const HOT_CUE_CAPTION: (f32, f32, f32, f32) = (667.0, 987.0, 763.0, 1002.0);
const HOT_CUE_KEYS: [(&str, Btn); 8] = [
    ("A", Btn::HotA),
    ("B", Btn::HotB),
    ("C", Btn::HotC),
    ("D", Btn::HotD),
    ("E", Btn::HotE),
    ("F", Btn::HotF),
    ("G", Btn::HotG),
    ("H", Btn::HotH),
];
/// The letters are amber LEDs; the deck's pad colour sets their brightness.
const COL_HOT_CUE_LIT: Color32 = COL_BTN_CUE;

// ── BACK / TAG TRACK ──────────────────────────────────────────────────────────
/// The pair's outer edge: two half-stadiums meeting flat sides in the middle.
const PAIR: (f32, f32, f32, f32) = (1226.5, 1023.0, 1381.5, 1078.0);
/// BACK's flat side, and TAG TRACK's, with the gap between them.
const PAIR_BACK_RIGHT: f32 = 1303.0;
const PAIR_TAG_LEFT: f32 = 1305.0;
/// Corner radius where a half meets the gap.
const PAIR_FLAT_R: f32 = 3.0;
/// The dash moulded into BACK's cap, and its shade.
const BACK_NOTCH: (f32, f32, f32, f32) = (1245.0, 1048.0, 1261.0, 1054.0);
const COL_NOTCH: Color32 = Color32::from_rgb(70, 70, 78);
const BACK_CAPTION: (f32, f32, f32, f32) = (1242.0, 997.0, 1289.0, 1009.0);
const TAG_CAPTION: (f32, f32, f32, f32) = (1326.0, 988.0, 1359.0, 1000.0);
const TRACK_CAPTION: (f32, f32, f32, f32) = (1314.0, 1005.0, 1371.0, 1017.0);

const LAMP_STROKE: f32 = 2.0;

fn caption_family() -> FontFamily {
    FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS_BOLD.into())
}

fn letter_family() -> FontFamily {
    FontFamily::Name(cdj3k_emu_platform::fonts::NIMBUS_SANS.into())
}

/// A drawing-pixel box as a screen rect.
fn screen_box(layout: &UiScale, (l, t, r, b): (f32, f32, f32, f32)) -> Rect {
    Rect::from_min_max(layout.sp(px(l), px(t)), layout.sp(px(r), px(b)))
}

/// A drawing-pixel box as a reference rect.
fn ref_box((l, t, r, b): (f32, f32, f32, f32)) -> Rect {
    Rect::from_min_max(Pos2::new(px(l), px(t)), Pos2::new(px(r), px(b)))
}

/// Captions and the DELETE lamp (cached; rebuilt on resize).
pub(super) fn collect_top_statics(list: &mut ShapeList, ctx: &egui::Context, layout: &UiScale) {
    for (text, ink) in [
        ("DELETE", DELETE_CAPTION),
        ("HOT CUE", HOT_CUE_CAPTION),
        ("BACK", BACK_CAPTION),
        ("TAG", TAG_CAPTION),
        ("TRACK", TRACK_CAPTION),
    ] {
        list.text_in_ink_box(
            ctx,
            screen_box(layout, ink),
            text,
            caption_family(),
            COL_BTN_TEXT,
        );
    }

    let lamp = layout.sp(px(DELETE_LAMP_CENTER.0), px(DELETE_LAMP_CENTER.1));
    list.circle_filled(lamp, layout.sc(px(DELETE_LAMP_R)), COL_DARK);
    list.circle_stroke(
        lamp,
        layout.sc(px(DELETE_LAMP_R)),
        Stroke::new(layout.sc(LAMP_STROKE), COL_SILVER.gamma_multiply(0.5)),
    );
}

/// The row's keys.
pub(super) fn draw_top_section(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    draw_delete(app, ui, layout);
    draw_hot_cues(app, ui, layout);
    draw_back_tag(app, ui, layout);
}

fn delete_outline() -> Outline {
    let c = Pos2::new(px(DELETE_CENTER.0), px(DELETE_CENTER.1));
    let r = px(DELETE_R);
    Outline::new(Rect::from_center_size(c, Vec2::splat(2.0 * r)), [r; 4])
}

fn hot_cue_outline(i: usize) -> Outline {
    let dx = i as f32 * HOT_CUE_PITCH;
    let (l, t, r, b) = HOT_CUE_A;
    Outline::new(ref_box((l + dx, t, r + dx, b)), [px(HOT_CUE_R); 4])
}

fn back_tag_outlines() -> [Outline; 2] {
    let (l, t, r, b) = PAIR;
    let end_r = px(b - t) * 0.5;
    let flat = px(PAIR_FLAT_R);
    [
        Outline::new(
            ref_box((l, t, PAIR_BACK_RIGHT, b)),
            [end_r, flat, flat, end_r],
        ),
        Outline::new(
            ref_box((PAIR_TAG_LEFT, t, r, b)),
            [flat, end_r, end_r, flat],
        ),
    ]
}

/// The caps of the row's keys, reference units: what stands above the
/// panel when the deck tilts.
pub(super) fn footprints() -> Vec<Vec<Pos2>> {
    std::iter::once(delete_outline())
        .chain((0..HOT_CUE_KEYS.len()).map(hot_cue_outline))
        .chain(back_tag_outlines())
        .map(|o| key::face(o).ref_points())
        .collect()
}

fn draw_delete(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    // The lamp's ring is drawn with the statics; lit, its dot covers the
    // dark fill inside it.
    if app.mosi().lit(Lamp::Standby) {
        let lamp = layout.sp(px(DELETE_LAMP_CENTER.0), px(DELETE_LAMP_CENTER.1));
        ui.painter().circle_filled(
            lamp,
            layout.sc(px(DELETE_LAMP_R)) - layout.sc(LAMP_STROKE) * 0.5,
            COL_STANDBY,
        );
    }
    let outline = delete_outline();
    let hit = outline.screen_rect(layout);
    app.shape_btn(ui, hit, 0, "delete", Some(Btn::Delete), |list, pressed| {
        key::collect_key(list, layout, outline, pressed);
    });
}

fn draw_hot_cues(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let ctx = ui.ctx().clone();
    for (i, (letter, btn)) in HOT_CUE_KEYS.iter().enumerate() {
        let dx = i as f32 * HOT_CUE_PITCH;
        let (l, t, ..) = HOT_CUE_A;
        let outline = hot_cue_outline(i);
        let letter_col = key::led(COL_HOT_CUE_LIT, app.mosi().drive(Lamp::HOT_CUES[i]));
        let ink_top_left = layout.sp(
            px(l + dx + HOT_CUE_LETTER_OFF.0),
            px(t + HOT_CUE_LETTER_OFF.1),
        );
        let ink_h = layout.sc(px(HOT_CUE_LETTER_H));
        let hit = outline.screen_rect(layout);
        let state = u32::from_le_bytes(letter_col.to_array());
        app.shape_btn(
            ui,
            hit,
            state,
            ("hotcue", i),
            Some(*btn),
            |list, pressed| {
                key::collect_key(list, layout, outline, pressed);
                collect_letter(list, &ctx, ink_top_left, ink_h, letter, letter_col);
            },
        );
    }
}

/// A key letter, its ink `h` tall with its top-left at `at`, at its own width.
fn collect_letter(
    list: &mut ShapeList,
    ctx: &egui::Context,
    at: Pos2,
    h: f32,
    letter: &str,
    col: Color32,
) {
    let font = egui::FontId::new(100.0, letter_family());
    let probe = ctx.fonts(|f| f.layout_no_wrap(letter.to_owned(), font, col));
    let k = h / probe.mesh_bounds.height().max(1.0);
    let font = egui::FontId::new(100.0 * k, letter_family());
    let galley = ctx.fonts(|f| f.layout_no_wrap(letter.to_owned(), font, col));
    let top_left = at - galley.mesh_bounds.min.to_vec2();
    list.add(egui::Shape::galley(top_left, galley, col));
}

fn draw_back_tag(app: &mut CdjApp, ui: &mut egui::Ui, layout: &UiScale) {
    let [back, tag] = back_tag_outlines();
    for (outline, id, btn, notch) in [
        (back, "back", Btn::Back, true),
        (tag, "tag_track", Btn::TagTrack, false),
    ] {
        let hit = outline.screen_rect(layout);
        app.shape_btn(ui, hit, 0, id, Some(btn), |list, pressed| {
            key::collect_key(list, layout, outline, pressed);
            if notch {
                let n = ref_box(BACK_NOTCH);
                list.rect_filled(
                    Rect::from_min_max(
                        layout.sp(n.left(), n.top()),
                        layout.sp(n.right(), n.bottom()),
                    ),
                    layout.sc(n.height() * 0.5),
                    COL_NOTCH,
                );
            }
        });
    }
}
