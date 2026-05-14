pub mod cfg;
pub mod config;
pub mod ffi;
pub mod instance;
#[cfg(target_os = "macos")]
mod macos_disk;
pub mod qmp;
pub mod shutdown;
pub mod tapbridge;
pub mod usb;
pub mod vmnet;

/// Re-exported from `cdj3k-emu-platform` so callers that already pull in
/// `cdj3k-emu-runtime` don't need a second `use` line.
pub use cdj3k_emu_platform::runtime_paths;

pub use cfg::{CfgClient, Latency};
pub use config::QemuConfig;
pub use instance::{
    cleanup_qemu_files, cleanup_runtime_files, kill_qemu_child, kill_qemu_child_now, InstanceError,
    QemuInstance, SHUTDOWN_SOCK_DIR,
};
pub use qmp::{QmpClient, QmpError};
pub use shutdown::{register_worker_thread, wait_for_worker, worker_is_finished};
pub use tapbridge::TapBridge;
pub use usb::{DiskProvider, MacOsDiskProvider, PhysicalDisk, UsbError, UsbManager};
pub use vmnet::SocketVmnet;
