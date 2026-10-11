//! The setup window's palette, metrics and widget style.
//!
//! The window uses a paper ground, graphite ink, hairline edges and flat
//! plates. The only hues on screen mark a deck or the state of a run. Every
//! step takes its colours from the [`Palette`] that the system theme picks,
//! and [`apply_setup_style`] styles every widget in the window once.

use egui::{Color32, Context, CornerRadius, Stroke, Vec2};

/// Every colour the setup window draws with.
pub(in crate::app) struct Palette {
    /// The window's ground.
    pub paper: Color32,
    /// Cards, bars and the raised parts of a control.
    pub plate: Color32,
    pub plate_hover: Color32,
    /// Sunk areas, such as the log.
    pub field: Color32,
    pub line: Color32,
    /// The edge of anything the user can click or type into.
    pub line_strong: Color32,
    pub ink: Color32,
    pub muted: Color32,
    /// Field labels and hints.
    pub dim: Color32,
    /// The faintest text in the window, such as the era line and the build
    /// number.
    pub faint: Color32,
    pub warn: Color32,
    pub ok: Color32,
    pub danger: Color32,
    /// A link that opens outside the app, such as the docs or a template.
    pub link: Color32,
    /// The hover colours of the filled ink and danger controls.
    pub ink_hover: Color32,
    pub danger_hover: Color32,
    /// Text on a filled ink or danger control.
    pub on_ink: Color32,
    /// The fill, edge and label colours of a disabled control.
    pub off: Color32,
    pub off_line: Color32,
    pub off_text: Color32,
    /// The deck miniature's outline, and the finer parts inside it.
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
    link: Color32::from_rgb(29, 86, 200),
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
    link: Color32::from_rgb(122, 162, 240),
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
    if ctx.theme() == egui::Theme::Dark {
        &DARK
    } else {
        &LIGHT
    }
}

// ── Metrics ───────────────────────────────────────────────────────────────────
// The sizes are written for the reference window width; callers multiply them
// by `k`.

/// The corner radius of controls and plates.
pub(in crate::app) const ROUND: f32 = 3.0;
/// The corner radius of chips, which are small enough that the control radius
/// would make them look like pills.
pub(in crate::app) const ROUND_CHIP: f32 = 2.0;
/// The width of every edge in the window.
pub(in crate::app) const HAIRLINE: f32 = 1.0;

/// The heights of the identity strip at the top and the action bar at the
/// foot.
pub(in crate::app) const TOPBAR_H: f32 = 44.0;
pub(in crate::app) const FOOTER_H: f32 = 56.0;
/// The page margin every step lines up on.
pub(in crate::app) const GUTTER: f32 = 24.0;

/// The height of text fields, combos and the button beside them.
pub(in crate::app) const FIELD_H: f32 = 34.0;
/// The size of every action button in the footer, so a row of them lines up
/// whatever the labels are.
pub(in crate::app) const BUTTON_SIZE: [f32; 2] = [104.0, 36.0];
/// The height of a card's foot pill, which names the card's action.
pub(in crate::app) const PILL_H: f32 = 36.0;
/// A glyph beside a button's label: its side as a share of the button's
/// height, and the space it keeps from the label.
const ICON_RATIO: f32 = 0.36;
const ICON_GAP: f32 = 7.0;

// ── Type ──────────────────────────────────────────────────────────────────────

/// The screen's own title.
pub(in crate::app) const TITLE_FONT: f32 = 24.0;
/// The line under the title that says what the step is for.
pub(in crate::app) const SUB_FONT: f32 = 13.0;
pub(in crate::app) const BODY_FONT: f32 = 13.0;
pub(in crate::app) const BUTTON_FONT: f32 = 12.5;
pub(in crate::app) const HINT_FONT: f32 = 11.0;
/// Field labels and the identity strip: small, wide-tracked capitals.
pub(in crate::app) const MICRO_FONT: f32 = 10.0;
pub(in crate::app) const MICRO_TRACK: f32 = 1.4;
/// The wordmark's tracking, which is wider still.
pub(in crate::app) const WORDMARK_TRACK: f32 = 1.8;

/// The step's main action: an ink button with paper-coloured text. An
/// `egui::Button` with a fixed fill keeps that fill in every state, so the
/// fill is set on the widget style and the hover colour still applies.
pub(in crate::app) fn primary(
    ui: &mut egui::Ui,
    size: [f32; 2],
    label: &str,
    pal: &Palette,
) -> egui::Response {
    filled(ui, size, label, pal.on_ink, pal.ink, pal.ink_hover)
}

/// The step's main action when it destroys an installation.
pub(in crate::app) fn destructive(
    ui: &mut egui::Ui,
    size: [f32; 2],
    label: &str,
    pal: &Palette,
) -> egui::Response {
    filled(ui, size, label, pal.on_ink, pal.danger, pal.danger_hover)
}

/// A destructive button that is not the step's main action: an outline that
/// fills on hover, so it never competes with the main action. It is drawn by
/// hand so that the caller's glyph and the label are centred as one group.
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
    p.rect_filled(rect, CornerRadius::from(ROUND), fill);
    p.rect_stroke(
        rect,
        CornerRadius::from(ROUND),
        Stroke::new(HAIRLINE, pal.danger),
        egui::StrokeKind::Middle,
    );

    // Lay out the glyph and the label as one block and centre them together,
    // whatever the label's width.
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

/// The step's main action while it is unavailable. It uses the disabled
/// colours, because a dimmed filled button would still be the most prominent
/// control, and it shows no pointing-hand cursor.
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
        .corner_radius(CornerRadius::from(ROUND)),
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
                .corner_radius(CornerRadius::from(ROUND)),
            ),
        );
    });
    pointer(resp.expect("the scope always runs"))
}

/// A secondary action beside the main one. It uses the window's widget style,
/// so it lightens on hover like the other controls.
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

/// Show the pointing-hand cursor over an enabled control.
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
    style.spacing.window_margin = egui::Margin::from(8);

    let v = &mut style.visuals;
    v.panel_fill = Color32::TRANSPARENT;
    v.extreme_bg_color = pal.field;
    v.faint_bg_color = pal.plate;
    v.window_fill = pal.plate;
    v.window_stroke = Stroke::new(HAIRLINE, pal.line);
    v.window_corner_radius = CornerRadius::from(ROUND);
    v.menu_corner_radius = CornerRadius::from(ROUND);
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;
    // Show a list's selected row as a sunk band, because egui's default
    // selection blue matches no colour in the window.
    v.selection.bg_fill = pal.field;
    v.selection.stroke = Stroke::new(HAIRLINE, pal.ink);
    // egui draws a disabled widget from `noninteractive`, so these settings
    // give a disabled control the disabled colours. Every label sets its own
    // colour, so this style affects only disabled controls.
    let n = &mut v.widgets.noninteractive;
    n.bg_fill = pal.off;
    n.weak_bg_fill = pal.off;
    n.bg_stroke = Stroke::new(HAIRLINE, pal.off_line);
    n.fg_stroke = Stroke::new(HAIRLINE, pal.off_text);
    n.corner_radius = CornerRadius::from(ROUND);

    // On hover and press, the plate lightens and the edge takes the ink
    // colour, as on the cards. Widgets do not expand or cast a shadow.
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
        w.corner_radius = CornerRadius::from(ROUND);
        w.expansion = 0.0;
    }
}
