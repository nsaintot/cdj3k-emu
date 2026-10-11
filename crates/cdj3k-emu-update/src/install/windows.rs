//! On Windows, the release's Inno Setup installer runs at the restart,
//! elevated and silent: it shows its progress window and asks no questions.
//!
//! A loaded executable or DLL cannot be overwritten, so this process never
//! touches the install directory: it starts Setup and exits. Setup's
//! `InitializeSetup` waits under `/UPDATE` for every instance to release
//! `Global\cdj3k-emu` - the QEMU processes go with them, through their job
//! object - and its launch entry starts the app again as the user who ran
//! it, not elevated (`packaging/windows/cdj3k-emu.iss`).
//!
//! Setup waits in the user's temporary folder until [`apply`], so `apply`
//! checks its SHA-256 again and keeps other writers out of the file until
//! Setup has started.

use std::fs::File;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::wrong_kind;
use crate::{Error, Kind};

/// `FILE_SHARE_READ`: other handles may read the file, but not write,
/// rename or delete it.
const SHARE_READ_ONLY: u32 = 0x1;

pub(super) struct Staged {
    setup: PathBuf,
    /// The SHA-256 of Setup, taken right after the download checked it
    /// against the signed index.
    sha256: [u8; 32],
}

/// Nothing to lay down: a loaded executable or DLL cannot be replaced while
/// the app runs, so the whole install waits for [`apply`].
pub(super) fn prepare(setup: &Path, kind: Kind) -> Result<Staged, Error> {
    if kind != Kind::Inno {
        return Err(wrong_kind(kind));
    }
    Ok(Staged {
        setup: setup.to_path_buf(),
        sha256: sha256(&mut File::open(setup)?)?,
    })
}

fn sha256(file: &mut File) -> std::io::Result<[u8; 32]> {
    let mut hash = Sha256::new();
    std::io::copy(file, &mut hash)?;
    Ok(hash.finalize().into())
}

pub(super) fn apply(s: &Staged, relaunch: bool) -> Result<(), Error> {
    // The handle stays open until Setup has started, so no other process can
    // change the file between this check and the launch.
    let mut setup = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(SHARE_READ_ONLY)
        .open(&s.setup)?;
    if sha256(&mut setup)? != s.sha256 {
        return Err(Error::new(
            "the downloaded installer changed after it was checked; update again",
        ));
    }

    // `/UPDATE=0` is a quit: Setup still waits for the app, and starts nothing.
    let params = HSTRING::from(format!(
        "/SILENT /SUPPRESSMSGBOXES /NORESTART /UPDATE={}",
        u8::from(relaunch)
    ));
    let file = HSTRING::from(s.setup.as_os_str());
    let verb = HSTRING::from("runas");
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        // Silent: Setup shows its progress window and asks nothing.
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: every string outlives the call. SEE_MASK_NOASYNC makes it
    // return only once Setup has started, while `setup` is still open.
    let started = unsafe { ShellExecuteExW(&mut info) };
    drop(setup);
    started.map_err(|e| {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            Error::cancelled()
        } else {
            Error::new(format!("starting the installer failed: {e}"))
        }
    })
}

pub(super) fn hand_over(_package: &Path, kind: Kind) -> Result<PathBuf, Error> {
    Err(wrong_kind(kind))
}
