//! Closing every window before the installed build is replaced.

use std::time::{Duration, Instant};

use cdj3k_emu_platform::menu_state::MAX_INSTANCES;

use crate::Error;

/// How long the other windows get to stop their emulations and close.
const CLOSE_WAIT: Duration = Duration::from_secs(45);
/// A quit request lapses after ten seconds; it is renewed well before.
const RENEW_EVERY: Duration = Duration::from_secs(4);

/// The slots other than `own` that a window has open.
pub fn other_slots_open(own: u32) -> Vec<u32> {
    (1..=MAX_INSTANCES)
        .filter(|&n| n != own && cdj3k_emu_storage::slot_holder(n).is_some())
        .collect()
}

/// Ask every window but `own`'s to close, and wait until each has. A request
/// renewed after its window began closing is withdrawn, so it cannot close
/// the next window to open that slot.
pub fn close_other_slots(own: u32) -> Result<(), Error> {
    let open = other_slots_open(own);
    let deadline = Instant::now() + CLOSE_WAIT;
    let mut renew_at = Instant::now();
    loop {
        let waiting: Vec<u32> = open
            .iter()
            .copied()
            .filter(|&n| cdj3k_emu_storage::slot_holder(n).is_some())
            .collect();
        if waiting.is_empty() {
            for &n in &open {
                cdj3k_emu_storage::withdraw_quit_request(n);
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            let list: Vec<String> = waiting.iter().map(u32::to_string).collect();
            let (noun, pronoun) = if list.len() == 1 {
                ("slot", "it is")
            } else {
                ("slots", "they are")
            };
            return Err(Error::new(format!(
                "{noun} {} did not close; finish what {pronoun} doing and try again",
                list.join(", ")
            )));
        }
        if Instant::now() >= renew_at {
            for &n in &waiting {
                cdj3k_emu_storage::request_quit(n)?;
            }
            renew_at = Instant::now() + RENEW_EVERY;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
