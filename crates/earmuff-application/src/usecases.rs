//! One-shot use cases for scripts and the command line.

use std::sync::Arc;
use std::time::{Duration, Instant};

use earmuff_domain::{Query, Setting, Status};

use crate::ports::{Backend, BackendError, Session, SessionError};

/// Opens the first headset any backend finds, in the backends' order.
///
/// When nothing opens, the error explains the most useful problem: a headset that
/// is there but can't be opened beats "nothing found".
pub fn open_first(backends: &[Arc<dyn Backend>]) -> Result<Box<dyn Session>, BackendError> {
    let mut problem = BackendError::NotFound;
    for backend in backends {
        let devices = match backend.discover() {
            Ok(devices) => devices,
            Err(error) => {
                problem = error;
                continue;
            }
        };
        for device in devices {
            match backend.open(&device) {
                Ok(session) => return Ok(session),
                Err(error) => problem = error,
            }
        }
    }
    Err(problem)
}

/// Polls until every queued command is answered, or `timeout` passes,
/// recording what the headset reports in `status`
pub fn settle(session: &mut dyn Session, status: &mut Status, timeout: Duration) -> Result<(), SessionError> {
    let end = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if session.is_idle() || now >= end {
            return Ok(());
        }
        for event in session.poll(end - now)? {
            status.apply(&event);
        }
    }
}

/// Everything the headset reports. Asks whether it's on first; when it's off, the
/// other answers would mean nothing, so the rest is only asked of a headset that's on.
pub fn read_status(session: &mut dyn Session, timeout: Duration) -> Result<Status, SessionError> {
    let mut status = Status::default();
    session.query(Query::Connection)?;
    settle(session, &mut status, timeout)?;
    if status.is_connected() {
        session.query(Query::All)?;
        settle(session, &mut status, timeout)?;
    }
    Ok(status)
}

/// Applies settings in order and waits until the headset has confirmed them all.
/// Fails before sending anything if the headset doesn't support one of them.
pub fn apply_settings(
    session: &mut dyn Session,
    settings: &[Setting],
    timeout: Duration,
) -> Result<Status, SessionError> {
    if let Some(unsupported) = settings.iter().find(|s| !session.info().supports(s.capability())) {
        return Err(SessionError::Unsupported(unsupported.capability()));
    }
    for setting in settings {
        session.apply(setting.clone())?;
    }
    let mut status = Status::default();
    settle(session, &mut status, timeout)?;
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeBackend, FakeSession};
    use earmuff_domain::{Capability, Event, Percent};

    const TIMEOUT: Duration = Duration::from_secs(1);

    #[test]
    fn read_status_skips_the_rest_when_the_headset_is_off() {
        let mut session = FakeSession::new("a").answering(Query::Connection, vec![Event::Connected(false)]);
        let status = read_status(&mut session, TIMEOUT).unwrap();
        assert_eq!(status.connected, Some(false));
        assert_eq!(session.queries(), [Query::Connection]);
    }

    #[test]
    fn read_status_asks_for_everything_when_on() {
        let mut session = FakeSession::new("a")
            .answering(Query::Connection, vec![Event::Connected(true)])
            .answering(Query::All, vec![Event::Connected(true), Event::Battery(Percent::saturating(70))]);
        let status = read_status(&mut session, TIMEOUT).unwrap();
        assert_eq!(status.battery, Some(Percent::saturating(70)));
        assert_eq!(session.queries(), [Query::Connection, Query::All]);
    }

    #[test]
    fn apply_settings_sends_in_order_and_collects_confirmations() {
        let mut session = FakeSession::new("a");
        let status =
            apply_settings(&mut session, &[Setting::Sidetone(true), Setting::MicMuted(true)], TIMEOUT).unwrap();
        assert_eq!(session.applied(), [Setting::Sidetone(true), Setting::MicMuted(true)]);
        assert_eq!(status.sidetone, Some(true));
        assert_eq!(status.mic_muted, Some(true));
    }

    #[test]
    fn apply_settings_refuses_unsupported_settings_before_sending() {
        let mut session = FakeSession::new("a").with_capabilities(&[Capability::MicMute]);
        let result = apply_settings(&mut session, &[Setting::MicMuted(true), Setting::Sidetone(true)], TIMEOUT);
        assert_eq!(result, Err(SessionError::Unsupported(Capability::Sidetone)));
        assert!(session.applied().is_empty());
    }

    #[test]
    fn settle_gives_up_after_the_timeout() {
        let mut session = FakeSession::new("a").never_idle();
        let mut status = Status::default();
        let start = Instant::now();
        settle(&mut session, &mut status, Duration::from_millis(50)).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn open_first_opens_the_first_device_found() {
        let empty: Arc<dyn Backend> = Arc::new(FakeBackend::new(vec![]));
        let full: Arc<dyn Backend> = Arc::new(FakeBackend::new(vec![FakeSession::new("x"), FakeSession::new("y")]));
        let session = open_first(&[empty, full]).unwrap();
        assert_eq!(session.info().id.instance, "x");
    }

    #[test]
    fn open_first_reports_the_most_useful_problem() {
        let denied = BackendError::PermissionDenied { path: "/dev/hidraw3".into(), hint: "udev".into() };
        let backend: Arc<dyn Backend> =
            Arc::new(FakeBackend::new(vec![FakeSession::new("x")]).failing_open(denied.clone()));
        assert_eq!(open_first(&[backend]).err(), Some(denied));
        assert_eq!(open_first(&[]).err(), Some(BackendError::NotFound));
    }
}
