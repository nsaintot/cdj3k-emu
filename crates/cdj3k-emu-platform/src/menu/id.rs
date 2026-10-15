//! Every action the menu can raise, as one closed set.
//!
//! Providers that address items by string — `muda` registers an id per
//! `NSMenuItem` — go through [`MenuId::to_wire`] and [`MenuId::from_wire`].
//! A provider that dispatches by value, such as an immediate-mode one, uses
//! the variants directly and never sees a string.

/// One menu action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuId {
    // Emulation
    ManageEmulation,
    ModsEnabled,
    Restart,
    ServiceMode,
    DevMode,
    Haptic,
    PcLink,
    CheckForUpdate,

    // View
    ScreenExtended,
    JogScreen,
    MainScreen,
    DebugScreen,

    // Audio
    Audio,
    Alc,
    /// Clear the output-device override and follow the system default.
    AudioDeviceDefault,
    /// Pin output to one device, by its platform UID.
    AudioDevice(String),

    // Network
    NetNat,
    NetVmnetHost,
    /// Index into `menu_state::net_ifaces`.
    NetInterface(u32),

    // Storage
    EjectCurrentMedia,
    MountVirtualUsb,
    CreateVirtualUsb,
    /// Index into `menu_state::usb_phys_disks`.
    PhysicalDisk(usize),

    // Instances
    Instance(u32),
}

/// The wire form of every variant without a payload, read in both directions.
const FIXED: [(MenuId, &str); 20] = [
    (MenuId::ManageEmulation, "manage_emulation"),
    (MenuId::ModsEnabled, "mods_enabled"),
    (MenuId::Restart, "restart"),
    (MenuId::ServiceMode, "service_mode"),
    (MenuId::DevMode, "dev_mode"),
    (MenuId::Haptic, "haptic"),
    (MenuId::PcLink, "pc_link"),
    (MenuId::CheckForUpdate, "check_for_update"),
    (MenuId::ScreenExtended, "screen_extended"),
    (MenuId::JogScreen, "jog_screen"),
    (MenuId::MainScreen, "main_screen"),
    (MenuId::DebugScreen, "debug_screen"),
    (MenuId::Audio, "audio"),
    (MenuId::Alc, "alc"),
    (MenuId::AudioDeviceDefault, "audio_dev_default"),
    (MenuId::NetNat, "net_none"),
    (MenuId::NetVmnetHost, "net_vmnet_host"),
    (MenuId::EjectCurrentMedia, "eject_current_media"),
    (MenuId::MountVirtualUsb, "mount_virtual_usb"),
    (MenuId::CreateVirtualUsb, "create_virtual_usb"),
];

/// Prefixes of the variants that carry a payload, which follows the prefix.
const AUDIO_DEVICE: &str = "audio_dev_uid_";
const NET_INTERFACE: &str = "net_if_";
const PHYSICAL_DISK: &str = "phys_select_";
const INSTANCE: &str = "instance_";

impl MenuId {
    /// Stable string form for providers whose widgets carry a string id.
    pub fn to_wire(&self) -> String {
        match self {
            Self::AudioDevice(uid) => format!("{AUDIO_DEVICE}{}", encode_hex(uid)),
            Self::NetInterface(n) => format!("{NET_INTERFACE}{n}"),
            Self::PhysicalDisk(i) => format!("{PHYSICAL_DISK}{i}"),
            Self::Instance(n) => format!("{INSTANCE}{n}"),
            fixed => FIXED
                .iter()
                .find(|(id, _)| id == fixed)
                .map(|(_, wire)| (*wire).to_owned())
                .expect("every variant without a payload is in FIXED"),
        }
    }

    /// Inverse of [`MenuId::to_wire`]. `None` for ids this build does not know,
    /// such as a stale widget from an older model.
    pub fn from_wire(s: &str) -> Option<Self> {
        if let Some((id, _)) = FIXED.iter().find(|(_, wire)| *wire == s) {
            return Some(id.clone());
        }
        if let Some(rest) = s.strip_prefix(AUDIO_DEVICE) {
            return decode_hex(rest).map(Self::AudioDevice);
        }
        if let Some(rest) = s.strip_prefix(NET_INTERFACE) {
            return rest.parse().ok().map(Self::NetInterface);
        }
        if let Some(rest) = s.strip_prefix(PHYSICAL_DISK) {
            return rest.parse().ok().map(Self::PhysicalDisk);
        }
        if let Some(rest) = s.strip_prefix(INSTANCE) {
            return rest.parse().ok().map(Self::Instance);
        }
        None
    }
}

/// Audio UIDs carry colons and spaces. Hex-encoding them keeps the wire form
/// to `[0-9a-z_]`, which every string-id backend accepts without escaping.
fn encode_hex(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.as_bytes() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn decode_hex(s: &str) -> Option<String> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        bytes.push(((hi << 4) | lo) as u8);
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_survives_the_wire() {
        let ids = [
            MenuId::ManageEmulation,
            MenuId::ModsEnabled,
            MenuId::Restart,
            MenuId::ServiceMode,
            MenuId::DevMode,
            MenuId::Haptic,
            MenuId::PcLink,
            MenuId::CheckForUpdate,
            MenuId::ScreenExtended,
            MenuId::JogScreen,
            MenuId::MainScreen,
            MenuId::DebugScreen,
            MenuId::Audio,
            MenuId::Alc,
            MenuId::AudioDeviceDefault,
            MenuId::AudioDevice("AppleHDA:1b,0:out".into()),
            MenuId::NetNat,
            MenuId::NetVmnetHost,
            MenuId::NetInterface(3),
            MenuId::EjectCurrentMedia,
            MenuId::MountVirtualUsb,
            MenuId::CreateVirtualUsb,
            MenuId::PhysicalDisk(2),
            MenuId::Instance(4),
        ];
        for id in ids {
            assert_eq!(MenuId::from_wire(&id.to_wire()).as_ref(), Some(&id));
        }
    }

    #[test]
    fn every_variant_has_its_own_fixed_wire_form() {
        let pinned = [
            (MenuId::ManageEmulation, "manage_emulation"),
            (MenuId::ModsEnabled, "mods_enabled"),
            (MenuId::Restart, "restart"),
            (MenuId::ServiceMode, "service_mode"),
            (MenuId::DevMode, "dev_mode"),
            (MenuId::Haptic, "haptic"),
            (MenuId::PcLink, "pc_link"),
            (MenuId::CheckForUpdate, "check_for_update"),
            (MenuId::ScreenExtended, "screen_extended"),
            (MenuId::JogScreen, "jog_screen"),
            (MenuId::MainScreen, "main_screen"),
            (MenuId::DebugScreen, "debug_screen"),
            (MenuId::Audio, "audio"),
            (MenuId::Alc, "alc"),
            (MenuId::AudioDeviceDefault, "audio_dev_default"),
            (MenuId::AudioDevice("a:b".into()), "audio_dev_uid_613a62"),
            (MenuId::NetNat, "net_none"),
            (MenuId::NetVmnetHost, "net_vmnet_host"),
            (MenuId::NetInterface(3), "net_if_3"),
            (MenuId::EjectCurrentMedia, "eject_current_media"),
            (MenuId::MountVirtualUsb, "mount_virtual_usb"),
            (MenuId::CreateVirtualUsb, "create_virtual_usb"),
            (MenuId::PhysicalDisk(2), "phys_select_2"),
            (MenuId::Instance(4), "instance_4"),
        ];
        for (id, wire) in pinned {
            assert_eq!(id.to_wire(), wire);
            assert_eq!(MenuId::from_wire(wire), Some(id));
        }
    }

    /// Every variant without a payload is in [`FIXED`]. A new variant fails
    /// to compile here until it is matched, and fails the test until it is in
    /// the table.
    #[test]
    fn fixed_covers_every_variant_without_a_payload() {
        for (id, _) in &FIXED {
            match id {
                MenuId::ManageEmulation
                | MenuId::ModsEnabled
                | MenuId::Restart
                | MenuId::ServiceMode
                | MenuId::DevMode
                | MenuId::Haptic
                | MenuId::PcLink
                | MenuId::CheckForUpdate
                | MenuId::ScreenExtended
                | MenuId::JogScreen
                | MenuId::MainScreen
                | MenuId::DebugScreen
                | MenuId::Audio
                | MenuId::Alc
                | MenuId::AudioDeviceDefault
                | MenuId::NetNat
                | MenuId::NetVmnetHost
                | MenuId::EjectCurrentMedia
                | MenuId::MountVirtualUsb
                | MenuId::CreateVirtualUsb => {}
                MenuId::AudioDevice(_)
                | MenuId::NetInterface(_)
                | MenuId::PhysicalDisk(_)
                | MenuId::Instance(_) => panic!("{id:?} carries a payload"),
            }
        }
        assert_eq!(FIXED.len(), 20);
    }

    #[test]
    fn no_two_fixed_ids_share_a_wire_form() {
        for (i, (a, wire)) in FIXED.iter().enumerate() {
            for (b, other) in &FIXED[i + 1..] {
                assert_ne!(a, b);
                assert_ne!(wire, other);
            }
        }
    }

    #[test]
    fn a_uid_with_separators_round_trips() {
        let uid = "BuiltInSpeakerDevice: 0x2a, name";
        let id = MenuId::AudioDevice(uid.into());
        let wire = id.to_wire();
        assert!(wire.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        assert_eq!(MenuId::from_wire(&wire), Some(id));
    }

    #[test]
    fn an_unknown_id_is_rejected_rather_than_guessed() {
        assert_eq!(MenuId::from_wire("not_a_menu_item"), None);
        assert_eq!(MenuId::from_wire("net_if_notanumber"), None);
        assert_eq!(MenuId::from_wire("audio_dev_uid_xyz"), None);
    }
}

// ── Accelerators ──────────────────────────────────────────────────────────────

/// A keyboard shortcut, as a property of the action rather than of any menu
/// that shows it.
///
/// Deriving it here means the native bar, the in-window bar and the key
/// handler cannot disagree about what `Ctrl Q` does, and a new provider gets
/// the bindings for free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accel {
    /// The platform's primary modifier: Command on macOS, Control elsewhere.
    pub primary: bool,
    pub shift: bool,
    /// The key, spelled as the user would read it.
    pub key: &'static str,
}

impl Accel {
    const fn primary(key: &'static str) -> Self {
        Self {
            primary: true,
            shift: false,
            key,
        }
    }

    const fn plain(key: &'static str) -> Self {
        Self {
            primary: false,
            shift: false,
            key,
        }
    }

    /// How this reads on the host, for a menu that draws its own hint.
    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.primary {
            s.push_str(crate::host::KEY_PRIMARY);
        }
        if self.shift {
            s.push_str(crate::host::KEY_SHIFT);
        }
        s.push_str(self.key);
        s
    }
}

impl MenuId {
    /// The shortcut for this action, if it has one.
    pub fn accelerator(&self) -> Option<Accel> {
        match self {
            Self::Restart => Some(Accel::primary("R")),
            Self::ScreenExtended => Some(Accel::plain("F11")),
            Self::DebugScreen => Some(Accel::primary("D")),
            _ => None,
        }
    }
}

#[cfg(test)]
mod accel_tests {
    use super::*;

    #[test]
    fn the_shortcut_belongs_to_the_action_not_to_a_menu() {
        // Whoever renders it, the same action yields the same binding.
        assert_eq!(MenuId::Restart.accelerator(), Some(Accel::primary("R")));
        assert_eq!(MenuId::EjectCurrentMedia.accelerator(), None);
    }

    #[test]
    fn the_primary_modifier_reads_as_the_host_spells_it() {
        let label = Accel::primary("Q").label();
        assert_eq!(label, format!("{}Q", crate::host::KEY_PRIMARY));
    }

    #[test]
    fn a_plain_key_carries_no_modifier() {
        assert_eq!(Accel::plain("F11").label(), "F11");
    }
}
