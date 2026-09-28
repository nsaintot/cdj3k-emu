pub mod cfg;
pub mod config;
pub mod disk;
pub mod elevate;
pub mod ffi;
pub mod instance;
pub mod net;
pub mod pc_link;
pub mod process;
pub mod provision;
pub mod qemu_exec;
pub mod qmp;
pub mod shutdown;

/// Re-exported from `cdj3k-emu-platform` so callers that already pull in
/// `cdj3k-emu-runtime` don't need a second `use` line.
pub use cdj3k_emu_platform::runtime_paths;

pub use cfg::{CfgClient, Latency};
pub use config::QemuConfig;
pub use disk::{
    host_disk_provider, DiskProvider, HostDiskProvider, PhysicalDisk, UsbError, UsbManager,
};
pub use instance::{
    cleanup_qemu_files, cleanup_runtime_files, kill_qemu_child, kill_qemu_child_now, InstanceError,
    QemuInstance, SHUTDOWN_SOCK_DIR,
};
pub use net::vmnet::VmnetMode;
pub use net::{
    attach as net_attach, attach_host_only as net_attach_host_only, NetAttachment, NetKeepAlive,
};
pub use provision::QemuGuestRunner;
pub use qmp::{QmpClient, QmpError};
pub use shutdown::{
    reap_finished_worker, register_worker_thread, wait_for_worker, worker_is_finished,
};
