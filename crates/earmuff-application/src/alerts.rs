//! When to tell the user about a headset's battery.

use std::collections::HashMap;

use earmuff_domain::{DeviceId, Status};

use crate::manager::AppEvent;

/// Which battery alerts the user wants
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertSettings {
    /// Warn when the battery drops to this percentage, and again at half of it
    pub low_at: Option<u8>,
    pub when_charged: bool,
}

impl Default for AlertSettings {
    fn default() -> Self {
        AlertSettings { low_at: Some(20), when_charged: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Alert {
    Low {
        device: DeviceId,
        name: String,
        level: u8,
    },
    /// At half the low mark
    Critical {
        device: DeviceId,
        name: String,
        level: u8,
    },
    Charged {
        device: DeviceId,
        name: String,
    },
}

#[derive(Default)]
struct Tracked {
    name: String,
    status: Status,
    warned_low: bool,
    warned_critical: bool,
    told_charged: bool,
}

/// Turns battery news into alerts: each warning once per discharge, and "charged"
/// once per charge
#[derive(Default)]
pub struct BatteryAlerts {
    settings: AlertSettings,
    devices: HashMap<DeviceId, Tracked>,
}

impl BatteryAlerts {
    pub fn new(settings: AlertSettings) -> Self {
        BatteryAlerts { settings, devices: HashMap::new() }
    }

    pub fn set_settings(&mut self, settings: AlertSettings) {
        self.settings = settings;
    }

    /// Takes every device manager event; returns the alerts to show now
    pub fn on_event(&mut self, event: &AppEvent) -> Vec<Alert> {
        match event {
            AppEvent::Added(info) => {
                self.devices.insert(info.id.clone(), Tracked { name: info.name.clone(), ..Tracked::default() });
                Vec::new()
            }
            AppEvent::Removed(id) => {
                self.devices.remove(id);
                Vec::new()
            }
            AppEvent::Changed(id, change) => {
                let Some(tracked) = self.devices.get_mut(id) else { return Vec::new() };
                tracked.status.apply(change);
                check(&self.settings, id, tracked)
            }
            AppEvent::Failed(..) | AppEvent::Problem(_) => Vec::new(),
        }
    }
}

fn check(settings: &AlertSettings, id: &DeviceId, tracked: &mut Tracked) -> Vec<Alert> {
    let s = &tracked.status;
    let (Some(true), Some(level), Some(charge)) = (s.connected, s.battery.map(|b| b.get()), s.charge) else {
        return Vec::new();
    };
    let mut alerts = Vec::new();
    let (device, name) = (id.clone(), tracked.name.clone());
    if charge.is_plugged_in() {
        // plugged in: the next discharge warns again
        tracked.warned_low = false;
        tracked.warned_critical = false;
        let full = matches!(charge, earmuff_domain::ChargeState::Full) || level >= 100;
        if full && !tracked.told_charged && settings.when_charged {
            alerts.push(Alert::Charged { device, name });
        }
        tracked.told_charged |= full;
        return alerts;
    }
    tracked.told_charged = false;
    let Some(low_at) = settings.low_at else { return alerts };
    if level <= low_at / 2 && !tracked.warned_critical {
        tracked.warned_critical = true;
        tracked.warned_low = true;
        alerts.push(Alert::Critical { device, name, level });
    } else if level <= low_at && !tracked.warned_low {
        tracked.warned_low = true;
        alerts.push(Alert::Low { device, name, level });
    }
    alerts
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{BackendKind, ChargeState, DeviceInfo, Event, Percent, UsbId};

    fn device() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(1, 2), instance: "a".into() },
            name: "Headset".into(),
            vendor: "V".into(),
            capabilities: vec![],
            equalizer: None,
        }
    }

    fn alerts_for(alerts: &mut BatteryAlerts, events: &[Event]) -> Vec<Alert> {
        events.iter().flat_map(|event| alerts.on_event(&AppEvent::Changed(device().id, event.clone()))).collect()
    }

    fn started() -> BatteryAlerts {
        let mut alerts = BatteryAlerts::default();
        alerts.on_event(&AppEvent::Added(device()));
        alerts_for(&mut alerts, &[Event::Connected(true), Event::Charge(ChargeState::Discharging)]);
        alerts
    }

    fn battery(level: u8) -> Event {
        Event::Battery(Percent::saturating(level.into()))
    }

    fn low(level: u8) -> Alert {
        Alert::Low { device: device().id, name: "Headset".into(), level }
    }

    fn critical(level: u8) -> Alert {
        Alert::Critical { device: device().id, name: "Headset".into(), level }
    }

    #[test]
    fn warns_once_at_the_low_mark_and_once_at_half_of_it() {
        let mut alerts = started();
        assert_eq!(alerts_for(&mut alerts, &[battery(50), battery(21)]), []);
        assert_eq!(alerts_for(&mut alerts, &[battery(20)]), [low(20)]);
        assert_eq!(alerts_for(&mut alerts, &[battery(19), battery(15), battery(11)]), []);
        assert_eq!(alerts_for(&mut alerts, &[battery(10)]), [critical(10)]);
        assert_eq!(alerts_for(&mut alerts, &[battery(5)]), []);
    }

    #[test]
    fn a_battery_already_low_at_start_warns_once() {
        let mut alerts = started();
        assert_eq!(alerts_for(&mut alerts, &[battery(8)]), [critical(8)]);
        assert_eq!(alerts_for(&mut alerts, &[battery(7)]), []);
    }

    #[test]
    fn charging_resets_the_warnings_and_says_when_full() {
        let mut alerts = started();
        alerts_for(&mut alerts, &[battery(15)]);
        assert_eq!(alerts_for(&mut alerts, &[Event::Charge(ChargeState::Charging), battery(60)]), []);
        assert_eq!(
            alerts_for(&mut alerts, &[Event::Charge(ChargeState::Full)]),
            [Alert::Charged { device: device().id, name: "Headset".into() }]
        );
        assert_eq!(alerts_for(&mut alerts, &[battery(100)]), []);
        // unplugged, it drains again and warns again
        assert_eq!(alerts_for(&mut alerts, &[Event::Charge(ChargeState::Discharging), battery(20)]), [low(20)]);
    }

    #[test]
    fn a_full_battery_counts_as_charged_without_the_full_state() {
        let mut alerts = started();
        let events = [Event::Charge(ChargeState::Charging), battery(100)];
        assert_eq!(alerts_for(&mut alerts, &events).len(), 1);
    }

    #[test]
    fn alerts_can_be_turned_off() {
        let mut alerts = started();
        alerts.set_settings(AlertSettings { low_at: None, when_charged: false });
        assert_eq!(alerts_for(&mut alerts, &[battery(5)]), []);
        assert_eq!(alerts_for(&mut alerts, &[Event::Charge(ChargeState::Full)]), []);
    }

    #[test]
    fn nothing_while_the_headset_is_off_or_unknown() {
        let mut alerts = BatteryAlerts::default();
        alerts.on_event(&AppEvent::Added(device()));
        assert_eq!(alerts_for(&mut alerts, &[battery(5), Event::Charge(ChargeState::Discharging)]), []);
        assert_eq!(alerts_for(&mut alerts, &[Event::Connected(false)]), []);
        alerts.on_event(&AppEvent::Removed(device().id));
        assert_eq!(alerts_for(&mut alerts, &[Event::Connected(true)]), []);
    }
}
