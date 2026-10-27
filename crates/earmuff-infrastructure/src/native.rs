//! The backend for headsets Earmuff drives itself, through a codec from earmuff-protocols.

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use earmuff_application::{Backend, BackendError, Session, SessionError, Waker};
use earmuff_domain::{BackendKind, DeviceId, DeviceInfo, Event, Query, Setting};
use earmuff_protocols::{Codec, Exchange};

use crate::hidraw::{self, HidrawTransport, Transport, Wakeup};

/// Finds natively supported headsets among the hidraw devices
#[derive(Debug)]
pub struct NativeBackend {
    sysfs: PathBuf,
}

impl NativeBackend {
    pub fn new() -> Self {
        NativeBackend::with_sysfs(hidraw::SYSFS_HIDRAW)
    }

    /// Looks for hidraw devices under another directory (for tests)
    pub fn with_sysfs(sysfs: impl Into<PathBuf>) -> Self {
        NativeBackend { sysfs: sysfs.into() }
    }
}

impl Default for NativeBackend {
    fn default() -> Self {
        NativeBackend::new()
    }
}

/// What Earmuff knows about a device a codec drives
pub fn device_info(codec: &dyn Codec, backend: BackendKind, instance: String) -> DeviceInfo {
    let model = codec.model();
    DeviceInfo {
        id: DeviceId { backend, usb: model.ids[0], instance },
        name: model.name.into(),
        vendor: model.vendor.into(),
        capabilities: model.capabilities.to_vec(),
        equalizer: None,
    }
}

impl Backend for NativeBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Native
    }

    fn discover(&self) -> Result<Vec<DeviceInfo>, BackendError> {
        Ok(hidraw::scan(&self.sysfs)
            .into_iter()
            .filter_map(|node| {
                let codec = earmuff_protocols::find(node.usb)?;
                let mut info = device_info(codec, BackendKind::Native, node.path.display().to_string());
                info.id.usb = node.usb;
                Some(info)
            })
            .collect())
    }

    fn open(&self, device: &DeviceInfo) -> Result<Box<dyn Session>, BackendError> {
        let codec = earmuff_protocols::find(device.id.usb).ok_or(BackendError::NotFound)?;
        let path = Path::new(&device.id.instance);
        let transport = HidrawTransport::open(path).map_err(|e| match e.kind() {
            io::ErrorKind::PermissionDenied => BackendError::PermissionDenied {
                path: device.id.instance.clone(),
                hint: "install Earmuff's udev rule, or HeadsetControl's".into(),
            },
            io::ErrorKind::NotFound => BackendError::NotFound,
            _ => BackendError::Failed(format!("{}: {e}", device.id.instance)),
        })?;
        Ok(Box::new(NativeSession::new(device.clone(), codec, transport)))
    }
}

fn disconnected(error: io::Error) -> SessionError {
    SessionError::Disconnected(error.to_string())
}

/// Talks to one headset through its codec.
///
/// Reports go out one at a time: the next one waits until the headset answered the
/// last one, or the codec's reply timeout passed. The HyperX dongle drops commands
/// sent back to back, and drops ours when another program talks to it at the same
/// moment, so an unanswered report is sent again, up to [`SEND_ATTEMPTS`] times.
pub struct NativeSession<T: Transport> {
    info: DeviceInfo,
    codec: &'static dyn Codec,
    transport: T,
    queue: VecDeque<Exchange>,
    waiting: Option<Waiting>,
    raw: bool,
}

/// How many times a report is sent before giving up on an answer
pub const SEND_ATTEMPTS: u8 = 3;

/// The exchange waiting for an answer
struct Waiting {
    exchange: Exchange,
    deadline: Instant,
    attempts: u8,
}

impl<T: Transport> NativeSession<T> {
    pub fn new(info: DeviceInfo, codec: &'static dyn Codec, transport: T) -> Self {
        NativeSession { info, codec, transport, queue: VecDeque::new(), waiting: None, raw: false }
    }

    /// Sends the next report once the last one was answered, or sends an unanswered
    /// one again; gives up on it after SEND_ATTEMPTS
    fn pump(&mut self) -> Result<(), SessionError> {
        if let Some(waiting) = &mut self.waiting {
            if Instant::now() < waiting.deadline {
                return Ok(());
            }
            if waiting.attempts < SEND_ATTEMPTS {
                self.transport.send(&waiting.exchange.report).map_err(disconnected)?;
                waiting.attempts += 1;
                waiting.deadline = Instant::now() + self.codec.reply_timeout();
                return Ok(());
            }
            self.waiting = None;
        }
        while let Some(exchange) = self.queue.pop_front() {
            self.transport.send(&exchange.report).map_err(disconnected)?;
            if exchange.reply_prefix.is_some() {
                let deadline = Instant::now() + self.codec.reply_timeout();
                self.waiting = Some(Waiting { exchange, deadline, attempts: 1 });
                break;
            }
        }
        Ok(())
    }

    /// Handles every report waiting on the transport
    fn receive_all(&mut self, events: &mut Vec<Event>) -> Result<(), SessionError> {
        while let Some(report) = self.transport.receive().map_err(disconnected)? {
            if self.waiting.as_ref().is_some_and(|waiting| waiting.exchange.answered_by(&report)) {
                self.waiting = None;
            }
            if let Some(event) = self.codec.decode(&report) {
                events.push(event);
            }
            if self.raw {
                events.push(Event::Raw(report));
            }
        }
        Ok(())
    }
}

impl<T: Transport> Session for NativeSession<T> {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn query(&mut self, query: Query) -> Result<(), SessionError> {
        self.queue.extend(self.codec.query(query));
        self.pump()
    }

    fn apply(&mut self, setting: Setting) -> Result<(), SessionError> {
        self.info.check(&setting)?;
        let unsupported = SessionError::Unsupported(setting.capability());
        self.queue.extend(self.codec.apply(setting).ok_or(unsupported)?);
        self.pump()
    }

    fn poll(&mut self, timeout: Duration) -> Result<Vec<Event>, SessionError> {
        let end = Instant::now() + timeout;
        let mut events = Vec::new();
        loop {
            self.pump()?;
            self.receive_all(&mut events)?;
            self.pump()?;
            let now = Instant::now();
            if !events.is_empty() || now >= end {
                return Ok(events);
            }
            let until = self.waiting.as_ref().map_or(end, |waiting| waiting.deadline.min(end));
            match self.transport.wait(until.saturating_duration_since(now)).map_err(disconnected)? {
                Wakeup::Woken => return Ok(events),
                Wakeup::Ready | Wakeup::TimedOut => {}
            }
        }
    }

    fn is_idle(&self) -> bool {
        self.queue.is_empty() && self.waiting.is_none()
    }

    fn waker(&self) -> Waker {
        self.transport.waker()
    }

    fn set_raw(&mut self, raw: bool) {
        self.raw = raw;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{Percent, UsbId};
    use earmuff_protocols::hyperx::CloudAlphaWireless;
    use earmuff_protocols::hyperx::cloud_alpha_wireless::{cmd, packet};
    use std::sync::{Arc, Mutex};

    /// A command byte and the reply the "headset" sends right away
    type Answer = (u8, Vec<u8>);

    /// Records what is sent, and answers each report from a script
    #[derive(Clone, Default)]
    struct Scripted {
        sent: Arc<Mutex<Vec<Vec<u8>>>>,
        inbox: Arc<Mutex<VecDeque<Vec<u8>>>>,
        answers: Arc<Mutex<Vec<Answer>>>,
        /// How many of the next reports go unanswered, like on a busy dongle
        drops: Arc<Mutex<usize>>,
    }

    impl Scripted {
        fn answer(&self, command: u8, reply: Vec<u8>) {
            self.answers.lock().unwrap().push((command, reply));
        }

        fn sent_commands(&self) -> Vec<u8> {
            self.sent.lock().unwrap().iter().map(|r| r[2]).collect()
        }
    }

    impl Transport for Scripted {
        fn send(&mut self, report: &[u8]) -> io::Result<()> {
            self.sent.lock().unwrap().push(report.to_vec());
            let mut drops = self.drops.lock().unwrap();
            if *drops > 0 {
                *drops -= 1;
                return Ok(());
            }
            let answers = self.answers.lock().unwrap();
            if let Some((_, reply)) = answers.iter().find(|(command, _)| *command == report[2]) {
                self.inbox.lock().unwrap().push_back(reply.clone());
            }
            Ok(())
        }

        fn receive(&mut self) -> io::Result<Option<Vec<u8>>> {
            Ok(self.inbox.lock().unwrap().pop_front())
        }

        fn wait(&mut self, timeout: Duration) -> io::Result<Wakeup> {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            Ok(if self.inbox.lock().unwrap().is_empty() { Wakeup::TimedOut } else { Wakeup::Ready })
        }

        fn waker(&self) -> Waker {
            Waker::new(|| {})
        }
    }

    fn session(transport: Scripted) -> NativeSession<Scripted> {
        let mut info = device_info(&CloudAlphaWireless, BackendKind::Demo, "test".into());
        info.id.usb = UsbId::new(0x03F0, 0x098D);
        NativeSession::new(info, &CloudAlphaWireless, transport)
    }

    #[test]
    fn sends_one_report_at_a_time() {
        let transport = Scripted::default();
        let mut session = session(transport.clone());
        session.query(Query::Battery).unwrap();
        // the second report waits for an answer to the first
        assert_eq!(transport.sent_commands(), [cmd::GET_BATTERY]);
        assert!(!session.is_idle());
    }

    #[test]
    fn sends_the_next_report_once_answered() {
        let transport = Scripted::default();
        transport.answer(cmd::GET_BATTERY, packet(cmd::GET_BATTERY, &[80]));
        transport.answer(cmd::GET_CHARGING, packet(cmd::GET_CHARGING, &[1]));
        let mut session = session(transport.clone());
        session.query(Query::Battery).unwrap();
        let mut events = session.poll(Duration::from_millis(50)).unwrap();
        events.extend(session.poll(Duration::from_millis(50)).unwrap());
        assert_eq!(transport.sent_commands(), [cmd::GET_BATTERY, cmd::GET_CHARGING]);
        assert!(events.contains(&Event::Battery(Percent::saturating(80))));
        assert!(session.is_idle());
    }

    #[test]
    fn sends_an_unanswered_report_again_then_moves_on() {
        let transport = Scripted::default(); // never answers
        let mut session = session(transport.clone());
        session.query(Query::Battery).unwrap();
        let start = Instant::now();
        while transport.sent_commands().len() < 4 {
            session.poll(Duration::from_millis(50)).unwrap();
            assert!(start.elapsed() < Duration::from_secs(5), "stuck on an unanswered report");
        }
        let battery = cmd::GET_BATTERY;
        assert_eq!(transport.sent_commands(), [battery, battery, battery, cmd::GET_CHARGING]);
        assert!(start.elapsed() >= CloudAlphaWireless.reply_timeout() * u32::from(SEND_ATTEMPTS));
    }

    #[test]
    fn a_report_the_dongle_dropped_still_gets_answered() {
        let transport = Scripted::default();
        transport.answer(cmd::GET_BATTERY, packet(cmd::GET_BATTERY, &[80]));
        transport.answer(cmd::GET_CHARGING, packet(cmd::GET_CHARGING, &[0]));
        *transport.drops.lock().unwrap() = 1;
        let mut session = session(transport.clone());
        session.query(Query::Battery).unwrap();
        let mut events = Vec::new();
        let start = Instant::now();
        while !session.is_idle() {
            events.extend(session.poll(Duration::from_millis(50)).unwrap());
            assert!(start.elapsed() < Duration::from_secs(5), "never answered");
        }
        assert!(events.contains(&Event::Battery(Percent::saturating(80))));
        assert_eq!(transport.sent_commands(), [cmd::GET_BATTERY, cmd::GET_BATTERY, cmd::GET_CHARGING]);
    }

    #[test]
    fn refuses_unsupported_settings() {
        let mut session = session(Scripted::default());
        session.info.capabilities.clear();
        assert!(matches!(session.apply(Setting::MicMuted(true)), Err(SessionError::Unsupported(_))));
    }

    #[test]
    fn raw_mode_also_returns_every_report() {
        let transport = Scripted::default();
        let reply = packet(cmd::GET_WIRELESS, &[2]);
        transport.answer(cmd::GET_WIRELESS, reply.clone());
        let mut session = session(transport);
        session.set_raw(true);
        session.query(Query::Connection).unwrap();
        let events = session.poll(Duration::from_millis(50)).unwrap();
        assert_eq!(events, [Event::Connected(true), Event::Raw(reply)]);
    }

    #[test]
    fn discovers_nothing_without_hidraw_devices() {
        let backend = NativeBackend::with_sysfs("/nonexistent/hidraw");
        assert_eq!(backend.discover(), Ok(vec![]));
    }
}
