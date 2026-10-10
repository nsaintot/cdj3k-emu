//! Closing every window before the installed build is replaced.

use std::time::{Duration, Instant};

use cdj3k_emu_platform::menu_state::MAX_INSTANCES;
use cdj3k_emu_platform::window_socket::{self, Command, Reply};

use crate::Error;

/// How long the other windows get to stop their emulations and close.
const CLOSE_WAIT: Duration = Duration::from_secs(45);
/// How often a window that has not accepted `quit` is asked again.
const ASK_EVERY: Duration = Duration::from_secs(4);

/// The slots other than `own` that a window has open.
pub fn other_slots_open(own: u32) -> Vec<u32> {
    (1..=MAX_INSTANCES)
        .filter(|&n| n != own && cdj3k_emu_storage::slot_holder(n).is_some())
        .collect()
}

/// Ask every window except `own`'s to close, and wait until they have. A
/// window that answers `busy`, or does not answer, is asked again.
pub fn close_other_slots(own: u32) -> Result<(), Error> {
    let open = other_slots_open(own);
    let deadline = Instant::now() + CLOSE_WAIT;
    let mut ask_at = Instant::now();
    let mut closing = Vec::new();
    loop {
        let waiting: Vec<u32> = open
            .iter()
            .copied()
            .filter(|&n| cdj3k_emu_storage::slot_holder(n).is_some())
            .collect();
        if waiting.is_empty() {
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
        if Instant::now() >= ask_at {
            // Ask the waiting windows in parallel, so that one that does not
            // answer does not delay the others.
            let asking: Vec<u32> = waiting
                .into_iter()
                .filter(|n| !closing.contains(n))
                .collect();
            std::thread::scope(|s| {
                let asked: Vec<_> = asking
                    .iter()
                    .map(|&n| s.spawn(move || (n, window_socket::send(n, Command::Quit))))
                    .collect();
                for handle in asked {
                    if let Ok((n, Ok(Some(Reply::Ok)))) = handle.join() {
                        closing.push(n);
                    }
                }
            });
            ask_at = Instant::now() + ASK_EVERY;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
