pub mod extract;
pub mod initramfs;
pub mod luks;

pub use extract::{
    extract_kernel, patch_kernel_smc_to_hvc, read_cabinet_image, read_firmware_info,
    read_images_targz, ExtractError, FirmwareInfo,
};
pub use initramfs::{extract_initramfs, patch_initramfs, PatchError};
pub use luks::{
    add_keyslot, cabinet_passphrase, decrypt_upd, vendor_passphrase, LuksKey, RekeyError,
};
