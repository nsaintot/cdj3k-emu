//! Native menu bar, via `muda`.
//!
//! `muda` is retained: items are built once and mutated afterwards. The
//! service is declarative and hands over a whole model each frame, so this is
//! where the two meet — the previous model is kept and compared, rows that
//! only changed text or state are written in place, and a menu is rebuilt
//! only when its shape actually changed.

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use super::id::MenuId;
use super::model::{same_shape, MenuModel, MenuNode, Predefined};
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

/// The action's shortcut, as muda spells it. `CmdOrCtrl` is muda's name for
/// the primary modifier.
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

/// Only the keys the menu actually uses; an unknown one yields no shortcut
/// rather than a wrong one.
fn code_for(key: &str) -> Option<Code> {
    Some(match key {
        "R" => Code::KeyR,
        "D" => Code::KeyD,
        "Q" => Code::KeyQ,
        "F11" => Code::F11,
        _ => return None,
    })
}

/// Handles for the rows we mutate. Kinds without mutable state hold nothing.
enum Native {
    Item(MenuItem),
    Check(CheckMenuItem),
    /// A disabled row whose text changes — section headers and the latency
    /// readout are both this.
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
        append_to_root(&root, node, &native);
        natives.push(native);
    }

    #[cfg(target_os = "macos")]
    root.init_for_nsapp();

    Installed {
        _root: root,
        natives,
        shown: model.clone(),
    }
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
        // A native menu has no place for a multi-line figure, so it reads as
        // the one disabled line it would have been anyway.
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
    match (node, native) {
        (MenuNode::Separator, _) => {
            parent.append(&PredefinedMenuItem::separator()).ok();
        }
        (MenuNode::Section(_), Native::Label(item)) => {
            if !first {
                parent.append(&PredefinedMenuItem::separator()).ok();
            }
            parent.append(item).ok();
        }
        (MenuNode::Predefined(Predefined::Quit), _) => {
            parent.append(&PredefinedMenuItem::quit(None)).ok();
        }
        (_, Native::Item(item)) | (_, Native::Label(item)) => {
            parent.append(item).ok();
        }
        (_, Native::Check(item)) => {
            parent.append(item).ok();
        }
        (_, Native::Submenu { handle, .. }) => {
            parent.append(handle).ok();
        }
        (_, Native::Inert) => {}
    }
}

fn append_to_root(root: &Menu, node: &MenuNode, native: &Native) {
    match (node, native) {
        (MenuNode::Separator, _) => {
            root.append(&PredefinedMenuItem::separator()).ok();
        }
        (MenuNode::Predefined(Predefined::Quit), _) => {
            root.append(&PredefinedMenuItem::quit(None)).ok();
        }
        (_, Native::Item(item)) | (_, Native::Label(item)) => {
            root.append(item).ok();
        }
        (_, Native::Check(item)) => {
            root.append(item).ok();
        }
        (_, Native::Submenu { handle, .. }) => {
            root.append(handle).ok();
        }
        (_, Native::Inert) => {}
    }
}

// ── Reconciling ───────────────────────────────────────────────────────────────

/// Update one list of siblings in place, or rebuild it into `parent`.
///
/// `parent` is `None` at the root, which never changes shape — the model
/// always yields the same six submenus — so the rebuild arm cannot be reached
/// there and a shape change is simply not applied.
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
    match (native, old, new) {
        (
            Native::Item(item),
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
        ) => {
            if was != now || was_detail != now_detail {
                item.set_text(row_text(now, now_detail));
            }
            if was_on != now_on {
                item.set_enabled(*now_on);
            }
        }
        (
            Native::Check(item),
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
        ) => {
            if was != now || was_detail != now_detail {
                item.set_text(row_text(now, now_detail));
            }
            if was_on != now_on {
                item.set_enabled(*now_on);
            }
            if was_tick != now_tick {
                item.set_checked(*now_tick);
            }
        }
        (Native::Label(item), MenuNode::Label(was), MenuNode::Label(now))
        | (Native::Label(item), MenuNode::Section(was), MenuNode::Section(now)) => {
            if was != now {
                item.set_text(now);
            }
        }
        (
            Native::Label(item),
            MenuNode::Readout { .. },
            MenuNode::Readout {
                title,
                value,
                detail,
            },
        ) => item.set_text(readout_text(title, value, detail)),
        (
            Native::Submenu { handle, children },
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
        ) => {
            if was_on != now_on {
                handle.set_enabled(*now_on);
            }
            let handle = handle.clone();
            reconcile_level(children, old_children, new_children, Some(&handle));
        }
        _ => {}
    }
}
