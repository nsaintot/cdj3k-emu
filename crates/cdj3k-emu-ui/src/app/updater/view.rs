//! What the update window shows for each [`Phase`], and the buttons that
//! answer it.

use std::path::Path;
use std::sync::OnceLock;

use cdj3k_emu_platform::app_meta::VERSION;
use cdj3k_emu_update::{Kind, Offer};
use egui::{Pos2, Rect, RichText, Stroke, Vec2};

use super::super::picker::{button_size, draw_footer, draw_header, footer_split, k_of, Header};
use super::super::theme::{self, Palette};
use super::notes::notes_box;
use super::{Action, Phase, Step};

const PROGRESS_H: f32 = 6.0;
const ROW_LABEL_W: f32 = 120.0;

/// Header text for a phase.
struct Page {
    state: Option<&'static str>,
    title: String,
    sub: String,
}

fn page(phase: &Phase) -> Page {
    let (state, title, sub) = match phase {
        Phase::Checking => (
            Some("CHECKING"),
            "Checking for updates".into(),
            format!("Installed: {}", installed()),
        ),
        Phase::Current { latest, .. } => (
            None,
            "Up to date".into(),
            format!("Installed: {} · Latest release: v{latest}", installed()),
        ),
        Phase::Unmanaged => (
            None,
            "Updates unavailable".into(),
            "This build was not installed from a release package.".into(),
        ),
        Phase::NoPackage { latest } => (
            None,
            format!("v{latest} is available"),
            "This release has no package for this installation.".into(),
        ),
        Phase::Available(offer) if !offer.kind.installs_itself() => (
            None,
            format!("v{} is available", offer.version),
            "Download it, then install it with your package manager.".into(),
        ),
        Phase::Available(offer) => (
            None,
            format!("v{} is available", offer.version),
            format!("Installed: {}", installed()),
        ),
        Phase::Downloading { offer, .. } => (
            Some("DOWNLOADING"),
            format!("Downloading v{}", offer.version),
            "You can close this window; the download continues.".into(),
        ),
        Phase::Preparing(offer) => (
            Some("PREPARING"),
            format!("Preparing v{}", offer.version),
            prepare_note(offer.kind).into(),
        ),
        Phase::Ready(offer) => (
            Some("READY"),
            format!("v{} is ready to install", offer.version),
            ready_note(offer.kind).into(),
        ),
        Phase::Downloaded { offer, .. } => (
            Some("DOWNLOADED"),
            format!("v{} is downloaded", offer.version),
            "Install it with your package manager.".into(),
        ),
        Phase::Restarting(Step::Closing) => (
            Some("RESTARTING"),
            "Closing other windows".into(),
            "Their emulations stop first.".into(),
        ),
        Phase::Restarting(Step::Stopping) => (
            Some("RESTARTING"),
            "Stopping the emulation".into(),
            String::new(),
        ),
        Phase::Restarting(Step::Installing(offer)) => (
            Some("INSTALLING"),
            format!("Installing v{}", offer.version),
            install_note(offer.kind).into(),
        ),
        Phase::Restarting(Step::HandedOff) | Phase::Exiting => (
            Some("RESTARTING"),
            "Restarting".into(),
            "The emulator restarts on the new version.".into(),
        ),
        Phase::Failed { .. } => (
            None,
            "Update failed".into(),
            "The installed version is unchanged.".into(),
        ),
    };
    Page { state, title, sub }
}

/// This build's version, package format and architecture, e.g.
/// `v0.3.0 (AppImage, x86_64)`.
fn installed() -> &'static str {
    static TEXT: OnceLock<String> = OnceLock::new();
    TEXT.get_or_init(|| {
        let arch = std::env::consts::ARCH;
        match cdj3k_emu_update::installed_kind().and_then(kind_name) {
            Some(kind) => format!("v{VERSION} ({kind}, {arch})"),
            None => format!("v{VERSION} ({arch})"),
        }
    })
}

fn kind_name(kind: Kind) -> Option<&'static str> {
    Some(match kind {
        Kind::Dmg => "app bundle",
        Kind::Deb => "deb",
        Kind::Rpm => "rpm",
        Kind::AppImage => "AppImage",
        Kind::Inno => "installer",
        Kind::Other => return None,
    })
}

fn prepare_note(kind: Kind) -> &'static str {
    match kind {
        Kind::Dmg => "Copying and verifying the new app.",
        Kind::Inno => "Verifying the installer.",
        Kind::AppImage => "Copying the new AppImage.",
        _ => "",
    }
}

fn ready_note(kind: Kind) -> &'static str {
    match kind {
        Kind::Inno => {
            "Restart now to install, or it installs when you quit. Windows asks \
             for permission."
        }
        _ => "Restart now to install, or it installs when you quit.",
    }
}

fn install_note(kind: Kind) -> &'static str {
    match kind {
        Kind::Dmg => "macOS may ask for an administrator password.",
        Kind::Inno => "Windows asks for permission.",
        Kind::AppImage => "Replacing the AppImage.",
        _ => "",
    }
}

/// The command that installs the deb or rpm at `path`.
fn package_command(kind: Kind, path: &Path) -> Option<String> {
    let path = path.to_string_lossy();
    // Single-quoted for the shell; a quote inside closes and reopens it.
    let path = format!("'{}'", path.replace('\'', r"'\''"));
    match kind {
        Kind::Deb => Some(format!("sudo apt install {path}")),
        Kind::Rpm => Some(format!("sudo dnf install {path}")),
        _ => None,
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    phase: &Phase,
    auto_check: &mut bool,
    auto_install: &mut bool,
) -> Option<Action> {
    let k = k_of(ui.max_rect());
    let pal = theme::palette(ui.ctx());
    let p = page(phase);
    let body = draw_header(
        ui,
        &Header {
            slot: None,
            model: None,
            state: p.state,
            release: None,
            title: &p.title,
            sub: &p.sub,
            switchable: false,
        },
    )
    .body;
    let (space, bar) = footer_split(body, k);
    let content = Rect::from_min_max(
        Pos2::new(space.left() + theme::GUTTER * k, space.top()),
        Pos2::new(space.right() - theme::GUTTER * k, space.bottom()),
    );
    let mut action = None;
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content), |ui| {
        theme::apply_setup_style(ui, pal);
        action = draw_content(ui, phase, auto_check, auto_install, pal, k);
    });
    let inner = draw_footer(ui, bar, pal, k);
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
        theme::apply_setup_style(ui, pal);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(a) = draw_buttons(ui, phase, pal, k) {
                action = Some(a);
            }
        });
    });
    action
}

fn draw_content(
    ui: &mut egui::Ui,
    phase: &Phase,
    auto_check: &mut bool,
    auto_install: &mut bool,
    pal: &Palette,
    k: f32,
) -> Option<Action> {
    let mut action = None;
    match phase {
        Phase::Checking | Phase::Preparing(_) | Phase::Restarting(_) | Phase::Exiting => {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.3);
                ui.spinner();
            });
        }
        Phase::Available(offer) => {
            version_rows(ui, offer, pal, k);
            notes_box(ui, &offer.release_notes, pal, k);
            release_page_link(ui, &offer.notes, pal, k);
            if offer.kind.installs_itself() {
                ui.add_space(6.0 * k);
                let label = "Automatically download and install updates in the future";
                if checkbox(ui, auto_install, label, k) {
                    action = Some(Action::AutoInstall(*auto_install));
                }
            }
        }
        Phase::Ready(offer) => {
            version_rows(ui, offer, pal, k);
            notes_box(ui, &offer.release_notes, pal, k);
            release_page_link(ui, &offer.notes, pal, k);
        }
        Phase::Downloaded { offer, path } => {
            version_rows(ui, offer, pal, k);
            if let Some(cmd) = package_command(offer.kind, path) {
                ui.add_space(6.0 * k);
                ui.label(
                    RichText::new("Install with:")
                        .size(theme::BODY_FONT * k)
                        .color(pal.muted),
                );
                command_box(ui, &cmd, pal, k);
            }
            notes_box(ui, &offer.release_notes, pal, k);
        }
        Phase::Current {
            latest,
            notes,
            release_notes,
        } => {
            // The notes are this build's only when it is that release.
            if *latest == VERSION {
                notes_box(ui, release_notes, pal, k);
                release_page_link(ui, notes, pal, k);
            }
            ui.add_space(10.0 * k);
            if checkbox(ui, auto_check, "Check for updates automatically", k) {
                action = Some(Action::AutoCheck(*auto_check));
            }
        }
        Phase::Downloading { done, total, .. } => {
            ui.add_space(16.0 * k);
            progress_bar(ui, *done, *total, pal, k);
            ui.add_space(8.0 * k);
            ui.label(
                RichText::new(format!("{} of {}", megabytes(*done), megabytes(*total)))
                    .size(theme::BODY_FONT * k)
                    .color(pal.muted),
            );
        }
        Phase::Failed { message, .. } => {
            let mut chars = message.chars();
            let message: String = chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default();
            ui.label(
                RichText::new(message)
                    .size(theme::BODY_FONT * k)
                    .color(pal.danger),
            );
        }
        Phase::Unmanaged | Phase::NoPackage { .. } => {}
    }
    action
}

/// A checkbox; true when it changed.
fn checkbox(ui: &mut egui::Ui, on: &mut bool, label: &str, k: f32) -> bool {
    ui.checkbox(on, RichText::new(label).size(theme::BODY_FONT * k))
        .changed()
}

fn version_rows(ui: &mut egui::Ui, offer: &Offer, pal: &Palette, k: f32) {
    let versions = format!("v{VERSION}  →  v{}", offer.version);
    row(ui, "VERSION", &versions, pal, k);
    let file = offer.package.url.rsplit('/').next().unwrap_or_default();
    let package = format!("{file}  ·  {}", megabytes(offer.package.size));
    row(ui, "PACKAGE", &package, pal, k);
}

fn release_page_link(ui: &mut egui::Ui, url: &str, pal: &Palette, k: f32) {
    ui.add_space(4.0 * k);
    let link = ui.add(
        egui::Label::new(
            RichText::new("Full release notes")
                .size(theme::BODY_FONT * k)
                .underline()
                .color(pal.ink),
        )
        .sense(egui::Sense::click()),
    );
    if theme::pointer(link).clicked() {
        cdj3k_emu_platform::desktop::open_url(url);
    }
}

fn draw_buttons(ui: &mut egui::Ui, phase: &Phase, pal: &Palette, k: f32) -> Option<Action> {
    let size = button_size(k);
    let wide = [size[0] * 1.6, size[1]];
    let primary = |ui: &mut egui::Ui, wide_: bool, label: &str| {
        theme::primary(ui, if wide_ { wide } else { size }, label, pal).clicked()
    };
    let secondary = |ui: &mut egui::Ui, wide_: bool, label: &str| {
        theme::secondary(ui, if wide_ { wide } else { size }, label, pal).clicked()
    };
    let mut action = None;
    match phase {
        Phase::Available(offer) => {
            let label = if offer.kind.installs_itself() {
                "Update"
            } else {
                "Download"
            };
            if primary(ui, false, label) {
                action = Some(Action::Update(offer.clone()));
            }
            if secondary(ui, true, "Remind Me Later") {
                action = Some(Action::RemindLater);
            }
            if secondary(ui, true, "Skip This Version") {
                action = Some(Action::Skip(offer.version.clone()));
            }
        }
        Phase::Downloading { .. } => {
            if secondary(ui, false, "Cancel") {
                action = Some(Action::Cancel);
            }
            if secondary(ui, false, "Hide") {
                action = Some(Action::Dismiss);
            }
        }
        Phase::Preparing(_) => {
            if secondary(ui, false, "Hide") {
                action = Some(Action::Dismiss);
            }
        }
        Phase::Downloaded { path, .. } => {
            if primary(ui, true, "Show in Folder") {
                cdj3k_emu_platform::desktop::reveal_in_file_manager(path);
            }
            if secondary(ui, false, "Close") {
                action = Some(Action::Dismiss);
            }
        }
        Phase::Ready(offer) => {
            if primary(ui, true, "Restart Now") {
                action = Some(Action::Restart(offer.clone()));
            }
            if secondary(ui, false, "Later") {
                action = Some(Action::Later);
            }
        }
        Phase::Failed { offer, .. } => {
            if primary(ui, false, "Try Again") {
                action = Some(Action::Retry(offer.clone()));
            }
            if secondary(ui, false, "Close") {
                action = Some(Action::Dismiss);
            }
        }
        Phase::Unmanaged | Phase::NoPackage { .. } => {
            if primary(ui, true, "Open Releases") {
                cdj3k_emu_platform::desktop::open_url(cdj3k_emu_update::RELEASES_PAGE);
            }
            if secondary(ui, false, "Close") {
                action = Some(Action::Dismiss);
            }
        }
        Phase::Checking | Phase::Current { .. } => {
            if secondary(ui, false, "Close") {
                action = Some(Action::Dismiss);
            }
        }
        Phase::Restarting(_) | Phase::Exiting => {}
    }
    action
}

/// A small-caps label and its value.
fn row(ui: &mut egui::Ui, label: &str, value: &str, pal: &Palette, k: f32) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(ROW_LABEL_W * k, theme::BODY_FONT * k * 1.4),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(ROW_LABEL_W * k);
                ui.label(
                    RichText::new(label)
                        .size(theme::MICRO_FONT * k)
                        .extra_letter_spacing(theme::MICRO_TRACK * k)
                        .color(pal.dim),
                );
            },
        );
        ui.label(
            RichText::new(value)
                .size(theme::BODY_FONT * k)
                .color(pal.ink),
        );
    });
}

fn command_box(ui: &mut egui::Ui, cmd: &str, pal: &Palette, k: f32) {
    egui::Frame::none()
        .fill(pal.field)
        .stroke(Stroke::new(theme::HAIRLINE * k, pal.line))
        .rounding(theme::ROUND * k)
        .inner_margin(egui::Margin::symmetric(10.0 * k, 7.0 * k))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(cmd)
                        .monospace()
                        .size(theme::BODY_FONT * k)
                        .color(pal.ink),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::pointer(ui.small_button("Copy")).clicked() {
                        ui.ctx().copy_text(cmd.to_owned());
                    }
                });
            });
        });
}

fn progress_bar(ui: &mut egui::Ui, done: u64, total: u64, pal: &Palette, k: f32) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), PROGRESS_H * k),
        egui::Sense::hover(),
    );
    let p = ui.painter();
    let round = PROGRESS_H * k * 0.5;
    p.rect_filled(rect, round, pal.field);
    p.rect_stroke(rect, round, Stroke::new(theme::HAIRLINE * k, pal.line));
    let t = if total == 0 {
        0.0
    } else {
        (done as f32 / total as f32).clamp(0.0, 1.0)
    };
    if t > 0.0 {
        let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * t, rect.height()));
        p.rect_filled(fill, round, pal.ink);
    }
}
