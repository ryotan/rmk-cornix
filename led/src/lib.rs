//! Status LED logic for the Cornix.
//!
//! [`Status`] holds what the firmware knows: links, power, layer, battery
//! and output route, updated by its `set_*` methods. [`Indicator`] turns a
//! `Status` and the current time into a [`Frame`]: one colour per pixel and
//! the moment the display can next change. The crate does not depend on
//! hardware or embassy, so it builds and tests on the host.

#![cfg_attr(not(test), no_std)]

pub mod color;
pub mod indicator;
pub mod status;

pub use color::{Grb, Lit};
pub use indicator::{Frame, Indicator, Role};
pub use status::{HostState, Ms, Status, Vbus};
