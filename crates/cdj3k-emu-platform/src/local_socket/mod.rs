//! This module provides stream sockets addressed by a file path. The app
//! connects to the sockets that QEMU's chardevs listen on in the runtime
//! directory ([`LocalStream`]), and each window listens on `window.sock` there
//! for commands from other windows ([`LocalListener`]).

#[cfg(unix)]
#[path = "unix.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(unix, windows)))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::{LocalListener, LocalStream};

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;

    #[test]
    fn a_listener_replaces_a_stale_socket_file_and_accepts() {
        let path = std::env::temp_dir().join(format!("cdj3k-listen-{}.sock", std::process::id()));
        // A listener that goes away leaves its socket file behind.
        drop(LocalListener::bind(&path).unwrap());
        assert!(path.exists());

        let listener = LocalListener::bind(&path).unwrap();
        let _client = LocalStream::connect(&path).unwrap();
        listener.accept().unwrap();
        drop(listener);
        let _ = std::fs::remove_file(&path);
    }
}
