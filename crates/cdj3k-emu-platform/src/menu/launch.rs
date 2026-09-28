//! Spawn-a-new-instance helper and the menu's alert dialogs.


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
    if let Some(app) = app_root.filter(|p| p.extension().is_some_and(|e| e == "app")) {
        reap(
            std::process::Command::new("/usr/bin/open")
                .arg("-n")
                .arg("-a")
                .arg(app)
                .arg("--args")
                .arg("--instance")
                .arg(target.to_string())
                .spawn(),
        );
    } else {
        // Dev mode: re-exec the binary directly.
        reap(
            std::process::Command::new(&exe)
                .arg("--instance")
                .arg(target.to_string())
                .spawn(),
        );
    }
}

/// Wait for a launched window's process on a thread of its own, so it does
/// not linger as a zombie once it exits.
fn reap(child: std::io::Result<std::process::Child>) {
    if let Ok(mut child) = child {
        let _ = std::thread::Builder::new()
            .name("cdj3k-emu-reap".into())
            .spawn(move || child.wait());
    }
}

/// Show a non-blocking error popup for a network setup failure
/// (vmnet / tapbridge).  Single OK button - user picks a different
/// interface from the menu to retry.
pub(super) fn show_net_error_alert(message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Network setup failed")
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// The physical disk could not be handed to the deck.
pub(super) fn show_usb_error_alert(message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("USB disk not attached")
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Shows a two-button alert and reports whether the first button was chosen.
/// With `OkCancelCustom`, every rfd backend returns the chosen button's label
/// as `MessageDialogResult::Custom`, not `Ok`.
fn confirm(title: &str, description: &str, ok: &str, cancel: &str) -> bool {
    let result = rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            ok.into(),
            cancel.into(),
        ))
        .show();
    matches!(result, rfd::MessageDialogResult::Custom(ref label) if label == ok)
}

/// Offered after the driver plugin is replaced under a running `MIDIServer`.
/// The new binary loads when that process next starts, so the choice is
/// whether to end it now.  Ending it drops every MIDI app's connection, and
/// those apps do not reconnect on their own, so it is the user's call.
pub(super) fn show_midi_driver_alert() {
    if confirm(
        "MIDI driver updated",
        "The emulator's MIDI driver was updated and takes effect when MIDI \
         services restart.\n\n\
         Restarting MIDI services now interrupts every MIDI application, and \
         some may need to be restarted.",
        "Restart MIDI Services",
        "Later",
    ) {
        let _ = std::process::Command::new("/usr/bin/killall")
            .arg("MIDIServer")
            .status();
    }
}

pub(super) fn show_raw_disk_alert(prompt: &str) {
    if confirm("Admin access required", prompt, "Retry", "Cancel") {
        menu_state::lock().usb_phys_retry_req = true;
    }
}
