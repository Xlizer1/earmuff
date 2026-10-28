//! Battery history per headset, and how long the battery will last.

use std::collections::HashMap;
use std::sync::Arc;

use earmuff_domain::{DeviceId, Status, UsbId};

use crate::manager::AppEvent;

/// One battery reading
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Unix time in seconds
    pub at: u64,
    pub level: u8,
    pub charging: bool,
}

/// Keeps battery readings per headset model
pub trait HistoryStore: Send + Sync {
    /// Readings at or after `since`, oldest first
    fn load(&self, headset: UsbId, since: u64) -> Vec<Sample>;

    fn append(&self, headset: UsbId, sample: Sample) -> Result<(), String>;
}

/// A reading at most this often, unless something changes
pub const SAMPLE_EVERY: u64 = 5 * 60;
/// How far back the history goes
pub const KEEP_FOR: u64 = 7 * 24 * 60 * 60;
/// How far back the time-left estimate looks
const ESTIMATE_OVER: u64 = 6 * 60 * 60;

/// How many minutes the battery has left, from the current discharge: a least-squares
/// line through the readings since it was last charging, within the last six hours.
/// None until there's enough to go on (15 minutes and a 2% drop).
pub fn minutes_left(samples: &[Sample], now: u64) -> Option<u32> {
    let last = samples.last()?;
    if last.charging {
        return None;
    }
    let run: Vec<&Sample> = samples
        .iter()
        .rev()
        .take_while(|s| !s.charging && s.at + ESTIMATE_OVER >= now)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let (first, last) = (run.first()?, run.last()?);
    if last.at.saturating_sub(first.at) < 15 * 60 || first.level.saturating_sub(last.level) < 2 {
        return None;
    }
    let n = run.len() as f64;
    let mean_t = run.iter().map(|s| s.at as f64).sum::<f64>() / n;
    let mean_l = run.iter().map(|s| f64::from(s.level)).sum::<f64>() / n;
    let covariance: f64 = run.iter().map(|s| (s.at as f64 - mean_t) * (f64::from(s.level) - mean_l)).sum();
    let variance: f64 = run.iter().map(|s| (s.at as f64 - mean_t).powi(2)).sum();
    let per_second = covariance / variance; // percent per second, negative while draining
    if !per_second.is_finite() || per_second >= 0.0 {
        return None;
    }
    let minutes = f64::from(last.level) / -per_second / 60.0;
    Some(minutes.min(100.0 * 60.0) as u32)
}

/// "About 9 h 20 min left", "About 45 min left"; past ten hours, minutes don't matter:
/// "About 59 h left"
pub fn time_left_text(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("About {m} min left"),
        (h, _) if h >= 10 => format!("About {h} h left"),
        (h, 0) => format!("About {h} h left"),
        (h, m) => format!("About {h} h {m} min left"),
    }
}

struct Tracked {
    usb: UsbId,
    status: Status,
    samples: Vec<Sample>,
}

/// Records battery readings and keeps the recent ones at hand for graphs and estimates
pub struct BatteryHistory {
    store: Arc<dyn HistoryStore>,
    devices: HashMap<DeviceId, Tracked>,
}

impl BatteryHistory {
    pub fn new(store: Arc<dyn HistoryStore>) -> Self {
        BatteryHistory { store, devices: HashMap::new() }
    }

    /// Takes every device manager event and the time in Unix seconds; returns true
    /// when a reading was added
    pub fn on_event(&mut self, event: &AppEvent, now: u64) -> bool {
        match event {
            AppEvent::Added(info) => {
                let samples = self.store.load(info.id.usb, now.saturating_sub(KEEP_FOR));
                self.devices.insert(info.id.clone(), Tracked { usb: info.id.usb, status: Status::default(), samples });
                false
            }
            AppEvent::Removed(id) => {
                self.devices.remove(id);
                false
            }
            AppEvent::Changed(id, change) => {
                let Some(tracked) = self.devices.get_mut(id) else { return false };
                tracked.status.apply(change);
                let s = &tracked.status;
                let (Some(true), Some(level), Some(charge)) = (s.connected, s.battery, s.charge) else {
                    return false;
                };
                let sample = Sample { at: now, level: level.get(), charging: charge.is_plugged_in() };
                let due = match tracked.samples.last() {
                    None => true,
                    Some(last) => now >= last.at + SAMPLE_EVERY || last.charging != sample.charging || last.at > now,
                };
                if !due {
                    return false;
                }
                tracked.samples.push(sample);
                tracked.samples.retain(|s| s.at + KEEP_FOR >= now);
                // a lost reading only leaves a gap in the graph
                let _ = self.store.append(tracked.usb, sample);
                true
            }
            AppEvent::Failed(..) | AppEvent::Problem(_) => false,
        }
    }

    pub fn samples(&self, device: &DeviceId) -> &[Sample] {
        self.devices.get(device).map_or(&[], |tracked| &tracked.samples)
    }

    pub fn minutes_left(&self, device: &DeviceId, now: u64) -> Option<u32> {
        minutes_left(self.samples(device), now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{BackendKind, ChargeState, DeviceInfo, Event, Percent};
    use std::sync::Mutex;

    const HOUR: u64 = 3600;

    #[derive(Default)]
    struct MemoryStore(Mutex<Vec<(UsbId, Sample)>>);

    impl HistoryStore for MemoryStore {
        fn load(&self, headset: UsbId, since: u64) -> Vec<Sample> {
            self.0.lock().unwrap().iter().filter(|(h, s)| *h == headset && s.at >= since).map(|(_, s)| *s).collect()
        }

        fn append(&self, headset: UsbId, sample: Sample) -> Result<(), String> {
            self.0.lock().unwrap().push((headset, sample));
            Ok(())
        }
    }

    fn draining(points: &[(u64, u8)]) -> Vec<Sample> {
        points.iter().map(|&(at, level)| Sample { at, level, charging: false }).collect()
    }

    fn device() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(1, 2), instance: "a".into() },
            name: "Headset".into(),
            vendor: "V".into(),
            capabilities: vec![],
            equalizer: None,
        }
    }

    fn event(event: Event) -> AppEvent {
        AppEvent::Changed(device().id, event)
    }

    #[test]
    fn estimates_from_a_steady_drain() {
        // 10% per hour, now at 60%: six hours left
        let samples = draining(&[(0, 80), (HOUR, 70), (2 * HOUR, 60)]);
        assert_eq!(minutes_left(&samples, 2 * HOUR), Some(360));
    }

    #[test]
    fn needs_enough_to_go_on() {
        assert_eq!(minutes_left(&draining(&[(0, 80)]), 0), None);
        assert_eq!(minutes_left(&draining(&[(0, 80), (10 * 60, 79)]), 10 * 60), None); // too short
        assert_eq!(minutes_left(&draining(&[(0, 80), (HOUR, 80)]), HOUR), None); // no drop
        let charging = [Sample { at: 0, level: 50, charging: true }];
        assert_eq!(minutes_left(&charging, 0), None);
    }

    #[test]
    fn only_counts_the_current_discharge() {
        let mut samples = draining(&[(0, 90), (HOUR, 40)]); // a fast drain before charging
        samples.push(Sample { at: 2 * HOUR, level: 100, charging: true });
        samples.extend(draining(&[(3 * HOUR, 100), (4 * HOUR, 95), (5 * HOUR, 90)])); // 5% an hour since
        assert_eq!(minutes_left(&samples, 5 * HOUR), Some(18 * 60));
    }

    #[test]
    fn time_left_reads_naturally() {
        assert_eq!(time_left_text(45), "About 45 min left");
        assert_eq!(time_left_text(120), "About 2 h left");
        assert_eq!(time_left_text(560), "About 9 h 20 min left");
        assert_eq!(time_left_text(3575), "About 59 h left");
    }

    #[test]
    fn records_a_reading_every_five_minutes_and_when_charging_changes() {
        let store = Arc::new(MemoryStore::default());
        let mut history = BatteryHistory::new(store.clone());
        history.on_event(&AppEvent::Added(device()), 0);
        history.on_event(&event(Event::Connected(true)), 0);
        history.on_event(&event(Event::Charge(ChargeState::Discharging)), 0);
        assert!(history.on_event(&event(Event::Battery(Percent::saturating(80))), 0));
        assert!(!history.on_event(&event(Event::Battery(Percent::saturating(79))), 60));
        assert!(history.on_event(&event(Event::Battery(Percent::saturating(78))), SAMPLE_EVERY));
        assert!(history.on_event(&event(Event::Charge(ChargeState::Charging)), SAMPLE_EVERY + 1));
        assert_eq!(history.samples(&device().id).len(), 3);
        assert_eq!(store.load(UsbId::new(1, 2), 0).len(), 3);
    }

    #[test]
    fn picks_up_the_saved_history() {
        let store = Arc::new(MemoryStore::default());
        for sample in draining(&[(0, 80), (HOUR, 70), (2 * HOUR, 60)]) {
            store.append(UsbId::new(1, 2), sample).unwrap();
        }
        let mut history = BatteryHistory::new(store);
        history.on_event(&AppEvent::Added(device()), 2 * HOUR);
        assert_eq!(history.samples(&device().id).len(), 3);
        assert_eq!(history.minutes_left(&device().id, 2 * HOUR), Some(360));
    }

    #[test]
    fn nothing_while_the_headset_is_off() {
        let store = Arc::new(MemoryStore::default());
        let mut history = BatteryHistory::new(store);
        history.on_event(&AppEvent::Added(device()), 0);
        history.on_event(&event(Event::Charge(ChargeState::Discharging)), 0);
        assert!(!history.on_event(&event(Event::Battery(Percent::saturating(80))), 0));
    }
}
