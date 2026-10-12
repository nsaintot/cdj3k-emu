use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The PID of the live QEMU child process, or -1 when none is running.
/// `QemuInstance::spawn` sets it, and `stop()` and `Drop` clear it.
pub static QEMU_CHILD_PID: AtomicI32 = AtomicI32::new(-1);

/// How long the player's sub-CPU waits after a power-off stimulus before it
/// cuts power, which gives EP122 time to unmount USB. This matches the
/// countdown on the real player.
const APP_CLEANUP_WAIT: Duration = Duration::from_secs(8);
/// How long to wait after QMP `quit` before killing the QEMU child.
/// QEMU flushes its disks and exits well within it.
const QMP_QUIT_WAIT: Duration = Duration::from_secs(2);
/// How long `kill_qemu_child` waits after SIGTERM before it sends SIGKILL.
const SIGTERM_GRACE: Duration = Duration::from_secs(3);
/// The poll interval of `wait_for_exit` while it waits for the QEMU child to
/// exit. It is short enough to keep a clean shutdown responsive, and long
/// enough not to busy-loop on a stuck guest.
const PROCESS_WAIT_POLL: Duration = Duration::from_millis(100);
/// The poll interval of `kill_qemu_child` while it waits for the QEMU child to
/// exit.
const SIGTERM_POLL: Duration = Duration::from_millis(50);
/// The size of the ivshmem jog LCD buffer. A 320×240 XRGB frame and its header
/// fit in less than 1 MiB.
const JOG_SHM_BYTES: u64 = 1 << 20;
/// The prefill size of `main.shm`. It covers any stride, so QEMU never has to
/// grow the file; the reader holds a fixed mapping and cannot follow
/// `ftruncate`. A 1280×720 RGBA8888 frame and its header take about 3.5 MiB,
/// so 8 MiB leaves a generous margin.
const MAIN_SHM_PREFILL: u64 = 8 * 1024 * 1024;

/// Stop the QEMU child gracefully from `on_exit`, on the main thread, where
/// blocking is allowed. Send SIGTERM, wait up to [`SIGTERM_GRACE`], then send
/// SIGKILL.
pub fn kill_qemu_child() {
    let pid = QEMU_CHILD_PID.load(Ordering::Relaxed);
    if pid <= 0 {
        return;
    }
    crate::process::terminate(pid);
    let deadline = Instant::now() + SIGTERM_GRACE;
    while Instant::now() < deadline && crate::process::is_alive(pid) {
        std::thread::sleep(SIGTERM_POLL);
    }
    crate::process::kill(pid);
    QEMU_CHILD_PID.store(-1, Ordering::Relaxed);
}

/// Kill the QEMU child at once. Signal handlers can call this, because it
/// neither sleeps nor allocates.
pub fn kill_qemu_child_now() {
    let pid = QEMU_CHILD_PID.load(Ordering::Relaxed);
    if pid > 0 {
        crate::process::kill(pid);
    }
}

use cdj3k_emu_platform::menu_state;

use crate::config::QemuConfig;
use crate::qmp::{QmpClient, QmpError};

#[derive(Debug)]
pub enum InstanceError {
    AlreadyRunning,
    DylibUnavailable,
    QmpConnect(QmpError),
    SockDir(std::io::Error),
    EmmcLocked(PathBuf),
}

impl From<QmpError> for InstanceError {
    fn from(e: QmpError) -> Self {
        InstanceError::QmpConnect(e)
    }
}

struct Inner {
    /// The monitor thread, which waits on the QEMU child process.
    thread: Option<JoinHandle<i32>>,
    qmp: QmpClient,
    /// The PID of the QEMU child process, used for SIGKILL.
    pid: u32,
    /// The eMMC lock, held for the lifetime of the instance. It stops any other
    /// cdj3k-emu process from booting the same eMMC qcow2, which does not
    /// support concurrent writers.
    _emmc_lock: Option<std::fs::File>,
}

/// A running QEMU instance. Call `stop()`, or let `Drop` run the same shutdown
/// sequence.
pub struct QemuInstance {
    config: QemuConfig,
    inner: Inner,
    running: Arc<AtomicBool>,
}

impl QemuInstance {
    /// Start QEMU in a child process. Before that, quit any stale QEMU that
    /// still answers on this instance's QMP port.
    pub fn spawn(config: QemuConfig) -> Result<Self, InstanceError> {
        let emmc_lock = lock_emmc(&config)?;

        // Kill a stale QEMU only after taking the lock. While another window's
        // emulation runs, that window holds the lock, and the QEMU on this QMP
        // port is live.
        kill_stale(config.qmp_port, &config.sock_dir());

        prepare_sock_dir(&config)?;
        let (pid, thread, running) = start_qemu(&config)?;

        let qmp = QmpClient::connect_with_retry(config.qmp_port, Duration::from_secs(15))?;
        crate::process::set_quit_channel(qmp.stream());

        Ok(Self {
            inner: Inner {
                thread: Some(thread),
                qmp,
                pid,
                _emmc_lock: emmc_lock,
            },
            config,
            running,
        })
    }

    pub fn sock_dir(&self) -> PathBuf {
        self.config.sock_dir()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Stop the guest through the steps of [`Self::shutdown_sequence`] and
    /// return QEMU's exit code.
    pub fn stop(&mut self) -> i32 {
        self.shutdown_sequence();
        let code = self
            .inner
            .thread
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or(-1);
        QEMU_CHILD_PID.store(-1, Ordering::Relaxed);
        code
    }

    /// Stop QEMU in steps, each one only while QEMU still runs:
    ///
    /// 1. Send the power-off stimuli and wait [`APP_CLEANUP_WAIT`], so that
    ///    EP122 can unmount USB.
    /// 2. Send QMP `quit` and wait [`QMP_QUIT_WAIT`].
    /// 3. Kill QEMU.
    ///
    /// The caller joins the monitor thread and resets the global PID.
    /// `stop()` and `Drop` share this.
    fn shutdown_sequence(&mut self) {
        if self.inner.thread.is_none() {
            return;
        }
        menu_state::lock().power_off_stimuli_requested = true;
        if wait_for_exit(&self.running, APP_CLEANUP_WAIT) {
            return;
        }
        let _ = self.inner.qmp.quit();
        if wait_for_exit(&self.running, QMP_QUIT_WAIT) {
            return;
        }
        eprintln!("cdj3k-emu: QEMU did not stop after the power-off and quit; killing it.");
        crate::process::kill(self.inner.pid as i32);
    }

    /// Stop and restart with a new config.
    pub fn restart(&mut self, new_config: QemuConfig) -> Result<(), InstanceError> {
        self.stop();
        // Release the flock before spawn tries to re-acquire it on a new fd.
        self.inner._emmc_lock = None;
        let new = Self::spawn(new_config)?;
        let new = std::mem::ManuallyDrop::new(new);
        // SAFETY: `new` is wrapped in `ManuallyDrop`, so its destructor does
        // not run. Each `ptr::read` moves one field of `new` into `self`, and
        // the assignment drops `self`'s old field; `self.stop()` and the
        // `_emmc_lock = None` above have already released the old QEMU and
        // the old lock. `new` is not used again, so no field is dropped
        // twice.
        unsafe {
            self.inner = std::ptr::read(&new.inner);
            self.config = std::ptr::read(&new.config);
            self.running = std::ptr::read(&new.running);
        }
        Ok(())
    }

    /// Give direct QMP access, for device_add and device_del.
    pub fn qmp(&mut self) -> &mut QmpClient {
        &mut self.inner.qmp
    }
}

impl Drop for QemuInstance {
    fn drop(&mut self) {
        self.shutdown_sequence();
        self.inner.thread.take().and_then(|t| t.join().ok());
        QEMU_CHILD_PID.store(-1, Ordering::Relaxed);
        cleanup_qemu_files_for_restart(&self.config.sock_dir());
    }
}

/// Take an exclusive, non-blocking flock on the eMMC qcow2, so that two
/// cdj3k-emu instances cannot corrupt the same image. The lock is released
/// when the returned handle is dropped. Another window that checks whether
/// the slot is in use holds the lock for an instant, so a failed attempt is
/// retried briefly before giving up.
fn lock_emmc(config: &QemuConfig) -> Result<Option<std::fs::File>, InstanceError> {
    let Some(emmc) = &config.emmc_img else {
        return Ok(None);
    };
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(emmc)
        .map_err(InstanceError::SockDir)?;
    let mut tries = 0;
    while cdj3k_emu_platform::file_lock::lock(&f, emmc).is_err() {
        tries += 1;
        if tries > 12 {
            return Err(InstanceError::EmmcLocked(emmc.clone()));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Ok(Some(f))
}

/// Create the instance directory and the files QEMU maps at boot.
fn prepare_sock_dir(config: &QemuConfig) -> Result<(), InstanceError> {
    // Create the runtime root first, because the host's isolation step runs
    // on the root when the root is created.
    cdj3k_emu_platform::runtime_paths::ensure_runtime_base_dir().map_err(InstanceError::SockDir)?;
    std::fs::create_dir_all(config.sock_dir()).map_err(InstanceError::SockDir)?;

    if config.shm {
        // The guest RAM file is QemuConfig::MEM_BYTES long and sparse, so it
        // uses no host disk space.
        prefill_sparse(&config.shm_path(), crate::config::QemuConfig::MEM_BYTES)
            .map_err(InstanceError::SockDir)?;
    }

    // Prefill main.shm to the largest framebuffer size before QEMU starts.
    // The reader in cdj3k-emu-streams maps the file when it opens it and
    // never resizes the mapping. If QEMU grew the file with ftruncate, the
    // reader would drop every frame as too large, and the window would show
    // "Display output is not active". With the prefill, QEMU's shm_remap
    // finds the file big enough and skips ftruncate. The reader sees a zero
    // header until QEMU writes it, and waits.
    prefill_sparse(&config.sock_dir().join("main.shm"), MAIN_SHM_PREFILL)
        .map_err(InstanceError::SockDir)?;

    // Always recreate the ivshmem jog frame buffer, so that a stale 'JOG1'
    // magic from a previous run does not make the host show that run's
    // pixels before the guest publishes its first frame.
    prefill_sparse(&config.jog_shm_path(), JOG_SHM_BYTES).map_err(InstanceError::SockDir)?;

    // Create a one-sector placeholder, so that the USB virtio-blk slot has a
    // valid backing file at boot.
    let ph = config.usb_placeholder_path();
    if !ph.exists() {
        prefill_sparse(&ph, 512).map_err(InstanceError::SockDir)?;
    }
    Ok(())
}

/// Start the QEMU child and the thread that waits on it. Returns the child's
/// PID, that thread (which yields the exit code), and the flag it clears when
/// the child exits.
fn start_qemu(
    config: &QemuConfig,
) -> Result<(u32, JoinHandle<i32>, Arc<AtomicBool>), InstanceError> {
    let qemu_argv = config.build_argv();
    eprintln!(
        "cdj3k-emu: starting the QEMU child process: {}",
        crate::qemu_exec::describe(&qemu_argv)
    );

    let mut cmd = crate::qemu_exec::qemu_command(config.net_tap_fd)
        .map_err(InstanceError::SockDir)?
        .with_argv(&qemu_argv);
    crate::process::own_group(&mut cmd);
    let child = cmd.spawn().map_err(InstanceError::SockDir)?;

    let pid = child.id();
    crate::process::adopt(pid);
    QEMU_CHILD_PID.store(pid as i32, Ordering::Relaxed);
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    let thread = std::thread::Builder::new()
        .name(format!("qemu-monitor-{}", config.instance_id))
        .spawn(move || {
            let code = child
                .wait_with_output()
                .map(|o| o.status.code().unwrap_or(-1))
                .unwrap_or(-1);
            eprintln!("cdj3k-emu: the QEMU child process exited with code {code}.");
            running_clone.store(false, Ordering::Release);
            code
        })
        .expect("failed to spawn QEMU monitor thread");
    Ok((pid, thread, running))
}

/// Wait up to `timeout` for the QEMU child to exit. Returns whether it exited.
fn wait_for_exit(running: &Arc<AtomicBool>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while running.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(PROCESS_WAIT_POLL);
    }
    !running.load(Ordering::Acquire)
}

/// Create or truncate `path` as a sparse file of exactly `len` bytes, so that
/// QEMU can map it at boot without growing it under an active reader.
fn prefill_sparse(path: &Path, len: u64) -> std::io::Result<()> {
    cdj3k_emu_platform::sparse_file::create(path, len)
}

/// Quit any stale QEMU that answers on `qmp_port`, and remove its files from
/// the sock dir.
fn kill_stale(qmp_port: u16, sock_dir: &Path) {
    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", qmp_port).parse().unwrap();

    if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
        eprintln!("cdj3k-emu: found a stale QEMU on port {qmp_port}; sending quit.");
        if let Ok(mut qmp) = QmpClient::connect(qmp_port) {
            let _ = qmp.quit();
        }
        // Wait up to 5 s for the QMP port to close.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            if TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_err() {
                break;
            }
        }
        // Without its PID, a QEMU that is still running cannot be killed.
    }

    // Keep the sock dir itself, which the next QEMU reuses.
    cleanup_qemu_files_for_restart(sock_dir);
}

/// The sock dir that the app registers for cleanup at shutdown. The app sets
/// it once at startup, and `cleanup_runtime_files` reads it from any exit
/// path (eframe on_exit, a signal handler, atexit), so no state has to pass
/// through the UI.
pub static SHUTDOWN_SOCK_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Clean up the runtime files at shutdown. Every exit path calls this (eframe
/// on_exit, a signal handler, atexit), and it is the only function that
/// removes the per-UID runtime dir.
///
/// 1. Remove this instance's sock dir, registered in `SHUTDOWN_SOCK_DIR`.
/// 2. Remove the per-UID runtime dir once it is empty; another slot's
///    `instance-N` keeps it.
///
/// Each step is idempotent and best-effort, so the function is safe to call
/// repeatedly and when nothing is left.
pub fn cleanup_runtime_files() {
    if let Some(dir) = SHUTDOWN_SOCK_DIR.get() {
        cleanup_qemu_files(dir);
    }
    prune_shared_dirs(&cdj3k_emu_platform::runtime_paths::runtime_base_dir());
}

/// Remove the per-UID runtime dir when it is empty. The non-recursive
/// `remove_dir` fails while another slot's `instance-N` remains.
fn prune_shared_dirs(base_dir: &Path) {
    let _ = std::fs::remove_dir(base_dir);
}

/// Remove the sock dir and everything in it. This is idempotent, so it is
/// safe to call when the directory does not exist.
pub fn cleanup_qemu_files(sock_dir: &Path) {
    cleanup_qemu_files_inner(sock_dir, /* keep_dir = */ false);
}

/// Whether a name in the sock dir belongs to the app rather than to QEMU.
///
/// Every other file belongs to QEMU, and the next QEMU rebuilds it, so a
/// restart removes it. The app's files outlive a respawn, and a restart must
/// not touch them:
///
/// * `net.claim` and `net.released` are the link's lease (`net::lease`);
///   the elevated watcher takes the link down when the claim goes.
/// * `tapbridge.*`, `linuxnet.*` and `winnet.*` are the macOS, Linux and
///   Windows links' elevated scripts and hand-over files.
/// * `midi-driver.sock` is bound by `MidiDriverLink` for as long as PC Link
///   is on, and the CoreMIDI plugin dials it by name.
/// * `window.sock` is bound by `WindowSocket` for as long as the window is
///   open, and other windows send it commands.
///
/// A new bridge back end must add its file names here.
fn app_owned(name: &str) -> bool {
    name == crate::net::lease::CLAIM_FILE
        || name == crate::net::lease::RELEASED_FILE
        || name.starts_with("tapbridge.")
        || name.starts_with("linuxnet.")
        || name.starts_with("winnet.")
        || name == "midi-driver.sock"
        || name == "window.sock"
}

/// Empty the sock dir for a restart, but keep the directory and the app's
/// files, which the next QEMU reuses.
pub fn cleanup_qemu_files_for_restart(sock_dir: &Path) {
    cleanup_qemu_files_inner(sock_dir, /* keep_dir = */ true);
}

fn cleanup_qemu_files_inner(sock_dir: &Path, keep_dir: bool) {
    // Zero the shm magic before unlinking the files, so that a live reader,
    // such as the UI's main_stream poll loop, sees a zero magic through its
    // mapping and drops it. Otherwise the reader would stay on the deleted
    // file after a restart.
    for shm_name in &["main.shm", "jog.shm"] {
        let shm = sock_dir.join(shm_name);
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&shm) {
            use std::io::Write;
            let _ = f.write_all(&[0u8; 4]);
        }
    }
    if let Ok(entries) = std::fs::read_dir(sock_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if keep_dir && app_owned(&name) {
                continue;
            }
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
    if !keep_dir {
        let _ = std::fs::remove_dir(sock_dir);
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::{cleanup_qemu_files, cleanup_qemu_files_for_restart};

    /// A restart leaves a bridge's files alone; shutdown takes them with
    /// everything else.
    #[test]
    fn restart_keeps_bridge_state_shutdown_does_not() {
        let dir = std::env::temp_dir().join(format!("cdj3k-tapclean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let seed = || {
            std::fs::create_dir_all(&dir).unwrap();
            for f in [
                "net.claim",
                "net.released",
                "winnet.result",
                "tapbridge.sh",
                "tapbridge.names",
                "linuxnet.sh",
                "linuxnet.ready",
                "midi-driver.sock",
                "window.sock",
                "ctrl.sock",
            ] {
                std::fs::write(dir.join(f), "").unwrap();
            }
        };

        seed();
        cleanup_qemu_files_for_restart(&dir);
        assert!(
            dir.join("net.claim").exists(),
            "the lease must survive a restart"
        );
        assert!(dir.join("net.released").exists());
        assert!(dir.join("winnet.result").exists());
        assert!(dir.join("tapbridge.sh").exists());
        assert!(dir.join("tapbridge.names").exists());
        assert!(dir.join("linuxnet.sh").exists());
        assert!(dir.join("linuxnet.ready").exists());
        assert!(dir.join("midi-driver.sock").exists());
        assert!(dir.join("window.sock").exists());
        assert!(!dir.join("ctrl.sock").exists(), "QEMU-owned files still go");
        assert!(dir.exists(), "restart keeps the dir");

        seed();
        cleanup_qemu_files(&dir);
        assert!(!dir.exists(), "shutdown takes the whole dir");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod prune_tests {
    use super::prune_shared_dirs;

    /// The runtime base dir is removed only once it is empty; another slot's
    /// `instance-N` inside it keeps the whole tree.
    #[test]
    fn prune_shared_dirs_removes_the_base_only_when_empty() {
        let root = std::env::temp_dir().join(format!("cdj3k-prune-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("cdj3k-emu");
        let other_slot = base.join("instance-7");
        std::fs::create_dir_all(&other_slot).unwrap();

        prune_shared_dirs(&base);
        assert!(base.exists(), "another slot's dir keeps the base");

        std::fs::remove_dir(&other_slot).unwrap();
        prune_shared_dirs(&base);
        assert!(!base.exists(), "the base dir is removed once it is empty");
        let _ = std::fs::remove_dir_all(&root);
    }
}
