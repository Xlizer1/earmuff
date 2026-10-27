//! Earmuff's use cases, and the ports (traits) that adapters implement.
//!
//! Depends only on the domain. Backends, the UI and the command line plug in from
//! the outside: infrastructure implements [`Backend`] and [`Session`], and the
//! presentation layer drives the use cases and the [`DeviceManager`].

mod manager;
mod ports;
mod preferences;
mod usecases;

#[cfg(test)]
mod testing;

pub use manager::{AppEvent, DeviceManager, ManagerOptions};
pub use ports::{Backend, BackendError, Session, SessionError, Waker};
pub use preferences::{Autostart, Preferences, PreferencesStore};
pub use usecases::{Found, apply_settings, discover, open_device, open_first, read_status, settle};
