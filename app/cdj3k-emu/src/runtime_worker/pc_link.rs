//! The host PC-link bridge and the guest's `pc_link` cfg setting, kept in
//! step with the menu toggle.

use std::time::{Duration, Instant};

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_runtime::pc_link::PcLink;
use cdj3k_emu_runtime::{CfgClient, QemuConfig};

use super::{persist_inst, Requests, Worker};

/// The resend interval for an unacknowledged `pc_link` command. cfgd echoes
/// the command only after its `systemctl` call returns.
const PC_LINK_CFG_RESEND: Duration = Duration::from_millis(1500);
/// The interval between `PcLink::start` attempts while the guest socket is absent.
const PC_LINK_RETRY: Duration = Duration::from_millis(500);
/// The number of `pc_link` failure replies after which the command is no
/// longer sent. An unanswered send means the guest has not opened the cfg
/// port yet, and it is retried without limit. A failure reply means cfgd ran
/// systemctl and the call failed.
const PC_LINK_CFG_MAX_FAILURES: u32 = 4;

pub(super) struct PcLinkState {
    /// The host-side PC-link bridge. It is `Some` from the first successful
    /// connect until the toggle goes off or the app stops. It keeps the
    /// endpoints the OS sees across a QEMU restart and reconnects the socket
    /// underneath them.
    link: Option<PcLink>,
    /// True when the user wants pc_link on but the latest connect attempt
    /// failed, usually because the guest's bridge service is not up yet. The
    /// next worker iteration retries.
    retry_pending: bool,
    last_cfg: Instant,
    last_retry: Instant,
    gave_up: bool,
    last_want: Option<bool>,
}

impl Worker {
    /// Save a toggle, then bring the guest's cfg setting, the host bridge and
    /// its socket in line with the menu's `pc_link_enabled`.
    pub(super) fn reconcile_pc_link(&mut self, req: &Requests) {
        if req.pc_link_toggle {
            persist_inst(self.config.instance_id, |s| {
                s.pc_link_enabled = req.pc_link_enabled
            });
            self.pc_link.toggle(req.pc_link_enabled);
        }
        let want = req.pc_link_enabled;
        let qemu_up = self.instance.is_some();
        self.pc_link.reconcile_cfg(&self.cfg_client, want, qemu_up);
        self.pc_link.ensure_started(want, qemu_up, &self.config);
        self.pc_link.keep_alive(want, qemu_up);
    }
}

impl PcLinkState {
    pub(super) fn new() -> Self {
        Self {
            link: None,
            retry_pending: false,
            last_cfg: Instant::now() - PC_LINK_CFG_RESEND,
            last_retry: Instant::now() - PC_LINK_RETRY,
            gave_up: false,
            last_want: None,
        }
    }

    pub(super) fn close_bridge(&mut self) {
        self.link = None;
    }

    /// Handle the menu's Stop by closing the bridge along with QEMU.
    pub(super) fn stop(&mut self) {
        self.link = None;
        self.retry_pending = false;
        self.gave_up = false;
    }

    /// A new QEMU gets a fresh allowance of refused `pc_link` commands.
    pub(super) fn on_qemu_stopped(&mut self) {
        self.gave_up = false;
    }

    fn toggle(&mut self, enabled: bool) {
        if enabled {
            self.retry_pending = true;
        } else {
            self.link = None;
            self.retry_pending = false;
        }
    }

    /// Resend `pc_link on` or `pc_link off` until cfgd echoes it. A single send
    /// can be lost while the guest boots, or because the cfg virtio port takes
    /// only one client. When the wanted state changes, clear the recorded echo
    /// so that a stale or lost echo cannot skip the resend.
    fn reconcile_cfg(&mut self, cfg: &CfgClient, want: bool, qemu_up: bool) {
        if self.last_want != Some(want) {
            self.last_want = Some(want);
            cfg.reset_pc_link_state();
            self.last_cfg = Instant::now() - PC_LINK_CFG_RESEND;
            self.gave_up = false;
        }
        let failures = cfg.pc_link_failures();
        if qemu_up
            && cfg.is_connected()
            && cfg.pc_link_state() != Some(want)
            && failures < PC_LINK_CFG_MAX_FAILURES
            && self.last_cfg.elapsed() >= PC_LINK_CFG_RESEND
        {
            match cfg.pc_link(want) {
                Ok(()) => eprintln!("cdj3k-emu: sent pc_link {} to the guest", on_off(want)),
                Err(e) => eprintln!("cdj3k-emu: sending pc_link to the guest failed: {e}"),
            }
            self.last_cfg = Instant::now();
        } else if failures >= PC_LINK_CFG_MAX_FAILURES && !self.gave_up {
            self.give_up(want);
        }
    }

    /// Stop sending a command that the guest keeps refusing. When the toggle
    /// is on, it goes back to off and the host endpoints close. The menu item
    /// stays clickable, and toggling it clears the failure count and tries
    /// again. The change goes to menu_state only, so the next launch starts
    /// from the user's saved setting.
    fn give_up(&mut self, want: bool) {
        self.gave_up = true;
        eprintln!(
            "cdj3k-emu: the guest refused pc_link {} {} times; the command is no longer sent",
            on_off(want),
            PC_LINK_CFG_MAX_FAILURES
        );
        if want {
            menu_state::lock().pc_link_enabled = false;
            self.link = None;
            self.retry_pending = false;
        }
    }

    /// Start the host bridge while the toggle is on and QEMU runs. Each attempt
    /// that connects registers an IOKit device, and repeated registrations make
    /// the HID name that rekordbox polls go stale, so attempts are spaced by
    /// `PC_LINK_RETRY`.
    fn ensure_started(&mut self, want: bool, qemu_up: bool, config: &QemuConfig) {
        if want && self.link.is_none() && qemu_up {
            self.retry_pending = true;
        }
        if self.retry_pending
            && self.link.is_none()
            && qemu_up
            && self.last_retry.elapsed() >= PC_LINK_RETRY
        {
            self.last_retry = Instant::now();
            // A failed start is not logged, because it is retried every
            // `PC_LINK_RETRY` while the guest boots.
            if let Ok(link) = PcLink::start(&config.sock_dir(), config.instance_id) {
                eprintln!("cdj3k-emu: the PC Link bridge connected.");
                self.link = Some(link);
                self.retry_pending = false;
            }
        }
    }

    /// A QEMU restart closes the socket and leaves the endpoints open, so a
    /// bridge whose socket closed is reconnected and keeps the HID identity
    /// that rekordbox polled. While QEMU is down, nothing reads the frames
    /// sent to the guest, so they are discarded as they arrive.
    fn keep_alive(&mut self, want: bool, qemu_up: bool) {
        if !qemu_up {
            if let Some(link) = self.link.as_mut() {
                link.drain_while_down();
            }
        }
        if want && qemu_up {
            self.redial_if_dead();
        }
    }

    fn redial_if_dead(&mut self) {
        let Some(link) = self.link.as_mut() else {
            return;
        };
        if link.is_alive() || self.last_retry.elapsed() < PC_LINK_RETRY {
            return;
        }
        self.last_retry = Instant::now();
        match link.reconnect() {
            Ok(()) => eprintln!("cdj3k-emu: the PC Link bridge reconnected."),
            // A refused connection is normal while the guest boots. The next
            // attempt follows after `PC_LINK_RETRY`.
            Err(e) if e.is_retryable() => {}
            // The outbound channel is gone, so the backends cannot reach the
            // guest. Rebuild the bridge, which gives it a new HID identity.
            Err(e) => {
                eprintln!("cdj3k-emu: the PC Link bridge cannot recover ({e}), so it is rebuilt.");
                self.link = None;
                self.retry_pending = true;
            }
        }
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}
