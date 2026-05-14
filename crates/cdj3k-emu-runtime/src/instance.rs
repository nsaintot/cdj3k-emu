use std::net::TcpStream;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// PID of the live QEMU child process, or -1 when none is running.
/// Written by `QemuInstance::spawn`, cleared by `stop()` / `Drop`.
pub static QEMU_CHILD_PID: AtomicI32 = AtomicI32::new(-1);

/// How long EP122's sub-CPU takes to unmount USB after a power-off stimulus.
/// Mirrors the countdown on real hardware.
const EP122_CLEANUP_WAIT: Duration = Duration::from_secs(8);
/// How long systemd needs to walk the unit graph for ACPI shutdown.
const ACPI_SHUTDOWN_WAIT: Duration = Duration::from_secs(20);
/// Window between sending QMP `quit` and SIGKILL-ing the QEMU child.
const QMP_QUIT_WAIT: Duration = Duration::from_secs(5);
/// External (signal-handler-safe) SIGTERM grace before SIGKILL.
const SIGTERM_GRACE: Duration = Duration::from_secs(3);
/// Poll cadence for `wait_or_kill` / `kill_qemu_child` while waiting for the
/// QEMU child to exit. Short enough to feel responsive on a clean shutdown,
/// long enough not to busy-loop on a stuck guest.
const PROCESS_WAIT_POLL: Duration = Duration::from_millis(100);
/// Same idea, signal-safe variant used in `kill_qemu_child` (no atomics involved).
const SIGTERM_POLL: Duration = Duration::from_millis(50);
/// ivshmem jog-LCD buffer size. Layout fits 320×240 XRGB plus header in &lt;1 MiB.
const JOG_SHM_BYTES: u64 = 1 << 20;
/// Prefill size for `main.shm`. Large enough to cover any future stride choice
/// without forcing QEMU to grow the file (the reader holds a fixed mmap and
/// cannot follow `ftruncate`). 1280×720 RGBA8888 + header ≈ 3.6 MiB - 8 MiB
/// leaves a generous margin.
const MAIN_SHM_PREFILL: u64 = 8 * 1024 * 1024;

/// Graceful child shutdown for use from `on_exit` (main thread, blocking OK).
/// SIGTERM → up to [`SIGTERM_GRACE`] → SIGKILL.
pub fn kill_qemu_child() {
    let pid = QEMU_CHILD_PID.load(Ordering::Relaxed);
    if pid <= 0 {
        return;
    }
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    let deadline = Instant::now() + SIGTERM_GRACE;
    while Instant::now() < deadline {
        if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
            break; // process gone
        }
        std::thread::sleep(SIGTERM_POLL);
    }
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    QEMU_CHILD_PID.store(-1, Ordering::Relaxed);
}

/// Signal-safe child kill for use inside signal handlers (no sleep, no alloc).
pub fn kill_qemu_child_now() {
    let pid = QEMU_CHILD_PID.load(Ordering::Relaxed);
    if pid > 0 {
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
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
    /// Monitor thread: waits on the QEMU child process.
    thread: Option<JoinHandle<i32>>,
    qmp: QmpClient,
    /// PID of the QEMU subprocess, used for SIGKILL.
    pid: u32,
    /// Held for the lifetime of the instance - blocks any other cdj3k-emu process
    /// from booting against the same eMMC qcow2 (qcow2 is not concurrent-safe).
    _emmc_lock: Option<std::fs::File>,
}

/// A running QEMU instance.  Call `stop()` or let `Drop` send a quit + SIGKILL.
pub struct QemuInstance {
    config: QemuConfig,
    inner: Inner,
    running: Arc<AtomicBool>,
}

impl QemuInstance {
    /// Spawn QEMU as a child subprocess (re-exec self with --qemu-worker).
    /// Kills any stale QEMU from a previous .app run before spawning.
    #[cfg(target_os = "macos")]
    pub fn spawn(config: QemuConfig) -> Result<Self, InstanceError> {
        kill_stale(config.qmp_port, &config.sock_dir());

        std::fs::create_dir_all(config.sock_dir()).map_err(InstanceError::SockDir)?;

        // Exclusive non-blocking flock on the eMMC qcow2 - prevents two cdj3k-emu
        // instances from corrupting the same image. The lock is released when
        // the file handle held in Inner is dropped.
        let emmc_lock = if let Some(emmc) = &config.emmc_img {
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(emmc)
                .map_err(InstanceError::SockDir)?;
            let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc != 0 {
                return Err(InstanceError::EmmcLocked(emmc.clone()));
            }
            Some(f)
        } else {
            None
        };

        if config.shm {
            // Guest RAM file matches QemuConfig::MEM_BYTES; sparse so host disk usage is zero.
            prefill_sparse(&config.shm_path(), crate::config::QemuConfig::MEM_BYTES)
                .map_err(InstanceError::SockDir)?;
        }

        // Prefill main.shm to the max framebuffer size BEFORE QEMU starts.
        // The reader (cdj3k-emu-streams) mmaps the file at open time and never
        // resizes its mapping; if QEMU later grew the file via ftruncate, the
        // reader would see frames too large for its mmap and silently drop
        // them ("Display output is not active" forever). Prefilling avoids the
        // grow path entirely - QEMU's shm_remap fstats, sees the file is
        // already big enough, and skips ftruncate. Reader sees a zero header
        // (no magic) until QEMU writes it, so it just waits.
        prefill_sparse(&config.sock_dir().join("main.shm"), MAIN_SHM_PREFILL)
            .map_err(InstanceError::SockDir)?;

        // ivshmem jog frame buffer. Always recreate fresh so a stale
        // 'JOG1' magic from a previous run doesn't fool the host into reading
        // the prior session's pixels before the guest publishes the first frame.
        prefill_sparse(&config.jog_shm_path(), JOG_SHM_BYTES).map_err(InstanceError::SockDir)?;

        // 1-sector placeholder so the USB virtio-blk slot has a valid backing file at boot.
        {
            let ph = config.usb_placeholder_path();
            if !ph.exists() {
                prefill_sparse(&ph, 512).map_err(InstanceError::SockDir)?;
            }
        }

        let qemu_argv = config.build_argv();
        let self_exe = std::env::current_exe().map_err(InstanceError::SockDir)?;

        eprintln!(
            "cdj3k-emu: spawning QEMU subprocess: {} --qemu-worker {}",
            self_exe.display(),
            qemu_argv.join(" ")
        );

        let tap_fd = config.net_tap_fd;
        let mut cmd = std::process::Command::new(&self_exe);
        cmd.arg("--qemu-worker").args(&qemu_argv);
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(move || {
                // Re-clear FD_CLOEXEC after fork so the tap fd reaches cdj3k_emu_qemu_run.
                if let Some(fd) = tap_fd {
                    libc::fcntl(fd, libc::F_SETFD, 0);
                }
                // Pin QEMU's main thread to USER_INTERACTIVE QoS so macOS keeps
                // it on P-cores. Process-default QoS is inherited by QEMU
                // threads spawned later (audio callback, vCPU threads), which
                // reduces gap_max in the virtio_snd TX-return path.
                extern "C" {
                    fn pthread_set_qos_class_self_np(
                        qos_class: u32,
                        relative_priority: i32,
                    ) -> libc::c_int;
                }
                const QOS_CLASS_USER_INTERACTIVE: u32 = 0x21;
                pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(InstanceError::SockDir)?;

        let pid = child.id();
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
                eprintln!("cdj3k-emu: QEMU subprocess exited with code {code}");
                running_clone.store(false, Ordering::Release);
                code
            })
            .expect("failed to spawn QEMU monitor thread");

        let qmp = QmpClient::connect_with_retry(config.qmp_port, Duration::from_secs(15))?;

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

    /// Graceful stop: SPI power-off stimuli → 8 s for EP122 USB cleanup →
    /// ACPI system_powerdown → wait 20 s for guest halt → QMP quit + SIGKILL fallback.
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

    /// Drive the guest through EP122 cleanup → ACPI powerdown → QMP quit →
    /// SIGKILL, but leave thread joining + global PID reset to the caller.
    /// Shared by `stop()` and `Drop`.
    fn shutdown_sequence(&mut self) {
        if self.inner.thread.is_none() {
            return;
        }
        menu_state::lock().power_off_stimuli_requested = true;
        // Give EP122 ~8 s to unmount USB (mirrors the real sub-CPU countdown).
        wait_or_kill(&self.running, self.inner.pid, EP122_CLEANUP_WAIT);
        if self.running.load(Ordering::Acquire) {
            // EP122 cleanup done; trigger clean Linux shutdown via ACPI.
            let _ = self.inner.qmp.system_powerdown();
            wait_or_kill(&self.running, self.inner.pid, ACPI_SHUTDOWN_WAIT);
        }
        if self.running.load(Ordering::Acquire) {
            let _ = self.inner.qmp.quit();
            wait_or_kill(&self.running, self.inner.pid, QMP_QUIT_WAIT);
        }
    }

    /// Stop and restart with a new config.
    #[cfg(target_os = "macos")]
    pub fn restart(&mut self, new_config: QemuConfig) -> Result<(), InstanceError> {
        self.stop();
        // Release the flock before spawn tries to re-acquire it on a new fd.
        self.inner._emmc_lock = None;
        let new = Self::spawn(new_config)?;
        let new = std::mem::ManuallyDrop::new(new);
        // SAFETY: `new` is wrapped in `ManuallyDrop`, so its destructor will
        // not run when `new` goes out of scope at the end of this function.
        // The three `ptr::read`s bitwise-move each field into `self`,
        // overwriting `self`'s old fields whose destructors already ran via
        // `self.stop()` above plus the `_emmc_lock = None` drop on line 265
        // (i.e. self's own resources are already released).  After the
        // moves, the source struct is logically uninitialised and
        // `ManuallyDrop` prevents a double-drop on the moved-from fields.
        unsafe {
            self.inner = std::ptr::read(&new.inner);
            self.config = std::ptr::read(&new.config);
            self.running = std::ptr::read(&new.running);
        }
        Ok(())
    }

    /// Direct QMP access for device_add / device_del.
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

/// Poll until the process exits, then SIGKILL if it didn't within `timeout`.
fn wait_or_kill(running: &Arc<AtomicBool>, pid: u32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while running.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(PROCESS_WAIT_POLL);
    }
    if running.load(Ordering::Acquire) {
        eprintln!(
            "cdj3k-emu: QEMU did not exit within {}s - sending SIGKILL",
            timeout.as_secs()
        );
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    }
}

/// Create (or truncate) `path` to be a sparse file of exactly `len` bytes.
/// Used to prepare host-side mmaps so QEMU can map them at boot without
/// `ftruncate` growing the file out from under any active reader.
fn prefill_sparse(path: &Path, len: u64) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    f.set_len(len)
}

/// Kill any stale QEMU from a previous .app run and remove its socket files.
fn kill_stale(qmp_port: u16, sock_dir: &Path) {
    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", qmp_port).parse().unwrap();

    if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
        eprintln!("cdj3k-emu: stale QEMU detected on port {qmp_port}, sending quit");
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
        // If still up, there's nothing more we can do without the PID.
    }

    // Use the restart-safe variant: SocketVmnet may already be live (set up
    // before this spawn during a network change), and wiping its socket here
    // would crash the daemon mid-restart. The vmnet sock is owned by
    // SocketVmnet's own lifetime, never by the QEMU spawn cycle.
    cleanup_qemu_files_for_restart(sock_dir);
}

/// Path registered by the app for shutdown-time cleanup. Set once at startup;
/// `cleanup_runtime_files` reads it from any exit path (eframe on_exit,
/// signal handler, atexit) without needing to thread state through the UI.
pub static SHUTDOWN_SOCK_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Wipe the sock dir registered via `SHUTDOWN_SOCK_DIR`. No-op if unset.
/// Safe to call repeatedly - `cleanup_qemu_files` is idempotent.
pub fn cleanup_runtime_files() {
    if let Some(dir) = SHUTDOWN_SOCK_DIR.get() {
        cleanup_qemu_files(dir);
    }
}

/// Remove the sock dir and everything inside it, plus the sibling ram.shm.
/// Idempotent - safe to call when the directory does not exist.
pub fn cleanup_qemu_files(sock_dir: &Path) {
    cleanup_qemu_files_inner(sock_dir, /* keep_vmnet = */ false);
}

/// Restart-time cleanup: removes QEMU-managed files but preserves
/// `vmnet-*.sock`, which is managed by `SocketVmnet`'s own lifetime and must
/// outlive QEMU restarts (its daemon would exit if the socket vanished).
pub fn cleanup_qemu_files_for_restart(sock_dir: &Path) {
    cleanup_qemu_files_inner(sock_dir, /* keep_vmnet = */ true);
}

fn cleanup_qemu_files_inner(sock_dir: &Path, keep_vmnet: bool) {
    // Zero the shm magic *before* unlinking so any live reader (e.g. the UI's
    // main_stream poll loop) sees magic=0 through its existing mmap and drops
    // its mapping instead of staying stuck on the dead inode after restart.
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
            if keep_vmnet && entry.file_name().to_string_lossy().starts_with("vmnet-") {
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
    if !keep_vmnet {
        let _ = std::fs::remove_dir(sock_dir);
    }
}
