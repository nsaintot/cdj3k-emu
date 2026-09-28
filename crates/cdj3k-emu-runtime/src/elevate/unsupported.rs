//! Hosts with no elevation backend.

use std::io;

pub fn run_elevated(_sh_cmd: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "privilege elevation is not implemented for this host",
    ))
}
