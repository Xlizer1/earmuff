//! What the application needs from the outside world.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use earmuff_domain::{BackendKind, Capability, DeviceInfo, Event, Query, Setting};

/// Finds headsets of one kind and opens them
pub trait Backend: Send + Sync {
    fn kind(&self) -> BackendKind;

    /// Headsets this backend can reach right now
    fn discover(&self) -> Result<Vec<DeviceInfo>, BackendError>;

    fn open(&self, device: &DeviceInfo) -> Result<Box<dyn Session>, BackendError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// No supported headset is plugged in
    NotFound,
    /// A headset is there, but this user may not open it
    PermissionDenied {
        path: String,
        hint: String,
    },
    Failed(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            BackendError::NotFound => f.write_str("no supported headset found"),
            BackendError::PermissionDenied { path, hint } => write!(f, "no permission to open {path} ({hint})"),
            BackendError::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for BackendError {}

/// An open connection to one headset.
///
/// A session belongs to one thread at a time. Commands are queued by `query` and
/// `apply` and go out as the headset allows; answers and anything the headset
/// reports on its own come back from `poll`.
pub trait Session: Send {
    fn info(&self) -> &DeviceInfo;

    /// Asks the headset for current values
    fn query(&mut self, query: Query) -> Result<(), SessionError>;

    /// Starts changing a setting; the value the headset kept arrives through `poll`
    fn apply(&mut self, setting: Setting) -> Result<(), SessionError>;

    /// Waits up to `timeout` for news, sending queued commands meanwhile. Returns
    /// early with whatever arrived, or with nothing when the waker fires.
    fn poll(&mut self, timeout: Duration) -> Result<Vec<Event>, SessionError>;

    /// True when nothing is queued or waiting for an answer
    fn is_idle(&self) -> bool;

    /// Lets another thread interrupt a `poll` in progress
    fn waker(&self) -> Waker;

    /// Whether `poll` also returns every report as [`Event::Raw`], for diagnostics
    fn set_raw(&mut self, raw: bool);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The headset or its dongle went away
    Disconnected(String),
    Unsupported(Capability),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SessionError::Disconnected(reason) => write!(f, "headset disconnected: {reason}"),
            SessionError::Unsupported(capability) => write!(f, "this headset doesn't support {capability:?}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Interrupts a blocked [`Session::poll`] from another thread
#[derive(Clone)]
pub struct Waker(Arc<dyn Fn() + Send + Sync>);

impl Waker {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Waker(Arc::new(wake))
    }

    pub fn wake(&self) {
        (self.0)()
    }
}

impl fmt::Debug for Waker {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Waker")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn waker_calls_its_function() {
        let count = Arc::new(AtomicUsize::new(0));
        let counted = count.clone();
        let waker = Waker::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        waker.clone().wake();
        waker.wake();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn errors_explain_themselves() {
        let denied =
            BackendError::PermissionDenied { path: "/dev/hidraw3".into(), hint: "install the udev rule".into() };
        assert_eq!(denied.to_string(), "no permission to open /dev/hidraw3 (install the udev rule)");
        assert_eq!(SessionError::Disconnected("unplugged".into()).to_string(), "headset disconnected: unplugged");
        assert_eq!(
            SessionError::Unsupported(Capability::Sidetone).to_string(),
            "this headset doesn't support Sidetone"
        );
    }
}
