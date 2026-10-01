//! Bridged networking on Windows: putting the guest on the real LAN.
//!
//! DJ-Link needs the guest on the same L2 segment as the decks and mixers.
//!
//! # Shape
//!
//! QEMU opens a TAP-Windows6 adapter by connection name (`-netdev tap,ifname=`).
//! The Windows Network Bridge (`ms_bridge`) joins that adapter to the chosen
//! NIC, so the guest's frames and the NIC's share one segment and the guest
//! keeps the MAC QEMU gives it.
//!
//! Making the adapter and the bridge needs admin. The app re-runs itself
//! elevated with a hidden subcommand ([`super::windows_helper`]) that does
//! both behind one UAC prompt.
//!
//! The installer provides the TAP-Windows6 driver: its INF, `tap0901.sys` and
//! `tap0901.cat` under `<resources>/tap-windows6/`.
//!
//! # Lifetime
//!
//! A link lasts as long as the slot uses it ([`super::lease`]): the helper
//! leaves a watcher running that takes the tap out of the bridge once the
//! lease goes or the app exits, crash included. Each bridged start asks for
//! admin once.

use std::fs;
use std::io;

use cdj3k_emu_platform::bundled;
use cdj3k_emu_platform::net::windows_kind::is_valid_adapter_name;
use cdj3k_emu_platform::net::{adapters, is_valid_mac};
use cdj3k_emu_platform::runtime_paths;

use super::lease::Lease;
use super::winbridge::{self, Outcome, Request, RESULT_FILE};
use crate::elevate::run_elevated;

/// The tap driver's INF, relative to the resources directory.
const TAP_INF: &str = "tap-windows6/OemVista.inf";

/// The guest's link for one instance, taken down when this is dropped
/// (module doc, Lifetime).
pub struct WindowsBridge {
    /// The tap adapter's connection name, which QEMU opens.
    pub iface: String,
    lease: Lease,
}

impl WindowsBridge {
    /// Put instance `instance_id` on the LAN through the NIC `iface`.
    ///
    /// The guest wears `mac` from QEMU's `-device`; the tap adapter's own MAC
    /// is derived from it ([`winbridge::tap_mac`]).
    pub fn setup(iface: &str, mac: &str, instance_id: u32) -> io::Result<Self> {
        if !is_valid_adapter_name(iface) {
            return Err(invalid(format!("invalid adapter name: {iface:?}")));
        }
        if !is_valid_mac(mac) {
            return Err(invalid(format!("invalid MAC: {mac:?}")));
        }
        let all = adapters();
        let nic = all
            .iter()
            .find(|a| a.friendly_name == iface && !a.is_tap() && !a.is_bridge())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("no adapter named {iface:?}"))
            })?;
        if nic.is_wifi() {
            eprintln!(
                "cdj3k-emu: {iface} is Wi-Fi; bridging a guest over 802.11 is unreliable \
                 (access points accept one MAC per station), so DJ-Link may not see the deck"
            );
        }

        let tap = winbridge::tap_name(instance_id);
        let dir = runtime_paths::instance_dir(instance_id);
        let have_tap = all.iter().any(|a| a.is_tap() && a.friendly_name == tap);
        let inf = bundled::resources().join(TAP_INF);
        if !have_tap && !inf.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "the TAP-Windows6 driver is not installed ({} is missing); reinstall cdj3k-emu",
                    inf.display()
                ),
            ));
        }
        if !winbridge::is_cmd_safe_path(&dir) {
            return Err(invalid(format!("path not usable on a command line: {dir:?}")));
        }

        runtime_paths::ensure_runtime_base_dir()?;
        let result_path = dir.join(RESULT_FILE);
        let _ = fs::remove_file(&result_path);
        // From here, dropping the bridge lets the claim go.
        let mut link = Self {
            iface: tap.clone(),
            lease: Lease::take(&dir)?,
        };
        let request = Request {
            dir: dir.clone(),
            tap: tap.clone(),
            nic: iface.to_string(),
            inf: inf.is_file().then_some(inf),
            pid: std::process::id(),
            claim: link.lease.claim().to_string(),
            mac: mac.to_string(),
        };

        let exe = std::env::current_exe()?;
        let ran = run_elevated(&request.cmdline(&exe));
        let outcome = fs::read_to_string(&result_path)
            .ok()
            .and_then(|t| Outcome::parse(&t));
        match outcome {
            Some(Outcome::Done) => link.lease.set_watched(),
            Some(Outcome::TapFailed(why)) => {
                return Err(io::Error::other(format!(
                    "could not create the TAP adapter {tap}; is the TAP-Windows6 driver \
                     installed? {why}"
                )))
            }
            Some(Outcome::BridgeFailed(why)) => {
                return Err(io::Error::other(format!(
                    "Windows refused to bridge {tap} with {iface} (Network Bridge needs \
                     Windows 11 22H2 with KB5030310, or a bridge made by hand in Network \
                     Connections): {why}"
                )))
            }
            // The copy never got as far as a verdict: refused prompt, or a crash.
            None => {
                ran?;
                return Err(io::Error::other("the elevated helper left no result"));
            }
        }
        eprintln!("cdj3k-emu: bridge up on {iface}  tap={tap}");
        Ok(link)
    }
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}
