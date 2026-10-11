//! Trackpad haptic feedback.
//!
//! Call `init()` once at startup, `actuate(waveform)` for each click and
//! `shutdown()` on exit. `available()` tells the menu whether to offer Jog
//! Haptics, and `stats()` is for debugging. Each `actuate()` only queues the
//! pulse, so a jog at any speed cannot stall the UI.
//!
//! macOS drives the trackpad actuator through the private
//! `MultitouchSupport.framework`; a host without one has the same five
//! functions as no-ops.
//!
//! Waveform IDs, and what each feels like on hardware that has one:
//!
//!   1  - weak click            (~5 ms, ~150 Hz natural cap)
//!   2  - strong click          (~10 ms, ~80 Hz)        - Force Touch feel
//!   3  - buzz / notification   (longer, blends to drone at high rate)
//!   4  - light tap             (~8 ms, ~100 Hz)
//!   5  - medium tap            (~12 ms, ~65 Hz)
//!   6  - strong tap            (~20 ms, ~48 Hz)        - sharp hard punch
//!   15 - soft thud
//!   16 - strong thud           (~30 ms, ~30 Hz)        - heaviest single pulse
//!
//! The caller chooses the ID; the hardware paces the rate.

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(not(any(target_os = "macos")))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::{actuate, available, init, shutdown, stats};
