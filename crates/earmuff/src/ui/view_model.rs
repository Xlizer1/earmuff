//! What the window shows, worked out from the device manager's events.
//! Plain Rust with no GTK, so all of the window's logic is unit-tested here.

use std::collections::HashMap;

use earmuff_application::{AppEvent, SoftwareEq, software_equalizer};
use earmuff_domain::{
    AutoOff, BackendKind, Capability, ChargeState, DeviceId, DeviceInfo, EqualizerSpec, Gain, NoiseFilter, Percent,
    Setting, Status,
};

pub const LOOKING: &str = "Looking for a supported headset…";
pub const WAITING: &str = "Waiting for the headset…";
pub const AUTO_OFF_MINUTES: [u8; 6] = [0, 10, 20, 30, 60, 90];
const MUTE_LED_LEVELS: [&str; 4] = ["Off", "Dim", "Medium", "Bright"];

/// The low-battery warning levels offered in Preferences; 0 is off
pub const BATTERY_ALERTS: [u8; 5] = [0, 10, 15, 20, 30];

pub fn battery_alert_label(level: u8) -> String {
    if level == 0 { "Off".into() } else { format!("At {level}%") }
}

/// Where a warning level sits in the Preferences list (other levels show as the nearest)
pub fn battery_alert_index(level: u8) -> u32 {
    BATTERY_ALERTS.iter().enumerate().min_by_key(|(_, l)| l.abs_diff(level)).map_or(0, |(i, _)| i as u32)
}

/// What a battery alert says: title and body
pub fn alert_text(alert: &earmuff_application::Alert) -> (String, String) {
    use earmuff_application::Alert;
    match alert {
        Alert::Low { name, level, .. } => ("Headset battery low".into(), format!("{name}: {level}% left")),
        Alert::Critical { name, level, .. } => {
            ("Headset battery almost empty".into(), format!("{name}: {level}% left. Charge it soon."))
        }
        Alert::Charged { name, .. } => ("Headset fully charged".into(), format!("{name} is fully charged.")),
    }
}

/// What to tell users whose desktop doesn't run autostart entries
pub fn autostart_hint(desktop: &str) -> Option<&'static str> {
    let desktop = desktop.to_ascii_lowercase();
    if desktop.contains("hyprland") {
        Some("Hyprland doesn't run autostart entries: add \"exec-once = earmuff --hidden\" to hyprland.conf.")
    } else if desktop.contains("sway") {
        Some("Sway doesn't run autostart entries: add \"exec earmuff --hidden\" to your Sway config.")
    } else {
        None
    }
}

const NOISE_FILTERS: [&str; 3] = ["Off", "Low", "High"];

/// The headsets the device manager reported, which one the window shows, and
/// what's stopping one from working
#[derive(Debug, Default)]
pub struct Model {
    devices: Vec<(DeviceInfo, Status)>,
    selected: Option<DeviceId>,
    problem: Option<String>,
    failure: Option<String>,
    /// The software equalizer of each headset that can have one
    software_eq: HashMap<DeviceId, SoftwareEq>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    /// No headset to show; the description says why
    Missing {
        description: String,
    },
    /// The dongle is there, the headset is off
    Off,
    Settings,
}

/// Something the user can see or change on the settings page
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Control {
    Mute,
    MicAttached,
    MicVolume,
    NoiseFilter,
    RotateToMute,
    MuteLed,
    Sidetone,
    SidetoneLevel,
    EqualizerPreset,
    Equalizer,
    SoftwareEq,
    SoftwareEqPreset,
    SoftwareEqCurve,
    ChatMix,
    VolumeLimiter,
    Lights,
    AutoOff,
    VoicePrompts,
    BtWhenPoweredOn,
    BtCallVolume,
    NotificationSound,
    MicTest,
    Model,
    UsbId,
    Connection,
}

impl Control {
    /// The widget's name, which the end-to-end tests look for
    pub fn name(self) -> &'static str {
        match self {
            Control::Mute => "mute",
            Control::MicAttached => "mic",
            Control::MicVolume => "mic-volume",
            Control::NoiseFilter => "noise-filter",
            Control::RotateToMute => "rotate-to-mute",
            Control::MuteLed => "mute-led",
            Control::Sidetone => "sidetone",
            Control::SidetoneLevel => "level",
            Control::EqualizerPreset => "eq-preset",
            Control::Equalizer => "equalizer",
            Control::SoftwareEq => "software-eq",
            Control::SoftwareEqPreset => "software-eq-preset",
            Control::SoftwareEqCurve => "software-eq-curve",
            Control::ChatMix => "chatmix",
            Control::VolumeLimiter => "volume-limiter",
            Control::Lights => "lights",
            Control::AutoOff => "auto-off",
            Control::VoicePrompts => "voice-prompts",
            Control::BtWhenPoweredOn => "bluetooth-on-power",
            Control::BtCallVolume => "bluetooth-call-volume",
            Control::NotificationSound => "notification-sound",
            Control::MicTest => "mic-test",
            Control::Model => "model",
            Control::UsbId => "usb-id",
            Control::Connection => "connection",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Widget {
    Switch(bool),
    /// A slider from 0 to 100%
    Level(Option<u8>),
    Choice {
        options: Vec<String>,
        selected: Option<u32>,
    },
    /// Read-only text
    Info(String),
    /// A read-only game/chat balance, 0 (game) to 128 (chat)
    Balance(Option<u8>),
    /// A button that does something once
    Button(&'static str),
    /// An equalizer curve to shape band by band
    Curve(CurveView),
    /// Records a few seconds from the mic and plays them back, with a level meter
    MicTest,
    /// One of a few named choices, shown side by side, like equalizer presets
    Chips {
        options: Vec<String>,
        selected: Option<u32>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurveView {
    /// The headset's curve, or the flat one while that isn't known
    pub gains: Vec<Gain>,
    pub min: Gain,
    pub max: Gain,
    pub step: Gain,
    pub baseline: Gain,
    /// What each band is called, low to high
    pub labels: Vec<String>,
}

/// Band names: the usual octave centres for a 10-band equalizer, numbers otherwise
/// (headsets don't say which frequencies their bands are at)
pub fn band_labels(bands: usize) -> Vec<String> {
    const OCTAVES: [&str; 10] = ["32", "64", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
    if bands == OCTAVES.len() {
        OCTAVES.map(String::from).to_vec()
    } else {
        (1..=bands).map(|band| band.to_string()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlView {
    pub control: Control,
    pub title: &'static str,
    pub subtitle: Option<&'static str>,
    pub widget: Widget,
    pub sensitive: bool,
}

/// The dashboard's panels, in reading order
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    Battery,
    Microphone,
    Monitoring,
    Sound,
    Power,
    Equalizer,
    History,
    Lights,
    Device,
}

impl Panel {
    pub const ORDER: [Panel; 9] = [
        Panel::Battery,
        Panel::Microphone,
        Panel::Monitoring,
        Panel::Sound,
        Panel::Power,
        Panel::Equalizer,
        Panel::History,
        Panel::Lights,
        Panel::Device,
    ];

    /// The panel's widget name, which the end-to-end tests look for
    pub fn name(self) -> &'static str {
        match self {
            Panel::Battery => "panel-battery",
            Panel::Microphone => "panel-microphone",
            Panel::Monitoring => "panel-monitoring",
            Panel::Sound => "panel-sound",
            Panel::Power => "panel-power",
            Panel::Equalizer => "panel-equalizer",
            Panel::History => "panel-history",
            Panel::Lights => "panel-lights",
            Panel::Device => "panel-device",
        }
    }
}

/// The dashboard's grid is this many columns wide
pub const GRID_COLUMNS: i32 = 6;

/// A panel's cell in the dashboard's grid
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub panel: Panel,
    pub column: i32,
    pub row: i32,
    pub width: i32,
    pub height: i32,
}

/// Where the panels go. Narrow: one column, in reading order. Wide: a top band, the
/// equalizer across, then the battery history beside a stack of the small panels. A tall
/// microphone panel spans two rows between Battery/Mic monitoring and Sound/Power; a short
/// one sits in a row of three. Panels that share a row share its height, and the placement
/// depends only on which panels exist, so nothing moves when a setting changes.
pub fn placements(panels: &[Panel], tall_microphone: bool, narrow: bool) -> Vec<Placement> {
    let present: Vec<Panel> = Panel::ORDER.into_iter().filter(|panel| panels.contains(panel)).collect();
    let has = |panel: Panel| present.contains(&panel);
    let pick = |wanted: &[Panel]| -> Vec<Panel> { wanted.iter().copied().filter(|p| has(*p)).collect() };
    let mut out = Vec::new();
    if narrow {
        for (row, panel) in present.iter().enumerate() {
            out.push(Placement { panel: *panel, column: 0, row: row as i32, width: GRID_COLUMNS, height: 1 });
        }
        return out;
    }
    let mut row = 0;
    let mut overflow = Vec::new();
    if tall_microphone && has(Panel::Microphone) {
        let columns: Vec<Vec<Panel>> =
            [pick(&[Panel::Battery, Panel::Monitoring]), vec![Panel::Microphone], pick(&[Panel::Sound, Panel::Power])]
                .into_iter()
                .filter(|column| !column.is_empty())
                .collect();
        let rows = columns.iter().map(Vec::len).max().unwrap_or(1) as i32;
        let width = GRID_COLUMNS / columns.len() as i32;
        for (index, column) in columns.iter().enumerate() {
            stack(&mut out, column, index as i32 * width, width, row, rows);
        }
        row += rows;
    } else {
        let top = pick(&[Panel::Battery, Panel::Microphone, Panel::Monitoring, Panel::Sound, Panel::Power]);
        let (first, rest) = top.split_at(top.len().min(3));
        row += in_a_row(&mut out, first, row);
        overflow.extend_from_slice(rest);
    }
    if has(Panel::Equalizer) {
        out.push(Placement { panel: Panel::Equalizer, column: 0, row, width: GRID_COLUMNS, height: 1 });
        row += 1;
    }
    overflow.extend(pick(&[Panel::Lights, Panel::Device]));
    match (has(Panel::History), overflow.is_empty()) {
        (true, true) => out.push(Placement { panel: Panel::History, column: 0, row, width: GRID_COLUMNS, height: 1 }),
        (true, false) => {
            let side = GRID_COLUMNS / 3;
            let height = overflow.len() as i32;
            out.push(Placement { panel: Panel::History, column: 0, row, width: GRID_COLUMNS - side, height });
            stack(&mut out, &overflow, GRID_COLUMNS - side, side, row, height);
        }
        (false, _) => {
            for chunk in overflow.chunks(3) {
                row += in_a_row(&mut out, chunk, row);
            }
        }
    }
    out
}

/// One panel per row from `row` down, the last one filling what's left of `rows`
fn stack(out: &mut Vec<Placement>, panels: &[Panel], column: i32, width: i32, row: i32, rows: i32) {
    for (index, panel) in panels.iter().enumerate() {
        let last = index + 1 == panels.len();
        let height = if last { rows - index as i32 } else { 1 };
        out.push(Placement { panel: *panel, column, row: row + index as i32, width, height });
    }
}

/// Up to three panels side by side, sharing the width; returns the rows used
fn in_a_row(out: &mut Vec<Placement>, panels: &[Panel], row: i32) -> i32 {
    if panels.is_empty() {
        return 0;
    }
    let width = GRID_COLUMNS / panels.len() as i32;
    for (index, panel) in panels.iter().enumerate() {
        out.push(Placement { panel: *panel, column: index as i32 * width, row, width, height: 1 });
    }
    1
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionView {
    pub panel: Panel,
    pub title: &'static str,
    pub description: Option<&'static str>,
    pub controls: Vec<ControlView>,
}

/// Everything the settings page shows, ready for the widgets
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceView {
    pub title: String,
    pub subtitle: String,
    /// Under the name in the banner: vendor and color
    pub detail: String,
    /// The banner's status pill, e.g. CONNECTED
    pub badge: &'static str,
    pub charging: bool,
    pub has_battery: bool,
    /// Battery percentage for the battery ring
    pub battery: Option<u8>,
    pub battery_text: String,
    pub charge_text: String,
    /// At or below the low-battery mark
    pub battery_low: bool,
    pub sections: Vec<SectionView>,
}

/// Battery at or below this is shown as low
pub const LOW_BATTERY: u8 = 15;

/// One headset in the headset switcher
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadsetItem {
    pub id: DeviceId,
    pub name: String,
    pub status: String,
    pub battery: Option<u8>,
}

impl DeviceView {
    /// The rows and their choices; the window rebuilds its rows when this changes
    pub fn layout(&self) -> Vec<(Control, Vec<String>)> {
        self.sections
            .iter()
            .flat_map(|section| &section.controls)
            .map(|c| match &c.widget {
                Widget::Choice { options, .. } | Widget::Chips { options, .. } => (c.control, options.clone()),
                _ => (c.control, Vec::new()),
            })
            .collect()
    }

    pub fn control(&self, control: Control) -> Option<&ControlView> {
        self.sections.iter().flat_map(|section| &section.controls).find(|c| c.control == control)
    }

    /// The panels this headset has, in reading order. Battery and its history have no
    /// controls of their own: they come with a battery.
    pub fn panels(&self) -> Vec<Panel> {
        Panel::ORDER
            .into_iter()
            .filter(|panel| match panel {
                Panel::Battery | Panel::History => self.has_battery,
                panel => self.sections.iter().any(|section| section.panel == *panel),
            })
            .collect()
    }

    /// A microphone panel with this many controls takes two rows
    const TALL_MICROPHONE: usize = 5;

    pub fn placements(&self, narrow: bool) -> Vec<Placement> {
        let microphone = self.sections.iter().find(|section| section.panel == Panel::Microphone);
        let tall = microphone.is_some_and(|section| section.controls.len() >= Self::TALL_MICROPHONE);
        placements(&self.panels(), tall, narrow)
    }

    pub fn section(&self, panel: Panel) -> Option<&SectionView> {
        self.sections.iter().find(|section| section.panel == panel)
    }
}

fn charge_text(status: &Status) -> &'static str {
    match status.charge {
        Some(ChargeState::Discharging) => "On battery",
        Some(ChargeState::Charging) => "Charging",
        Some(ChargeState::Full) => "Fully charged",
        Some(ChargeState::Unknown) | None => "",
    }
}

/// What the user did to a control
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Switch(bool),
    /// 0 to 100
    Level(u8),
    /// The index of the chosen option
    Choice(u32),
    Press,
    Curve(Vec<Gain>),
}

impl Model {
    /// Records an event; returns true if the window needs redrawing
    pub fn apply(&mut self, event: &AppEvent) -> bool {
        match event {
            AppEvent::Added(info) => {
                self.devices.retain(|(known, _)| known.id != info.id);
                self.devices.push((info.clone(), Status::default()));
                true
            }
            AppEvent::Changed(id, event) => {
                self.devices.iter_mut().find(|(info, _)| info.id == *id).is_some_and(|(_, status)| status.apply(event))
            }
            AppEvent::Removed(id) => {
                self.software_eq.remove(id);
                let before = self.devices.len();
                self.devices.retain(|(info, _)| info.id != *id);
                self.devices.len() != before
            }
            // redraw, so the control the user changed shows the headset's value again
            AppEvent::Failed(_, reason) => {
                self.failure = Some(reason.clone());
                true
            }
            AppEvent::Problem(problem) => {
                let changed = self.problem != *problem;
                self.problem = problem.clone();
                changed
            }
        }
    }

    /// The headset's software equalizer, or None if it can't have one
    pub fn set_software_eq(&mut self, device: &DeviceId, eq: Option<SoftwareEq>) {
        match eq {
            Some(eq) => self.software_eq.insert(device.clone(), eq),
            None => self.software_eq.remove(device),
        };
    }

    /// The software equalizer a change to `control` asks for, on the headset the window shows
    pub fn software_eq_change(&self, control: Control, input: &Input) -> Option<(DeviceId, SoftwareEq)> {
        let (info, _) = self.current()?;
        let mut eq = self.software_eq.get(&info.id)?.clone();
        let spec = software_equalizer();
        match (control, input) {
            (Control::SoftwareEq, Input::Switch(on)) => eq.enabled = *on,
            (Control::SoftwareEqPreset, Input::Choice(i)) => {
                eq.curve = spec.presets.get(usize::try_from(*i).ok()?)?.curve.clone();
            }
            (Control::SoftwareEqCurve, Input::Curve(gains)) if gains.len() == spec.bands => eq.curve = spec.snap(gains),
            _ => return None,
        }
        Some((info.id.clone(), eq))
    }

    /// The last setting a headset refused, once
    pub fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }

    pub fn devices(&self) -> impl Iterator<Item = &DeviceInfo> {
        self.devices.iter().map(|(info, _)| info)
    }

    /// Every connected headset, for the headset switcher
    pub fn headsets(&self) -> Vec<HeadsetItem> {
        self.devices
            .iter()
            .map(|(info, s)| HeadsetItem {
                id: info.id.clone(),
                name: info.name.clone(),
                status: match (s.connected, s.battery) {
                    (None, _) => "Connecting…".into(),
                    (Some(false), _) => "Off".into(),
                    (Some(true), Some(level)) if s.charge.is_some_and(|c| c.is_plugged_in()) => {
                        format!("{level} · charging")
                    }
                    (Some(true), Some(level)) => level.to_string(),
                    (Some(true), None) => "Connected".into(),
                },
                battery: s.battery.map(Percent::get),
            })
            .collect()
    }

    /// Shows another headset; returns false if it isn't connected
    pub fn select(&mut self, id: &DeviceId) -> bool {
        let known = self.devices.iter().any(|(info, _)| info.id == *id);
        if known {
            self.selected = Some(id.clone());
        }
        known
    }

    /// The headset the window shows: the one picked, or else the first one found
    pub fn current(&self) -> Option<&(DeviceInfo, Status)> {
        let picked = self.selected.as_ref().and_then(|id| self.devices.iter().find(|(info, _)| info.id == *id));
        picked.or(self.devices.first())
    }

    pub fn page(&self) -> Page {
        match self.current() {
            None => Page::Missing { description: self.problem.clone().unwrap_or_else(|| LOOKING.into()) },
            Some((_, status)) => match status.connected {
                None => Page::Missing { description: WAITING.into() },
                Some(false) => Page::Off,
                Some(true) => Page::Settings,
            },
        }
    }

    pub fn title(&self) -> String {
        self.current().map_or_else(|| "Earmuff".into(), |(info, _)| info.name.clone())
    }

    pub fn view(&self) -> Option<DeviceView> {
        let (info, s) = self.current()?;
        let charging = s.charge.is_some_and(|c| c.is_plugged_in());
        let detail = [Some(info.vendor.clone()), s.color.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        Some(DeviceView {
            title: info.name.clone(),
            subtitle: s.color.clone().unwrap_or_default(),
            detail,
            badge: match (s.connected, s.charge) {
                (Some(true), Some(ChargeState::Full)) => "Fully charged",
                (Some(true), Some(ChargeState::Charging)) => "Charging",
                (Some(true), _) => "Connected",
                (Some(false), _) => "Off",
                (None, _) => "Connecting",
            },
            charging,
            has_battery: info.supports(Capability::Battery),
            battery: s.battery.map(|b| b.get()),
            battery_text: s.battery.map(|b| b.to_string()).unwrap_or_default(),
            charge_text: charge_text(s).into(),
            battery_low: !charging && s.battery.is_some_and(|b| b.get() <= LOW_BATTERY),
            sections: sections(info, s, self.software_eq.get(&info.id)),
        })
    }

    /// The setting a change to `control` asks for, on the headset the window shows
    pub fn setting(&self, control: Control, input: Input) -> Option<Setting> {
        let (info, status) = self.current()?;
        let choice = |index: u32| usize::try_from(index).ok();
        Some(match (control, input) {
            (Control::Mute, Input::Switch(on)) => Setting::MicMuted(on),
            (Control::RotateToMute, Input::Switch(on)) => Setting::RotateToMute(on),
            (Control::Sidetone, Input::Switch(on)) => Setting::Sidetone(on),
            (Control::VolumeLimiter, Input::Switch(on)) => Setting::VolumeLimiter(on),
            (Control::Lights, Input::Switch(on)) => Setting::Lights(on),
            (Control::VoicePrompts, Input::Switch(on)) => Setting::VoicePrompts(on),
            (Control::BtWhenPoweredOn, Input::Switch(on)) => Setting::BtWhenPoweredOn(on),
            (Control::MicVolume, Input::Level(level)) => Setting::MicVolume(Percent::saturating(level.into())),
            (Control::SidetoneLevel, Input::Level(level)) => Setting::SidetoneLevel(Percent::saturating(level.into())),
            (Control::BtCallVolume, Input::Level(level)) => Setting::BtCallVolume(Percent::saturating(level.into())),
            (Control::NoiseFilter, Input::Choice(i)) => Setting::NoiseFilter(*NoiseFilter::ALL.get(choice(i)?)?),
            (Control::MuteLed, Input::Choice(i)) if (i as usize) < MUTE_LED_LEVELS.len() => {
                Setting::MicMuteLedBrightness(u8::try_from(i).ok()?)
            }
            (Control::EqualizerPreset, Input::Choice(i)) => {
                let presets = info.equalizer.as_ref().map_or(0, |eq| eq.presets.len());
                (choice(i)? < presets).then_some(Setting::EqualizerPreset(u8::try_from(i).ok()?))?
            }
            (Control::NotificationSound, Input::Press) => Setting::PlayNotificationSound(0),
            (Control::Equalizer, Input::Curve(gains)) => {
                let spec = info.equalizer.as_ref()?;
                (gains.len() == spec.bands).then(|| Setting::Equalizer(spec.snap(&gains)))?
            }
            (Control::AutoOff, Input::Choice(i)) => {
                let minutes = *auto_off_choices(status.auto_off.map(AutoOff::get)).get(choice(i)?)?;
                Setting::AutoOff(AutoOff::minutes(minutes))
            }
            _ => return None,
        })
    }
}

fn switch(control: Control, title: &'static str, subtitle: Option<&'static str>, value: Option<bool>) -> ControlView {
    ControlView { control, title, subtitle, widget: Widget::Switch(value.unwrap_or(false)), sensitive: true }
}

fn level(control: Control, title: &'static str, value: Option<Percent>) -> ControlView {
    ControlView { control, title, subtitle: None, widget: Widget::Level(value.map(Percent::get)), sensitive: true }
}

fn choice(control: Control, title: &'static str, options: Vec<String>, selected: Option<usize>) -> ControlView {
    let selected = selected.and_then(|i| u32::try_from(i).ok());
    ControlView { control, title, subtitle: None, widget: Widget::Choice { options, selected }, sensitive: true }
}

fn info_row(control: Control, title: &'static str, text: String) -> ControlView {
    ControlView { control, title, subtitle: None, widget: Widget::Info(text), sensitive: true }
}

/// The preset list and the curve. A curve that matches no preset shows as "Custom".
/// `known` is the curve in use, if that's known, and `reported` the preset the headset says
/// it uses. `controls` are the preset and curve controls: the headset's own or the software ones.
fn equalizer_controls(
    eq: &EqualizerSpec,
    known: Option<Vec<Gain>>,
    reported: Option<usize>,
    controls: (Control, Control),
    presets: bool,
    curve: bool,
) -> Vec<Option<ControlView>> {
    let known = known.filter(|gains| gains.len() == eq.bands);
    let preset_view = (presets && !eq.presets.is_empty()).then(|| {
        let mut options: Vec<String> = eq.presets.iter().map(|p| p.name.clone()).collect();
        let matched = match &known {
            None => reported,
            Some(gains) => reported
                .filter(|i| eq.presets.get(*i).is_some_and(|p| &p.curve == gains))
                .or_else(|| eq.presets.iter().position(|p| &p.curve == gains)),
        };
        if !curve {
            return choice(controls.0, "Preset", options, matched);
        }
        // a curve of the user's own shows as "Custom", which is always there so the
        // presets never change length
        options.push(CUSTOM.into());
        let selected = match (&known, matched) {
            (Some(_), None) => Some(options.len() - 1),
            (_, matched) => matched,
        };
        let selected = selected.and_then(|i| u32::try_from(i).ok());
        ControlView {
            control: controls.0,
            title: "Preset",
            subtitle: None,
            widget: Widget::Chips { options, selected },
            sensitive: true,
        }
    });
    let curve_view = curve.then(|| {
        let preset = reported.and_then(|i| eq.presets.get(i)).map(|p| p.curve.clone());
        let gains = known.or(preset).filter(|gains| gains.len() == eq.bands).unwrap_or_else(|| eq.flat());
        ControlView {
            control: controls.1,
            title: "Curve",
            subtitle: None,
            widget: Widget::Curve(CurveView {
                gains,
                min: eq.min,
                max: eq.max,
                step: eq.step,
                baseline: eq.baseline,
                labels: band_labels(eq.bands),
            }),
            sensitive: true,
        }
    });
    vec![preset_view, curve_view]
}

/// The preset list's entry for a curve of the user's own
pub const CUSTOM: &str = "Custom";

/// The settings page's groups, with only the controls this headset has
fn sections(info: &DeviceInfo, s: &Status, software_eq: Option<&SoftwareEq>) -> Vec<SectionView> {
    let has = |capability| info.supports(capability);
    let mut sections = Vec::new();
    let mut add = |panel, title, description, controls: Vec<Option<ControlView>>| {
        let controls: Vec<ControlView> = controls.into_iter().flatten().collect();
        if !controls.is_empty() {
            sections.push(SectionView { panel, title, description, controls });
        }
    };

    // any headset with a sound card can be tested, whatever else its mic can do
    let muted = s.mic_muted == Some(true);
    add(
        Panel::Microphone,
        "Microphone",
        None,
        vec![
            has(Capability::MicMute)
                .then(|| switch(Control::Mute, "Mute", Some("Same as the mute button on the headset"), s.mic_muted)),
            has(Capability::MicAttached).then(|| {
                let text = match s.mic_attached {
                    Some(true) => "Boom attached",
                    Some(false) => "Boom not attached",
                    None => "",
                };
                info_row(Control::MicAttached, "Mic boom", text.into())
            }),
            has(Capability::MicVolume).then(|| level(Control::MicVolume, "Volume", s.mic_volume)),
            has(Capability::NoiseFilter).then(|| {
                let options = NOISE_FILTERS.map(String::from).to_vec();
                choice(Control::NoiseFilter, "Noise filter", options, s.noise_filter.map(|f| usize::from(f.level())))
            }),
            has(Capability::MicMuteLedBrightness).then(|| {
                let options = MUTE_LED_LEVELS.map(String::from).to_vec();
                choice(Control::MuteLed, "Mute light", options, s.mic_mute_led_brightness.map(usize::from))
            }),
            has(Capability::RotateToMute).then(|| {
                switch(Control::RotateToMute, "Mute when raised", Some("Flip the mic up to mute it"), s.rotate_to_mute)
            }),
            Some(ControlView {
                control: Control::MicTest,
                title: "Test your mic",
                subtitle: Some(if muted { "Unmute the mic first" } else { "Hear how you sound" }),
                widget: Widget::MicTest,
                sensitive: !muted,
            }),
        ],
    );

    let sidetone_on = s.sidetone.unwrap_or(false);
    add(
        Panel::Monitoring,
        "Mic monitoring",
        Some("Hear your own voice in the headset while you talk."),
        vec![
            has(Capability::Sidetone).then(|| switch(Control::Sidetone, "Mic monitoring", None, s.sidetone)),
            has(Capability::SidetoneLevel).then(|| ControlView {
                sensitive: sidetone_on || !has(Capability::Sidetone),
                ..level(Control::SidetoneLevel, "Level", s.sidetone_level)
            }),
        ],
    );

    // a headset with presets but no curve of its own picks them here; the software
    // equalizer, if it has one, takes the equalizer panel
    let presets_only = !has(Capability::Equalizer) && has(Capability::EqualizerPreset);
    let preset_row = info.equalizer.as_ref().filter(|_| presets_only).and_then(|eq| {
        let controls = (Control::EqualizerPreset, Control::Equalizer);
        let reported = s.equalizer_preset.map(usize::from);
        equalizer_controls(eq, s.equalizer.clone(), reported, controls, true, false).into_iter().flatten().next()
    });
    add(
        Panel::Sound,
        "Sound",
        None,
        vec![
            has(Capability::ChatMix).then_some(ControlView {
                control: Control::ChatMix,
                title: "ChatMix",
                subtitle: None,
                widget: Widget::Balance(s.chatmix),
                sensitive: true,
            }),
            preset_row.map(|row| ControlView { title: "Equalizer preset", ..row }),
            has(Capability::VolumeLimiter).then(|| {
                switch(Control::VolumeLimiter, "Volume limiter", Some("Keeps loud sounds in check"), s.volume_limiter)
            }),
        ],
    );

    if let (Some(eq), true) = (&info.equalizer, has(Capability::Equalizer)) {
        let controls = (Control::EqualizerPreset, Control::Equalizer);
        let (known, reported) = (s.equalizer.clone(), s.equalizer_preset.map(usize::from));
        let presets = has(Capability::EqualizerPreset);
        add(Panel::Equalizer, "Equalizer", None, equalizer_controls(eq, known, reported, controls, presets, true));
    } else if let Some(soft) = software_eq {
        let mut controls = vec![Some(switch(
            Control::SoftwareEq,
            "Use the equalizer",
            Some("Runs on this computer, for everything played through the headset"),
            Some(soft.enabled),
        ))];
        // while it's off, its presets and curve stay in place, dimmed, so nothing moves
        let ids = (Control::SoftwareEqPreset, Control::SoftwareEqCurve);
        let parts = equalizer_controls(&software_equalizer(), Some(soft.curve.clone()), None, ids, true, true);
        controls.extend(parts.into_iter().map(|part| part.map(|c| ControlView { sensitive: soft.enabled, ..c })));
        add(Panel::Equalizer, "Equalizer", Some("On this computer"), controls);
    }

    add(
        Panel::Power,
        "Power and sounds",
        None,
        vec![
            has(Capability::AutoOff).then(|| {
                let current = s.auto_off.map(AutoOff::get);
                let choices = auto_off_choices(current);
                let options = choices.iter().map(|m| minutes_text(*m)).collect();
                let selected = current.and_then(|minutes| choices.iter().position(|m| *m == minutes));
                choice(Control::AutoOff, "Turn off when not in use", options, selected)
            }),
            has(Capability::VoicePrompts).then(|| {
                switch(Control::VoicePrompts, "Voice prompts", Some("Spoken alerts from the headset"), s.voice_prompts)
            }),
            has(Capability::NotificationSound).then_some(ControlView {
                control: Control::NotificationSound,
                title: "Notification sound",
                subtitle: None,
                widget: Widget::Button("Play"),
                sensitive: true,
            }),
        ],
    );

    let lights = has(Capability::Lights);
    let bluetooth = has(Capability::BtWhenPoweredOn) || has(Capability::BtCallVolume);
    add(
        Panel::Lights,
        match (lights, bluetooth) {
            (true, true) => "Lights and Bluetooth",
            (true, false) => "Lights",
            _ => "Bluetooth",
        },
        None,
        vec![
            lights.then(|| switch(Control::Lights, "Lights", None, s.lights)),
            has(Capability::BtWhenPoweredOn)
                .then(|| switch(Control::BtWhenPoweredOn, "Bluetooth on with the headset", None, s.bt_when_powered_on)),
            has(Capability::BtCallVolume).then(|| level(Control::BtCallVolume, "Call volume", s.bt_call_volume)),
        ],
    );

    let connection = match info.id.backend {
        BackendKind::Native => "Earmuff's own driver",
        BackendKind::HeadsetControl => "Through HeadsetControl",
        BackendKind::Demo => "Simulated (demo)",
    };
    let model =
        if info.name.starts_with(&info.vendor) { info.name.clone() } else { format!("{} {}", info.vendor, info.name) };
    add(
        Panel::Device,
        "This headset",
        None,
        vec![
            Some(info_row(Control::Model, "Model", model)),
            Some(info_row(Control::UsbId, "USB ID", info.id.usb.to_string())),
            Some(info_row(Control::Connection, "Connection", connection.into())),
        ],
    );
    sections
}

pub fn minutes_text(minutes: u8) -> String {
    match minutes {
        0 => "Never".into(),
        m if m % 60 != 0 => format!("{m} minutes"),
        60 => "1 hour".into(),
        m => format!("{} hours", m / 60),
    }
}

/// The auto-off choices: the usual ones, plus the headset's current value if it's
/// something else (e.g. set from the command line)
pub fn auto_off_choices(current: Option<u8>) -> Vec<u8> {
    let mut choices = AUTO_OFF_MINUTES.to_vec();
    if let Some(minutes) = current
        && !choices.contains(&minutes)
    {
        choices.push(minutes);
        choices.sort_unstable();
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{BackendKind, EqPreset, Event, UsbId};

    const CLOUD_ALPHA: [Capability; 7] = [
        Capability::Battery,
        Capability::MicMute,
        Capability::MicAttached,
        Capability::Sidetone,
        Capability::SidetoneLevel,
        Capability::AutoOff,
        Capability::VoicePrompts,
    ];

    fn info(instance: &str, capabilities: &[Capability]) -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(0x03F0, 0x098D), instance: instance.into() },
            name: format!("Headset {instance}"),
            vendor: "HyperX".into(),
            capabilities: capabilities.to_vec(),
            equalizer: Some(EqualizerSpec {
                bands: 2,
                min: Gain::tenths(-60),
                max: Gain::tenths(60),
                step: Gain::tenths(10),
                baseline: Gain::tenths(0),
                presets: vec![
                    EqPreset { name: "Flat".into(), curve: vec![Gain::tenths(0); 2] },
                    EqPreset { name: "Bass".into(), curve: vec![Gain::tenths(40), Gain::tenths(0)] },
                ],
            }),
        }
    }

    fn alpha(instance: &str) -> DeviceInfo {
        info(instance, &CLOUD_ALPHA)
    }

    fn changed(instance: &str, event: Event) -> AppEvent {
        AppEvent::Changed(alpha(instance).id, event)
    }

    fn model_with(capabilities: &[Capability], events: Vec<Event>) -> Model {
        let mut model = Model::default();
        let device = info("a", capabilities);
        model.apply(&AppEvent::Added(device.clone()));
        for event in events {
            model.apply(&AppEvent::Changed(device.id.clone(), event));
        }
        model
    }

    #[test]
    fn looks_for_a_headset_until_one_is_added() {
        let model = Model::default();
        assert_eq!(model.page(), Page::Missing { description: LOOKING.into() });
        assert_eq!(model.title(), "Earmuff");
        assert_eq!(model.view(), None);
    }

    #[test]
    fn explains_a_problem_while_no_headset_works() {
        let mut model = Model::default();
        assert!(model.apply(&AppEvent::Problem(Some("no permission".into()))));
        assert!(!model.apply(&AppEvent::Problem(Some("no permission".into()))));
        assert_eq!(model.page(), Page::Missing { description: "no permission".into() });
        model.apply(&AppEvent::Problem(None));
        assert_eq!(model.page(), Page::Missing { description: LOOKING.into() });
    }

    #[test]
    fn follows_the_connection() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(alpha("a")));
        assert_eq!(model.page(), Page::Missing { description: WAITING.into() });
        assert_eq!(model.title(), "Headset a");
        model.apply(&changed("a", Event::Connected(false)));
        assert_eq!(model.page(), Page::Off);
        model.apply(&changed("a", Event::Connected(true)));
        assert_eq!(model.page(), Page::Settings);
        assert!(model.apply(&AppEvent::Removed(alpha("a").id)));
        assert_eq!(model.page(), Page::Missing { description: LOOKING.into() });
    }

    #[test]
    fn redraws_only_on_real_changes() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(alpha("a")));
        assert!(model.apply(&changed("a", Event::MicMuted(true))));
        assert!(!model.apply(&changed("a", Event::MicMuted(true))));
        assert!(!model.apply(&changed("other", Event::MicMuted(false))));
        assert!(!model.apply(&AppEvent::Removed(alpha("other").id)));
    }

    #[test]
    fn a_refused_setting_redraws_and_is_reported_once() {
        let mut model = Model::default();
        assert!(model.apply(&AppEvent::Failed(alpha("a").id, "the headset is off".into())));
        assert_eq!(model.take_failure().as_deref(), Some("the headset is off"));
        assert_eq!(model.take_failure(), None);
    }

    #[test]
    fn shows_the_first_headset_until_another_is_picked() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(alpha("a")));
        model.apply(&AppEvent::Added(alpha("b")));
        assert_eq!(model.title(), "Headset a");
        assert!(model.select(&alpha("b").id));
        assert_eq!(model.title(), "Headset b");
        assert!(!model.select(&alpha("gone").id));
        model.apply(&AppEvent::Removed(alpha("b").id));
        assert_eq!(model.title(), "Headset a");
        assert_eq!(model.devices().count(), 1);
    }

    #[test]
    fn the_cloud_alpha_gets_its_own_controls_only() {
        let model = model_with(&CLOUD_ALPHA, vec![Event::Connected(true), Event::Sidetone(false)]);
        let view = model.view().unwrap();
        let titles: Vec<&str> = view.sections.iter().map(|s| s.title).collect();
        assert_eq!(titles, ["Microphone", "Mic monitoring", "Power and sounds", "This headset"]);
        let controls: Vec<Control> = view.layout().iter().map(|(c, _)| *c).collect();
        assert_eq!(
            controls,
            [
                Control::Mute,
                Control::MicAttached,
                Control::MicTest,
                Control::Sidetone,
                Control::SidetoneLevel,
                Control::AutoOff,
                Control::VoicePrompts,
                Control::Model,
                Control::UsbId,
                Control::Connection
            ]
        );
        assert_eq!(view.control(Control::Model).unwrap().widget, Widget::Info("HyperX Headset a".into()));
        assert_eq!(view.control(Control::UsbId).unwrap().widget, Widget::Info("03f0:098d".into()));
        assert_eq!(view.control(Control::Connection).unwrap().widget, Widget::Info("Simulated (demo)".into()));
        assert!(
            !view.control(Control::SidetoneLevel).unwrap().sensitive,
            "level is greyed out while monitoring is off"
        );
        assert!(view.has_battery);
        assert_eq!(
            view.panels(),
            [Panel::Battery, Panel::Microphone, Panel::Monitoring, Panel::Power, Panel::History, Panel::Device]
        );
    }

    #[test]
    fn the_mic_test_waits_for_an_unmuted_mic() {
        let test = |muted| {
            let model = model_with(&CLOUD_ALPHA, vec![Event::Connected(true), Event::MicMuted(muted)]);
            let control = model.view().unwrap().control(Control::MicTest).cloned().unwrap();
            (control.sensitive, control.subtitle.unwrap())
        };
        assert_eq!(test(false), (true, "Hear how you sound"));
        assert_eq!(test(true), (false, "Unmute the mic first"));
    }

    #[test]
    fn a_headset_with_everything_gets_every_control() {
        let capabilities = [
            Capability::Lights,
            Capability::ChatMix,
            Capability::NoiseFilter,
            Capability::MicMuteLedBrightness,
            Capability::EqualizerPreset,
            Capability::BtCallVolume,
            Capability::SidetoneLevel,
        ];
        let model = model_with(
            &capabilities,
            vec![Event::ChatMix(30), Event::NoiseFilter(NoiseFilter::High), Event::EqualizerPreset(1)],
        );
        let view = model.view().unwrap();
        assert!(!view.has_battery);
        assert_eq!(view.control(Control::ChatMix).unwrap().widget, Widget::Balance(Some(30)));
        assert_eq!(
            view.control(Control::NoiseFilter).unwrap().widget,
            Widget::Choice { options: vec!["Off".into(), "Low".into(), "High".into()], selected: Some(2) }
        );
        assert_eq!(
            view.control(Control::EqualizerPreset).unwrap().widget,
            Widget::Choice { options: vec!["Flat".into(), "Bass".into()], selected: Some(1) }
        );
        // without an on/off switch, the sidetone level is always adjustable
        assert!(view.control(Control::SidetoneLevel).unwrap().sensitive);
        assert!(view.control(Control::Lights).is_some() && view.control(Control::BtCallVolume).is_some());
        // presets without a curve of their own are picked in the sound panel
        assert_eq!(view.section(Panel::Sound).unwrap().controls[1].control, Control::EqualizerPreset);
        assert_eq!(view.section(Panel::Lights).unwrap().title, "Lights and Bluetooth");
        assert_eq!(view.panels(), [Panel::Microphone, Panel::Monitoring, Panel::Sound, Panel::Lights, Panel::Device]);
    }

    fn curve_of(view: &DeviceView) -> Vec<Gain> {
        match &view.control(Control::Equalizer).unwrap().widget {
            Widget::Curve(curve) => curve.gains.clone(),
            other => panic!("not a curve: {other:?}"),
        }
    }

    fn presets_of(view: &DeviceView) -> (Vec<String>, Option<u32>) {
        match &view.control(Control::EqualizerPreset).unwrap().widget {
            Widget::Chips { options, selected } => (options.clone(), *selected),
            other => panic!("not a choice: {other:?}"),
        }
    }

    #[test]
    fn the_equalizer_shows_the_curve_and_calls_a_curve_of_your_own_custom() {
        const EQ: [Capability; 2] = [Capability::EqualizerPreset, Capability::Equalizer];
        let (flat, bass) = (vec![Gain::tenths(0); 2], vec![Gain::tenths(40), Gain::tenths(0)]);
        // nothing known yet: flat
        let view = model_with(&EQ, vec![]).view().unwrap();
        assert_eq!(curve_of(&view), flat);
        assert!(view.sections.iter().any(|s| s.title == "Equalizer" && s.controls.len() == 2));
        // a preset shows its curve
        let view = model_with(&EQ, vec![Event::EqualizerPreset(1)]).view().unwrap();
        assert_eq!(curve_of(&view), bass);
        // a curve that matches a preset picks it
        let view = model_with(&EQ, vec![Event::Equalizer(bass.clone())]).view().unwrap();
        assert_eq!(presets_of(&view), (vec!["Flat".into(), "Bass".into(), CUSTOM.into()], Some(1)));
        // anything else is the user's own
        let own = vec![Gain::tenths(10), Gain::tenths(-20)];
        let view = model_with(&EQ, vec![Event::EqualizerPreset(1), Event::Equalizer(own.clone())]).view().unwrap();
        assert_eq!(curve_of(&view), own);
        assert_eq!(presets_of(&view), (vec!["Flat".into(), "Bass".into(), CUSTOM.into()], Some(2)));
        // and picking "Custom" itself does nothing
        let model = model_with(&EQ, vec![Event::Equalizer(own)]);
        assert_eq!(model.setting(Control::EqualizerPreset, Input::Choice(2)), None);
    }

    #[test]
    fn a_headset_without_an_equalizer_gets_the_software_one() {
        let mut model = model_with(&CLOUD_ALPHA, vec![Event::Connected(true)]);
        let id = info("a", &CLOUD_ALPHA).id;
        // until the window says it can have one, it has none
        assert!(model.view().unwrap().control(Control::SoftwareEq).is_none());
        model.set_software_eq(&id, Some(SoftwareEq::default()));
        let view = model.view().unwrap();
        assert_eq!(view.control(Control::SoftwareEq).unwrap().widget, Widget::Switch(false));
        // while it's off, its presets and curve stay in place, dimmed
        assert!(!view.control(Control::SoftwareEqCurve).unwrap().sensitive);
        assert!(!view.control(Control::SoftwareEqPreset).unwrap().sensitive);

        let (device, on) = model.software_eq_change(Control::SoftwareEq, &Input::Switch(true)).unwrap();
        assert_eq!((device, on.enabled), (id.clone(), true));
        model.set_software_eq(&id, Some(on));
        let view = model.view().unwrap();
        assert_eq!(presets_of_control(&view, Control::SoftwareEqPreset).1, Some(0), "flat");
        assert_eq!(curve_of_control(&view, Control::SoftwareEqCurve), software_equalizer().flat());

        // a preset sets the curve; a curve of one's own is snapped and shows as custom
        let (_, bass) = model.software_eq_change(Control::SoftwareEqPreset, &Input::Choice(1)).unwrap();
        assert_eq!(bass.curve, software_equalizer().presets[1].curve);
        let mut own = vec![Gain::tenths(0); 10];
        own[9] = Gain::tenths(33);
        let (_, mine) = model.software_eq_change(Control::SoftwareEqCurve, &Input::Curve(own)).unwrap();
        assert_eq!(mine.curve[9], Gain::tenths(35));
        model.set_software_eq(&id, Some(mine));
        let (options, selected) = presets_of_control(&model.view().unwrap(), Control::SoftwareEqPreset);
        assert_eq!((options.last().map(String::as_str), selected), (Some(CUSTOM), Some(5)));

        // unplugged, it's forgotten
        model.apply(&AppEvent::Removed(id));
        assert!(model.software_eq_change(Control::SoftwareEq, &Input::Switch(false)).is_none());
    }

    fn curve_of_control(view: &DeviceView, control: Control) -> Vec<Gain> {
        match &view.control(control).unwrap().widget {
            Widget::Curve(curve) => curve.gains.clone(),
            other => panic!("not a curve: {other:?}"),
        }
    }

    fn presets_of_control(view: &DeviceView, control: Control) -> (Vec<String>, Option<u32>) {
        match &view.control(control).unwrap().widget {
            Widget::Chips { options, selected } => (options.clone(), *selected),
            other => panic!("not a choice: {other:?}"),
        }
    }

    #[test]
    fn bands_are_named_by_frequency_when_there_are_ten() {
        assert_eq!(band_labels(10)[5], "1k");
        assert_eq!(band_labels(3), ["1", "2", "3"]);
    }

    #[test]
    fn the_banner_says_how_the_headset_is_doing() {
        let on_battery = model_with(
            &CLOUD_ALPHA,
            vec![
                Event::Connected(true),
                Event::Battery(Percent::saturating(12)),
                Event::Charge(ChargeState::Discharging),
            ],
        );
        let view = on_battery.view().unwrap();
        assert_eq!((view.badge, view.charging, view.battery_low), ("Connected", false, true));
        let charging = model_with(
            &CLOUD_ALPHA,
            vec![Event::Connected(true), Event::Battery(Percent::saturating(12)), Event::Charge(ChargeState::Charging)],
        );
        let view = charging.view().unwrap();
        // a charging headset isn't low on battery
        assert_eq!((view.badge, view.charging, view.battery_low), ("Charging", true, false));
        let full = model_with(&CLOUD_ALPHA, vec![Event::Connected(true), Event::Charge(ChargeState::Full)]);
        assert_eq!(full.view().unwrap().badge, "Fully charged");
    }

    #[test]
    fn the_switcher_lists_every_headset() {
        let mut model = Model::default();
        model.apply(&AppEvent::Added(alpha("a")));
        model.apply(&AppEvent::Added(alpha("b")));
        model.apply(&changed("a", Event::Connected(true)));
        model.apply(&changed("a", Event::Battery(Percent::saturating(80))));
        model.apply(&changed("b", Event::Connected(false)));
        let items = model.headsets();
        assert_eq!(items.iter().map(|i| i.status.as_str()).collect::<Vec<_>>(), ["80%", "Off"]);
        assert_eq!((items[0].name.as_str(), items[0].battery), ("Headset a", Some(80)));
        model.apply(&changed("a", Event::Charge(ChargeState::Charging)));
        assert_eq!(model.headsets()[0].status, "80% · charging");
    }

    fn cell(placements: &[Placement], panel: Panel) -> (i32, i32, i32, i32) {
        let p = placements.iter().find(|p| p.panel == panel).unwrap_or_else(|| panic!("{panel:?} isn't placed"));
        (p.column, p.row, p.width, p.height)
    }

    #[test]
    fn a_tall_microphone_panel_spans_two_rows_between_two_stacks() {
        let every = Panel::ORDER;
        let wide = placements(&every, true, false);
        assert_eq!(cell(&wide, Panel::Battery), (0, 0, 2, 1));
        assert_eq!(cell(&wide, Panel::Monitoring), (0, 1, 2, 1));
        assert_eq!(cell(&wide, Panel::Microphone), (2, 0, 2, 2));
        assert_eq!(cell(&wide, Panel::Sound), (4, 0, 2, 1));
        assert_eq!(cell(&wide, Panel::Power), (4, 1, 2, 1));
        assert_eq!(cell(&wide, Panel::Equalizer), (0, 2, 6, 1));
        // the history beside the small panels, as tall as they are together
        assert_eq!(cell(&wide, Panel::History), (0, 3, 4, 2));
        assert_eq!(cell(&wide, Panel::Lights), (4, 3, 2, 1));
        assert_eq!(cell(&wide, Panel::Device), (4, 4, 2, 1));
    }

    #[test]
    fn a_short_microphone_panel_sits_in_a_row_of_three() {
        // the Cloud Alpha Wireless with the software equalizer
        let panels = [
            Panel::Battery,
            Panel::Microphone,
            Panel::Monitoring,
            Panel::Power,
            Panel::Equalizer,
            Panel::History,
            Panel::Device,
        ];
        let wide = placements(&panels, false, false);
        assert_eq!(cell(&wide, Panel::Battery), (0, 0, 2, 1));
        assert_eq!(cell(&wide, Panel::Microphone), (2, 0, 2, 1));
        assert_eq!(cell(&wide, Panel::Monitoring), (4, 0, 2, 1));
        assert_eq!(cell(&wide, Panel::Equalizer), (0, 1, 6, 1));
        // power moves down beside the history
        assert_eq!(cell(&wide, Panel::History), (0, 2, 4, 2));
        assert_eq!(cell(&wide, Panel::Power), (4, 2, 2, 1));
        assert_eq!(cell(&wide, Panel::Device), (4, 3, 2, 1));
    }

    #[test]
    fn panels_widen_to_fill_their_row_and_stack_on_narrow_windows() {
        // a wired headset: no battery, so no history either
        let panels = [Panel::Microphone, Panel::Monitoring, Panel::Equalizer, Panel::Device];
        let wide = placements(&panels, false, false);
        assert_eq!(cell(&wide, Panel::Microphone), (0, 0, 3, 1));
        assert_eq!(cell(&wide, Panel::Monitoring), (3, 0, 3, 1));
        assert_eq!(cell(&wide, Panel::Device), (0, 2, 6, 1));
        let narrow = placements(&panels, false, true);
        let order: Vec<(Panel, i32, i32)> = narrow.iter().map(|p| (p.panel, p.row, p.width)).collect();
        assert_eq!(
            order,
            [
                (Panel::Microphone, 0, GRID_COLUMNS),
                (Panel::Monitoring, 1, GRID_COLUMNS),
                (Panel::Equalizer, 2, GRID_COLUMNS),
                (Panel::Device, 3, GRID_COLUMNS)
            ]
        );
    }

    #[test]
    fn the_layout_depends_on_what_the_headset_has_not_on_its_state() {
        let placed = |events: Vec<Event>| model_with(&CLOUD_ALPHA, events).view().unwrap().placements(false);
        let on = placed(vec![Event::Connected(true), Event::MicMuted(false), Event::Sidetone(true)]);
        let muted = placed(vec![Event::Connected(true), Event::MicMuted(true), Event::Sidetone(false)]);
        assert_eq!(on, muted);
    }

    #[test]
    fn controls_turn_into_settings() {
        let model =
            model_with(&[Capability::AutoOff, Capability::EqualizerPreset], vec![Event::AutoOff(AutoOff::minutes(45))]);
        assert_eq!(model.setting(Control::Lights, Input::Switch(true)), Some(Setting::Lights(true)));
        assert_eq!(
            model.setting(Control::MicVolume, Input::Level(40)),
            Some(Setting::MicVolume(Percent::saturating(40)))
        );
        assert_eq!(model.setting(Control::NoiseFilter, Input::Choice(1)), Some(Setting::NoiseFilter(NoiseFilter::Low)));
        assert_eq!(model.setting(Control::MuteLed, Input::Choice(3)), Some(Setting::MicMuteLedBrightness(3)));
        assert_eq!(model.setting(Control::MuteLed, Input::Choice(4)), None);
        assert_eq!(model.setting(Control::EqualizerPreset, Input::Choice(1)), Some(Setting::EqualizerPreset(1)));
        assert_eq!(model.setting(Control::EqualizerPreset, Input::Choice(2)), None);
        // a curve goes onto the equalizer's steps and into its range
        assert_eq!(
            model.setting(Control::Equalizer, Input::Curve(vec![Gain::tenths(23), Gain::tenths(-90)])),
            Some(Setting::Equalizer(vec![Gain::tenths(20), Gain::tenths(-60)]))
        );
        assert_eq!(model.setting(Control::Equalizer, Input::Curve(vec![Gain::tenths(0)])), None);
        // the auto-off list includes the headset's own 45 minutes
        assert_eq!(model.setting(Control::AutoOff, Input::Choice(4)), Some(Setting::AutoOff(AutoOff::minutes(45))));
        assert_eq!(model.setting(Control::ChatMix, Input::Switch(true)), None);
        assert_eq!(model.setting(Control::NotificationSound, Input::Press), Some(Setting::PlayNotificationSound(0)));
    }

    #[test]
    fn builds_the_battery_line() {
        let model = model_with(
            &CLOUD_ALPHA,
            vec![
                Event::Battery(Percent::saturating(99)),
                Event::Charge(ChargeState::Charging),
                Event::Color("Black/Red".into()),
            ],
        );
        let view = model.view().unwrap();
        assert_eq!((view.battery, view.battery_text.as_str()), (Some(99), "99%"));
        assert_eq!((view.charge_text.as_str(), view.subtitle.as_str()), ("Charging", "Black/Red"));
        assert_eq!(view.detail, "HyperX · Black/Red");
    }

    #[test]
    fn battery_alert_choices() {
        assert_eq!(battery_alert_label(0), "Off");
        assert_eq!(battery_alert_label(20), "At 20%");
        assert_eq!(battery_alert_index(20), 3);
        assert_eq!(battery_alert_index(0), 0);
        assert_eq!(battery_alert_index(25), 3); // nearest offered level
    }

    #[test]
    fn alerts_read_naturally() {
        use earmuff_application::Alert;
        let device = alpha("a").id;
        let low = Alert::Low { device: device.clone(), name: "Cloud Alpha".into(), level: 18 };
        assert_eq!(alert_text(&low), ("Headset battery low".into(), "Cloud Alpha: 18% left".into()));
        let charged = Alert::Charged { device, name: "Cloud Alpha".into() };
        assert_eq!(alert_text(&charged).1, "Cloud Alpha is fully charged.");
    }

    #[test]
    fn autostart_hints_for_desktops_without_autostart() {
        assert!(autostart_hint("Hyprland").unwrap().contains("exec-once = earmuff --hidden"));
        assert!(autostart_hint("sway").unwrap().contains("exec earmuff --hidden"));
        assert_eq!(autostart_hint("GNOME"), None);
        assert_eq!(autostart_hint("KDE"), None);
    }

    #[test]
    fn minutes_read_naturally() {
        assert_eq!(minutes_text(0), "Never");
        assert_eq!(minutes_text(20), "20 minutes");
        assert_eq!(minutes_text(60), "1 hour");
        assert_eq!(minutes_text(90), "90 minutes");
        assert_eq!(minutes_text(120), "2 hours");
    }

    #[test]
    fn auto_off_choices_include_the_current_value() {
        assert_eq!(auto_off_choices(None), AUTO_OFF_MINUTES);
        assert_eq!(auto_off_choices(Some(20)), AUTO_OFF_MINUTES);
        assert_eq!(auto_off_choices(Some(45)), [0, 10, 20, 30, 45, 60, 90]);
    }
}
