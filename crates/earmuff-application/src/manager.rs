//! Keeps track of connected headsets and talks to each one on its own thread.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use earmuff_domain::{BackendKind, DeviceId, DeviceInfo, Event, Query, Setting, Status, UsbId};

use crate::ports::{Backend, BackendError, Session, SessionError, Waker};

/// What the device manager reports. Events about one headset arrive in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEvent {
    /// A headset was found and opened
    Added(DeviceInfo),
    /// A headset reported a change
    Changed(DeviceId, Event),
    /// A headset went away: unplugged, or it stopped answering
    Removed(DeviceId),
    /// Why a headset that is there can't be used (e.g. no permission); `None` once resolved
    Problem(Option<String>),
}

#[derive(Clone, Copy, Debug)]
pub struct ManagerOptions {
    /// How often to look for newly plugged-in headsets
    pub rescan: Duration,
    /// How often to ask for the battery level, which headsets don't always report on their own
    pub battery_refresh: Duration,
}

impl Default for ManagerOptions {
    fn default() -> Self {
        ManagerOptions { rescan: Duration::from_secs(2), battery_refresh: Duration::from_secs(60) }
    }
}

type Sink = Box<dyn Fn(AppEvent) + Send + Sync>;

enum Command {
    Apply(Setting),
    Query(Query),
    Stop,
}

struct Worker {
    commands: Sender<Command>,
    waker: Waker,
    usb: UsbId,
    backend: BackendKind,
}

struct Shared {
    backends: Vec<Arc<dyn Backend>>,
    options: ManagerOptions,
    sink: Sink,
    workers: Mutex<HashMap<DeviceId, Worker>>,
    stopped: Mutex<bool>,
    wake_scanner: Condvar,
}

impl Shared {
    fn workers(&self) -> std::sync::MutexGuard<'_, HashMap<DeviceId, Worker>> {
        self.workers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_stopped(&self) -> bool {
        *self.stopped.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Finds headsets through every backend, opens each one, and runs it on its own thread
/// so callers (like the GTK main loop) never block on a headset.
///
/// Backends are listed in priority order: when two reach the same headset
/// (same USB ID), the earlier one keeps it.
pub struct DeviceManager {
    shared: Arc<Shared>,
    scanner: Option<thread::JoinHandle<()>>,
}

impl DeviceManager {
    /// Starts looking for headsets right away. `sink` receives every [`AppEvent`],
    /// from the manager's own threads.
    pub fn start(
        backends: Vec<Arc<dyn Backend>>,
        options: ManagerOptions,
        sink: impl Fn(AppEvent) + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            backends,
            options,
            sink: Box::new(sink),
            workers: Mutex::new(HashMap::new()),
            stopped: Mutex::new(false),
            wake_scanner: Condvar::new(),
        });
        let scanner = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("earmuff-scan".into())
                .spawn(move || scan_loop(&shared))
                .expect("failed to start the scanner thread")
        };
        DeviceManager { shared, scanner: Some(scanner) }
    }

    pub fn apply(&self, device: &DeviceId, setting: Setting) {
        self.send(device, Command::Apply(setting));
    }

    pub fn query(&self, device: &DeviceId, query: Query) {
        self.send(device, Command::Query(query));
    }

    /// Looks for headsets now instead of at the next rescan
    pub fn rescan(&self) {
        self.shared.wake_scanner.notify_all();
    }

    /// Stops every thread; no events are sent afterwards
    pub fn stop(&mut self) {
        *self.shared.stopped.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.shared.wake_scanner.notify_all();
        for worker in self.shared.workers().values() {
            let _ = worker.commands.send(Command::Stop);
            worker.waker.wake();
        }
        if let Some(scanner) = self.scanner.take() {
            let _ = scanner.join();
        }
    }

    fn send(&self, device: &DeviceId, command: Command) {
        if let Some(worker) = self.shared.workers().get(device)
            && worker.commands.send(command).is_ok()
        {
            worker.waker.wake();
        }
    }
}

impl Drop for DeviceManager {
    fn drop(&mut self) {
        self.stop();
    }
}

fn scan_loop(shared: &Arc<Shared>) {
    let mut last_problem = None;
    loop {
        if shared.is_stopped() {
            return;
        }
        let problem = scan_once(shared);
        if problem != last_problem {
            (shared.sink)(AppEvent::Problem(problem.clone()));
            last_problem = problem;
        }
        let stopped = shared.stopped.lock().unwrap_or_else(PoisonError::into_inner);
        if *stopped {
            return;
        }
        drop(shared.wake_scanner.wait_timeout(stopped, shared.options.rescan));
    }
}

/// Opens every headset that isn't open yet; returns why one couldn't be, if any
fn scan_once(shared: &Arc<Shared>) -> Option<String> {
    let mut problem = None;
    for backend in &shared.backends {
        let devices = match backend.discover() {
            Ok(devices) => devices,
            Err(BackendError::NotFound) => continue,
            Err(error) => {
                problem = Some(error.to_string());
                continue;
            }
        };
        for info in devices {
            let taken = shared.workers().iter().any(|(id, worker)| {
                // already open, or the same headset reached through an earlier backend
                *id == info.id || (worker.usb == info.id.usb && worker.backend != info.id.backend)
            });
            if taken {
                continue;
            }
            match backend.open(&info) {
                Ok(session) => start_worker(shared, info, session),
                Err(error) => problem = Some(error.to_string()),
            }
        }
    }
    problem
}

fn start_worker(shared: &Arc<Shared>, info: DeviceInfo, session: Box<dyn Session>) {
    let (commands, receiver) = mpsc::channel();
    let worker = Worker { commands, waker: session.waker(), usb: info.id.usb, backend: info.id.backend };
    shared.workers().insert(info.id.clone(), worker);
    (shared.sink)(AppEvent::Added(info.clone()));

    let shared = shared.clone();
    let id = info.id;
    thread::Builder::new()
        .name(format!("earmuff-{}", id.usb))
        .spawn(move || {
            let stopped = run_session(&shared, session, &receiver).is_ok();
            shared.workers().remove(&id);
            if !stopped && !shared.is_stopped() {
                (shared.sink)(AppEvent::Removed(id));
            }
        })
        .expect("failed to start a headset thread");
}

/// Talks to one headset until told to stop (Ok) or the headset goes away (Err)
fn run_session(
    shared: &Shared,
    mut session: Box<dyn Session>,
    commands: &Receiver<Command>,
) -> Result<(), SessionError> {
    let id = session.info().id.clone();
    let battery_refresh = shared.options.battery_refresh;
    let mut status = Status::default();
    let mut next_battery = Instant::now() + battery_refresh;
    session.query(Query::All)?;
    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Apply(setting)) => match session.apply(setting) {
                    Err(SessionError::Unsupported(_)) => {} // the UI only offers supported settings
                    result => result?,
                },
                Ok(Command::Query(query)) => session.query(query)?,
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                Err(TryRecvError::Empty) => break,
            }
        }
        let now = Instant::now();
        if now >= next_battery {
            session.query(Query::Battery)?;
            next_battery = now + battery_refresh;
        }
        for event in session.poll(next_battery.saturating_duration_since(now))? {
            let was_off = status.connected == Some(false);
            if !status.apply(&event) {
                continue;
            }
            if was_off && status.is_connected() {
                // settings may have changed while the headset was off
                session.query(Query::All)?;
            }
            (shared.sink)(AppEvent::Changed(id.clone(), event));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeBackend, FakeSession};
    use earmuff_domain::{Capability, Percent};
    use std::sync::mpsc::Receiver;

    const WAIT: Duration = Duration::from_secs(2);

    fn fast() -> ManagerOptions {
        ManagerOptions { rescan: Duration::from_millis(20), battery_refresh: Duration::from_secs(60) }
    }

    fn start(backends: Vec<Arc<dyn Backend>>, options: ManagerOptions) -> (DeviceManager, Receiver<AppEvent>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let manager = DeviceManager::start(backends, options, move |event| {
            let _ = tx.lock().unwrap().send(event);
        });
        (manager, rx)
    }

    /// Waits for the first event matching `want`, failing after a timeout
    fn expect(rx: &Receiver<AppEvent>, want: impl Fn(&AppEvent) -> bool) -> AppEvent {
        let end = Instant::now() + WAIT;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(event) if want(&event) => return event,
                Ok(_) => {}
                Err(_) => panic!("expected event did not arrive"),
            }
        }
    }

    #[test]
    fn opens_found_headsets_and_reports_their_state() {
        let session = FakeSession::new("a").answering(Query::All, vec![Event::Connected(true), Event::MicMuted(false)]);
        let (_manager, rx) = start(vec![Arc::new(FakeBackend::new(vec![session]))], fast());
        let AppEvent::Added(info) = expect(&rx, |e| matches!(e, AppEvent::Added(_))) else { unreachable!() };
        assert_eq!(info.id.instance, "a");
        expect(&rx, |e| *e == AppEvent::Changed(info.id.clone(), Event::Connected(true)));
        expect(&rx, |e| *e == AppEvent::Changed(info.id.clone(), Event::MicMuted(false)));
    }

    #[test]
    fn forwards_only_changes() {
        let session = FakeSession::new("a").answering(Query::All, vec![Event::MicMuted(false)]);
        let handle = session.handle();
        let (manager, rx) = start(vec![Arc::new(FakeBackend::new(vec![session]))], fast());
        let id = handle.info().id;
        expect(&rx, |e| *e == AppEvent::Changed(id.clone(), Event::MicMuted(false)));
        handle.emit(Event::MicMuted(false)); // no change: not forwarded
        handle.emit(Event::MicMuted(true));
        assert_eq!(expect(&rx, |e| matches!(e, AppEvent::Changed(..))), AppEvent::Changed(id, Event::MicMuted(true)));
        drop(manager);
    }

    #[test]
    fn applies_settings_on_the_headset_thread() {
        let session = FakeSession::new("a");
        let handle = session.handle();
        let (manager, rx) = start(vec![Arc::new(FakeBackend::new(vec![session]))], fast());
        let id = handle.info().id;
        expect(&rx, |e| matches!(e, AppEvent::Added(_)));
        manager.apply(&id, Setting::SidetoneLevel(Percent::saturating(40)));
        expect(&rx, |e| *e == AppEvent::Changed(id.clone(), Event::SidetoneLevel(Percent::saturating(40))));
        assert_eq!(handle.applied(), [Setting::SidetoneLevel(Percent::saturating(40))]);
    }

    #[test]
    fn unsupported_settings_are_ignored_without_dropping_the_headset() {
        let session = FakeSession::new("a").with_capabilities(&[Capability::MicMute]);
        let handle = session.handle();
        let (manager, rx) = start(vec![Arc::new(FakeBackend::new(vec![session]))], fast());
        let id = handle.info().id;
        expect(&rx, |e| matches!(e, AppEvent::Added(_)));
        manager.apply(&id, Setting::Sidetone(true));
        manager.apply(&id, Setting::MicMuted(true));
        expect(&rx, |e| *e == AppEvent::Changed(id.clone(), Event::MicMuted(true)));
    }

    #[test]
    fn removes_unplugged_headsets_and_finds_them_again() {
        let session = FakeSession::new("a");
        let handle = session.handle();
        let (_manager, rx) = start(vec![Arc::new(FakeBackend::new(vec![session]))], fast());
        let id = handle.info().id;
        expect(&rx, |e| matches!(e, AppEvent::Added(_)));
        handle.unplug();
        expect(&rx, |e| *e == AppEvent::Removed(id.clone()));
        handle.plug();
        expect(&rx, |e| matches!(e, AppEvent::Added(info) if info.id == id));
    }

    #[test]
    fn asks_for_everything_again_when_the_headset_comes_back_on() {
        let session = FakeSession::new("a");
        let handle = session.handle();
        let (_manager, rx) = start(vec![Arc::new(FakeBackend::new(vec![session]))], fast());
        let id = handle.info().id;
        handle.emit(Event::Connected(false));
        expect(&rx, |e| *e == AppEvent::Changed(id.clone(), Event::Connected(false)));
        handle.emit(Event::Connected(true));
        expect(&rx, |e| *e == AppEvent::Changed(id.clone(), Event::Connected(true)));
        let end = Instant::now() + WAIT;
        while handle.queries().iter().filter(|q| **q == Query::All).count() < 2 {
            assert!(Instant::now() < end, "no second Query::All");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn refreshes_the_battery_periodically() {
        let session = FakeSession::new("a");
        let handle = session.handle();
        let options = ManagerOptions { battery_refresh: Duration::from_millis(30), ..fast() };
        let (_manager, _rx) = start(vec![Arc::new(FakeBackend::new(vec![session]))], options);
        let end = Instant::now() + WAIT;
        while handle.queries().iter().filter(|q| **q == Query::Battery).count() < 2 {
            assert!(Instant::now() < end, "battery not refreshed");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn reports_a_problem_once_and_its_resolution() {
        let denied = BackendError::PermissionDenied { path: "/dev/hidraw3".into(), hint: "udev".into() };
        let backend = Arc::new(FakeBackend::new(vec![FakeSession::new("a")]).failing_open(denied.clone()));
        let (_manager, rx) = start(vec![backend.clone()], fast());
        expect(&rx, |e| *e == AppEvent::Problem(Some(denied.to_string())));
        thread::sleep(Duration::from_millis(100)); // several rescans
        assert!(rx.try_iter().all(|e| !matches!(e, AppEvent::Problem(_))), "problem reported twice");
        backend.allow_open();
        // the headset opens during the scan; the scan then reports that nothing is wrong anymore
        expect(&rx, |e| matches!(e, AppEvent::Added(_)));
        expect(&rx, |e| *e == AppEvent::Problem(None));
    }

    #[test]
    fn earlier_backends_win_for_the_same_headset() {
        let usb = UsbId::new(0x03F0, 0x098D);
        let native = FakeSession::new("native").with_usb(usb).with_backend(BackendKind::Native);
        let demo = FakeSession::new("demo").with_usb(usb).with_backend(BackendKind::Demo);
        let (_manager, rx) =
            start(vec![Arc::new(FakeBackend::new(vec![native])), Arc::new(FakeBackend::new(vec![demo]))], fast());
        let AppEvent::Added(info) = expect(&rx, |e| matches!(e, AppEvent::Added(_))) else { unreachable!() };
        assert_eq!(info.id.instance, "native");
        thread::sleep(Duration::from_millis(100));
        assert!(rx.try_iter().all(|e| !matches!(e, AppEvent::Added(_))), "opened twice");
    }

    #[test]
    fn two_identical_headsets_on_one_backend_both_open() {
        let usb = UsbId::new(0x03F0, 0x098D);
        let sessions = vec![FakeSession::new("one").with_usb(usb), FakeSession::new("two").with_usb(usb)];
        let (_manager, rx) = start(vec![Arc::new(FakeBackend::new(sessions))], fast());
        expect(&rx, |e| matches!(e, AppEvent::Added(_)));
        expect(&rx, |e| matches!(e, AppEvent::Added(_)));
    }

    #[test]
    fn stopping_sends_nothing_more() {
        let session = FakeSession::new("a");
        let handle = session.handle();
        let (mut manager, rx) = start(vec![Arc::new(FakeBackend::new(vec![session]))], fast());
        let id = handle.info().id;
        expect(&rx, |e| matches!(e, AppEvent::Added(_)));
        manager.stop();
        thread::sleep(Duration::from_millis(50));
        let _ = rx.try_iter().count();
        handle.emit(Event::MicMuted(true));
        manager.apply(&id, Setting::MicMuted(true));
        thread::sleep(Duration::from_millis(50));
        assert_eq!(rx.try_iter().count(), 0);
    }
}
