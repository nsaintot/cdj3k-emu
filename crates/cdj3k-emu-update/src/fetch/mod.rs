//! HTTPS: the index, its signature and the package.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::{Error, Package};

#[cfg(not(windows))]
#[path = "rustls.rs"]
mod tls;
#[cfg(windows)]
#[path = "windows.rs"]
mod tls;

/// Far above the index and its signature, which are a few kilobytes.
const SMALL_LIMIT: u64 = 1 << 20;
const CHUNK: usize = 64 * 1024;
/// Receiving the index or its signature.
const SMALL_BODY_WAIT: Duration = Duration::from_secs(30);
/// Receiving a package: a few hundred megabytes at a slow link's rate.
const PACKAGE_BODY_WAIT: Duration = Duration::from_secs(30 * 60);

fn agent(body_wait: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(tls::config())
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(body_wait))
        .user_agent(format!(
            "cdj3k-emu/{}",
            cdj3k_emu_platform::app_meta::VERSION
        ))
        .build()
        .into()
}

fn http_error(url: &str, e: ureq::Error) -> Error {
    match e {
        ureq::Error::StatusCode(404) => Error {
            not_found: true,
            ..Error::new(format!("{url} was not found"))
        },
        ureq::Error::StatusCode(code) => Error::new(format!("{url} answered HTTP {code}")),
        e => Error::new(format!("could not reach {url}: {e}")),
    }
}

/// A small file, whole.
pub fn get(url: &str) -> Result<Vec<u8>, Error> {
    let mut resp = agent(SMALL_BODY_WAIT)
        .get(url)
        .call()
        .map_err(|e| http_error(url, e))?;
    resp.body_mut()
        .with_config()
        .limit(SMALL_LIMIT)
        .read_to_vec()
        .map_err(|e| http_error(url, e))
}

/// Fetch `package` into `dir`, checking its size and SHA-256 as it arrives.
/// `progress` hears the bytes so far and the total; setting `cancel` stops
/// the download. Returns the finished file, named as the URL names it.
pub fn download(
    package: &Package,
    dir: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &AtomicBool,
) -> Result<PathBuf, Error> {
    let name = package
        .url
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty() && !n.contains(['\\', ':']) && *n != "..")
        .ok_or_else(|| Error::new(format!("{} names no file", package.url)))?;
    std::fs::create_dir_all(dir)?;
    let resp = agent(PACKAGE_BODY_WAIT)
        .get(&package.url)
        .call()
        .map_err(|e| http_error(&package.url, e))?;
    let reader = resp.into_body().into_reader();
    save(reader, package, &dir.join(name), progress, cancel)
}

/// Copy `reader` to `dest` through a `.part` file of this process's own,
/// created new, that is renamed into place only once its size and hash match
/// `package`.
fn save(
    mut reader: impl Read,
    package: &Package,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancel: &AtomicBool,
) -> Result<PathBuf, Error> {
    let part = dest.with_extension(format!("{}.part", std::process::id()));
    let _ = std::fs::remove_file(&part);
    let result = (|| {
        let mut file = std::fs::File::create_new(&part)?;
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        let mut done: u64 = 0;
        progress(0, package.size);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::cancelled());
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            done += n as u64;
            if done > package.size {
                return Err(Error::new(format!(
                    "the download is larger than the {} bytes the index gives",
                    package.size
                )));
            }
            hash.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            progress(done, package.size);
        }
        if done != package.size {
            return Err(Error::new(format!(
                "the download stopped at {done} of {} bytes",
                package.size
            )));
        }
        let got = hex::encode(hash.finalize());
        if !got.eq_ignore_ascii_case(&package.sha256) {
            return Err(Error::new(format!(
                "the download's SHA-256 is {got}, not the {} the index gives",
                package.sha256
            )));
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&part, dest)?;
        Ok(dest.to_path_buf())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Kind;

    fn package(body: &[u8]) -> Package {
        Package {
            os: "linux".into(),
            arch: "x86_64".into(),
            kind: Kind::Deb,
            url: "https://example.invalid/p.deb".into(),
            size: body.len() as u64,
            sha256: hex::encode(Sha256::digest(body)),
        }
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cdj3k-emu-update-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_matching_download_lands_whole() {
        let body = vec![7u8; CHUNK * 3 + 11];
        let d = dir("ok");
        let mut seen = Vec::new();
        let out = save(
            &body[..],
            &package(&body),
            &d.join("p.deb"),
            &mut |done, total| seen.push((done, total)),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), body);
        assert!(!d.join("p.part").exists());
        assert_eq!(seen.first(), Some(&(0, body.len() as u64)));
        assert_eq!(seen.last(), Some(&(body.len() as u64, body.len() as u64)));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_wrong_hash_short_body_or_long_body_leaves_nothing() {
        let body = b"package bytes".to_vec();
        let d = dir("bad");
        let dest = d.join("p.deb");
        let none = AtomicBool::new(false);

        let mut wrong = package(&body);
        wrong.sha256 = "00".repeat(32);
        let short = package(b"package bytes and more");
        let mut long = package(&body);
        long.size -= 1;
        for (pkg, says) in [(wrong, "SHA-256"), (short, "stopped"), (long, "larger")] {
            let e = save(&body[..], &pkg, &dest, &mut |_, _| {}, &none).unwrap_err();
            assert!(e.to_string().contains(says), "{e}");
            assert!(!dest.exists() && !d.join("p.part").exists());
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_cancel_stops_it() {
        let body = vec![1u8; CHUNK * 2];
        let d = dir("cancel");
        let e = save(
            &body[..],
            &package(&body),
            &d.join("p.deb"),
            &mut |_, _| {},
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert!(e.is_cancelled());
        assert!(!d.join("p.part").exists());
        let _ = std::fs::remove_dir_all(d);
    }
}
