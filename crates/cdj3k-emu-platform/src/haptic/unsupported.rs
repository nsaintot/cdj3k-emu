//! A host with no haptic actuator.

pub fn init() -> bool {
    false
}

pub fn available() -> bool {
    false
}

pub fn actuate(_waveform: i32) {}

pub fn shutdown() {}

pub fn stats() -> (u64, u64) {
    (0, 0)
}
