//! Windows: the release's Inno Setup installer, elevated and silent - its
//! progress window, no questions - at the restart.
//!
//! A loaded executable or DLL cannot be overwritten, so this process never
//! touches the install directory: it starts Setup and exits. Setup's
//! `InitializeSetup` waits under `/UPDATE` for every instance to release
//! `Global\cdj3k-emu` - the QEMU processes go with them, through their job
//! object - and its launch entry starts the app again as the user who ran
//! it, not elevated (`packaging/windows/cdj3k-emu.iss`).

use std::path::{Path, PathBuf};

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::wrong_kind;
use crate::{Error, Kind};

pub(super) struct Staged {
    setup: PathBuf,
}

/// Nothing to lay down: a loaded executable or DLL cannot be replaced while
/// the app runs, so the whole install waits for [`apply`].
pub(super) fn prepare(setup: &Path, kind: Kind) -> Result<Staged, Error> {
    if kind != Kind::Inno {
        return Err(wrong_kind(kind));
    }
    Ok(Staged {
        setup: setup.to_path_buf(),
    })
}

pub(super) fn apply(s: &Staged, relaunch: bool) -> Result<(), Error> {
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
    // SAFETY: every string outlives the call.
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
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
