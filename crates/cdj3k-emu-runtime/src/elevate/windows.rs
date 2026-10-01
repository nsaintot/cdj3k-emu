//! UAC, through `ShellExecuteExW` with the `runas` verb. The command runs in
//! `cmd.exe /C`, hidden, and its exit code is the result.

use std::io;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

pub fn run_elevated(cmd: &str) -> io::Result<()> {
    if cmd.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command contains null byte",
        ));
    }
    let params = HSTRING::from(format!("/C {cmd}"));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: w!("cmd.exe"),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: `info` and the strings it points at outlive the call.
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        return Err(if e.code() == ERROR_CANCELLED.to_hresult() {
            io::Error::new(io::ErrorKind::PermissionDenied, "admin elevation refused")
        } else {
            io::Error::other(format!("could not start the elevated command: {e}"))
        });
    }
    let process = info.hProcess;
    if process.is_invalid() {
        return Err(io::Error::other("elevated command returned no process"));
    }
    // SAFETY: `process` is the handle `SEE_MASK_NOCLOSEPROCESS` handed us; it
    // is closed exactly once, below.
    let code = unsafe {
        let waited = WaitForSingleObject(process, INFINITE);
        let mut code = 0u32;
        let got = (waited == WAIT_OBJECT_0)
            .then(|| GetExitCodeProcess(process, &mut code))
            .and_then(Result::ok);
        let _ = CloseHandle(process);
        got.map(|()| code)
    };
    match code {
        Some(0) => Ok(()),
        Some(code) => Err(io::Error::other(format!(
            "elevated command failed with status {code}"
        ))),
        None => Err(io::Error::other("could not read the elevated command's exit code")),
    }
}
