//! Adapters: everything in Earmuff that touches the outside world.
//!
//! - [`hidraw`]: finds USB headsets and moves their reports through `/dev/hidraw*`
//! - [`native`]: the backend for headsets with a codec in earmuff-protocols
//! - [`demo`]: simulated headsets, for the tests and as demo devices

pub mod demo;
pub mod hidraw;
pub mod native;
