//! Earmuff's use cases, and the ports (traits) that adapters implement.
//!
//! Depends only on the domain. Backends, the UI and the command line plug in from
//! the outside: infrastructure implements [`Backend`] and [`Session`], and the
//! presentation layer drives the use cases and the [`DeviceManager`].

mod alerts;
mod audio;
mod history;
mod manager;
mod ports;
mod preferences;
mod profiles;
mod usecases;

#[cfg(test)]
mod testing;

pub use alerts::{Alert, AlertSettings, BatteryAlerts};
pub use audio::{AudioSwitch, AudioSystem, Switch, use_headset_when_ready};
pub use history::{BatteryHistory, HistoryStore, KEEP_FOR, SAMPLE_EVERY, Sample, minutes_left, time_left_text};
pub use manager::{AppEvent, DeviceManager, ManagerOptions};
pub use ports::{Backend, BackendError, Session, SessionError, Waker};
pub use preferences::{Autostart, Preferences, PreferencesStore};
pub use profiles::{ProfileStore, Profiles, remembered};
pub use usecases::{Found, apply_settings, discover, open_device, open_first, read_status, settle, toggle_mute};
