//! The update window for Windows and Linux, in the setup window's style.
//!
//! Flow: offer, download, prepare (while the emulation runs), offer the
//! restart. The restart closes the other windows, stops the emulation and
//! installs the prepared build: a rename for an AppImage, the installer on
//! Windows. "Later" installs it at quit. A deb or an rpm is downloaded to the
//! downloads folder instead, with the command that installs it.
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

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_storage::AppSettings;
use cdj3k_emu_update::native::Event as NativeEvent;
use cdj3k_emu_update::{Error, Kind, Offer, Prepared, Status, Version};
use egui::{Pos2, Vec2};

use super::theme;

/// Same width as the setup window.
const WINDOW_SIZE: [f32; 2] = [760.0, 560.0];
/// How long "Remind Me Later" pauses automatic offers.
const REMIND_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// Interval between automatic checks.
const RECHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// Retry schedule for a failed automatic check (no network, or a slot's
/// network bridge coming up): every [`RETRY_FIRST`] for [`RETRY_STEADY`],
/// then doubling up to [`RETRY_MAX`]. Reset on success.
const RETRY_FIRST: Duration = Duration::from_secs(10);
const RETRY_STEADY: Duration = Duration::from_secs(2 * 60);
const RETRY_MAX: Duration = Duration::from_secs(30 * 60);
/// How long the window keeps itself in front after the emulation restarts.
const RAISE_FOR: Duration = Duration::from_secs(3);
/// Maximum wait for the emulation to stop during a restart.
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
    /// Prepared; waiting for the restart.
    Ready(Offer),
    /// A deb or an rpm in the downloads folder, for the package manager.
    Downloaded {
        offer: Offer,
        path: PathBuf,
    },
    Restarting(Step),
    /// Installed, or the installer is waiting; this window closes.
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

/// State shared with the worker threads.
#[derive(Default)]
struct Shared {
    phase: Mutex<Phase>,
    /// The prepared update, until a restart or quit installs it.
    prepared: Mutex<Option<Prepared>>,
    /// A Sparkle step for the UI thread, which owns Sparkle.
    native_go: Mutex<Option<Finish>>,
    /// A Sparkle install that could not proceed; retried from the menu or
    /// "Try Again".
    native_waiting: Mutex<Option<Finish>>,
    /// A check, download or restart thread is running.
    busy: AtomicBool,
    /// Stops the download.
    cancel: AtomicBool,
    /// The restart is waiting for this window's emulation to stop.
    stop_wanted: AtomicBool,
    /// This window's emulation is stopped, as reported by the shell.
    stopped: AtomicBool,
    /// The restart failed after stopping the emulation.
    resume_wanted: AtomicBool,
    /// The install was declined at the system prompt; it is not installed at
    /// quit until the user picks "Later".
    declined: AtomicBool,
    /// A thread asks for the window to open.
    open_wanted: AtomicBool,
}

impl Shared {
    fn phase(&self) -> Phase {
        lock(&self.phase).clone()
    }

    fn set(&self, ctx: &egui::Context, phase: Phase) {
        *lock(&self.phase) = phase;
        ctx.request_repaint();
    }

    fn has_prepared(&self) -> bool {
        lock(&self.prepared).is_some()
    }
}

pub(in crate::app) struct Updater {
    instance: u32,
    shared: Arc<Shared>,
    open: bool,
    /// The running check is automatic: the window opens only for an offer.
    silent: bool,
    /// When the next automatic check is due.
    next_auto: Instant,
    /// Wait before the next retry.
    retry_delay: Duration,
    /// When automatic checks started failing.
    failing_since: Option<Instant>,
    /// The next automatic check is this window's startup check.
    first_round: bool,
    /// The next automatic check retries this window's failed check.
    retrying: bool,
    /// The window has been opened for the current ready update or download.
    ready_shown: bool,
    /// The version whose offer was closed: not offered again in this run.
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
        // Release bundles only; a developer build has no feed.
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

    /// Run `work` on a thread of its own, unless one is already running.
    fn spawn(&self, name: &str, work: impl FnOnce(&Shared) + Send + 'static) {
        if self.shared.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                work(&shared);
                shared.busy.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            self.shared.busy.store(false, Ordering::Release);
        }
    }

    /// Starts due automatic checks and menu checks, and opens the window when
    /// there is something to show. Call every frame.
    fn poll(&mut self, ctx: &egui::Context) {
        let busy = self.busy();
        // One window per check: the startup check runs in the first window (no
        // other slot open), the periodic check in the lowest open slot, a retry
        // in the window whose check failed. Other windows skip the round.
        if self.auto_check
            && Instant::now() >= self.next_auto
            && !busy
            && !self.open
            && !self.shared.has_prepared()
        {
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
            if let Phase::Available(offer) = &phase {
                if worth_offering(offer) && self.dismissed.as_ref() != Some(&offer.version) {
                    if self.auto_install && offer.kind.installs_itself() {
                        // Silent too: a failure retries as a failed check does.
                        self.update(ctx, offer.clone());
                        self.silent = true;
                    } else {
                        self.open = true;
                    }
                }
            }
        }
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

    /// The menu and Sparkle's requests, where Sparkle runs the updates.
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

    fn check(&mut self, ctx: &egui::Context, silent: bool) {
        if self.busy() {
            return;
        }
        *lock(&self.shared.phase) = Phase::Checking;
        self.silent = silent;
        let ctx = ctx.clone();
        self.spawn("cdj3k-emu-update-check", move |sh| {
            let next = match cdj3k_emu_update::check() {
                Ok(Status::Current {
                    latest,
                    notes,
                    release_notes,
                }) => Phase::Current {
                    latest,
                    notes,
                    release_notes,
                },
                Ok(Status::Available(offer)) => {
                    eprintln!("cdj3k-emu: update v{} is available", offer.version);
                    Phase::Available(offer)
                }
                Ok(Status::NoPackage { latest }) => Phase::NoPackage { latest },
                Ok(Status::Unmanaged) => Phase::Unmanaged,
                Err(e) => {
                    eprintln!("cdj3k-emu: update check failed: {e}");
                    Phase::Failed {
                        message: e.to_string(),
                        offer: None,
                    }
                }
            };
            sh.set(&ctx, next);
        });
    }

    /// Download and prepare the update while the emulation runs; a deb or an
    /// rpm goes to the downloads folder instead.
    fn update(&mut self, ctx: &egui::Context, offer: Offer) {
        self.shared.cancel.store(false, Ordering::Relaxed);
        let ctx = ctx.clone();
        self.spawn("cdj3k-emu-update-download", move |sh| {
            let result = (|| -> Result<Phase, Error> {
                let mut progress = |done, total| {
                    let offer = offer.clone();
                    sh.set(&ctx, Phase::Downloading { offer, done, total })
                };
                let package = cdj3k_emu_update::download(&offer, &mut progress, &sh.cancel)?;
                if !offer.kind.installs_itself() {
                    let path = cdj3k_emu_update::hand_over(&package, offer.kind)?;
                    cdj3k_emu_update::discard_download(&offer.version);
                    return Ok(Phase::Downloaded {
                        offer: offer.clone(),
                        path,
                    });
                }
                sh.set(&ctx, Phase::Preparing(offer.clone()));
                let p = cdj3k_emu_update::prepare(&package, offer.kind)?;
                *lock(&sh.prepared) = Some(p);
                Ok(Phase::Ready(offer.clone()))
            })();
            let next = match result {
                Ok(next) => next,
                Err(e) if e.is_cancelled() => {
                    cdj3k_emu_update::discard_download(&offer.version);
                    Phase::Available(offer)
                }
                Err(e) => {
                    eprintln!("cdj3k-emu: update failed: {e}");
                    Phase::Failed {
                        message: e.to_string(),
                        offer: Some(offer),
                    }
                }
            };
            sh.set(&ctx, next);
        });
    }

    /// Close the other windows, stop the emulation, then `finish`.
    fn restart(&mut self, ctx: &egui::Context, finish: Finish) {
        let own = self.instance;
        let ctx = ctx.clone();
        self.spawn("cdj3k-emu-update-restart", move |sh| {
            let mut stopped_here = false;
            let result = (|| {
                sh.set(&ctx, Phase::Restarting(Step::Closing));
                cdj3k_emu_update::close_other_slots(own)?;
                sh.set(&ctx, Phase::Restarting(Step::Stopping));
                // Resumed on failure, including a stop that outlasts the wait.
                stopped_here = !sh.stopped.load(Ordering::Acquire);
                stop_emulation(sh, &ctx)?;
                match &finish {
                    Finish::Install(offer) => {
                        sh.set(&ctx, Phase::Restarting(Step::Installing(offer.clone())));
                        let p = lock(&sh.prepared).take();
                        let p = p.ok_or_else(|| Error::new("nothing is prepared"))?;
                        cdj3k_emu_update::apply(&p, true)
                            .inspect_err(|_| *lock(&sh.prepared) = Some(p))
                    }
                    native => {
                        *lock(&sh.native_go) = Some(native.clone());
                        Ok(())
                    }
                }
            })();
            let next = match (result, finish) {
                (Ok(()), Finish::Install(_)) => Phase::Exiting,
                (Ok(()), _) => Phase::Restarting(Step::HandedOff),
                (Err(e), finish) => {
                    if stopped_here {
                        sh.resume_wanted.store(true, Ordering::Release);
                    }
                    let message = if e.is_cancelled() {
                        sh.declined.store(true, Ordering::Release);
                        "installation declined; continuing with the current version".into()
                    } else {
                        eprintln!("cdj3k-emu: update failed: {e}");
                        e.to_string()
                    };
                    let offer = match finish {
                        Finish::Install(offer) => Some(offer),
                        native => {
                            *lock(&sh.native_waiting) = Some(native);
                            sh.open_wanted.store(true, Ordering::Release);
                            None
                        }
                    };
                    Phase::Failed { message, offer }
                }
            };
            sh.set(&ctx, next);
        });
    }

    /// Called when the window closes for good: install a prepared update
    /// without restarting. Skipped while another window is open, which would
    /// keep running the old build.
    ///
    /// Returns true when Sparkle took over and ends the process itself; the
    /// window must stay open.
    pub(in crate::app) fn apply_on_quit(&mut self) -> bool {
        if !cdj3k_emu_update::other_slots_open(self.instance).is_empty() {
            return false;
        }
        if self.native {
            return cdj3k_emu_update::native::install_now(false);
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
        // Sparkle runs on this thread: a restart that reached it hands over
        // here.
        let go = lock(&self.shared.native_go).take();
        if let Some(go) = go {
            let went = match go {
                Finish::NativeRelaunch => cdj3k_emu_update::native::proceed_relaunch(),
                _ => cdj3k_emu_update::native::install_now(true),
            };
            if !went {
                eprintln!("cdj3k-emu: Sparkle had no install waiting");
            }
        }
        let mut event = UpdaterEvent::None;
        if self.shared.stop_wanted.load(Ordering::Acquire) {
            event = UpdaterEvent::StopEmulation;
        } else if self.shared.resume_wanted.swap(false, Ordering::AcqRel) {
            event = UpdaterEvent::ResumeEmulation;
            self.raise_until = Some(Instant::now() + RAISE_FOR);
        }
        if self.shared.open_wanted.swap(false, Ordering::AcqRel) {
            self.open = true;
        }
        let phase = self.shared.phase();
        if matches!(phase, Phase::Exiting) {
            return UpdaterEvent::Exit;
        }
        if matches!(
            phase,
            Phase::Checking | Phase::Downloading { .. } | Phase::Preparing(_)
        ) || self.next_auto > Instant::now()
        {
            // Progress to draw, or the next automatic check to start.
            ctx.request_repaint_after(if self.open {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(60)
            });
        }
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
            |ctx, _class| {
                closed = ctx.input(|i| i.viewport().close_requested());
                if raise {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    ctx.request_repaint_after(Duration::from_millis(100));
                }
                // Pinned size, as in the setup window.
                let size = Vec2::from(WINDOW_SIZE);
                if first_frame || (ctx.screen_rect().size() - size).length() > 1.0 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
                    ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(theme::palette(ctx).paper))
                    .show(ctx, |ui| {
                        action =
                            view::draw(ui, phase, &mut self.auto_check, &mut self.auto_install);
                    });
            },
        );
        if closed {
            // Ignored during the restart. A closed offer returns at the next
            // start or from the menu; a download continues and the window
            // reopens when it ends; a ready update installs at quit.
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
            Action::Retry(offer) => {
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
            Action::Cancel => self.shared.cancel.store(true, Ordering::Relaxed),
            Action::Dismiss => {
                if let Phase::Available(offer) | Phase::Downloaded { offer, .. } =
                    self.shared.phase()
                {
                    self.dismissed = Some(offer.version);
                }
                self.open = false;
            }
            Action::Later => {
                self.shared.declined.store(false, Ordering::Release);
                self.open = false;
            }
            Action::AutoCheck(on) => persist(|s| s.update_auto_check = on),
            Action::AutoInstall(on) => persist(|s| s.update_auto_install = on),
            Action::Keep => {}
        }
    }
}

/// Ask the shell to stop this window's emulation and wait until it has.
fn stop_emulation(sh: &Shared, ctx: &egui::Context) -> Result<(), Error> {
    sh.stop_wanted.store(true, Ordering::Release);
    let deadline = Instant::now() + STOP_WAIT;
    while !sh.stopped.load(Ordering::Acquire) {
        if Instant::now() >= deadline {
            sh.stop_wanted.store(false, Ordering::Release);
            return Err(Error::new("the emulation did not stop"));
        }
        ctx.request_repaint();
        std::thread::sleep(Duration::from_millis(100));
    }
    sh.stop_wanted.store(false, Ordering::Release);
    Ok(())
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

/// The periodic check runs in one window only: the lowest-numbered slot that
/// is open.
fn lowest_open_slot(own: u32) -> bool {
    (1..own).all(|n| cdj3k_emu_storage::slot_holder(n).is_none())
}

/// Whether an automatic check offers `offer`: outside a "Remind Me Later"
/// pause and newer than a skipped release.
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
