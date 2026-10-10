//! Native menu bar, via `muda`.
//!
//! `muda` is retained: items are built once and mutated afterwards. The
//! service hands over a whole model each frame, so this provider keeps the
//! previous model and compares the two. Rows whose text or state changed are
//! updated in place, and a menu is rebuilt only when its shape changed.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use super::id::MenuId;
use super::model::{same_shape, MenuModel, MenuNode, Notice, Predefined};
use super::provider::MenuProvider;

/// A native menu has one text column, so a trailing value joins the label.
fn row_text(label: &str, detail: &Option<String>) -> String {
    match detail {
        Some(d) => format!("{label} — {d}"),
        None => label.to_string(),
    }
}

fn readout_text(title: &str, value: &str, detail: &Option<String>) -> String {
    match detail {
        Some(d) => format!("{title}: {value} ({d})"),
        None => format!("{title}: {value}"),
    }
}

/// The action's shortcut, as muda spells it. The primary modifier maps to
/// `Modifiers::META`, which is Command on macOS.
fn accelerator_for(id: &MenuId) -> Option<Accelerator> {
    let accel = id.accelerator()?;
    let mut mods = Modifiers::empty();
    if accel.primary {
        mods |= Modifiers::META;
    }
    if accel.shift {
        mods |= Modifiers::SHIFT;
    }
    Some(Accelerator::new(Some(mods), code_for(accel.key)?))
}

/// Map the keys the menu uses; any other key gets no shortcut.
fn code_for(key: &str) -> Option<Code> {
    Some(match key {
        "R" => Code::KeyR,
        "D" => Code::KeyD,
        "Q" => Code::KeyQ,
        "F11" => Code::F11,
        _ => return None,
    })
}

/// Handles for the rows that change after they are built. Rows without
/// mutable state hold nothing.
enum Native {
    Item(MenuItem),
    Check(CheckMenuItem),
    /// A disabled row whose text changes, such as a section header or the
    /// latency readout.
    Label(MenuItem),
    Inert,
    Submenu {
        handle: Submenu,
        children: Vec<Native>,
    },
}

#[derive(Default)]
pub struct MudaProvider {
    installed: Option<Installed>,
}

struct Installed {
    /// The root must outlive every item in it: `muda` stores a raw
    /// `*const MenuChild` in each `NSMenuItem`, and dropping the root frees
    /// the `Rc` chain those pointers address.
    _root: Menu,
    natives: Vec<Native>,
    /// The notice's menu, after the model's roots.
    _notice: Option<Submenu>,
    /// What is currently on screen, to compare the next model against.
    shown: MenuModel,
}

impl MenuProvider for MudaProvider {
    fn apply(&mut self, model: &MenuModel) {
        match &mut self.installed {
            None => self.installed = Some(install(model)),
            Some(installed) => {
                reconcile_level(
                    &mut installed.natives,
                    &installed.shown.roots,
                    &model.roots,
                    None,
                );
                installed.shown = model.clone();
            }
        }
    }

    fn poll(&mut self) -> Vec<MenuId> {
        let mut out = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(id) = MenuId::from_wire(event.id.0.as_str()) {
                out.push(id);
            }
        }
        out
    }
}

fn install(model: &MenuModel) -> Installed {
    let root = Menu::new();
    let mut natives = Vec::with_capacity(model.roots.len());
    for node in &model.roots {
        let native = build(node);
        add_row(&root, node, &native);
        natives.push(native);
    }
    // The service reads the notice once per window, so it is built once.
    let notice = model.notice.as_ref().map(notice_menu);
    if let Some(menu) = &notice {
        root.append(menu).ok();
    }

    root.init_for_nsapp();

    Installed {
        _root: root,
        natives,
        _notice: notice,
        shown: model.clone(),
    }
}

/// The strip's notice card as a menu: the title, then each paragraph as
/// disabled rows, one line each.
fn notice_menu(notice: &Notice) -> Submenu {
    let menu = Submenu::new("TCG", true);
    menu.append(&MenuItem::new(&notice.title, false, None)).ok();
    for paragraph in &notice.body {
        menu.append(&PredefinedMenuItem::separator()).ok();
        for line in wrap(paragraph, NOTICE_LINE) {
            menu.append(&MenuItem::new(line, false, None)).ok();
        }
    }
    menu
}

/// Characters per notice row: a menu row does not wrap.
const NOTICE_LINE: usize = 60;

/// `text` broken at spaces into lines of at most `width` characters; a word
/// longer than that is a line of its own.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

// ── Building ──────────────────────────────────────────────────────────────────

fn build(node: &MenuNode) -> Native {
    match node {
        MenuNode::Item {
            id,
            label,
            enabled,
            detail,
        } => Native::Item(MenuItem::with_id(
            id.to_wire(),
            row_text(label, detail),
            *enabled,
            accelerator_for(id),
        )),
        MenuNode::Check {
            id,
            label,
            enabled,
            checked,
            detail,
        } => Native::Check(CheckMenuItem::with_id(
            id.to_wire(),
            row_text(label, detail),
            *enabled,
            *checked,
            accelerator_for(id),
        )),
        MenuNode::Label(text) | MenuNode::Section(text) => {
            Native::Label(MenuItem::new(text, false, None))
        }
        // A native menu cannot show a multi-line figure, so the readout becomes
        // one disabled line.
        MenuNode::Readout {
            title,
            value,
            detail,
        } => Native::Label(MenuItem::new(
            readout_text(title, value, detail),
            false,
            None,
        )),
        MenuNode::Separator | MenuNode::Predefined(_) => Native::Inert,
        // `status` and `icon` are for a provider that draws its own opener;
        // a native menu bar shows neither.
        MenuNode::Submenu {
            label,
            enabled,
            children,
            ..
        } => {
            let handle = Submenu::new(label, *enabled);
            let natives = children
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let native = build(child);
                    append(&handle, child, &native, i == 0);
                    native
                })
                .collect();
            Native::Submenu {
                handle,
                children: natives,
            }
        }
    }
}

/// Add one row to `parent`. A section heading below other rows gets a
/// separator above it, the way a native menu sets a group apart.
fn append(parent: &Submenu, node: &MenuNode, native: &Native, first: bool) {
    if !first && matches!((node, native), (MenuNode::Section(_), Native::Label(_))) {
        parent.add(&PredefinedMenuItem::separator());
    }
    add_row(parent, node, native);
}

/// What a row is added to: the bar itself, or one of its menus.
trait Container {
    fn add(&self, item: &dyn IsMenuItem);
}

impl Container for Menu {
    fn add(&self, item: &dyn IsMenuItem) {
        self.append(item).ok();
    }
}

impl Container for Submenu {
    fn add(&self, item: &dyn IsMenuItem) {
        self.append(item).ok();
    }
}

/// Add the native widget for one row; muda supplies separators and Quit.
fn add_row(parent: &impl Container, node: &MenuNode, native: &Native) {
    match (node, native) {
        (MenuNode::Separator, _) => parent.add(&PredefinedMenuItem::separator()),
        (MenuNode::Predefined(Predefined::Quit), _) => parent.add(&PredefinedMenuItem::quit(None)),
        (_, Native::Item(item)) | (_, Native::Label(item)) => parent.add(item),
        (_, Native::Check(item)) => parent.add(item),
        (_, Native::Submenu { handle, .. }) => parent.add(handle),
        (_, Native::Inert) => {}
    }
}

// ── Reconciling ───────────────────────────────────────────────────────────────

/// Update one list of siblings in place, or rebuild it into `parent`.
///
/// `parent` is `None` at the root. The root never changes shape, because the
/// model always yields the same six submenus, so a shape change there is not
/// applied.
fn reconcile_level(
    natives: &mut Vec<Native>,
    old: &[MenuNode],
    new: &[MenuNode],
    parent: Option<&Submenu>,
) {
    if same_shape(old, new) {
        for ((native, old), new) in natives.iter_mut().zip(old).zip(new) {
            update(native, old, new);
        }
        return;
    }

    let Some(parent) = parent else { return };

    while parent.remove_at(0).is_some() {}
    natives.clear();
    for (i, node) in new.iter().enumerate() {
        let native = build(node);
        append(parent, node, &native, i == 0);
        natives.push(native);
    }
}

/// Write the differences for one row that kept its slot.
fn update(native: &mut Native, old: &MenuNode, new: &MenuNode) {
    match native {
        Native::Item(item) => update_item(item, old, new),
        Native::Check(item) => update_check(item, old, new),
        Native::Label(item) => update_label(item, old, new),
        Native::Submenu { handle, children } => update_submenu(handle, children, old, new),
        Native::Inert => {}
    }
}

/// The row's new text, when its label or its value changed.
fn changed_text(
    was: &str,
    was_detail: &Option<String>,
    now: &str,
    now_detail: &Option<String>,
) -> Option<String> {
    (was != now || was_detail != now_detail).then(|| row_text(now, now_detail))
}

fn update_item(item: &MenuItem, old: &MenuNode, new: &MenuNode) {
    let (
        MenuNode::Item {
            label: was,
            enabled: was_on,
            detail: was_detail,
            ..
        },
        MenuNode::Item {
            label: now,
            enabled: now_on,
            detail: now_detail,
            ..
        },
    ) = (old, new)
    else {
        return;
    };
    if let Some(text) = changed_text(was, was_detail, now, now_detail) {
        item.set_text(text);
    }
    if was_on != now_on {
        item.set_enabled(*now_on);
    }
}

fn update_check(item: &CheckMenuItem, old: &MenuNode, new: &MenuNode) {
    let (
        MenuNode::Check {
            label: was,
            enabled: was_on,
            checked: was_tick,
            detail: was_detail,
            ..
        },
        MenuNode::Check {
            label: now,
            enabled: now_on,
            checked: now_tick,
            detail: now_detail,
            ..
        },
    ) = (old, new)
    else {
        return;
    };
    if let Some(text) = changed_text(was, was_detail, now, now_detail) {
        item.set_text(text);
    }
    if was_on != now_on {
        item.set_enabled(*now_on);
    }
    if was_tick != now_tick {
        item.set_checked(*now_tick);
    }
}

fn update_label(item: &MenuItem, old: &MenuNode, new: &MenuNode) {
    match (old, new) {
        (MenuNode::Label(was), MenuNode::Label(now))
        | (MenuNode::Section(was), MenuNode::Section(now)) => {
            if was != now {
                item.set_text(now);
            }
        }
        (
            MenuNode::Readout { .. },
            MenuNode::Readout {
                title,
                value,
                detail,
            },
        ) => item.set_text(readout_text(title, value, detail)),
        _ => {}
    }
}

fn update_submenu(handle: &Submenu, children: &mut Vec<Native>, old: &MenuNode, new: &MenuNode) {
    let (
        MenuNode::Submenu {
            enabled: was_on,
            children: old_children,
            ..
        },
        MenuNode::Submenu {
            enabled: now_on,
            children: new_children,
            ..
        },
    ) = (old, new)
    else {
        return;
    };
    if was_on != now_on {
        handle.set_enabled(*now_on);
    }
    reconcile_level(children, old_children, new_children, Some(handle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_paragraph_wraps_at_spaces() {
        let lines = wrap(
            "This host is x86_64 and the guest is arm64, so every instruction is translated.",
            24,
        );
        assert!(lines.iter().all(|l| l.chars().count() <= 24), "{lines:?}");
        assert_eq!(
            lines.join(" "),
            "This host is x86_64 and the guest is arm64, so every instruction is translated."
        );
    }

    #[test]
    fn a_word_longer_than_the_line_stands_alone() {
        assert_eq!(
            wrap("a CDJ3K_EMU_TCG_IS_SET b", 5),
            ["a", "CDJ3K_EMU_TCG_IS_SET", "b"]
        );
    }
}
