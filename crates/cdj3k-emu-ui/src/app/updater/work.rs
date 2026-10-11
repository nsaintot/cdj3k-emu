//! The update's background work: the check, the download and preparation,
//! and the restart, each on a thread of its own.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cdj3k_emu_update::{Error, Offer, Prepared, Status};

use super::{lock, Finish, Phase, Step, Updater, STOP_WAIT};

/// State shared with the worker threads.
#[derive(Default)]
pub(super) struct Shared {
    pub(super) phase: Mutex<Phase>,
    /// The prepared update, until a restart or quit installs it.
    pub(super) prepared: Mutex<Option<Prepared>>,
    /// A Sparkle step for the UI thread, which owns Sparkle.
    pub(super) native_go: Mutex<Option<Finish>>,
    /// A Sparkle install that could not proceed. The menu or "Try Again"
    /// retries it.
    pub(super) native_waiting: Mutex<Option<Finish>>,
    /// Whether a check, download or restart thread is running.
    pub(super) busy: AtomicBool,
    /// Stops the download.
    pub(super) cancel: AtomicBool,
    /// The restart is waiting for this window's emulation to stop.
    pub(super) stop_wanted: AtomicBool,
    /// This window's emulation is stopped, as reported by the shell.
    pub(super) stopped: AtomicBool,
    /// The restart failed after stopping the emulation.
    pub(super) resume_wanted: AtomicBool,
    /// The install was declined at the system prompt; it is not installed at
    /// quit until the user picks "Later".
    pub(super) declined: AtomicBool,
    /// A thread asks for the window to open.
    pub(super) open_wanted: AtomicBool,
}

impl Shared {
    pub(super) fn phase(&self) -> Phase {
        lock(&self.phase).clone()
    }

    pub(super) fn set(&self, ctx: &egui::Context, phase: Phase) {
        *lock(&self.phase) = phase;
        ctx.request_repaint();
    }

    pub(super) fn has_prepared(&self) -> bool {
        lock(&self.prepared).is_some()
    }
}

impl Updater {
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

    pub(super) fn check(&mut self, ctx: &egui::Context, silent: bool) {
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

    /// Download and prepare the update while the emulation runs. A deb or an
    /// rpm is saved to the downloads folder instead.
    pub(super) fn update(&mut self, ctx: &egui::Context, offer: Offer) {
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
    pub(super) fn restart(&mut self, ctx: &egui::Context, finish: Finish) {
        let own = self.instance;
        let ctx = ctx.clone();
        self.spawn("cdj3k-emu-update-restart", move |sh| {
            let mut stopped_here = false;
            let result = (|| {
                sh.set(&ctx, Phase::Restarting(Step::Closing));
                cdj3k_emu_update::close_other_slots(own)?;
                sh.set(&ctx, Phase::Restarting(Step::Stopping));
                // The emulation is resumed on failure, including when stopping
                // it takes longer than the wait.
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
                        "the installation was declined; the current version stays installed".into()
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
