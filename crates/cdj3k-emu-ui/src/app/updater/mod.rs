//! The update window for Windows and Linux, in the setup window's style.
//!
//! The window offers the update, downloads it, prepares it while the
//! emulation runs, and then offers the restart. The restart closes the other
//! windows, stops the emulation and installs the prepared build: a rename for
//! an AppImage, the installer on Windows. "Later" installs it at quit. A deb
//! or an rpm is downloaded to the downloads folder instead, with the command
//! that installs it.
//!
//! Automatic checks run when the first window opens, then every 6 hours in
//! the lowest-numbered open slot. A skipped release and older ones are not
//! offered; "Remind Me Later" pauses offers for 24 hours. With automatic
//! download on, the update is downloaded and prepared without the window,
//! which then offers the restart. Work runs on background threads and
//! reports through [`Phase`].
//!
//! On macOS, Sparkle handles the update and its window
//! ([`cdj3k_emu_update::native`]); the same restart closes the other slots
//! and stops the emulation, then lets Sparkle install.

mod notes;
mod view;
mod work;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_storage::AppSettings;
use cdj3k_emu_update::native::Event as NativeEvent;
use cdj3k_emu_update::{Kind, Offer, Version};
use egui::{Pos2, Vec2};

use super::theme;
use work::Shared;

/// The window has the same width as the setup window.
const WINDOW_SIZE: [f32; 2] = [760.0, 560.0];
/// How long "Remind Me Later" pauses automatic offers.
const REMIND_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// The interval between automatic checks.
const RECHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The retry schedule for a failed automatic check, such as one made with no
/// network or while a slot's network bridge comes up: every [`RETRY_FIRST`]
/// for [`RETRY_STEADY`], then doubling up to [`RETRY_MAX`]. A successful
/// check resets it.
const RETRY_FIRST: Duration = Duration::from_secs(10);
const RETRY_STEADY: Duration = Duration::from_secs(2 * 60);
const RETRY_MAX: Duration = Duration::from_secs(30 * 60);
/// How long the window keeps itself in front after the emulation restarts.
const RAISE_FOR: Duration = Duration::from_secs(3);
/// The longest wait for the emulation to stop during a restart.
const STOP_WAIT: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
enum Phase {
    #[default]
    Checking,
    Current {
        latest: String,
        notes: String,
        release_notes: String,
    },
    Unmanaged,
    NoPackage {
        latest: String,
    },
    Available(Offer),
    Downloading {
        offer: Offer,
        done: u64,
        total: u64,
    },
    /// The new build is being prepared next to this one.
    Preparing(Offer),
    /// The update is prepared and waits for the restart.
    Ready(Offer),
    /// A deb or an rpm in the downloads folder, for the package manager.
    Downloaded {
        offer: Offer,
        path: PathBuf,
    },
    Restarting(Step),
    /// The update is installed, or the installer is waiting; this window
    /// closes.
    Exiting,
    Failed {
        message: String,
        offer: Option<Offer>,
    },
}

#[derive(Clone)]
enum Step {
    /// The other windows are closing.
    Closing,
    /// This window's emulation is stopping.
    Stopping,
    /// The prepared build is being installed.
    Installing(Offer),
    /// Sparkle took over and ends the process itself.
    HandedOff,
}

/// How a restart ends, once the other windows are closed and the emulation
/// has stopped.
#[derive(Clone)]
enum Finish {
    /// Install the prepared build and start the app again.
    Install(Offer),
    /// Let Sparkle proceed with the install chosen in its window.
    NativeRelaunch,
    /// Let Sparkle install the update pending until quit, and relaunch.
    NativeInstall,
}

/// What the shell does after a frame of the update window.
pub(in crate::app) enum UpdaterEvent {
    None,
    /// Close this window; the new build starts after it.
    Exit,
    /// Stop the emulation and keep the window open.
    StopEmulation,
    /// The restart failed after stopping the emulation; start it again.
    ResumeEmulation,
}

pub(in crate::app) struct Updater {
    instance: u32,
    shared: Arc<Shared>,
    open: bool,
    /// The running check is automatic: the window opens only for an offer.
    silent: bool,
    /// When the next automatic check is due.
    next_auto: Instant,
    /// The wait before the next retry.
    retry_delay: Duration,
    /// When automatic checks started failing.
    failing_since: Option<Instant>,
    /// The next automatic check is this window's startup check.
    first_round: bool,
    /// The next automatic check retries this window's failed check.
    retrying: bool,
    /// The window has been opened for the current ready update or download.
    ready_shown: bool,
    /// The version whose offer was closed. It is not offered again in this run.
    dismissed: Option<String>,
    /// Keep the window in front until this time.
    raise_until: Option<Instant>,
    auto_check: bool,
    auto_install: bool,
    /// Sparkle handles updates on this host.
    native: bool,
    pos: Option<Pos2>,
    shaped: bool,
}

impl Updater {
    pub(in crate::app) fn new(instance: u32) -> Self {
        let settings = AppSettings::load();
        // Only release bundles check for updates; a developer build has no
        // feed.
        let native = cdj3k_emu_update::installed_kind() == Some(Kind::Dmg)
            && cdj3k_emu_update::native::start(instance);
        Self {
            instance,
            shared: Arc::default(),
            open: false,
            silent: false,
            next_auto: Instant::now(),
            retry_delay: RETRY_FIRST,
            failing_since: None,
            first_round: true,
            retrying: false,
            ready_shown: false,
            dismissed: None,
            raise_until: None,
            auto_check: settings.update_auto_check,
            auto_install: settings.update_auto_install,
            native,
            pos: None,
            shaped: false,
        }
    }

    fn busy(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire)
    }

    /// Starts due automatic checks and menu checks, and opens the window when
    /// there is something to show. Call every frame.
    fn poll(&mut self, ctx: &egui::Context) {
        let busy = self.busy();
        if self.auto_check
            && Instant::now() >= self.next_auto
            && !busy
            && !self.open
            && !self.shared.has_prepared()
        {
            self.auto_check_round(ctx);
        }
        if std::mem::take(&mut menu_state::lock().update_check_requested) {
            self.open = true;
            // A download, preparation or prepared update is shown as is.
            if !busy && !self.shared.has_prepared() {
                self.check(ctx, false);
            }
        }
        let phase = self.shared.phase();
        if self.silent && !self.busy() {
            self.silent = false;
            self.schedule_retry(&phase);
            if let Phase::Available(offer) = &phase {
                self.offer_found(ctx, offer);
            }
        }
        self.open_when_done(&phase);
    }

    /// Only one window runs each check. The startup check runs in the first
    /// window, when no other slot is open; the periodic check runs in the
    /// lowest open slot, and a retry runs in the window whose check failed.
    /// Other windows skip the round.
    fn auto_check_round(&mut self, ctx: &egui::Context) {
        self.next_auto = Instant::now() + RECHECK_EVERY;
        let ours = if std::mem::take(&mut self.retrying) {
            true
        } else if std::mem::take(&mut self.first_round) {
            cdj3k_emu_update::other_slots_open(self.instance).is_empty()
        } else {
            lowest_open_slot(self.instance)
        };
        if ours {
            self.check(ctx, true);
        }
    }

    /// Handles the end of an automatic check: a failure retries on the
    /// backoff schedule, and any other result resets the schedule.
    fn schedule_retry(&mut self, phase: &Phase) {
        if matches!(phase, Phase::Failed { .. }) {
            let since = *self.failing_since.get_or_insert_with(Instant::now);
            self.next_auto = Instant::now() + self.retry_delay;
            self.retrying = true;
            if since.elapsed() >= RETRY_STEADY {
                self.retry_delay = (self.retry_delay * 2).min(RETRY_MAX);
            }
        } else {
            self.failing_since = None;
            self.retry_delay = RETRY_FIRST;
        }
    }

    /// An automatic check found `offer`: download it without the window, or
    /// open the window to offer it.
    fn offer_found(&mut self, ctx: &egui::Context, offer: &Offer) {
        if !worth_offering(offer) || self.dismissed.as_ref() == Some(&offer.version) {
            return;
        }
        if self.auto_install && offer.kind.installs_itself() {
            // The download is silent as well, so a failure retries like a
            // failed check.
            self.update(ctx, offer.clone());
            self.silent = true;
        } else {
            self.open = true;
        }
    }

    /// Open the window once for a ready update or a finished download.
    fn open_when_done(&mut self, phase: &Phase) {
        let ready = matches!(phase, Phase::Ready(_));
        let done = ready || matches!(phase, Phase::Downloaded { .. });
        if done && !std::mem::replace(&mut self.ready_shown, true) {
            self.open = true;
        }
        if !done {
            self.ready_shown = false;
        }
        menu_state::lock().update_ready = ready;
    }

    /// Handles the requests from the menu and from Sparkle on hosts where
    /// Sparkle runs the updates.
    fn poll_native(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut menu_state::lock().update_check_requested) {
            let waiting = lock(&self.shared.native_waiting).take();
            if let Some(finish) = waiting {
                self.restart(ctx, finish);
            } else if cdj3k_emu_update::native::install_pending() {
                self.restart(ctx, Finish::NativeInstall);
            } else {
                cdj3k_emu_update::native::check_now();
            }
        }
        for event in cdj3k_emu_update::native::take_events() {
            if event == NativeEvent::RelaunchRequested {
                self.restart(ctx, Finish::NativeRelaunch);
            }
        }
        menu_state::lock().update_ready = cdj3k_emu_update::native::install_pending()
            || lock(&self.shared.native_waiting).is_some();
    }

    /// Called when the window closes for good: install a prepared update
    /// without restarting. Skipped while another window is open, which would
    /// keep running the old build.
    ///
    /// Returns true when Sparkle took over and ends the process itself; the
    /// window must stay open. `host_quits` says that AppKit is ending the app,
    /// as on Cmd+Q: Sparkle then installs its update as the app terminates,
    /// and it cannot end the app itself while AppKit waits for an answer.
    pub(in crate::app) fn apply_on_quit(&mut self, host_quits: bool) -> bool {
        if !cdj3k_emu_update::other_slots_open(self.instance).is_empty() {
            return false;
        }
        if self.native {
            return !host_quits && cdj3k_emu_update::native::install_now(false);
        }
        if self.shared.declined.load(Ordering::Acquire) {
            return false;
        }
        if let Some(p) = lock(&self.shared.prepared).take() {
            if let Err(e) = cdj3k_emu_update::apply(&p, false) {
                eprintln!("cdj3k-emu: installing the update on quit failed: {e}");
            }
        }
        false
    }

    /// Runs the checks and draws the window while open. Call every frame;
    /// `stopped` is true when this window's emulation is not running.
    pub(in crate::app) fn show(&mut self, ctx: &egui::Context, stopped: bool) -> UpdaterEvent {
        self.shared.stopped.store(stopped, Ordering::Release);
        if self.native {
            self.poll_native(ctx);
        } else {
            self.poll(ctx);
        }
        hand_over_to_native(&self.shared);
        let event = self.take_event();
        if self.shared.open_wanted.swap(false, Ordering::AcqRel) {
            self.open = true;
        }
        let phase = self.shared.phase();
        if matches!(phase, Phase::Exiting) {
            return UpdaterEvent::Exit;
        }
        self.schedule_repaint(ctx, &phase);
        if !self.open {
            self.pos = None;
            self.shaped = false;
            return event;
        }
        if let Some(action) = self.draw_window(ctx, &phase) {
            self.act(ctx, action);
        }
        event
    }

    /// What the restart thread asks of the shell.
    fn take_event(&mut self) -> UpdaterEvent {
        if self.shared.stop_wanted.load(Ordering::Acquire) {
            UpdaterEvent::StopEmulation
        } else if self.shared.resume_wanted.swap(false, Ordering::AcqRel) {
            self.raise_until = Some(Instant::now() + RAISE_FOR);
            UpdaterEvent::ResumeEmulation
        } else {
            UpdaterEvent::None
        }
    }

    /// Requests a repaint while there is progress to draw or an automatic
    /// check is pending.
    fn schedule_repaint(&self, ctx: &egui::Context, phase: &Phase) {
        if matches!(
            phase,
            Phase::Checking | Phase::Downloading { .. } | Phase::Preparing(_)
        ) || self.next_auto > Instant::now()
        {
            ctx.request_repaint_after(if self.open {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(60)
            });
        }
    }

    fn draw_window(&mut self, ctx: &egui::Context, phase: &Phase) -> Option<Action> {
        if self.pos.is_none() {
            self.pos = ctx
                .input(|i| i.viewport().outer_rect)
                .map(|r| r.center() - Vec2::from(WINDOW_SIZE) * 0.5);
        }
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Software Update")
            .with_inner_size(WINDOW_SIZE);
        if let Some(pos) = self.pos {
            builder = builder.with_position(pos);
        }
        let first_frame = !std::mem::replace(&mut self.shaped, true);
        let raise = first_frame || self.raise_until.is_some_and(|t| Instant::now() < t);
        if !raise {
            self.raise_until = None;
        }
        let mut action = None;
        let mut closed = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("cdj3k_update"),
            builder,
            |root, _class| {
                closed = root.input(|i| i.viewport().close_requested());
                if raise {
                    root.send_viewport_cmd(egui::ViewportCommand::Focus);
                    root.request_repaint_after(Duration::from_millis(100));
                }
                // Pin the size, as the setup window does.
                let size = Vec2::from(WINDOW_SIZE);
                if first_frame || (root.content_rect().size() - size).length() > 1.0 {
                    root.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
                    root.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
                    root.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(theme::palette(root).paper))
                    .show(root, |ui| {
                        action =
                            view::draw(ui, phase, &mut self.auto_check, &mut self.auto_install);
                    });
            },
        );
        if closed {
            // A close is ignored during the restart. Otherwise a closed offer
            // returns at the next start or from the menu, a download continues
            // and the window reopens when it ends, and a ready update installs
            // at quit.
            action = Some(match phase {
                Phase::Restarting(_) => Action::Keep,
                _ => Action::Dismiss,
            });
        }
        action
    }

    fn act(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Update(offer) => self.update(ctx, offer),
            Action::Skip(version) => {
                persist(|s| s.update_skip_version = Some(version));
                self.open = false;
            }
            Action::RemindLater => {
                persist(|s| s.update_remind_after = unix_now() + REMIND_AFTER.as_secs());
                self.open = false;
            }
            Action::Restart(offer) => self.restart(ctx, Finish::Install(offer)),
            Action::Retry(offer) => self.retry(ctx, offer),
            Action::Cancel => self.shared.cancel.store(true, Ordering::Relaxed),
            Action::Dismiss => self.dismiss(),
            Action::Later => {
                self.shared.declined.store(false, Ordering::Release);
                self.open = false;
            }
            Action::AutoCheck(on) => persist(|s| s.update_auto_check = on),
            Action::AutoInstall(on) => persist(|s| s.update_auto_install = on),
            Action::Keep => {}
        }
    }

    /// "Try Again" retries whichever step failed: the Sparkle install that
    /// could not proceed, the restart onto a prepared update, the download, or
    /// the check.
    fn retry(&mut self, ctx: &egui::Context, offer: Option<Offer>) {
        let waiting = lock(&self.shared.native_waiting).take();
        match (waiting, offer) {
            (Some(finish), _) => self.restart(ctx, finish),
            (None, Some(offer)) if self.shared.has_prepared() => {
                self.restart(ctx, Finish::Install(offer))
            }
            (None, Some(offer)) => self.update(ctx, offer),
            (None, None) => self.check(ctx, false),
        }
    }

    /// Close the window; a closed offer is not offered again in this run.
    fn dismiss(&mut self) {
        if let Phase::Available(offer) | Phase::Downloaded { offer, .. } = self.shared.phase() {
            self.dismissed = Some(offer.version);
        }
        self.open = false;
    }
}

/// Sparkle runs on this thread, so a restart that has reached the Sparkle
/// step hands over to it here.
fn hand_over_to_native(shared: &Shared) {
    let go = lock(&shared.native_go).take();
    if let Some(go) = go {
        let went = match go {
            Finish::NativeRelaunch => cdj3k_emu_update::native::proceed_relaunch(),
            _ => cdj3k_emu_update::native::install_now(true),
        };
        if !went {
            eprintln!("cdj3k-emu: Sparkle had no install waiting");
        }
    }
}

enum Action {
    Update(Offer),
    Skip(String),
    RemindLater,
    Restart(Offer),
    Retry(Option<Offer>),
    Cancel,
    Dismiss,
    /// "Later" on the restart offer: install at quit.
    Later,
    AutoCheck(bool),
    AutoInstall(bool),
    /// A close request that is ignored.
    Keep,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether this window runs the periodic check. Only the lowest-numbered open
/// slot runs it.
fn lowest_open_slot(own: u32) -> bool {
    (1..own).all(|n| cdj3k_emu_storage::slot_holder(n).is_none())
}

/// Whether an automatic check offers `offer`: only outside a "Remind Me
/// Later" pause, and only when it is newer than a skipped release.
fn worth_offering(offer: &Offer) -> bool {
    let s = AppSettings::load();
    offers(
        &offer.version,
        s.update_skip_version.as_deref(),
        s.update_remind_after,
        unix_now(),
    )
}

fn offers(version: &str, skipped: Option<&str>, remind_after: u64, now: u64) -> bool {
    if now < remind_after {
        return false;
    }
    match (skipped.and_then(Version::parse), Version::parse(version)) {
        (Some(skipped), Some(offered)) => offered > skipped,
        _ => true,
    }
}

fn persist(f: impl FnOnce(&mut AppSettings)) {
    if let Err(e) = AppSettings::update(f) {
        eprintln!("cdj3k-emu: saving the update settings failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::offers;

    #[test]
    fn a_skipped_release_waits_for_a_newer_one() {
        assert!(!offers("0.4.0", Some("0.4.0"), 0, 100));
        assert!(offers("0.4.1", Some("0.4.0"), 0, 100));
        assert!(offers("0.5.0", Some("0.4.0"), 0, 100));
        assert!(offers("0.4.0", None, 0, 100));
    }

    #[test]
    fn remind_me_later_holds_until_its_time() {
        assert!(!offers("0.4.0", None, 200, 100));
        assert!(offers("0.4.0", None, 200, 200));
        // A skip still applies once the reminder is over.
        assert!(!offers("0.4.0", Some("0.4.0"), 200, 300));
    }
}
