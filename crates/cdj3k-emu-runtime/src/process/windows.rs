//! Win32 processes. QEMU lives in a job object that kills it when this
//! process exits, crash included.

use std::io::Write;
use std::net::TcpStream;
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    ExitProcess, OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SET_QUOTA,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

/// The job every QEMU child joins. Never closed: the handle goes with the
/// process, and closing it is what kills the members.
static JOB: OnceLock<usize> = OnceLock::new();

/// The monitor session [`terminate`] asks QEMU to quit on.
static QUIT_CHANNEL: Mutex<Option<TcpStream>> = Mutex::new(None);

/// `atexit`, and a console handler for the events that end a console app,
/// which cleans up inline and exits.
pub fn install_exit_hooks() {
    extern "C" {
        fn atexit(callback: extern "C" fn()) -> i32;
    }
    extern "C" fn at_exit() {
        crate::cleanup_runtime_files();
    }
    unsafe extern "system" fn on_event(event: u32) -> i32 {
        if !matches!(
            event,
            CTRL_C_EVENT
                | CTRL_BREAK_EVENT
                | CTRL_CLOSE_EVENT
                | CTRL_LOGOFF_EVENT
                | CTRL_SHUTDOWN_EVENT
        ) {
            return 0;
        }
        cdj3k_emu_platform::menu_state::APP_SHUTDOWN
            .store(true, std::sync::atomic::Ordering::Relaxed);
        crate::kill_qemu_child_now();
        crate::cleanup_runtime_files();
        // SAFETY: ends the process; the job takes QEMU with it.
        unsafe { ExitProcess(0) }
    }
    // SAFETY: both callbacks are functions that live forever.
    unsafe {
        atexit(at_exit);
        SetConsoleCtrlHandler(Some(on_event), 1);
    }
}

/// Put the process `pid` in this app's kill-on-close job.
pub fn adopt(pid: u32) {
    let job = *JOB.get_or_init(|| {
        // SAFETY: a null name and default security make an unnamed job.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if !job.is_null() {
            // SAFETY: all-zero is valid for this plain-data struct.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `info` is the structure the class names, sized as given.
            unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
            }
        }
        job as usize
    }) as HANDLE;
    if job.is_null() {
        return;
    }
    // SAFETY: the process handle is closed before returning.
    unsafe {
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if !process.is_null() {
            AssignProcessToJobObject(job, process);
            CloseHandle(process);
        }
    }
}

/// Keep a handle on the QEMU monitor session, which [`terminate`] writes
/// `quit` to: QEMU serves one monitor client at a time, and the instance
/// holds it.
pub fn set_quit_channel(monitor: &TcpStream) {
    if let (Ok(clone), Ok(mut slot)) = (monitor.try_clone(), QUIT_CHANNEL.lock()) {
        *slot = Some(clone);
    }
}

/// Ask QEMU to exit through its monitor (QMP `quit`).
pub fn terminate(_pid: i32) {
    if let Ok(slot) = QUIT_CHANNEL.try_lock() {
        if let Some(mut monitor) = slot.as_ref() {
            let _ = monitor.write_all(b"{\"execute\":\"quit\"}\n");
        }
    }
}

/// End the process now (`TerminateProcess`).
pub fn kill(pid: i32) {
    // SAFETY: the process handle is closed before returning.
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE, 0, pid as u32);
        if !process.is_null() {
            TerminateProcess(process, 1);
            CloseHandle(process);
        }
    }
}

/// Nothing is inherited by descriptor number here.
pub fn keep_across_exec(_fd: i32) {}

pub fn is_alive(pid: i32) -> bool {
    // SAFETY: the process handle is closed before returning.
    unsafe {
        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid as u32);
        if process.is_null() {
            return false;
        }
        let waiting = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
        CloseHandle(process);
        waiting
    }
}
