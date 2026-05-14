//! Cooperative 60 Hz cap shared across all stream wakeups.
//!
//! ## Design
//!
//! A dedicated **metronome thread** owns the rate-limiting. It sleeps for
//! `min_interval` between ticks; on each tick it checks an `AtomicBool` flag
//! and, if set, calls `egui::Context::request_repaint()` to wake the UI.
//!
//! Stream threads call [`RepaintGate::request`] which is a single
//! `AtomicBool::store` - never sleeps, never locks, never calls into egui
//! directly. The cost of a stream wakeup is a few nanoseconds.
//!
//! ## Why a separate thread
//!
//! On macOS + ProMotion, `request_repaint_after(d)` cannot enforce the cap:
//!
//! - With **vsync=true**, eframe paints at every vsync (120 Hz on ProMotion)
//!   when *any* repaint is pending - the delay is only an upper bound, not
//!   a "paint no sooner than" floor. A `thread::sleep` at the top of
//!   `update()` would enforce the cap but blocks the winit event loop,
//!   dropping stream wakeups that arrive during the sleep.
//! - With **vsync=false**, eframe doesn't sync to the display at all; any
//!   pending repaint is painted immediately, and self-rescheduling closures
//!   become tight busy loops at thousands of fps.
//!
//! The metronome decouples cap timing from the UI thread entirely. The main
//! thread never sleeps; eframe paints at vsync only when the metronome
//! actually requested it, which happens at most once per `min_interval`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Clone)]
pub struct RepaintGate {
    /// Set by `request()`, consumed (swap to false) by the metronome thread
    /// on each tick. When the swap returns `true`, the metronome calls
    /// `request_repaint()` once.
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
            .expect("spawn repaint-gate thread");
        Self { pending }
    }

    /// Request a repaint at the next metronome tick. Cheap (~5 ns); safe to
    /// call from any thread at arbitrary rates. Multiple calls between ticks
    /// collapse into a single repaint.
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
