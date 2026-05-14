//! Spawn-a-new-instance helper plus the macOS Full Disk Access alert
//! shown when a USB pass-through op fails for permission reasons.

use crate::menu_state;

pub(super) fn launch_instance(target: u32) {
    if target < 1 {
        return;
    }
    let cur = menu_state::lock().current_instance_id;
    if target == cur {
        return;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    // Walk up to the .app bundle root.
    // Layout: <App>.app/Contents/MacOS/cdj3k-emu -> <App>.app
    let app_root = exe
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent());
    if let Some(app) = app_root.filter(|p| p.extension().map_or(false, |e| e == "app")) {
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("-n")
            .arg("-a")
            .arg(app)
            .arg("--args")
            .arg("--instance")
            .arg(target.to_string())
            .spawn();
    } else {
        // Dev mode: re-exec the binary directly.
        let _ = std::process::Command::new(&exe)
            .arg("--instance")
            .arg(target.to_string())
            .spawn();
    }
}

/// Show a non-blocking error popup for a network setup failure
/// (socket_vmnet / tapbridge).  Single OK button - user picks a different
/// interface from the menu to retry.
pub(super) fn show_net_error_alert(message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Network setup failed")
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

pub(super) fn show_fda_alert() {
    let result = rfd::MessageDialog::new()
        .set_title("Admin access required")
        .set_description(
            "cdj3k-emu needs write access to the raw disk device.\n\n\
             Click Retry to show a macOS password prompt that grants \
             temporary write permission (chmod 660). \
             This resets automatically when the drive is unplugged.",
        )
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            "Retry".into(),
            "Cancel".into(),
        ))
        .show();
    if result == rfd::MessageDialogResult::Ok {
        menu_state::lock().usb_phys_retry_req = true;
    }
}
