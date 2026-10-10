//! Commands between windows on the slots' window sockets: the ones this
//! window answers, and the replies to the ones it sends.

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_platform::window_socket::{Command, PendingReply, Reply, WindowSocket};

use super::{CdjShell, Screen, SetupStep};
use crate::app::mods_view;

/// What another window learns from a reply to `delete`, for the picker's
/// status line.
fn delete_note(reply: &std::io::Result<Option<Reply>>) -> &'static str {
    match reply {
        Ok(Some(Reply::Ok | Reply::NotRunning)) => {
            "its window is stopping the emulation to empty the slot"
        }
        Ok(Some(Reply::Busy)) => "its window is busy; try again in a moment",
        Ok(None) => "its window is not listening; close it, then delete the slot",
        Err(_) | Ok(Some(Reply::Yes | Reply::No)) => "asking its window to empty the slot failed",
    }
}

/// The Mods view's note for a reply to `restart-mods`; `None` when the
/// restart was accepted.
fn restart_mods_note(slot: u32, reply: &std::io::Result<Option<Reply>>) -> Option<String> {
    match reply {
        Ok(Some(Reply::Ok)) => None,
        Ok(Some(Reply::Busy)) => Some(format!("Slot {slot} is busy; try again in a moment")),
        Ok(Some(Reply::NotRunning) | None) => Some(format!("Slot {slot} is not running")),
        Err(e) => Some(format!("Restarting slot {slot} failed: {e}")),
        Ok(Some(Reply::Yes | Reply::No)) => Some(format!(
            "Restarting slot {slot} failed: its window sent an unexpected reply"
        )),
    }
}

impl CdjShell {
    /// Answer the commands that other windows sent on the slot's window
    /// socket.
    pub(super) fn serve_window_socket(&mut self) {
        while let Some((command, responder)) =
            self.window_socket.as_ref().and_then(WindowSocket::next)
        {
            let reply = self.on_command(command);
            // A Mods view asks `restart-pending` every 2 s.
            if command != Command::RestartPending {
                eprintln!(
                    "cdj3k-emu: Slot {}: another window sent {}, and this window replied {}.",
                    self.instance,
                    command.as_str(),
                    reply.as_str()
                );
            }
            responder.reply(reply);
        }
    }

    fn on_command(&mut self, command: Command) -> Reply {
        if self.shutdown_in_progress || self.close_window {
            return match command {
                Command::Quit => Reply::Ok,
                _ => Reply::Busy,
            };
        }
        match command {
            // The updater in another window needs this window closed. A
            // firmware install running here must finish first.
            Command::Quit if self.wizard.is_running() => Reply::Busy,
            Command::Quit => {
                self.close_window = true;
                Reply::Ok
            }
            // Emptying the slot resets the wizard, so an install running in
            // this window, into any slot, must finish first.
            Command::Delete if self.wizard.is_running() => Reply::Busy,
            Command::Delete => self.delete_on_request(),
            Command::RestartMods => self.restart_for_mods(),
            Command::RestartInstall => self.restart_into_install(),
            Command::InstallReady => {
                self.install_check_due = true;
                Reply::Ok
            }
            Command::RestartPending => self.restart_pending_reply(),
        }
    }

    /// Another window asked this one to empty its slot.
    fn delete_on_request(&mut self) -> Reply {
        if !matches!(self.setup, Some(SetupStep::StoppingToDelete)) {
            self.view_slot(self.instance);
            self.begin_delete();
        }
        Reply::Ok
    }

    /// Whether a restart would boot different mods than the running
    /// emulation did.
    fn restart_pending_reply(&self) -> Reply {
        if !self.emulation_running() {
            Reply::NotRunning
        } else if mods_view::restart_pending(self.instance) {
            Reply::Yes
        } else {
            Reply::No
        }
    }

    /// Restart the emulation so that it boots with the slot's current mods.
    /// While a restart is already pending or under way, only increment
    /// `mods_restart_seq`; the runtime worker restarts QEMU once more if the
    /// request came after that restart read the mods.
    pub(super) fn restart_for_mods(&mut self) -> Reply {
        if !matches!(self.screen, Screen::Panel) || cdj3k_emu_runtime::worker_is_finished() {
            return Reply::NotRunning;
        }
        if self.worker_stopping {
            return Reply::Busy;
        }
        let mut s = menu_state::lock();
        // If QEMU is not running and no restart is about to start it, QEMU
        // failed to start or was stopped. `qemu_running` turns true as soon as
        // QEMU is spawned.
        if !s.qemu_running && !s.qemu_respawning && !s.restart_requested {
            return Reply::NotRunning;
        }
        s.mods_restart_seq += 1;
        if !s.restart_requested && !s.shade_forced && !s.qemu_respawning {
            s.shade_forced = true;
            s.restart_requested = true;
        }
        Reply::Ok
    }

    /// Restart the running emulation into the finished install waiting for
    /// the slot. If no install is waiting, the emulation already runs the
    /// newest one, because an install is applied only while no emulation runs.
    fn restart_into_install(&mut self) -> Reply {
        if self.pending_launch.is_some() {
            // The pending launch applies the install.
            return Reply::Ok;
        }
        if self.installing_here() || self.worker_stopping {
            return Reply::Busy;
        }
        if cdj3k_emu_runtime::worker_is_finished() {
            return Reply::NotRunning;
        }
        match cdj3k_emu_storage::pending_install_retry(self.instance) {
            Some(record) => {
                self.reboot_into(record.model);
                Reply::Ok
            }
            None if menu_state::lock().qemu_running => Reply::Ok,
            // QEMU failed to start or was stopped.
            None => Reply::NotRunning,
        }
    }

    /// Send `command` to `slot`'s window. [`take_replies`](Self::take_replies)
    /// acts on the reply when it arrives.
    pub(super) fn send_command(&mut self, slot: u32, command: Command) {
        let ctx = self.ctx.clone();
        let pending = PendingReply::send(slot, command, move || ctx.request_repaint());
        self.sent.push((slot, command, pending));
    }

    /// Act on the replies that arrived for commands sent to other windows.
    pub(super) fn take_replies(&mut self) {
        for (slot, command, pending) in std::mem::take(&mut self.sent) {
            match pending.take() {
                Some(reply) => self.on_reply(slot, command, reply),
                None => self.sent.push((slot, command, pending)),
            }
        }
    }

    fn on_reply(&mut self, slot: u32, command: Command, reply: std::io::Result<Option<Reply>>) {
        if let Err(e) = &reply {
            eprintln!(
                "cdj3k-emu: Sending {} to slot {slot}'s window failed: {e}",
                command.as_str()
            );
        }
        match command {
            Command::Delete => self.on_delete_reply(slot, delete_note(&reply)),
            Command::RestartMods => {
                if self.mods.as_ref().is_some_and(|p| p.slot == slot) {
                    self.note_mods(restart_mods_note(slot, &reply));
                }
            }
            // If the slot's window did not accept the restart, or no window
            // holds the slot, open or raise that window. The install is
            // applied when the slot's emulation next starts.
            Command::RestartInstall => {
                if !matches!(reply, Ok(Some(Reply::Ok))) {
                    cdj3k_emu_platform::menu::open_instance(slot);
                }
            }
            Command::RestartPending => self.on_restart_pending_reply(slot, &reply),
            Command::Quit | Command::InstallReady => {}
        }
    }

    /// Show `note` on the picker's status line, if this slot's delete
    /// request is still the latest one.
    fn on_delete_reply(&mut self, slot: u32, note: &'static str) {
        if let Some((asked, at, _)) = self.delete_asked {
            if asked == slot {
                self.delete_asked = Some((slot, at, note));
            }
        }
    }

    /// Record whether the window that runs `slot` would boot different mods
    /// on a restart. A reply other than yes or no keeps the last answer until
    /// the window can give one.
    fn on_restart_pending_reply(&mut self, slot: u32, reply: &std::io::Result<Option<Reply>>) {
        let answer = match reply {
            Ok(Some(Reply::Yes)) => true,
            Ok(Some(Reply::No)) => false,
            _ => return,
        };
        if let Some(page) = self.mods.as_mut().filter(|p| p.slot == slot) {
            page.remote_pending = answer;
            if let mods_view::Run::Restart { pending } = &mut page.run {
                *pending = page.remote_pending;
            }
        }
    }
}
