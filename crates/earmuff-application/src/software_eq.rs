//! An equalizer that runs on the computer, for headsets without one of their own.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use earmuff_domain::{Capability, DeviceId, EqPreset, EqualizerSpec, Gain, UsbId};

use crate::manager::AppEvent;

/// The centres of the software equalizer's bands, in Hz
pub const SOFTWARE_EQ_FREQUENCIES: [u32; 10] = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

fn curve(db: [i16; 10]) -> Vec<Gain> {
    db.iter().map(|d| Gain::tenths(d * 10)).collect()
}

/// What the software equalizer can do: ±12 dB in half-dB steps on ten octave bands
pub fn software_equalizer() -> EqualizerSpec {
    EqualizerSpec {
        bands: SOFTWARE_EQ_FREQUENCIES.len(),
        min: Gain::tenths(-120),
        max: Gain::tenths(120),
        step: Gain::tenths(5),
        baseline: Gain::tenths(0),
        presets: vec![
            EqPreset { name: "Flat".into(), curve: curve([0; 10]) },
            EqPreset { name: "Bass boost".into(), curve: curve([6, 5, 4, 2, 0, 0, 0, 0, 0, 0]) },
            EqPreset { name: "Footsteps".into(), curve: curve([-4, -3, -2, -1, 0, 2, 4, 5, 3, 0]) },
            EqPreset { name: "Vocal".into(), curve: curve([-2, -2, -1, 1, 3, 4, 3, 1, 0, -1]) },
            EqPreset { name: "Treble boost".into(), curve: curve([0, 0, 0, 0, 0, 0, 2, 4, 6, 6]) },
        ],
    }
}

/// One headset's software equalizer
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoftwareEq {
    pub enabled: bool,
    pub curve: Vec<Gain>,
}

impl Default for SoftwareEq {
    fn default() -> Self {
        SoftwareEq { enabled: false, curve: software_equalizer().flat() }
    }
}

/// Runs equalizers on the computer's sound system
pub trait SoftwareEqualizer: Send + Sync {
    /// Puts an equalizer with this curve in front of the headset's speakers, or re-tunes
    /// the one already there. Ok(false) when the sound system doesn't know the headset's
    /// speakers (yet).
    fn apply(&self, headset: UsbId, name: &str, curve: &[Gain]) -> Result<bool, String>;

    /// Takes the headset's equalizer away
    fn stop(&self, headset: UsbId) -> Result<(), String>;
}

/// Keeps each headset's software equalizer between runs
pub trait SoftwareEqStore: Send + Sync {
    /// The saved equalizer, or the default (off and flat)
    fn load(&self, headset: UsbId) -> SoftwareEq;

    fn save(&self, headset: UsbId, eq: &SoftwareEq) -> Result<(), String>;
}

/// Something to do to the sound system
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EqAction {
    Apply { headset: UsbId, name: String, curve: Vec<Gain> },
    Stop(UsbId),
}

impl EqAction {
    fn headset(&self) -> UsbId {
        match self {
            EqAction::Apply { headset, .. } | EqAction::Stop(headset) => *headset,
        }
    }
}

struct Tracked {
    name: String,
    eq: SoftwareEq,
}

/// Says when each headset's equalizer should run: from when it's plugged in until it's
/// unplugged, while the user has it on. Headsets with an equalizer of their own don't get one.
pub struct SoftwareEqs {
    store: Arc<dyn SoftwareEqStore>,
    devices: HashMap<DeviceId, Tracked>,
}

impl SoftwareEqs {
    pub fn new(store: Arc<dyn SoftwareEqStore>) -> Self {
        SoftwareEqs { store, devices: HashMap::new() }
    }

    /// Takes every device manager event; returns what to do now
    pub fn on_event(&mut self, event: &AppEvent) -> Option<EqAction> {
        match event {
            AppEvent::Added(info) if !info.supports(Capability::Equalizer) => {
                let eq = self.store.load(info.id.usb);
                let action = eq.enabled.then(|| EqAction::Apply {
                    headset: info.id.usb,
                    name: info.name.clone(),
                    curve: eq.curve.clone(),
                });
                self.devices.insert(info.id.clone(), Tracked { name: info.name.clone(), eq });
                action
            }
            AppEvent::Removed(id) => {
                let tracked = self.devices.remove(id)?;
                tracked.eq.enabled.then_some(EqAction::Stop(id.usb))
            }
            _ => None,
        }
    }

    /// The headset's software equalizer, if it can have one
    pub fn get(&self, device: &DeviceId) -> Option<&SoftwareEq> {
        self.devices.get(device).map(|tracked| &tracked.eq)
    }

    /// The user changed the equalizer: saves it and says what to do
    pub fn change(&mut self, device: &DeviceId, eq: SoftwareEq) -> Result<Option<EqAction>, String> {
        let Some(tracked) = self.devices.get_mut(device) else { return Ok(None) };
        let action = match (tracked.eq.enabled, eq.enabled) {
            (_, true) => {
                Some(EqAction::Apply { headset: device.usb, name: tracked.name.clone(), curve: eq.curve.clone() })
            }
            (true, false) => Some(EqAction::Stop(device.usb)),
            (false, false) => None,
        };
        tracked.eq = eq;
        self.store.save(device.usb, &tracked.eq)?;
        Ok(action)
    }

    /// Every equalizer that's running, to take away when Earmuff quits
    pub fn running(&self) -> Vec<UsbId> {
        self.devices.iter().filter(|(_, t)| t.eq.enabled).map(|(id, _)| id.usb).collect()
    }
}

/// How often to look for a headset's speakers before giving up, and how long to wait between
const TRIES: u32 = 10;
const WAIT: Duration = Duration::from_millis(500);

enum Job {
    Run(EqAction),
    /// Runs everything sent before, then answers
    Flush(Sender<()>),
}

/// Applies equalizer changes one at a time, off the caller's thread. Of a burst of
/// changes (a drag on a slider), only the last one per headset is applied.
pub struct EqualizerWorker {
    jobs: Sender<Job>,
    thread: Option<JoinHandle<()>>,
}

impl EqualizerWorker {
    /// `failed` hears about every change that didn't work, on the worker's thread
    pub fn start(equalizer: Arc<dyn SoftwareEqualizer>, failed: impl Fn(String) + Send + 'static) -> Self {
        let (jobs, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || work(equalizer.as_ref(), &receiver, &failed));
        EqualizerWorker { jobs, thread: Some(thread) }
    }

    pub fn send(&self, action: EqAction) {
        let _ = self.jobs.send(Job::Run(action));
    }

    /// Waits until everything sent so far has been applied
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();
        if self.jobs.send(Job::Flush(done)).is_ok() {
            let _ = wait.recv();
        }
    }
}

impl Drop for EqualizerWorker {
    fn drop(&mut self) {
        // closing the channel ends the worker once it's done
        let (closed, _) = mpsc::channel();
        self.jobs = closed;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn work(equalizer: &dyn SoftwareEqualizer, jobs: &Receiver<Job>, failed: &dyn Fn(String)) {
    while let Ok(first) = jobs.recv() {
        let mut batch = vec![first];
        batch.extend(jobs.try_iter());
        let mut pending: Vec<EqAction> = Vec::new();
        for job in batch {
            match job {
                Job::Run(action) => {
                    // a newer change to the same headset replaces the older one
                    pending.retain(|older| older.headset() != action.headset());
                    pending.push(action);
                }
                Job::Flush(done) => {
                    run_all(equalizer, std::mem::take(&mut pending), failed);
                    let _ = done.send(());
                }
            }
        }
        run_all(equalizer, pending, failed);
    }
}

fn run_all(equalizer: &dyn SoftwareEqualizer, actions: Vec<EqAction>, failed: &dyn Fn(String)) {
    for action in actions {
        let result = match &action {
            EqAction::Apply { headset, name, curve } => apply_when_ready(equalizer, *headset, name, curve),
            EqAction::Stop(headset) => equalizer.stop(*headset),
        };
        if let Err(error) = result {
            failed(error);
        }
    }
}

/// A headset's speakers can show up a moment after the headset itself
fn apply_when_ready(
    equalizer: &dyn SoftwareEqualizer,
    headset: UsbId,
    name: &str,
    curve: &[Gain],
) -> Result<(), String> {
    for _ in 0..TRIES {
        if equalizer.apply(headset, name, curve)? {
            return Ok(());
        }
        std::thread::sleep(WAIT);
    }
    Err("the computer has no speakers for this headset".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{BackendKind, DeviceInfo};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryStore(Mutex<HashMap<UsbId, SoftwareEq>>);

    impl SoftwareEqStore for MemoryStore {
        fn load(&self, headset: UsbId) -> SoftwareEq {
            self.0.lock().unwrap().get(&headset).cloned().unwrap_or_default()
        }

        fn save(&self, headset: UsbId, eq: &SoftwareEq) -> Result<(), String> {
            self.0.lock().unwrap().insert(headset, eq.clone());
            Ok(())
        }
    }

    /// Records what it's asked to do. The first change takes a while, like starting PipeWire.
    #[derive(Default)]
    struct FakeEqualizer(Mutex<Vec<EqAction>>);

    impl SoftwareEqualizer for FakeEqualizer {
        fn apply(&self, headset: UsbId, name: &str, curve: &[Gain]) -> Result<bool, String> {
            if self.0.lock().unwrap().is_empty() {
                std::thread::sleep(Duration::from_millis(100));
            }
            self.0.lock().unwrap().push(EqAction::Apply { headset, name: name.into(), curve: curve.to_vec() });
            Ok(true)
        }

        fn stop(&self, headset: UsbId) -> Result<(), String> {
            self.0.lock().unwrap().push(EqAction::Stop(headset));
            if headset == UsbId::new(9, 9) { Err("no such equalizer".into()) } else { Ok(()) }
        }
    }

    fn device(capabilities: &[Capability]) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(1, 2), instance: "a".into() },
            name: "Headset".into(),
            vendor: "V".into(),
            capabilities: capabilities.to_vec(),
            equalizer: None,
        }
    }

    fn bass() -> Vec<Gain> {
        software_equalizer().presets[1].curve.clone()
    }

    #[test]
    fn the_presets_fit_the_equalizer() {
        let spec = software_equalizer();
        for preset in &spec.presets {
            assert_eq!(spec.check(&preset.curve), Ok(()), "{}", preset.name);
        }
    }

    #[test]
    fn turning_it_on_and_off() {
        let store = Arc::new(MemoryStore::default());
        let mut eqs = SoftwareEqs::new(store.clone());
        assert_eq!(eqs.on_event(&AppEvent::Added(device(&[]))), None);
        let id = device(&[]).id;
        assert_eq!(eqs.get(&id), Some(&SoftwareEq::default()));

        let on = SoftwareEq { enabled: true, curve: bass() };
        let apply = EqAction::Apply { headset: id.usb, name: "Headset".into(), curve: bass() };
        assert_eq!(eqs.change(&id, on.clone()), Ok(Some(apply.clone())));
        assert_eq!(store.load(id.usb), on);
        assert_eq!(eqs.running(), [id.usb]);
        // re-tuned while on
        let flat = SoftwareEq { enabled: true, curve: software_equalizer().flat() };
        assert!(matches!(eqs.change(&id, flat), Ok(Some(EqAction::Apply { .. }))));
        let off = SoftwareEq { enabled: false, curve: bass() };
        assert_eq!(eqs.change(&id, off.clone()), Ok(Some(EqAction::Stop(id.usb))));
        // changing the curve while off does nothing yet
        assert_eq!(eqs.change(&id, off), Ok(None));
        assert!(eqs.running().is_empty());
    }

    #[test]
    fn a_saved_equalizer_starts_with_the_headset_and_stops_when_unplugged() {
        let store = Arc::new(MemoryStore::default());
        store.save(UsbId::new(1, 2), &SoftwareEq { enabled: true, curve: bass() }).unwrap();
        let mut eqs = SoftwareEqs::new(store);
        let apply = EqAction::Apply { headset: UsbId::new(1, 2), name: "Headset".into(), curve: bass() };
        assert_eq!(eqs.on_event(&AppEvent::Added(device(&[]))), Some(apply));
        assert_eq!(eqs.on_event(&AppEvent::Removed(device(&[]).id)), Some(EqAction::Stop(UsbId::new(1, 2))));
    }

    #[test]
    fn headsets_with_their_own_equalizer_dont_get_one() {
        let mut eqs = SoftwareEqs::new(Arc::new(MemoryStore::default()));
        let info = device(&[Capability::Equalizer]);
        assert_eq!(eqs.on_event(&AppEvent::Added(info.clone())), None);
        assert_eq!(eqs.get(&info.id), None);
        assert_eq!(eqs.change(&info.id, SoftwareEq { enabled: true, curve: bass() }), Ok(None));
    }

    #[test]
    fn the_worker_applies_only_the_last_of_a_burst() {
        let equalizer = Arc::new(FakeEqualizer::default());
        let failures = Arc::new(Mutex::new(Vec::new()));
        let failed = failures.clone();
        let worker = EqualizerWorker::start(equalizer.clone(), move |error| failed.lock().unwrap().push(error));
        let apply =
            |db: i16| EqAction::Apply { headset: UsbId::new(1, 2), name: "H".into(), curve: vec![Gain::tenths(db)] };
        // the rest arrive while the first is still being applied
        for db in 0..20 {
            worker.send(apply(db));
        }
        worker.send(EqAction::Stop(UsbId::new(9, 9)));
        worker.flush();
        let done = equalizer.0.lock().unwrap().clone();
        assert_eq!(done.last(), Some(&EqAction::Stop(UsbId::new(9, 9))));
        assert!(done.contains(&apply(19)), "the last curve is applied");
        assert!(done.len() <= 3, "at most the first, the last and the stop: {done:?}");
        assert_eq!(*failures.lock().unwrap(), ["no such equalizer"]);
    }
}
