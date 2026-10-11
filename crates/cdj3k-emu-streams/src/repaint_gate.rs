//! A 60 Hz cap that all stream wakeups share.
//!
//! ## Design
//!
//! A dedicated metronome thread owns the rate limit. It sleeps for
//! `min_interval` between ticks; on each tick it checks an `AtomicBool` flag
//! and, if it is set, calls `egui::Context::request_repaint()` to wake the UI.
//!
//! Stream threads call [`RepaintGate::request`], which is a single
//! `AtomicBool::store`: it does not sleep, lock or call into egui. A stream
//! wakeup costs a few nanoseconds.
//!
//! ## Why a separate thread
//!
//! On macOS with ProMotion, `request_repaint_after(d)` cannot enforce the cap:
//!
//! - With vsync on, eframe paints at every vsync (120 Hz on ProMotion) while
//!   any repaint is pending; the delay is only an upper bound. Do not sleep at
//!   the top of `update()`: it blocks the winit event loop and drops the
//!   stream wakeups that arrive during the sleep.
//! - With vsync off, eframe does not sync to the display: it paints any
//!   pending repaint immediately, and self-rescheduling closures become busy
//!   loops at thousands of frames per second.
//!
//! The metronome keeps the cap's timing off the UI thread. The main thread
//! never sleeps, and eframe paints only when the metronome requests it, at
//! most once per `min_interval`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Clone)]
pub struct RepaintGate {
    /// Set by `request()`. The metronome thread swaps it to false on each
    /// tick and, when the swap returns `true`, calls `request_repaint()` once.
    pending: Arc<AtomicBool>,
}

impl RepaintGate {
    pub fn new_60fps(ctx: egui::Context) -> Self {
        Self::new(ctx, Duration::from_micros(16_667))
    }

    pub fn new(ctx: egui::Context, min_interval: Duration) -> Self {
        let pending = Arc::new(AtomicBool::new(false));
        let pending_clone = Arc::clone(&pending);
        thread::Builder::new()
            .name("repaint-gate".into())
            .spawn(move || metronome(pending_clone, ctx, min_interval))
            .expect("the repaint-gate thread could not be started");
        Self { pending }
    }

    /// Requests a repaint at the next metronome tick. The call costs about
    /// 5 ns and is safe from any thread at any rate. Several calls between
    /// ticks give one repaint.
    pub fn request(&self) {
        self.pending.store(true, Ordering::Relaxed);
    }
}

fn metronome(pending: Arc<AtomicBool>, ctx: egui::Context, min_interval: Duration) {
    loop {
        thread::sleep(min_interval);
        if pending.swap(false, Ordering::Relaxed) {
            ctx.request_repaint();
        }
    }
}
