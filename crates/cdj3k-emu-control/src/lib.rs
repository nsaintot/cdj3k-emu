//! The developer control API.
//!
//! In Developer Mode, the window that holds slot N listens on TCP port
//! [`PORT_BASE`] + N on every interface. Other programs drive the deck through
//! it: they press buttons, touch the screen, read the lamps and capture the
//! LCDs. `docs/control-api.md` describes the protocol.
//!
//! | Module | Holds |
//! |---|---|
//! | `command` | The request grammar and its parser |
//! | `server` | The listener and its connections |
//! | `client` | A connection from another program |
//! | `image` | RGBA images, PNG and base64 |
//! | `diff` | The pixel comparison of two captures |
//! | `cli` | The `cdj3k-emu ctl` subcommand |
//! | `steps` | The commands `ctl` and scenarios run, over one session |
//! | `e2e` | `ctl e2e`, the scenario runner |

pub mod cli;
pub mod client;
pub mod command;
pub mod diff;
mod e2e;
pub mod image;
pub mod server;
mod steps;

pub use command::{
    Command, JogGrip, JogMove, Knob, LampWant, Lever, LogSource, MediaCommand, ModsCommand, Screen,
    Target, Topic, WaitFor,
};
pub use image::Image;
pub use server::{ConnId, ControlServer, EventSink, Incoming, Responder};

/// Slot N listens on this port plus N.
pub const PORT_BASE: u16 = 4400;

/// An ephemeral session of slot N (`--ephemeral`) runs as slot N plus this,
/// the number of slots. The E2E runner drives those.
pub const EPHEMERAL_SLOT_OFFSET: u32 = 4;

/// The protocol version that `hello` reports. It changes when a command or a
/// reply changes in a way that breaks an existing client.
pub const PROTOCOL: u32 = 1;

/// The control port of slot `instance`.
pub fn port(instance: u32) -> u16 {
    PORT_BASE.saturating_add(instance.min(u16::MAX as u32) as u16)
}

/// A successful reply with no other fields.
pub fn ok() -> serde_json::Map<String, serde_json::Value> {
    let mut m = serde_json::Map::new();
    m.insert("ok".into(), true.into());
    m
}

/// A failed reply.
pub fn error(why: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": why.into() })
}
