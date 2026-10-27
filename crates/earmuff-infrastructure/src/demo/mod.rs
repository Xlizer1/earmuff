//! Simulated headsets: the tests run against them, and anyone can try Earmuff without one.

mod simulator;

use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use earmuff_application::{Backend, BackendError, Session, Waker};
use earmuff_domain::{BackendKind, DeviceInfo};
use earmuff_protocols::hyperx::CloudAlphaWireless;

use crate::hidraw::{Transport, Wakeup};
use crate::native::{NativeSession, device_info};

pub use simulator::CloudAlphaSimulator;

#[derive(Debug)]
struct State {
    simulator: CloudAlphaSimulator,
    plugged: bool,
    woken: bool,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

/// A simulated Cloud Alpha Wireless and its USB dongle. Clones control the same headset.
#[derive(Clone, Debug)]
pub struct DemoHeadset {
    shared: Arc<Shared>,
    instance: String,
}

impl DemoHeadset {
    pub fn new(instance: impl Into<String>) -> Self {
        DemoHeadset {
            shared: Arc::new(Shared {
                state: Mutex::new(State { simulator: CloudAlphaSimulator::default(), plugged: true, woken: false }),
                changed: Condvar::new(),
            }),
            instance: instance.into(),
        }
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        let result = change(&mut self.lock());
        self.shared.changed.notify_all();
        result
    }

    /// Does something to the simulated headset, e.g. press its mute button:
    /// `demo.with(|headset, now| headset.press_mute_button(now))`
    pub fn with<R>(&self, change: impl FnOnce(&mut CloudAlphaSimulator, Instant) -> R) -> R {
        self.update(|state| change(&mut state.simulator, Instant::now()))
    }

    /// Pulls the dongle out of the computer
    pub fn unplug(&self) {
        self.update(|state| state.plugged = false);
    }

    pub fn plug(&self) {
        self.update(|state| state.plugged = true);
    }

    pub fn is_plugged(&self) -> bool {
        self.lock().plugged
    }

    fn info(&self) -> DeviceInfo {
        let mut info = device_info(&CloudAlphaWireless, BackendKind::Demo, self.instance.clone());
        info.name.push_str(" (demo)");
        info
    }
}

fn unplugged() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "demo dongle unplugged")
}

/// Moves reports between a session and a simulated headset, with real timing
#[derive(Debug)]
struct SimTransport(DemoHeadset);

impl Transport for SimTransport {
    fn send(&mut self, report: &[u8]) -> io::Result<()> {
        self.0.update(|state| {
            if !state.plugged {
                return Err(unplugged());
            }
            state.simulator.receive(report, Instant::now());
            Ok(())
        })
    }

    fn receive(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut state = self.0.lock();
        if !state.plugged {
            return Err(unplugged());
        }
        Ok(state.simulator.take_due(Instant::now()))
    }

    fn wait(&mut self, timeout: Duration) -> io::Result<Wakeup> {
        let end = Instant::now() + timeout;
        let mut state = self.0.lock();
        loop {
            if !state.plugged {
                return Ok(Wakeup::Ready); // receive() reports the unplug
            }
            if state.woken {
                state.woken = false;
                return Ok(Wakeup::Woken);
            }
            let now = Instant::now();
            let due = state.simulator.next_due();
            if due.is_some_and(|due| due <= now) {
                return Ok(Wakeup::Ready);
            }
            if now >= end {
                return Ok(Wakeup::TimedOut);
            }
            let until = due.map_or(end, |due| due.min(end));
            state = self.0.shared.changed.wait_timeout(state, until - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    fn waker(&self) -> Waker {
        let headset = self.0.clone();
        Waker::new(move || headset.update(|state| state.woken = true))
    }
}

/// Offers simulated headsets as devices, through the same native driver real ones use
#[derive(Debug)]
pub struct DemoBackend {
    headsets: Vec<DemoHeadset>,
}

impl DemoBackend {
    /// One simulated Cloud Alpha Wireless, plugged in
    pub fn new() -> Self {
        DemoBackend { headsets: vec![DemoHeadset::new("demo-1")] }
    }

    /// Handles to control the simulated headsets
    pub fn headsets(&self) -> &[DemoHeadset] {
        &self.headsets
    }
}

impl Default for DemoBackend {
    fn default() -> Self {
        DemoBackend::new()
    }
}

impl Backend for DemoBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Demo
    }

    fn discover(&self) -> Result<Vec<DeviceInfo>, BackendError> {
        Ok(self.headsets.iter().filter(|h| h.is_plugged()).map(DemoHeadset::info).collect())
    }

    fn open(&self, device: &DeviceInfo) -> Result<Box<dyn Session>, BackendError> {
        let headset = self
            .headsets
            .iter()
            .find(|h| h.instance == device.id.instance && h.is_plugged())
            .ok_or(BackendError::NotFound)?;
        Ok(Box::new(NativeSession::new(device.clone(), &CloudAlphaWireless, SimTransport(headset.clone()))))
    }
}
