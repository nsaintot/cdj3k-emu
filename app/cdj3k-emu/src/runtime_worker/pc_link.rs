//! The host PC-link bridge and the guest's `pc_link` cfg setting, kept in
//! step with the menu toggle.

use std::time::{Duration, Instant};

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_runtime::pc_link::PcLink;
use cdj3k_emu_runtime::{CfgClient, QemuConfig};

use super::{persist_inst, Requests, Worker};

// Resend interval for an unacknowledged `pc_link` command (cfgd echoes only
// after its `systemctl` returns).
const PC_LINK_CFG_RESEND: Duration = Duration::from_millis(1500);
/// Spacing between `PcLink::start` attempts while the guest socket is absent.
const PC_LINK_RETRY: Duration = Duration::from_millis(500);
/// `pc_link` failure replies tolerated before the reconcile stops reissuing
/// the command.  An unanswered send means the guest has not opened the cfg
/// port yet and is retried indefinitely; a failure reply means cfgd ran
/// systemctl and it did not work.
const PC_LINK_CFG_MAX_FAILURES: u32 = 4;

pub(super) struct PcLinkState {
    /// Host-side PC-link bridge.  `Some` from the first successful connect
    /// until the toggle goes off or the app stops.  Owns the OS-visible
    /// endpoints across a QEMU respawn and re-dials the socket underneath them.
    link: Option<PcLink>,
    /// True if the user wants pc_link on but the latest connect attempt
    /// failed (typically because the guest's bridge service isn't up yet).
    /// Next worker iteration retries.
    retry_pending: bool,
    last_cfg: Instant,
    last_retry: Instant,
    gave_up: bool,
    last_want: Option<bool>,
}

impl Worker {
    /// Persist a toggle, then drive the guest cfg, the host bridge and its
    /// socket towards the menu's `pc_link_enabled`.
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

    /// The menu's Stop: the bridge comes down with QEMU.
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

    /// Resend `pc_link on/off` until cfgd echoes it.  A lone send can be
    /// dropped (boot race / single-client cfg vport).  On a desired change,
    /// clear the echo so a stale/lost one can't skip it.
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
                Ok(()) => eprintln!("cdj3k-emu: pc_link cfg to guest ({})", on_off(want)),
                Err(e) => eprintln!("cdj3k-emu: pc_link cfg send failed: {e}"),
            }
            self.last_cfg = Instant::now();
        } else if failures >= PC_LINK_CFG_MAX_FAILURES && !self.gave_up {
            self.give_up(want);
        }
    }

    /// The bridge is not running, so the toggle goes back to off and the host
    /// endpoints come down with it.  The item stays clickable: toggling clears
    /// the failure count and tries again.  Written to menu_state rather than
    /// persisted, so the next launch starts from the user's own setting.
    fn give_up(&mut self, want: bool) {
        self.gave_up = true;
        eprintln!(
            "cdj3k-emu: guest refused pc_link {} {} times; not reissuing",
            on_off(want),
            PC_LINK_CFG_MAX_FAILURES
        );
        if want {
            menu_state::lock().pc_link_enabled = false;
            self.link = None;
            self.retry_pending = false;
        }
    }

    /// Bring the host bridge up while the toggle is on and QEMU is up.  Each
    /// attempt that gets past the dial registers an IOKit device, and
    /// rekordbox's polled HID name goes stale if that happens repeatedly, so
    /// the attempts are paced rather than run at the poll cadence.
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
            // A failed start retries on a later tick, unlogged: the worker
            // polls every 50 ms.
            if let Ok(link) = PcLink::start(&config.sock_dir(), config.instance_id) {
                eprintln!("cdj3k-emu: pc-link bridge connected");
                self.link = Some(link);
                self.retry_pending = false;
            }
        }
    }

    /// A QEMU respawn closes the socket without touching the endpoints, so a
    /// live bridge whose socket died is re-dialed rather than rebuilt, keeping
    /// the HID identity rekordbox polled.  While QEMU is down nothing consumes
    /// host->guest frames, so they are discarded as they arrive.
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
            Ok(()) => eprintln!("cdj3k-emu: pc-link re-dialed"),
            // Refused dial is normal during guest boot; retry next tick.
            Err(e) if e.is_retryable() => {}
            // Outbound channel gone; the backends cannot reach the
            // guest.  Rebuild (costs a fresh HID identity).
            Err(e) => {
                eprintln!("cdj3k-emu: pc-link unrecoverable ({e}) - rebuilding");
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
