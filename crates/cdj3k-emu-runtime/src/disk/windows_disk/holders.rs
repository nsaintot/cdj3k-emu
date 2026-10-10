//! Find the names of the programs that keep a volume busy, through the
//! Restart Manager.

use std::collections::VecDeque;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::ERROR_MORE_DATA;
use windows_sys::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmStartSession, CCH_RM_SESSION_KEY,
    RM_PROCESS_INFO,
};

use super::wide_os;

/// The maximum number of entries passed to the Restart Manager when it looks
/// for the programs that hold a volume. On a stick with more files than this,
/// only the first entries are checked.
const HOLDER_SCAN_LIMIT: usize = 4096;

/// Return the names of the programs that have a file open under `root`,
/// without duplicates.
///
/// The Restart Manager reports the holders of files and cannot report the
/// holders of a volume, so this registers the entries under `root`, breadth
/// first, up to [`HOLDER_SCAN_LIMIT`] entries.
pub(super) fn holders_of(root: &str) -> Vec<String> {
    let paths = files_under(root);
    if paths.is_empty() {
        return Vec::new();
    }
    let Some(session) = RmSession::start() else {
        return Vec::new();
    };
    if !session.register(&paths) {
        return Vec::new();
    }
    session
        .processes()
        .map(|infos| app_names(&infos))
        .unwrap_or_default()
}

/// Return up to [`HOLDER_SCAN_LIMIT`] entries under `root`, breadth first, as
/// NUL-terminated wide paths.
fn files_under(root: &str) -> Vec<Vec<u16>> {
    let mut paths: Vec<Vec<u16>> = Vec::new();
    let mut queue = VecDeque::from([PathBuf::from(root)]);
    while paths.len() < HOLDER_SCAN_LIMIT {
        let Some(dir) = queue.pop_front() else {
            break;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten().take(HOLDER_SCAN_LIMIT - paths.len()) {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                queue.push_back(path.clone());
            }
            paths.push(wide_os(&path));
        }
    }
    paths
}

/// Return each program's name once, in the order listed.
fn app_names(infos: &[RM_PROCESS_INFO]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for info in infos {
        let len = info
            .strAppName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.strAppName.len());
        let name = String::from_utf16_lossy(&info.strAppName[..len]);
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// A Restart Manager session that ends when the value is dropped.
struct RmSession(u32);

impl RmSession {
    fn start() -> Option<Self> {
        let mut session = 0u32;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        // SAFETY: the key buffer has the room the API documents.
        if unsafe { RmStartSession(&mut session, 0, key.as_mut_ptr()) } != 0 {
            return None;
        }
        Some(Self(session))
    }

    /// Register `paths` as the files to ask about. Return whether the session
    /// accepted them.
    fn register(&self, paths: &[Vec<u16>]) -> bool {
        let ptrs: Vec<*const u16> = paths.iter().map(|p| p.as_ptr()).collect();
        // SAFETY: every pointer is a NUL-terminated string that outlives the call.
        let registered = unsafe {
            RmRegisterResources(
                self.0,
                ptrs.len() as u32,
                ptrs.as_ptr(),
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        registered == 0
    }

    /// Return the processes that hold the registered files.
    fn processes(&self) -> Option<Vec<RM_PROCESS_INFO>> {
        let mut have = 16u32;
        loop {
            // SAFETY: RM_PROCESS_INFO is plain data; zero is a valid value.
            let mut infos: Vec<RM_PROCESS_INFO> =
                (0..have).map(|_| unsafe { std::mem::zeroed() }).collect();
            let mut needed = 0u32;
            let mut reasons = 0u32;
            let mut count = have;
            // SAFETY: `infos` holds `count` entries.
            let rc = unsafe {
                RmGetList(
                    self.0,
                    &mut needed,
                    &mut count,
                    infos.as_mut_ptr(),
                    &mut reasons,
                )
            };
            if rc == ERROR_MORE_DATA && needed > have {
                have = needed;
                continue;
            }
            if rc != 0 {
                return None;
            }
            infos.truncate(count as usize);
            return Some(infos);
        }
    }
}

impl Drop for RmSession {
    fn drop(&mut self) {
        // SAFETY: the session came from RmStartSession and ends once.
        unsafe { RmEndSession(self.0) };
    }
}
