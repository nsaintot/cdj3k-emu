use std::process::Command;

/// A Unix child opens no window unless it asks for one.
pub fn quiet(_command: &mut Command) {}
