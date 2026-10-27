//! Fakes for the ports, used by this crate's unit tests.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use earmuff_domain::{BackendKind, Capability, DeviceId, DeviceInfo, Event, Query, Setting, UsbId};

use crate::ports::{Backend, BackendError, Session, SessionError, Waker};

const ALL_CAPABILITIES: [Capability; 7] = [
    Capability::Battery,
    Capability::MicMute,
    Capability::MicAttached,
    Capability::Sidetone,
    Capability::SidetoneLevel,
    Capability::AutoOff,
    Capability::VoicePrompts,
];

#[derive(Default)]
struct State {
    outbox: VecDeque<Event>,
    woken: bool,
    unplugged: bool,
    queries: Vec<Query>,
    applied: Vec<Setting>,
}

struct Shared {
    info: Mutex<DeviceInfo>,
    state: Mutex<State>,
    changed: Condvar,
}

/// Controls a fake headset from the test, even after its session moved to another thread
#[derive(Clone)]
pub(crate) struct FakeHandle(Arc<Shared>);

impl FakeHandle {
    fn update(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.0.state.lock().unwrap());
        self.0.changed.notify_all();
    }

    pub fn info(&self) -> DeviceInfo {
        self.0.info.lock().unwrap().clone()
    }

    /// The headset reports something on its own
    pub fn emit(&self, event: Event) {
        self.update(|s| s.outbox.push_back(event));
    }

    pub fn unplug(&self) {
        self.update(|s| s.unplugged = true);
    }

    pub fn plug(&self) {
        self.update(|s| s.unplugged = false);
    }

    pub fn queries(&self) -> Vec<Query> {
        self.0.state.lock().unwrap().queries.clone()
    }

    pub fn applied(&self) -> Vec<Setting> {
        self.0.state.lock().unwrap().applied.clone()
    }
}

/// A headset that answers queries with scripted events and confirms every setting
#[derive(Clone)]
pub(crate) struct FakeSession {
    info: DeviceInfo,
    answers: HashMap<Query, Vec<Event>>,
    never_idle: bool,
    handle: FakeHandle,
}

impl FakeSession {
    pub fn new(instance: &str) -> Self {
        let info = DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(0x1234, 0x0001), instance: instance.into() },
            name: format!("Fake {instance}"),
            vendor: "Fake".into(),
            capabilities: ALL_CAPABILITIES.to_vec(),
        };
        FakeSession {
            info: info.clone(),
            answers: HashMap::new(),
            never_idle: false,
            handle: FakeHandle(Arc::new(Shared {
                info: Mutex::new(info),
                state: Mutex::default(),
                changed: Condvar::new(),
            })),
        }
    }

    fn edit_info(mut self, edit: impl FnOnce(&mut DeviceInfo)) -> Self {
        edit(&mut self.info);
        *self.handle.0.info.lock().unwrap() = self.info.clone();
        self
    }

    pub fn answering(mut self, query: Query, events: Vec<Event>) -> Self {
        self.answers.insert(query, events);
        self
    }

    pub fn with_capabilities(self, capabilities: &[Capability]) -> Self {
        self.edit_info(|info| info.capabilities = capabilities.to_vec())
    }

    pub fn with_usb(self, usb: UsbId) -> Self {
        self.edit_info(|info| info.id.usb = usb)
    }

    pub fn with_backend(self, backend: BackendKind) -> Self {
        self.edit_info(|info| info.id.backend = backend)
    }

    /// Always has something pending, so `settle` can only end by timing out
    pub fn never_idle(mut self) -> Self {
        self.never_idle = true;
        self
    }

    pub fn handle(&self) -> FakeHandle {
        self.handle.clone()
    }

    pub fn queries(&self) -> Vec<Query> {
        self.handle.queries()
    }

    pub fn applied(&self) -> Vec<Setting> {
        self.handle.applied()
    }

    fn confirmation(setting: Setting) -> Event {
        match setting {
            Setting::MicMuted(v) => Event::MicMuted(v),
            Setting::Sidetone(v) => Event::Sidetone(v),
            Setting::SidetoneLevel(v) => Event::SidetoneLevel(v),
            Setting::AutoOff(v) => Event::AutoOff(v),
            Setting::VoicePrompts(v) => Event::VoicePrompts(v),
        }
    }
}

impl Session for FakeSession {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn query(&mut self, query: Query) -> Result<(), SessionError> {
        let answers = self.answers.get(&query).cloned().unwrap_or_default();
        self.handle.update(|s| {
            s.queries.push(query);
            s.outbox.extend(answers);
        });
        Ok(())
    }

    fn apply(&mut self, setting: Setting) -> Result<(), SessionError> {
        if !self.info.supports(setting.capability()) {
            return Err(SessionError::Unsupported(setting.capability()));
        }
        self.handle.update(|s| {
            s.applied.push(setting);
            s.outbox.push_back(FakeSession::confirmation(setting));
        });
        Ok(())
    }

    fn poll(&mut self, timeout: Duration) -> Result<Vec<Event>, SessionError> {
        let end = Instant::now() + timeout;
        let mut state = self.handle.0.state.lock().unwrap();
        loop {
            if state.unplugged {
                return Err(SessionError::Disconnected("unplugged".into()));
            }
            if !state.outbox.is_empty() {
                return Ok(state.outbox.drain(..).collect());
            }
            if state.woken {
                state.woken = false;
                return Ok(vec![]);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(vec![]);
            }
            state = self.handle.0.changed.wait_timeout(state, left).unwrap().0;
        }
    }

    fn is_idle(&self) -> bool {
        !self.never_idle && self.handle.0.state.lock().unwrap().outbox.is_empty()
    }

    fn waker(&self) -> Waker {
        let handle = self.handle.clone();
        Waker::new(move || handle.update(|s| s.woken = true))
    }

    fn set_raw(&mut self, _raw: bool) {}
}

/// Hands out fake sessions; unplugged ones aren't discovered
pub(crate) struct FakeBackend {
    sessions: Vec<FakeSession>,
    open_error: Mutex<Option<BackendError>>,
}

impl FakeBackend {
    pub fn new(sessions: Vec<FakeSession>) -> Self {
        FakeBackend { sessions, open_error: Mutex::new(None) }
    }

    pub fn failing_open(self, error: BackendError) -> Self {
        *self.open_error.lock().unwrap() = Some(error);
        self
    }

    pub fn allow_open(&self) {
        *self.open_error.lock().unwrap() = None;
    }
}

impl Backend for FakeBackend {
    fn kind(&self) -> BackendKind {
        self.sessions.first().map_or(BackendKind::Demo, |s| s.info.id.backend)
    }

    fn discover(&self) -> Result<Vec<DeviceInfo>, BackendError> {
        Ok(self
            .sessions
            .iter()
            .filter(|s| !s.handle.0.state.lock().unwrap().unplugged)
            .map(|s| s.info.clone())
            .collect())
    }

    fn open(&self, device: &DeviceInfo) -> Result<Box<dyn Session>, BackendError> {
        if let Some(error) = self.open_error.lock().unwrap().clone() {
            return Err(error);
        }
        let session = self.sessions.iter().find(|s| s.info.id == device.id).ok_or(BackendError::NotFound)?;
        Ok(Box::new(session.clone()))
    }
}
