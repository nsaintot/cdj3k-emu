//! The Mods view's side of the shell: reading the slot, editing its list,
//! adding mods from files, folders and URLs, and restarting onto them.

use std::time::{Duration, Instant};

use cdj3k_emu_platform::window_socket::{Command, Reply};
use cdj3k_emu_platform::{desktop, menu_state};
use cdj3k_emu_storage::mods::SlotMods;

use super::{CdjShell, Screen, SetupStep};
use crate::app::mods_view::{self, ModsAction, ModsPage};

/// How often the Mods view and the Mods bar read their slot again.
const MODS_READ_EVERY: Duration = Duration::from_secs(2);
/// The largest mod archive Add from URL takes.
const MOD_DOWNLOAD_LIMIT: u64 = 512 << 20;

/// The file name a download from `url` is saved under: the URL's last path
/// segment, or `mod.tgz` when that is empty or not a plain file name.
fn download_name(url: &str) -> String {
    url.rsplit('/')
        .next()
        .map(|n| n.split(['?', '#']).next().unwrap_or(n))
        .filter(|n| !n.is_empty() && !n.contains(['\\', ':']) && *n != "..")
        .unwrap_or("mod.tgz")
        .to_string()
}

impl CdjShell {
    /// Show `slot`'s mods, over Manage Emulation.
    pub(super) fn open_mods(&mut self, slot: u32) {
        self.leave_mods();
        let mut page = ModsPage::new(slot);
        page.refresh(
            self.instance,
            self.emulation_running(),
            self.slot_runs_elsewhere(slot),
        );
        self.mods = Some(page);
        self.mods_read = Instant::now();
        self.setup = Some(SetupStep::Mods);
        self.ask_restart_pending();
    }

    pub(super) fn emulation_running(&self) -> bool {
        matches!(self.screen, Screen::Panel) && menu_state::lock().qemu_running
    }

    pub(super) fn refresh_mods_bar(&mut self) {
        let stale = self.mods_bar.as_ref().is_none_or(|(slot, at, _)| {
            *slot != self.viewed_slot || at.elapsed() >= MODS_READ_EVERY
        });
        if stale && self.setup.is_some() {
            self.mods_bar = Some((
                self.viewed_slot,
                Instant::now(),
                mods_view::mods_bar(self.viewed_slot),
            ));
        }
    }

    /// Close the Mods view. A pending replace is discarded, and a file pick or
    /// download still in progress is dropped.
    pub(super) fn leave_mods(&mut self) {
        self.discard_pending_mod();
        self.mods = None;
        self.mods_pick = None;
        self.mods_download = None;
    }

    /// Re-read the slot when due, and collect a finished file pick or
    /// download.
    pub(super) fn poll_mods(&mut self) {
        if let Some(answer) = self.mods_pick.as_ref().and_then(|p| p.take()) {
            self.mods_pick = None;
            if let Some(path) = answer {
                self.add_mod_path(&path, None);
            }
        }
        let landed = self
            .mods_download
            .as_ref()
            .and_then(|(url, rx)| rx.try_recv().ok().map(|r| (url.clone(), r)));
        if let Some((url, result)) = landed {
            self.mods_download = None;
            match result {
                Ok(file) => {
                    // The add unpacks the archive, so the download can be deleted.
                    self.add_mod_path(&file, Some(&url));
                    if let Some(dir) = file.parent() {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                }
                Err(e) => self.note_mods(Some(e)),
            }
        }
        if self.mods_read.elapsed() >= MODS_READ_EVERY {
            self.reread_mods();
        }
    }

    fn reread_mods(&mut self) {
        let running = self.emulation_running();
        let elsewhere = self
            .mods
            .as_ref()
            .is_some_and(|page| self.slot_runs_elsewhere(page.slot));
        if let Some(page) = self.mods.as_mut() {
            page.refresh(self.instance, running, elsewhere);
        }
        self.mods_read = Instant::now();
        self.mods_bar = None;
        self.ask_restart_pending();
    }

    /// Send `restart-pending` to the other window that runs the viewed slot,
    /// unless an earlier one is still waiting for its reply. Do nothing when
    /// this window or no window runs the slot.
    fn ask_restart_pending(&mut self) {
        let Some(slot) = self.mods.as_ref().map(|p| p.slot) else {
            return;
        };
        let asking = self
            .sent
            .iter()
            .any(|&(s, c, _)| s == slot && c == Command::RestartPending);
        if slot != self.instance && self.slot_runs_elsewhere(slot) && !asking {
            self.send_command(slot, Command::RestartPending);
        }
    }

    /// Show `error` on the Mods view's toolbar, or clear what it showed.
    pub(super) fn note_mods(&mut self, error: Option<String>) {
        if let Some(page) = self.mods.as_mut() {
            page.note = error.map(|e| (e, true));
        }
        self.reread_mods();
    }

    /// Add a picked, dropped or downloaded path: an archive is installed, a
    /// folder (or its `mod.toml`) is used in place. A mod whose name is in the
    /// list already waits for the Replace dialog.
    fn add_mod_path(&mut self, path: &std::path::Path, origin: Option<&str>) {
        let Some(slot) = self.mods.as_ref().map(|p| p.slot) else {
            return;
        };
        let mut list = SlotMods::load(slot);
        let result = list
            .prepare(path, origin)
            .and_then(|add| match &add.replaces {
                Some(old) => {
                    if let Some(page) = self.mods.as_mut() {
                        page.replace = Some((
                            add.manifest.name.clone(),
                            old.clone(),
                            add.manifest.version.clone(),
                        ));
                    }
                    self.mods_pending = Some(add);
                    Ok(())
                }
                None => list.commit(add).map(|_| ()),
            });
        self.note_mods(result.err().map(|e| e.to_string()));
    }

    /// Drop a same-name add the Replace dialog was asking about.
    fn discard_pending_mod(&mut self) {
        self.mods_pending = None;
        if let Some(page) = self.mods.as_mut() {
            page.replace = None;
        }
    }

    /// Act on what the Mods view asked for. The view handles the actions
    /// that change only what it shows; the rest read or write the slot.
    pub(super) fn apply_mods_action(&mut self, action: ModsAction) {
        let Some(page) = self.mods.as_mut() else {
            return;
        };
        let slot = page.slot;
        let Some(action) = page.apply(action) else {
            return;
        };
        match action {
            ModsAction::Back => {
                self.leave_mods();
                self.setup = Some(SetupStep::Pick);
            }
            ModsAction::Close => self.close_setup(),
            ModsAction::Restart => self.restart_mods_slot(slot),
            ModsAction::Start => cdj3k_emu_platform::menu::open_instance(slot),
            ModsAction::Toggle(name) => self.edit_mods(slot, |l| {
                let on = l.get(&name).is_some_and(|m| m.enabled);
                l.set_enabled(&name, !on)
            }),
            ModsAction::Reorder(from, to) => self.edit_mods(slot, |l| l.reorder(from, to)),
            ModsAction::Remove(name) | ModsAction::Eject(name) => self.remove_mod(slot, &name),
            ModsAction::Add => self.pick_mod(),
            ModsAction::Replace => self.replace_mod(slot),
            ModsAction::KeepOld => self.discard_pending_mod(),
            ModsAction::AddUrl(url) => self.download_mod(slot, url),
            ModsAction::Open(url) => desktop::open_url(&url),
            ModsAction::Dropped(paths) => paths.iter().for_each(|p| self.add_mod_path(p, None)),
            // The view applied these itself.
            ModsAction::AskRemove(_)
            | ModsAction::CancelRemove
            | ModsAction::AskUrl
            | ModsAction::CancelUrl
            | ModsAction::OpenLog(_)
            | ModsAction::CloseLog => {}
        }
    }

    /// Change `slot`'s list with `edit`, then show how it went.
    fn edit_mods(&mut self, slot: u32, edit: impl FnOnce(&mut SlotMods) -> std::io::Result<()>) {
        let result = edit(&mut SlotMods::load(slot));
        self.note_mods(result.err().map(|e| e.to_string()));
    }

    /// Remove a mod, or eject a folder used in place, once the remove dialog
    /// is gone.
    fn remove_mod(&mut self, slot: u32, name: &str) {
        if let Some(page) = self.mods.as_mut() {
            page.remove = None;
        }
        self.edit_mods(slot, |l| l.remove(name));
    }

    /// The Replace dialog was accepted: commit the add it was asking about.
    fn replace_mod(&mut self, slot: u32) {
        if let Some(page) = self.mods.as_mut() {
            page.replace = None;
        }
        let result = match self.mods_pending.take() {
            Some(add) => SlotMods::load(slot).commit(add).map(|_| ()),
            None => Ok(()),
        };
        self.note_mods(result.err().map(|e| e.to_string()));
    }

    /// Open the Add mod dialog, unless it is up already.
    fn pick_mod(&mut self) {
        if self.mods_pick.is_none() {
            self.mods_pick = Some(desktop::PendingPick::open_mod(
                "Choose a mod: a .tgz archive, or a mod folder",
            ));
        }
    }

    /// Restart the emulation of the viewed slot onto its current mods: this
    /// window's own, or by asking the window that runs it.
    fn restart_mods_slot(&mut self, slot: u32) {
        if slot == self.instance {
            let note = match self.restart_for_mods() {
                Reply::Ok | Reply::Yes | Reply::No => None,
                Reply::Busy => Some("The emulation is busy; try again in a moment"),
                Reply::NotRunning => Some("The emulation is not running"),
            };
            self.note_mods(note.map(str::to_string));
            return;
        }
        if let Some(page) = self.mods.as_mut() {
            page.note = Some((format!("Asking slot {slot} to restart"), false));
        }
        self.send_command(slot, Command::RestartMods);
    }

    /// Fetch `url` on a thread of its own, unless a download is under way;
    /// [`Self::poll_mods`] installs it.
    fn download_mod(&mut self, slot: u32, url: String) {
        if self.mods_download.is_some() {
            return;
        }
        if let Some(page) = self.mods.as_mut() {
            page.url = None;
            page.note = Some((format!("Downloading {url}…"), false));
        }
        let name = download_name(&url);
        static DOWNLOADS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = DOWNLOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = cdj3k_emu_storage::mods::mods_dir(slot)
            .join(format!(".download-{}-{n}", std::process::id()));
        let (tx, rx) = std::sync::mpsc::channel();
        let fetch_url = url.clone();
        let spawned = std::thread::Builder::new()
            .name("cdj3k-emu-mod-download".into())
            .spawn(move || {
                let result = std::fs::create_dir_all(&dir)
                    .map_err(|e| e.to_string())
                    .and_then(|()| {
                        let dest = dir.join(&name);
                        cdj3k_emu_update::fetch_file(&fetch_url, &dest, MOD_DOWNLOAD_LIMIT)
                            .map(|()| dest)
                            .map_err(|e| e.to_string())
                    });
                // Delete the download if it failed or nobody waits for it.
                let failed = result.is_err();
                if tx.send(result).is_err() || failed {
                    let _ = std::fs::remove_dir_all(&dir);
                }
            });
        match spawned {
            Ok(_) => self.mods_download = Some((url, rx)),
            Err(e) => self.note_mods(Some(e.to_string())),
        }
    }
}
