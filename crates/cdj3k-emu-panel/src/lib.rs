//! The players the emulator boots, and the front panel each one presents.
//!
//! A model is a [`ModelSpec`] constant - [`cdj3k::SPEC`] is the CDJ-3000,
//! [`cdj3kx::SPEC`] the CDJ-3000X, [`cdj1500x::SPEC`] the CDJ-1500X - holding
//! its scalars, its MISO encoder and its MOSI decoder. The UI describes the
//! panel in a [`PanelState`] and the player's [`MisoCodec`] writes its frame,
//! quirks included; the UI names a [`Lamp`] and the player's [`MosiCodec`]
//! reads it back out of a [`MosiFrame`]. Adding a player is a new spec file
//! and a new [`Model`] variant, and every consumer follows without a new
//! match arm.
//!
//! The [`Btn`](button::Btn) bits, the CDJ-3000's `LED_*` bits and its MISO
//! field offsets are in its frame coordinates, which the CDJ-3000X's map and
//! encoder move from.

pub mod button;
pub mod cdj1500x;
pub mod cdj3k;
pub mod cdj3kx;
pub mod crc;
pub mod direction;
pub mod frame;
pub mod lamp;
pub mod miso_frame;
pub mod model;
pub mod mosi_frame;
pub mod spec;

pub use button::Btn;
pub use crc::crc16_x25;
pub use direction::Direction;
pub use frame::{JogBrightness, MosiMap, RgbLamps, StepLedMask};
pub use lamp::Lamp;
pub use miso_frame::{JogState, MisoCodec, PanelState};
pub use model::Model;
pub use mosi_frame::{LampState, MosiCodec, MosiFrame, StepLed};
pub use spec::{JogBrake, ModelSpec, TouchSpace, TOUCH_DOWN};
