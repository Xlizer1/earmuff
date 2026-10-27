//! The device manager running a simulated headset through the real backend and session.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use earmuff_application::{AppEvent, Backend, DeviceManager, ManagerOptions};
use earmuff_domain::{AutoOff, BackendKind, DeviceId, Event, Percent, Setting};
use earmuff_infrastructure::demo::{DemoBackend, DemoHeadset};
use earmuff_infrastructure::native::NativeBackend;

const WAIT: Duration = Duration::from_secs(3);

struct Running {
    manager: DeviceManager,
    events: Receiver<AppEvent>,
    headset: DemoHeadset,
    id: DeviceId,
}

fn start() -> Running {
    let demo = DemoBackend::new();
    let headset = demo.headsets()[0].clone();
    // a native backend with no hidraw devices goes first, like in the real app
    let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(NativeBackend::with_sysfs("/nonexistent")), Arc::new(demo)];
    let (tx, events) = mpsc::channel();
    let tx = Mutex::new(tx);
    let options = ManagerOptions { rescan: Duration::from_millis(20), battery_refresh: Duration::from_secs(60) };
    let manager = DeviceManager::start(backends, options, move |event| {
        let _ = tx.lock().unwrap().send(event);
    });
    let mut running = Running { manager, events, headset, id: placeholder_id() };
    let AppEvent::Added(info) = running.expect(|e| matches!(e, AppEvent::Added(_))) else { unreachable!() };
    assert_eq!(info.id.backend, BackendKind::Demo);
    running.id = info.id;
    running
}

fn placeholder_id() -> DeviceId {
    DeviceId { backend: BackendKind::Demo, usb: earmuff_domain::UsbId::new(0, 0), instance: String::new() }
}

impl Running {
    fn expect(&self, want: impl Fn(&AppEvent) -> bool) -> AppEvent {
        let end = Instant::now() + WAIT;
        loop {
            match self.events.recv_timeout(end.saturating_duration_since(Instant::now())) {
                Ok(event) if want(&event) => return event,
                Ok(_) => {}
                Err(_) => panic!("expected event did not arrive"),
            }
        }
    }

    fn expect_change(&self, event: Event) {
        let id = self.id.clone();
        self.expect(move |e| *e == AppEvent::Changed(id.clone(), event.clone()));
    }
}

#[test]
fn reports_the_headset_state_after_adding_it() {
    let running = start();
    running.expect_change(Event::Connected(true));
    running.expect_change(Event::Color("Black/Red".into()));
}

#[test]
fn applies_settings_and_reports_what_the_headset_kept() {
    let running = start();
    running.expect_change(Event::Color("Black/Red".into()));
    running.manager.apply(&running.id, Setting::SidetoneLevel(Percent::FULL));
    running.expect_change(Event::SidetoneLevel(Percent::FULL));
    running.manager.apply(&running.id, Setting::AutoOff(AutoOff::minutes(60)));
    running.expect_change(Event::AutoOff(AutoOff::minutes(60)));
    assert_eq!(running.headset.with(|sim, _| (sim.sidetone_level, sim.auto_off)), (18, 60));
}

#[test]
fn passes_on_button_presses_instantly() {
    let running = start();
    running.expect_change(Event::Color("Black/Red".into()));
    let pressed = Instant::now();
    running.headset.with(|sim, now| sim.press_mute_button(now));
    running.expect_change(Event::MicMuted(true));
    assert!(pressed.elapsed() < Duration::from_millis(500), "took {:?}", pressed.elapsed());
}

#[test]
fn follows_the_headset_powering_off_and_on() {
    let running = start();
    running.expect_change(Event::Color("Black/Red".into()));
    running.headset.with(|sim, now| sim.set_powered(false, now));
    running.expect_change(Event::Connected(false));
    // while it was off, someone changed a setting on another computer
    running.headset.with(|sim, _| sim.auto_off = 90);
    running.headset.with(|sim, now| sim.set_powered(true, now));
    running.expect_change(Event::Connected(true));
    running.expect_change(Event::AutoOff(AutoOff::minutes(90)));
}

#[test]
fn removes_and_re_adds_an_unplugged_dongle() {
    let running = start();
    running.expect_change(Event::Color("Black/Red".into()));
    running.headset.unplug();
    let id = running.id.clone();
    running.expect(move |e| *e == AppEvent::Removed(id.clone()));
    running.headset.plug();
    let id = running.id.clone();
    running.expect(move |e| matches!(e, AppEvent::Added(info) if info.id == id));
    running.expect_change(Event::Connected(true));
}
