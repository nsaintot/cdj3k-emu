//! The setup window's palette, metrics and widget style.
//!
//! One language, "Panel": a paper ground, graphite ink, hairline edges and
//! flat plates. Colour is a signal - the only hues on screen belong to a deck
//! or to the state of a run. Every step of the window takes its colours from
//! the [`Palette`] the system theme picks, and every widget drawn under the
//! window is styled once by [`apply_setup_style`] rather than tinted call by
//! call.

use egui::{Color32, Context, Rounding, Stroke, Vec2};

/// Every colour the setup window draws with.
pub(in crate::app) struct Palette {
    /// The window's ground.
    pub paper: Color32,
    /// Cards, bars and the raised parts of a control.
    pub plate: Color32,
    pub plate_hover: Color32,
    /// Sunk areas: the log.
    pub field: Color32,
    pub line: Color32,
    /// The edge of anything the user can click or type into.
    pub line_strong: Color32,
    pub ink: Color32,
    pub muted: Color32,
    /// Field labels and hints.
    pub dim: Color32,
    /// The quietest text the window uses: the era line, the build number.
    pub faint: Color32,
    pub warn: Color32,
    pub ok: Color32,
    pub danger: Color32,
    /// A filled control under the pointer: the ink and the danger, lifted.
    pub ink_hover: Color32,
    pub danger_hover: Color32,
    /// Text on a filled ink or danger control.
    pub on_ink: Color32,
    /// A control that is offering nothing: its fill, its edge, its label.
    pub off: Color32,
    pub off_line: Color32,
    pub off_text: Color32,
    /// The deck miniature: its outline, and the finer parts inside it.
    pub glyph: Color32,
    pub glyph_detail: Color32,
}

pub(in crate::app) const LIGHT: Palette = Palette {
    paper: Color32::from_rgb(242, 240, 234),
    plate: Color32::from_rgb(251, 250, 247),
    plate_hover: Color32::from_rgb(255, 255, 255),
    field: Color32::from_rgb(236, 233, 226),
    line: Color32::from_rgb(218, 214, 204),
    line_strong: Color32::from_rgb(189, 184, 172),
    ink: Color32::from_rgb(23, 23, 26),
    muted: Color32::from_rgb(85, 82, 75),
    dim: Color32::from_rgb(99, 96, 90),
    faint: Color32::from_rgb(141, 138, 129),
    warn: Color32::from_rgb(138, 90, 8),
    ok: Color32::from_rgb(31, 107, 69),
    danger: Color32::from_rgb(163, 50, 36),
    ink_hover: Color32::from_rgb(52, 52, 58),
    danger_hover: Color32::from_rgb(190, 64, 50),
    on_ink: Color32::from_rgb(251, 250, 247),
    off: Color32::from_rgb(242, 240, 234),
    off_line: Color32::from_rgb(213, 208, 197),
    off_text: Color32::from_rgb(155, 151, 141),
    glyph: Color32::from_rgb(43, 42, 40),
    glyph_detail: Color32::from_rgb(122, 118, 108),
};

pub(in crate::app) const DARK: Palette = Palette {
    paper: Color32::from_rgb(27, 27, 25),
    plate: Color32::from_rgb(35, 35, 32),
    plate_hover: Color32::from_rgb(43, 43, 39),
    field: Color32::from_rgb(20, 20, 18),
    line: Color32::from_rgb(52, 51, 46),
    line_strong: Color32::from_rgb(74, 72, 66),
    ink: Color32::from_rgb(240, 238, 232),
    muted: Color32::from_rgb(165, 161, 153),
    dim: Color32::from_rgb(142, 138, 129),
    faint: Color32::from_rgb(124, 120, 111),
    warn: Color32::from_rgb(217, 162, 60),
    ok: Color32::from_rgb(95, 190, 138),
    danger: Color32::from_rgb(224, 112, 92),
    ink_hover: Color32::from_rgb(255, 255, 255),
    danger_hover: Color32::from_rgb(235, 133, 116),
    on_ink: Color32::from_rgb(27, 27, 25),
    off: Color32::from_rgb(27, 27, 25),
    off_line: Color32::from_rgb(46, 45, 41),
    off_text: Color32::from_rgb(106, 103, 95),
    glyph: Color32::from_rgb(232, 229, 222),
    glyph_detail: Color32::from_rgb(142, 138, 129),
};

/// The palette the system theme asks for. eframe follows the desktop setting,
/// so the window is light unless the desktop is dark.
pub(in crate::app) fn palette(ctx: &Context) -> &'static Palette {
    if ctx.style().visuals.dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

// ── Metrics ───────────────────────────────────────────────────────────────────
// Authored against the window's own width; every user multiplies by `k`.

/// Controls and plates. Nothing in the window is softer than this.
pub(in crate::app) const ROUND: f32 = 3.0;
/// Chips, which are small enough that the control radius reads as a pill.
pub(in crate::app) const ROUND_CHIP: f32 = 2.0;
/// Every edge in the window is one of these.
pub(in crate::app) const HAIRLINE: f32 = 1.0;

/// The identity strip at the top, and the action bar at the foot.
pub(in crate::app) const TOPBAR_H: f32 = 44.0;
pub(in crate::app) const FOOTER_H: f32 = 56.0;
/// The page margin every step lines up on.
pub(in crate::app) const GUTTER: f32 = 24.0;

/// Text fields, combos and the button beside them.
pub(in crate::app) const FIELD_H: f32 = 34.0;
/// The size every action button in the footer takes, so a row of them lines
/// up whatever the labels say.
pub(in crate::app) const BUTTON_SIZE: [f32; 2] = [104.0, 36.0];
/// A card's foot pill, which is the one thing that card does.
pub(in crate::app) const PILL_H: f32 = 36.0;
/// A glyph beside a button's label: its side as a share of the button's
/// height, and the space it keeps from the label.
const ICON_RATIO: f32 = 0.36;
const ICON_GAP: f32 = 7.0;

// ── Type ──────────────────────────────────────────────────────────────────────

/// The screen's own title.
pub(in crate::app) const TITLE_FONT: f32 = 24.0;
/// The line under it that says what the step is for.
pub(in crate::app) const SUB_FONT: f32 = 13.0;
pub(in crate::app) const BODY_FONT: f32 = 13.0;
pub(in crate::app) const BUTTON_FONT: f32 = 12.5;
pub(in crate::app) const HINT_FONT: f32 = 11.0;
/// Field labels and the identity strip: small, wide-tracked capitals.
pub(in crate::app) const MICRO_FONT: f32 = 10.0;
pub(in crate::app) const MICRO_TRACK: f32 = 1.4;
/// The wordmark, which is tracked wider still.
pub(in crate::app) const WORDMARK_TRACK: f32 = 1.8;

/// The one thing a step is for: ink, with the paper showing through the text.
/// An `egui::Button` given a fixed fill keeps it in every state, so the fill
/// goes on the widget style instead and the hover survives.
pub(in crate::app) fn primary(
    ui: &mut egui::Ui,
    size: [f32; 2],
    label: &str,
    pal: &Palette,
) -> egui::Response {
    filled(ui, size, label, pal.on_ink, pal.ink, pal.ink_hover)
}

/// The one thing a step is for, when doing it destroys an installation.
pub(in crate::app) fn destructive(
    ui: &mut egui::Ui,
    size: [f32; 2],
    label: &str,
    pal: &Palette,
) -> egui::Response {
    filled(ui, size, label, pal.on_ink, pal.danger, pal.danger_hover)
}

/// Destructive, but not what the step is for: an outline that fills as the
/// pointer lands, so it never competes with the step's own action. The button
/// is drawn rather than built from an `egui::Button`, so the caller's glyph
/// and the label sit as one centred group.
pub(in crate::app) fn destructive_line(
    ui: &mut egui::Ui,
    size: [f32; 2],
    label: &str,
    pal: &Palette,
    icon: impl FnOnce(&egui::Painter, egui::Rect, Color32),
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::from(size), egui::Sense::click());
    let hot = resp.hovered() || resp.is_pointer_button_down_on();
    let (fill, fg) = if hot {
        (pal.danger, pal.on_ink)
    } else {
        (pal.plate, pal.danger)
    };
    let p = ui.painter();
    p.rect_filled(rect, Rounding::same(ROUND), fill);
    p.rect_stroke(
        rect,
        Rounding::same(ROUND),
        Stroke::new(HAIRLINE, pal.danger),
    );

    // Glyph and label are laid out as one block and centred together, so the
    // button reads as a single thing whatever the slot number does to the
    // label's width.
    let galley = p.layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(BUTTON_FONT),
        fg,
    );
    let side = (rect.height() * ICON_RATIO).round();
    let left = rect.center().x - (side + ICON_GAP + galley.size().x) * 0.5;
    icon(
        p,
        egui::Rect::from_center_size(
            egui::Pos2::new(left + side * 0.5, rect.center().y),
            Vec2::splat(side),
        ),
        fg,
    );
    let text_pos = egui::Pos2::new(
        left + side + ICON_GAP,
        rect.center().y - galley.size().y * 0.5,
    );
    p.galley(text_pos, galley, fg);
    pointer(resp)
}

/// The step's one action, when it cannot be taken yet. A filled button that is
/// merely dimmed still reads as the loudest thing on the step, so this goes
/// quiet instead - and takes no pointer, having nothing to offer it.
pub(in crate::app) fn primary_off(
    ui: &mut egui::Ui,
    size: [f32; 2],
    label: &str,
    pal: &Palette,
) -> egui::Response {
    ui.add_sized(
        size,
        egui::Button::new(
            egui::RichText::new(label.to_owned())
                .size(BUTTON_FONT)
                .color(pal.off_text),
        )
        .fill(pal.off)
        .stroke(Stroke::new(HAIRLINE, pal.off_line))
        .rounding(Rounding::same(ROUND)),
    )
}

fn filled(
    ui: &mut egui::Ui,
    size: [f32; 2],
    label: &str,
    text: Color32,
    rest: Color32,
    hovered: Color32,
) -> egui::Response {
    let mut resp = None;
    ui.scope(|ui| {
        let w = &mut ui.style_mut().visuals.widgets;
        for (state, fill) in [
            (&mut w.inactive, rest),
            (&mut w.hovered, hovered),
            (&mut w.active, hovered),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(HAIRLINE, fill);
            state.expansion = 0.0;
        }
        resp = Some(
            ui.add_sized(
                size,
                egui::Button::new(
                    egui::RichText::new(label.to_owned())
                        .size(BUTTON_FONT)
                        .strong()
                        .color(text),
                )
                .rounding(Rounding::same(ROUND)),
            ),
        );
    });
    pointer(resp.expect("the scope always runs"))
}

/// Anything offered beside it. Left to the window's own widget style, so it
/// lifts on hover like the rest.
pub(in crate::app) fn secondary(
    ui: &mut egui::Ui,
    size: [f32; 2],
    label: &str,
    pal: &Palette,
) -> egui::Response {
    pointer(
        ui.add_sized(
            size,
            egui::Button::new(
                egui::RichText::new(label.to_owned())
                    .size(BUTTON_FONT)
                    .color(pal.ink),
            ),
        ),
    )
}

/// Anything the pointer can act on says so.
pub(in crate::app) fn pointer(resp: egui::Response) -> egui::Response {
    if resp.enabled() {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Style every widget drawn under `ui` to `pal`.
pub(in crate::app) fn apply_setup_style(ui: &mut egui::Ui, pal: &Palette) {
    let style = ui.style_mut();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 7.0);
    style.spacing.interact_size.y = FIELD_H;
    style.spacing.combo_height = 240.0;
    style.spacing.window_margin = egui::Margin::same(8.0);

    let v = &mut style.visuals;
    v.panel_fill = Color32::TRANSPARENT;
    v.extreme_bg_color = pal.field;
    v.faint_bg_color = pal.plate;
    v.window_fill = pal.plate;
    v.window_stroke = Stroke::new(HAIRLINE, pal.line);
    v.window_rounding = Rounding::same(ROUND);
    v.menu_rounding = Rounding::same(ROUND);
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;
    // The row a list already holds: a sunk band, not a highlight. egui's own
    // selection blue is the one colour in this window that belongs to nothing.
    v.selection.bg_fill = pal.field;
    v.selection.stroke = Stroke::new(HAIRLINE, pal.ink);
    // egui draws a disabled widget from `noninteractive`: a control with
    // nothing to offer goes quiet rather than dimming the window behind it.
    // Every label in the window names its own colour, so this is the disabled
    // look and nothing else.
    let n = &mut v.widgets.noninteractive;
    n.bg_fill = pal.off;
    n.weak_bg_fill = pal.off;
    n.bg_stroke = Stroke::new(HAIRLINE, pal.off_line);
    n.fg_stroke = Stroke::new(HAIRLINE, pal.off_text);
    n.rounding = Rounding::same(ROUND);

    // Rest, hover, press: the plate lifts and the edge darkens, which is the
    // same move the cards make. Nothing grows, and nothing casts a shadow.
    for (w, fill, edge, fg) in [
        (&mut v.widgets.inactive, pal.plate, pal.line_strong, pal.ink),
        (&mut v.widgets.hovered, pal.plate_hover, pal.ink, pal.ink),
        (&mut v.widgets.active, pal.plate_hover, pal.ink, pal.ink),
        (&mut v.widgets.open, pal.plate_hover, pal.ink, pal.ink),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = Stroke::new(HAIRLINE, edge);
        w.fg_stroke = Stroke::new(HAIRLINE, fg);
        w.rounding = Rounding::same(ROUND);
        w.expansion = 0.0;
    }
}
