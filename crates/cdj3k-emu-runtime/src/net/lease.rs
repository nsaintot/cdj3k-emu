//! A slot's hold on the link it is bridged through.
//!
//! Every host makes the link in an elevated helper that stays running as its
//! watcher. The app writes a claim, `<pid>-<ms>`, to [`CLAIM_FILE`] in the
//! instance directory before it elevates. The watcher keeps the link while
//! that file holds the claim and the app process is alive; once either goes
//! (the slot switched network or quit, or the app died) it takes the link
//! down and writes the claim to [`RELEASED_FILE`]. Dropping the [`Lease`]
//! removes the claim and waits for that answer, so a setup that follows finds
//! the host settled.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// In the instance directory: the slot's claim on its link.
pub const CLAIM_FILE: &str = "net.claim";
/// In the instance directory: the watcher's answer, the claim it let go.
pub const RELEASED_FILE: &str = "net.released";

/// How long a dropped lease waits for the watcher's answer.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(15);

/// The claim on one slot's link, let go when dropped.
#[derive(Debug)]
pub struct Lease {
    dir: PathBuf,
    claim: String,
    watched: bool,
}

impl Lease {
    /// Write a fresh claim into `dir`, replacing any earlier one.
    pub fn take(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let claim = format!("{}-{ms}", std::process::id());
        let _ = fs::remove_file(dir.join(RELEASED_FILE));
        fs::write(dir.join(CLAIM_FILE), &claim)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            claim,
            watched: false,
        })
    }

    /// The claim, as the watcher is given it.
    pub fn claim(&self) -> &str {
        &self.claim
    }

    pub fn claim_path(&self) -> PathBuf {
        self.dir.join(CLAIM_FILE)
    }

    pub fn released_path(&self) -> PathBuf {
        self.dir.join(RELEASED_FILE)
    }

    /// Record that a watcher is running, so dropping waits for its answer.
    pub fn set_watched(&mut self) {
        self.watched = true;
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let claim = self.claim_path();
        if fs::read_to_string(&claim).is_ok_and(|c| c.trim() == self.claim) {
            let _ = fs::remove_file(&claim);
        }
        if !self.watched {
            return;
        }
        let released = self.released_path();
        let deadline = Instant::now() + RELEASE_TIMEOUT;
        while Instant::now() < deadline {
            if fs::read_to_string(&released).is_ok_and(|c| c.trim() == self.claim) {
                let _ = fs::remove_file(&released);
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        eprintln!(
            "cdj3k-emu: the link's watcher did not answer within {}s",
            RELEASE_TIMEOUT.as_secs()
        );
    }
}

/// A claim: the app's pid and a timestamp, `<digits>-<digits>`.
pub fn is_claim(s: &str) -> bool {
    s.split_once('-').is_some_and(|(a, b)| {
        [a, b]
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 20 && p.bytes().all(|c| c.is_ascii_digit()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cdj3k-lease-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_claim_is_the_pid_and_a_timestamp() {
        let dir = temp_dir("shape");
        let lease = Lease::take(&dir).unwrap();
        assert!(is_claim(lease.claim()), "{}", lease.claim());
        assert!(lease
            .claim()
            .starts_with(&format!("{}-", std::process::id())));
        assert_eq!(
            fs::read_to_string(dir.join(CLAIM_FILE)).unwrap(),
            lease.claim()
        );
        drop(lease);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn claims_have_a_fixed_shape() {
        assert!(is_claim("4356-1790933216403"));
        assert!(!is_claim("4356"));
        assert!(!is_claim("-1"));
        assert!(!is_claim("a-1"));
        assert!(!is_claim("1-2 & calc"));
    }

    #[test]
    fn dropping_removes_the_claim_and_takes_the_answer() {
        let dir = temp_dir("answer");
        let mut lease = Lease::take(&dir).unwrap();
        lease.set_watched();
        let claim = lease.claim().to_string();
        let released = lease.released_path();
        let watcher = std::thread::spawn({
            let claim_path = lease.claim_path();
            move || {
                while claim_path.exists() {
                    std::thread::sleep(Duration::from_millis(20));
                }
                fs::write(released, claim).unwrap();
            }
        });
        drop(lease);
        watcher.join().unwrap();
        assert!(!dir.join(CLAIM_FILE).exists());
        assert!(!dir.join(RELEASED_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A later lease for the slot owns the claim file; dropping the earlier
    /// one leaves it.
    #[test]
    fn dropping_leaves_a_later_claim() {
        let dir = temp_dir("later");
        let first = Lease::take(&dir).unwrap();
        fs::write(dir.join(CLAIM_FILE), "1-2").unwrap();
        drop(first);
        assert_eq!(fs::read_to_string(dir.join(CLAIM_FILE)).unwrap(), "1-2");
        let _ = fs::remove_dir_all(&dir);
    }
}
