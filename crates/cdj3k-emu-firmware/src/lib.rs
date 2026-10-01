pub mod cpio;
pub mod extract;
mod file_mode;
pub mod initramfs;
pub mod initramfs_guest;
pub mod luks;

pub use extract::{
    extract_kernel, patch_kernel_smc_to_hvc, read_cabinet_image, read_firmware_info,
    read_images_targz, ExtractError, FirmwareInfo,
};
pub use initramfs::{extract_initramfs, patch_initramfs, PatchError};
pub use initramfs_guest::{patch_initramfs_in_guest, GuestProvision, GuestRunner};
pub use luks::{
    add_keyslot, cabinet_passphrase, decrypt_upd, vendor_passphrase, LuksKey, RekeyError,
};
