//! The menu as data.
//!
//! The service emits one of these per sync; a provider renders it. The model
//! is a superset: it carries everything any backend could want, and a provider
//! drops what it cannot express rather than the service trimming to the
//! weakest backend.

use super::id::MenuId;

/// A whole menu bar: the top-level submenus, left to right.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuModel {
    pub roots: Vec<MenuNode>,
    /// A condition worth flagging on the strip, with its explanation.
    pub notice: Option<Notice>,
}

/// A condition flagged on the strip, and the card that explains it. The
/// glyph is the provider's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    /// Paragraphs of the card, in order.
    pub body: Vec<String>,
}

/// A glyph a provider may draw beside a row or in a status strip.
///
/// The *state* is decided here because it comes from application state; which
/// picture stands for it is the provider's business.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuIcon {
    Emulation,
    Restart { armed: bool },
    Storage { mounted: bool },
    Network(NetKind),
    Audio { on: bool },
    View,
    Instances,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetKind {
    Nat,
    LinkLocal,
    Bridged,
}

/// One row, or one nested menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuNode {
    /// Clickable row.
    Item {
        id: MenuId,
        label: String,
        enabled: bool,
        /// Trailing value — a device node, an address. Set apart from the
        /// label because it is data, not wording.
        detail: Option<String>,
    },
    /// Clickable row carrying a tick. Radio groups are expressed as a run of
    /// these with one `checked` — the model does not distinguish the two,
    /// because no backend we target enforces radio semantics for us.
    Check {
        id: MenuId,
        label: String,
        enabled: bool,
        checked: bool,
        detail: Option<String>,
    },
    /// Non-clickable text.
    Label(String),
    /// A heading over the rows beneath it.
    Section(String),
    /// A figure worth reading at a glance — the audio pipeline latency.
    /// Distinct from a label because it is three pieces of text with a
    /// hierarchy, not one line.
    Readout {
        title: String,
        value: String,
        detail: Option<String>,
    },
    Separator,
    /// A row the host toolkit owns rather than us. A backend without the
    /// concept renders nothing.
    Predefined(Predefined),
    Submenu {
        label: String,
        enabled: bool,
        children: Vec<MenuNode>,
        /// What this menu is currently set to, for a provider that shows the
        /// state on the opener rather than only inside — `en7`, `141 ms`.
        status: Option<String>,
        /// The part of the status that is a bare value rather than a name —
        /// the firmware release beside the deck. Set apart so a provider can
        /// set it in the face it uses for data.
        detail: Option<String>,
        icon: Option<MenuIcon>,
    },
}

/// Rows supplied by the platform, not by us.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Predefined {
    Quit,
}

impl MenuNode {
    pub fn item(id: MenuId, label: impl Into<String>) -> Self {
        Self::Item {
            id,
            label: label.into(),
            enabled: true,
            detail: None,
        }
    }

    pub fn item_enabled(id: MenuId, label: impl Into<String>, enabled: bool) -> Self {
        Self::Item {
            id,
            label: label.into(),
            enabled,
            detail: None,
        }
    }

    pub fn check(id: MenuId, label: impl Into<String>, checked: bool) -> Self {
        Self::Check {
            id,
            label: label.into(),
            enabled: true,
            checked,
            detail: None,
        }
    }

    pub fn submenu(label: impl Into<String>, children: Vec<MenuNode>) -> Self {
        Self::Submenu {
            label: label.into(),
            enabled: true,
            children,
            status: None,
            detail: None,
            icon: None,
        }
    }

    /// Grey out a menu or row, and everything under a menu.
    pub fn with_enabled(mut self, on: bool) -> Self {
        match &mut self {
            Self::Item { enabled, .. }
            | Self::Check { enabled, .. }
            | Self::Submenu { enabled, .. } => *enabled = on,
            _ => {}
        }
        self
    }

    /// Attach the trailing value.
    pub fn with_detail(mut self, text: impl Into<String>) -> Self {
        match &mut self {
            Self::Item { detail, .. }
            | Self::Check { detail, .. }
            | Self::Submenu { detail, .. } => *detail = Some(text.into()),
            _ => {}
        }
        self
    }

    /// Attach what a status strip shows for this menu.
    pub fn with_status(mut self, text: impl Into<String>, glyph: MenuIcon) -> Self {
        if let Self::Submenu { status, icon, .. } = &mut self {
            *status = Some(text.into());
            *icon = Some(glyph);
        }
        self
    }

    /// Attach only the glyph, for a menu with no state worth spelling out.
    pub fn with_icon(mut self, glyph: MenuIcon) -> Self {
        if let Self::Submenu { icon, .. } = &mut self {
            *icon = Some(glyph);
        }
        self
    }

    /// The rows inside this node, empty for anything that is not a menu.
    pub fn children(&self) -> &[MenuNode] {
        match self {
            Self::Submenu { children, .. } => children,
            _ => &[],
        }
    }

    /// The action this row raises, if it raises one.
    pub fn id(&self) -> Option<&MenuId> {
        match self {
            Self::Item { id, .. } | Self::Check { id, .. } => Some(id),
            _ => None,
        }
    }

    /// Whether two nodes occupy the same slot — same kind, same identity.
    ///
    /// A provider that holds native widgets uses this to decide between
    /// updating a row in place and rebuilding the menu it sits in. Labels,
    /// checkmarks and enablement are excluded: they change every frame and
    /// must not force a rebuild.
    pub fn same_slot_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Item { id: a, .. }, Self::Item { id: b, .. }) => a == b,
            (Self::Check { id: a, .. }, Self::Check { id: b, .. }) => a == b,
            // Headers are positional, so a label only has to stay a label;
            // its text is updated in place (this is how the latency row works).
            (Self::Label(_), Self::Label(_)) => true,
            (Self::Section(a), Self::Section(b)) => a == b,
            (Self::Readout { .. }, Self::Readout { .. }) => true,
            (Self::Separator, Self::Separator) => true,
            (Self::Predefined(a), Self::Predefined(b)) => a == b,
            (Self::Submenu { label: a, .. }, Self::Submenu { label: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// Whether two node lists can be updated in place, or need a rebuild.
pub fn same_shape(a: &[MenuNode], b: &[MenuNode]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same_slot_as(y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkmark_change_does_not_change_the_shape() {
        let a = vec![MenuNode::check(MenuId::Audio, "Enable Audio", false)];
        let b = vec![MenuNode::check(MenuId::Audio, "Enable Audio", true)];
        assert_ne!(a, b);
        assert!(same_shape(&a, &b));
    }

    #[test]
    fn a_label_change_does_not_change_the_shape() {
        let a = vec![MenuNode::Label("Current Latency: -- ms".into())];
        let b = vec![MenuNode::Label("Latency: 9 ms (guest 4, host 5)".into())];
        assert!(same_shape(&a, &b));
    }

    #[test]
    fn a_different_id_in_the_same_slot_changes_the_shape() {
        let a = vec![MenuNode::check(MenuId::NetInterface(0), "en0", true)];
        let b = vec![MenuNode::check(MenuId::NetInterface(1), "en1", true)];
        assert!(!same_shape(&a, &b));
    }

    #[test]
    fn adding_a_device_changes_the_shape() {
        let a = vec![MenuNode::check(MenuId::AudioDeviceDefault, "Default", true)];
        let b = vec![
            MenuNode::check(MenuId::AudioDeviceDefault, "Default", true),
            MenuNode::Separator,
            MenuNode::check(MenuId::AudioDevice("x".into()), "X", false),
        ];
        assert!(!same_shape(&a, &b));
    }
}

#[cfg(test)]
mod design_tests {
    use super::*;

    /// The in-window bar shows each menu's state on its opener; the native bar
    /// shows neither. Both read one model, so the state has to live in it.
    #[test]
    fn a_status_and_glyph_ride_on_the_opener() {
        let n = MenuNode::submenu("Network", vec![])
            .with_status("en7", MenuIcon::Network(NetKind::Bridged));
        let MenuNode::Submenu { status, icon, .. } = &n else {
            panic!("expected a submenu")
        };
        assert_eq!(status.as_deref(), Some("en7"));
        assert_eq!(icon, &Some(MenuIcon::Network(NetKind::Bridged)));
    }

    /// The icon carries the state, not the picture: which glyph stands for a
    /// mounted disk is the provider's business.
    #[test]
    fn the_icon_carries_state_rather_than_a_picture() {
        assert_ne!(
            MenuIcon::Storage { mounted: true },
            MenuIcon::Storage { mounted: false }
        );
        assert_ne!(
            MenuIcon::Network(NetKind::Nat),
            MenuIcon::Network(NetKind::Bridged)
        );
    }

    #[test]
    fn a_trailing_value_is_separate_from_the_wording() {
        let n = MenuNode::check(MenuId::PhysicalDisk(0), "USB Flash Disk · 32 GB", false)
            .with_detail("sdb");
        let MenuNode::Check { label, detail, .. } = &n else {
            panic!("expected a check row")
        };
        assert_eq!(label, "USB Flash Disk · 32 GB");
        assert_eq!(detail.as_deref(), Some("sdb"));
    }

    /// A status change must not force a native menu rebuild: the opener's
    /// state changes constantly and its children do not.
    #[test]
    fn changing_only_the_status_keeps_the_shape() {
        let a =
            vec![MenuNode::submenu("Audio", vec![])
                .with_status("141 ms", MenuIcon::Audio { on: true })];
        let b =
            vec![MenuNode::submenu("Audio", vec![])
                .with_status("96 ms", MenuIcon::Audio { on: true })];
        assert_ne!(a, b);
        assert!(same_shape(&a, &b));
    }

    #[test]
    fn a_readout_updates_in_place_but_a_section_is_positional() {
        let a = vec![MenuNode::Readout {
            title: "Pipeline latency".into(),
            value: "141 ms".into(),
            detail: None,
        }];
        let b = vec![MenuNode::Readout {
            title: "Pipeline latency".into(),
            value: "96 ms".into(),
            detail: Some("guest 70 · host 26".into()),
        }];
        assert!(same_shape(&a, &b));

        assert!(!same_shape(
            &[MenuNode::Section("Physical".into())],
            &[MenuNode::Section("Virtual (.img)".into())]
        ));
    }
}
