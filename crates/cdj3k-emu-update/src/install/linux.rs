//! Linux: the AppImage laid down beside `$APPIMAGE` and renamed over it at
//! the restart. The running image keeps its open file.
//!
//! A deb or an rpm goes to the downloads folder ([`hand_over`]) for the
//! package manager.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::wrong_kind;
use crate::{Error, Kind};

pub(super) struct Staged {
    image: PathBuf,
    staged: PathBuf,
}

/// A staged image that is never applied is removed with it.
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.staged);
    }
}

pub(super) fn prepare(package: &Path, kind: Kind) -> Result<Staged, Error> {
    if kind != Kind::AppImage {
        return Err(wrong_kind(kind));
    }
    let image = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("this build is not running from an AppImage"))?;
    let dir = image
        .parent()
        .ok_or_else(|| Error::new("the AppImage has no folder"))?;
    let name = image
        .file_name()
        .ok_or_else(|| Error::new("the AppImage has no name"))?;
    let staged = dir.join(format!(".{}.update", name.to_string_lossy()));
    let _ = std::fs::remove_file(&staged);
    if let Err(e) = copy_executable(package, &staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(denied(dir, e));
    }
    Ok(Staged { image, staged })
}

/// Copy `from` to a new executable file `to`, synced to disk.
fn copy_executable(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o755)
        .open(to)?;
    std::io::copy(&mut std::fs::File::open(from)?, &mut out)?;
    out.flush()?;
    out.sync_all()
}

/// A deb or an rpm, copied into the downloads folder under its own name and
/// removed from where it was verified.
pub(super) fn hand_over(package: &Path, kind: Kind) -> Result<PathBuf, Error> {
    if !matches!(kind, Kind::Deb | Kind::Rpm) {
        return Err(wrong_kind(kind));
    }
    let name = package
        .file_name()
        .ok_or_else(|| Error::new("the package has no name"))?;
    let dir = downloads_dir();
    let dest = dir.join(name);
    let part = dir.join(format!(".{}.part", name.to_string_lossy()));
    let copied = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::copy(package, &part))
        .and_then(|_| std::fs::rename(&part, &dest));
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&part);
        return Err(Error::new(format!("cannot write {}: {e}", dir.display())));
    }
    let _ = std::fs::remove_file(package);
    Ok(dest)
}

/// The XDG downloads folder: `XDG_DOWNLOAD_DIR`, else the one `user-dirs.dirs`
/// names, else `~/Downloads`.
fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if let Some(dir) = std::env::var_os("XDG_DOWNLOAD_DIR") {
        return PathBuf::from(dir);
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|text| user_dir(&text, "XDG_DOWNLOAD_DIR", &home))
        .unwrap_or_else(|| home.join("Downloads"))
}

/// `key`'s folder in a `user-dirs.dirs` text: `KEY="$HOME/Downloads"`.
fn user_dir(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let value = text
        .lines()
        .find_map(|l| l.trim().strip_prefix(key)?.trim_start().strip_prefix('='))?
        .trim()
        .trim_matches('"');
    match value.strip_prefix("$HOME") {
        Some(rest) => Some(home.join(rest.trim_start_matches('/'))),
        None => Some(PathBuf::from(value)).filter(|p| p.is_absolute()),
    }
}

pub(super) fn apply(s: &Staged, again: bool) -> Result<(), Error> {
    std::fs::rename(&s.staged, &s.image)
        .map_err(|e| denied(s.image.parent().unwrap_or(Path::new("/")), e))?;
    if again {
        relaunch(&s.image);
    }
    Ok(())
}

fn denied(dir: &Path, e: std::io::Error) -> Error {
    Error::new(format!(
        "cannot write {}: {e}; move the AppImage to a folder you can write and update again",
        dir.display()
    ))
}

/// What an AppImage's runtime and launcher leave in the environment, all of
/// it naming this build's mount, which goes away with this process.
const IMAGE_ENV: [&str; 5] = ["APPDIR", "APPIMAGE", "ARGV0", "OWD", "CDJ3K_RESOURCES"];

/// Start `image` once with `--after-update`, as a plain launch would.
///
/// The child keeps nothing of this process but stdio: an AppImage's runtime
/// passes its mount to the app as an open descriptor, and a child holding it
/// keeps the old image mounted for as long as the child runs.
fn relaunch(image: &Path) {
    close_on_exec_above_stdio();
    let mut cmd = Command::new(image);
    cmd.arg("--after-update");
    for var in IMAGE_ENV {
        cmd.env_remove(var);
    }
    if let Err(e) = cmd.spawn() {
        eprintln!(
            "cdj3k-emu-update: starting {} again failed: {e}",
            image.display()
        );
    }
}

/// Mark every descriptor of this process above stderr close-on-exec.
fn close_on_exec_above_stdio() {
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        return;
    };
    let fds: Vec<i32> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .filter(|&fd| fd > 2)
        .collect();
    for fd in fds {
        // SAFETY: fcntl on a descriptor number; one closed since the listing
        // (the listing's own) fails with EBADF and is left alone.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_name_the_downloads_folder() {
        let home = Path::new("/home/u");
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\n";
        assert_eq!(
            user_dir(text, "XDG_DOWNLOAD_DIR", home),
            Some(PathBuf::from("/home/u/Téléchargements"))
        );
        assert_eq!(
            user_dir("XDG_DOWNLOAD_DIR=\"/data/dl\"", "XDG_DOWNLOAD_DIR", home),
            Some(PathBuf::from("/data/dl"))
        );
        assert_eq!(
            user_dir("XDG_DOWNLOAD_DIR=\"dl\"", "XDG_DOWNLOAD_DIR", home),
            None
        );
    }
}
