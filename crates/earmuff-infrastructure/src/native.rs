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
/// sent back to back.
pub struct NativeSession<T: Transport> {
    info: DeviceInfo,
    codec: &'static dyn Codec,
    transport: T,
    queue: VecDeque<Exchange>,
    /// The exchange waiting for an answer, and until when
    waiting: Option<(Exchange, Instant)>,
    raw: bool,
}

impl<T: Transport> NativeSession<T> {
    pub fn new(info: DeviceInfo, codec: &'static dyn Codec, transport: T) -> Self {
        NativeSession { info, codec, transport, queue: VecDeque::new(), waiting: None, raw: false }
    }

    /// Sends the next report once the last one was answered or timed out
    fn pump(&mut self) -> Result<(), SessionError> {
        if let Some((_, deadline)) = &self.waiting {
            if Instant::now() < *deadline {
                return Ok(());
            }
            self.waiting = None;
        }
        while let Some(exchange) = self.queue.pop_front() {
            self.transport.send(&exchange.report).map_err(disconnected)?;
            if exchange.reply_prefix.is_some() {
                self.waiting = Some((exchange, Instant::now() + self.codec.reply_timeout()));
                break;
            }
        }
        Ok(())
    }

    /// Handles every report waiting on the transport
    fn receive_all(&mut self, events: &mut Vec<Event>) -> Result<(), SessionError> {
        while let Some(report) = self.transport.receive().map_err(disconnected)? {
            if self.waiting.as_ref().is_some_and(|(exchange, _)| exchange.answered_by(&report)) {
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
        let unsupported = SessionError::Unsupported(setting.capability());
        if !self.info.supports(setting.capability()) {
            return Err(unsupported);
        }
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
            let until = self.waiting.as_ref().map_or(end, |(_, deadline)| (*deadline).min(end));
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
    fn moves_on_after_the_reply_timeout() {
        let transport = Scripted::default(); // never answers
        let mut session = session(transport.clone());
        session.query(Query::Battery).unwrap();
        let start = Instant::now();
        while transport.sent_commands().len() < 2 {
            session.poll(Duration::from_millis(50)).unwrap();
            assert!(start.elapsed() < Duration::from_secs(2), "stuck on an unanswered report");
        }
        assert!(start.elapsed() >= CloudAlphaWireless.reply_timeout());
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
