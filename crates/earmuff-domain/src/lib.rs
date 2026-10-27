//! Earmuff's domain model: what a headset is, what it can do and what state it is in.
//!
//! No I/O and no dependencies, so every rule here is plain, testable Rust. The other
//! layers depend on this crate; it depends on nothing.

mod device;
mod equalizer;
mod status;
mod values;

pub use device::{BackendKind, Capability, DeviceId, DeviceInfo, SettingError, UsbId, UsbIdError};
pub use equalizer::{EqPreset, EqualizerSpec, Gain, GainError};
pub use status::{Event, Query, Setting, Status};
pub use values::{AutoOff, ChargeState, NoiseFilter, Percent, PercentError};
