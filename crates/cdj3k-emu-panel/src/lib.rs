//! The players the emulator boots, and the front panel each one presents.
//!
//! A model is a [`ModelSpec`] constant - [`cdj3k::SPEC`] is the CDJ-3000,
//! [`cdj3kx::SPEC`] the CDJ-3000X - holding its scalars and the two frame maps
//! that say where its sub-CPU keeps each control and lamp. [`MisoFrame`] and
//! [`MosiFrame`] read and write those frames against the map, so the codec is
//! written ONCE and parameterised by the player, not forked: adding one is a
//! new spec file and a new [`Model`] variant, and every consumer follows
//! without a new match arm.
//!
//! `LED_*`, `BTN_*` and the MISO field offsets are in the CDJ-3000's frame
//! coordinates, the shared origin every map is expressed relative to.

pub mod button;
pub mod cdj3k;
pub mod cdj3kx;
pub mod crc;
pub mod direction;
pub mod frame;
pub mod miso_frame;
pub mod model;
pub mod mosi_frame;
pub mod spec;

pub use button::Btn;
pub use crc::crc16_x25;
pub use direction::Direction;
pub use frame::{JogBrightness, MisoMap, MosiMap, RgbLamps, StepLedMask};
pub use miso_frame::MisoFrame;
pub use model::Model;
pub use mosi_frame::{MosiFrame, StepLed};
pub use spec::{ModelSpec, TouchSpace, TOUCH_DOWN};
