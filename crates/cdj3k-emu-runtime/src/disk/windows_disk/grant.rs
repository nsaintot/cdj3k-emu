//! The elevated PowerShell steps, which grant this user access to a disk and
//! its volumes and run anything else that needs administrator rights.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::HANDLE;

use super::enumerate::Volume;
use crate::disk::windows_parse as parse;

/// Run the grant script elevated and wait for it.
pub(super) fn grant_access(n: u32, volumes: &[Volume]) -> io::Result<()> {
    let sid = current_user_sid()?;
    let paths: Vec<String> = volumes.iter().map(|v| v.path.clone()).collect();
    let err_file = scratch_file(&format!("grant-{n}.err"));
    let script = parse::grant_script(&sid, &parse::drive_path(n), &paths, &path_str(&err_file)?);
    run_script(&script, &err_file).map_err(|e| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("administrator access to the disk was not granted: {e}"),
        )
    })
}

/// Run PowerShell elevated, and turn a failure into the message the script
/// left in `err_file`, or the launcher's own.
pub(super) fn run_script(script: &str, err_file: &Path) -> io::Result<()> {
    let _ = std::fs::remove_file(err_file);
    let result = crate::elevate::run_elevated(&parse::powershell_command(script));
    let message = std::fs::read_to_string(err_file).ok();
    let _ = std::fs::remove_file(err_file);
    match (result, message) {
        (Ok(()), None) => Ok(()),
        (Ok(()), Some(m)) | (Err(_), Some(m)) => Err(io::Error::other(m.trim().to_string())),
        (Err(e), None) => Err(e),
    }
}

fn scratch_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("cdj3k-emu-{}-{name}", std::process::id()))
}

pub(super) fn path_str(p: &Path) -> io::Result<String> {
    p.to_str().map(str::to_string).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path is not valid Unicode: {}", p.display()),
        )
    })
}

/// Return the user that this process runs as, as a SID string.
fn current_user_sid() -> io::Result<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle is always valid; `token` receives a new handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `token` is a valid handle this function now owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };

    // The buffer holds u64 values so that it is 8-byte aligned, as TOKEN_USER
    // requires.
    let mut buf = vec![0u64; 64];
    let mut needed = 0u32;
    // SAFETY: the buffer's byte length is passed.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle() as HANDLE,
            TokenUser,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call filled the buffer with a TOKEN_USER whose SID points
    // into the same buffer.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: `sid` is valid; `text` receives a LocalAlloc'd string.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `text` is a NUL-terminated string until it is freed below.
    let s = unsafe {
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        s
    };
    if parse::is_valid_sid(&s) {
        Ok(s)
    } else {
        Err(io::Error::other(format!("unexpected SID: {s}")))
    }
}
