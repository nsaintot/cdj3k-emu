//! The dialogs over the list: remove, replace, add from a URL.

use super::*;

pub(super) fn draw_replace(
    ui: &mut egui::Ui,
    name: &str,
    old: &str,
    new: &str,
    k: f32,
    pal: &Palette,
) -> Option<ModsAction> {
    let title = format!("Replace {name}?");
    let text = format!(
        "{name} {} is installed. Replace it with {}?",
        version_label(old),
        version_label(new),
    );
    let mut action = None;
    modal(ui, &title, &text, k, pal, |ui| {
        if theme::primary(ui, [BACK_W * k, theme::BUTTON_SIZE[1] * k], "Replace", pal).clicked() {
            action = Some(ModsAction::Replace);
        }
        if theme::secondary(ui, [BACK_W * k, theme::BUTTON_SIZE[1] * k], "Cancel", pal).clicked() {
            action = Some(ModsAction::KeepOld);
        }
    });
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some(ModsAction::KeepOld);
    }
    action
}

pub(super) fn draw_remove(
    ui: &mut egui::Ui,
    slot: u32,
    name: &str,
    k: f32,
    pal: &Palette,
) -> Option<ModsAction> {
    let title = format!("Remove {name}?");
    let text = format!(
        "Its files are deleted from slot {slot}. The running deck keeps it until its next \
         restart."
    );
    let mut action = None;
    modal(ui, &title, &text, k, pal, |ui| {
        if theme::destructive(ui, [BACK_W * k, theme::BUTTON_SIZE[1] * k], "Remove", pal).clicked()
        {
            action = Some(ModsAction::Remove(name.to_string()));
        }
        if theme::secondary(ui, [BACK_W * k, theme::BUTTON_SIZE[1] * k], "Cancel", pal).clicked() {
            action = Some(ModsAction::CancelRemove);
        }
    });
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some(ModsAction::CancelRemove);
    }
    action
}

pub(super) fn draw_url(
    ui: &mut egui::Ui,
    page: &mut ModsPage,
    k: f32,
    pal: &Palette,
) -> Option<ModsAction> {
    let text = "A .tgz or .tar archive of a mod".to_string();
    let mut action = None;
    let mut url = page.url.take().unwrap_or_default();
    let ready = url.trim().starts_with("https://") || url.trim().starts_with("http://");
    let submit = std::cell::Cell::new(false);
    modal_with(
        ui,
        "Add a mod from a URL",
        &text,
        theme::FIELD_H * k,
        k,
        pal,
        |ui| {
            // The margin, not the widget's height, centres the line.
            let font = FontId::monospace(URL_FONT * k);
            let row = ui.fonts_mut(|f| f.row_height(&font));
            let pad_y = ((theme::FIELD_H * k - row) * 0.5).max(0.0);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut url)
                    .desired_width(ui.available_width())
                    .min_size(Vec2::new(0.0, theme::FIELD_H * k))
                    .margin(egui::epaint::MarginF32::symmetric(URL_MARGIN * k, pad_y))
                    .hint_text("https://…/mod.tgz")
                    .font(font),
            );
            // Enter submits: the field is the dialog's only input, so it
            // keeps the focus and the key is read from the frame's input.
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit.set(true);
            }
            edit.request_focus();
        },
        |ui| {
            let add = if ready {
                theme::primary(ui, [BACK_W * k, theme::BUTTON_SIZE[1] * k], "Add", pal)
            } else {
                theme::primary_off(ui, [BACK_W * k, theme::BUTTON_SIZE[1] * k], "Add", pal)
            };
            if ready && add.clicked() {
                submit.set(true);
            }
            if theme::secondary(ui, [BACK_W * k, theme::BUTTON_SIZE[1] * k], "Cancel", pal)
                .clicked()
            {
                action = Some(ModsAction::CancelUrl);
            }
        },
    );
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some(ModsAction::CancelUrl);
    }
    if submit.get() && ready && action.is_none() {
        action = Some(ModsAction::AddUrl(url.trim().to_string()));
    }
    page.url = Some(url);
    action
}

/// A modal over the whole window with a title, a paragraph and buttons laid
/// right to left.
pub(super) fn modal(
    ui: &mut egui::Ui,
    title: &str,
    text: &str,
    k: f32,
    pal: &Palette,
    buttons: impl FnOnce(&mut egui::Ui),
) {
    modal_with(ui, title, text, 0.0, k, pal, |_| {}, buttons);
}

/// [`modal`] with `extra_h` of room under the paragraph for `extra`.
#[allow(clippy::too_many_arguments)]
pub(super) fn modal_with(
    ui: &mut egui::Ui,
    title: &str,
    text: &str,
    extra_h: f32,
    k: f32,
    pal: &Palette,
    extra: impl FnOnce(&mut egui::Ui),
    buttons: impl FnOnce(&mut egui::Ui),
) {
    let area = ui.max_rect();
    // While the modal is up, nothing behind it receives clicks.
    ui.interact(
        area,
        ui.id().with("mods_modal_block"),
        Sense::click_and_drag(),
    );
    let w = (MODAL_W * k).min(area.width() - 2.0 * theme::GUTTER * k);
    let p = ui.painter().clone();
    p.rect_filled(area, 0.0, MODAL_SCRIM);
    let wrap = w - 2.0 * MODAL_PAD_X * k;
    let galley = p.layout(
        text.to_owned(),
        FontId::proportional(theme::BODY_FONT * k),
        pal.muted,
        wrap,
    );
    let title_galley = p.layout(
        title.to_owned(),
        FontId::proportional(MODAL_TITLE_FONT * k),
        pal.ink,
        wrap,
    );
    let top = area.top() + area.height() * MODAL_TOP;
    let left = area.center().x - w * 0.5;
    let x = left + MODAL_PAD_X * k;
    let title_top = top + MODAL_PAD_TOP * k;
    let text_top = title_top + title_galley.size().y + MODAL_TEXT_GAP * k;
    let mut bottom = text_top + galley.size().y;
    let extra_rect = (extra_h > 0.0).then(|| {
        let r = Rect::from_min_size(
            Pos2::new(x, bottom + MODAL_TEXT_GAP * k * 1.5),
            Vec2::new(wrap, extra_h),
        );
        bottom = r.bottom();
        r
    });
    let bar_top = bottom + MODAL_BODY_BOTTOM * k;
    let bar_h = theme::BUTTON_SIZE[1] * k + 2.0 * MODAL_BAR_PAD * k;
    let card = Rect::from_min_max(Pos2::new(left, top), Pos2::new(left + w, bar_top + bar_h));
    p.rect_filled(card, theme::ROUND * k, pal.plate);
    p.rect_stroke(
        card,
        theme::ROUND * k,
        Stroke::new(theme::HAIRLINE * k, pal.line_strong),
        egui::StrokeKind::Middle,
    );
    p.galley(Pos2::new(x, title_top), title_galley, pal.ink);
    p.galley(Pos2::new(x, text_top), galley, pal.muted);
    hairline(
        &p,
        Pos2::new(card.left(), bar_top),
        Pos2::new(card.right(), bar_top),
        pal.line,
        k,
    );
    if let Some(r) = extra_rect {
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            theme::apply_setup_style(ui, pal);
            extra(ui);
        });
    }
    let bar = Rect::from_min_max(
        Pos2::new(card.left() + MODAL_PAD_X * k, bar_top),
        Pos2::new(card.right() - MODAL_PAD_X * k, card.bottom()),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), buttons);
    });
}
