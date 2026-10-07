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
    Restart,
    ServiceMode,
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

impl MenuId {
    /// Stable string form for providers whose widgets carry a string id.
    pub fn to_wire(&self) -> String {
        match self {
            Self::ManageEmulation => "manage_emulation".into(),
            Self::Restart => "restart".into(),
            Self::ServiceMode => "service_mode".into(),
            Self::Haptic => "haptic".into(),
            Self::PcLink => "pc_link".into(),
            Self::CheckForUpdate => "check_for_update".into(),
            Self::ScreenExtended => "screen_extended".into(),
            Self::JogScreen => "jog_screen".into(),
            Self::MainScreen => "main_screen".into(),
            Self::DebugScreen => "debug_screen".into(),
            Self::Audio => "audio".into(),
            Self::Alc => "alc".into(),
            Self::AudioDeviceDefault => "audio_dev_default".into(),
            Self::AudioDevice(uid) => format!("audio_dev_uid_{}", encode_hex(uid)),
            Self::NetNat => "net_none".into(),
            Self::NetVmnetHost => "net_vmnet_host".into(),
            Self::NetInterface(n) => format!("net_if_{n}"),
            Self::EjectCurrentMedia => "eject_current_media".into(),
            Self::MountVirtualUsb => "mount_virtual_usb".into(),
            Self::CreateVirtualUsb => "create_virtual_usb".into(),
            Self::PhysicalDisk(i) => format!("phys_select_{i}"),
            Self::Instance(n) => format!("instance_{n}"),
        }
    }

    /// Inverse of [`MenuId::to_wire`]. `None` for ids this build does not know,
    /// such as a stale widget from an older model.
    pub fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "manage_emulation" => Self::ManageEmulation,
            "restart" => Self::Restart,
            "service_mode" => Self::ServiceMode,
            "haptic" => Self::Haptic,
            "pc_link" => Self::PcLink,
            "check_for_update" => Self::CheckForUpdate,
            "screen_extended" => Self::ScreenExtended,
            "jog_screen" => Self::JogScreen,
            "main_screen" => Self::MainScreen,
            "debug_screen" => Self::DebugScreen,
            "audio" => Self::Audio,
            "alc" => Self::Alc,
            "audio_dev_default" => Self::AudioDeviceDefault,
            "net_none" => Self::NetNat,
            "net_vmnet_host" => Self::NetVmnetHost,
            "eject_current_media" => Self::EjectCurrentMedia,
            "mount_virtual_usb" => Self::MountVirtualUsb,
            "create_virtual_usb" => Self::CreateVirtualUsb,
            other => {
                if let Some(rest) = other.strip_prefix("audio_dev_uid_") {
                    Self::AudioDevice(decode_hex(rest)?)
                } else if let Some(rest) = other.strip_prefix("net_if_") {
                    Self::NetInterface(rest.parse().ok()?)
                } else if let Some(rest) = other.strip_prefix("phys_select_") {
                    Self::PhysicalDisk(rest.parse().ok()?)
                } else {
                    Self::Instance(other.strip_prefix("instance_")?.parse().ok()?)
                }
            }
        })
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
            MenuId::Restart,
            MenuId::ServiceMode,
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
