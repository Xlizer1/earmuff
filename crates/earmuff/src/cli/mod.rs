//! Command line: `earmuff` alone opens the window, subcommands work from scripts.

mod output;

use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use earmuff_application::{
    Backend, Session, SessionError, apply_settings, discover, open_device, read_status, toggle_mute,
};
use earmuff_domain::{AutoOff, DeviceInfo, Gain, NoiseFilter, Percent, Query, Setting, UsbId};
use earmuff_infrastructure::hidraw::{SYSFS_HIDRAW, Sniffer};
use gtk::glib;

pub use output::{chatmix_text, event_line, list_json, list_text, preset_name, status_json, status_text, waybar_json};

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Parser)]
#[command(
    name = "earmuff",
    version,
    about = "Battery and settings for your gaming headset. Without a command, opens the window."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// The headset to use, by USB ID (see `earmuff list`); the first one found by default
    #[arg(short, long, global = true, value_name = "VID:PID")]
    pub device: Option<UsbId>,

    /// Try Earmuff with simulated headsets instead of real ones
    #[arg(long, global = true)]
    pub demo: bool,

    /// Start in the tray without showing the window (for starting at login)
    #[arg(long)]
    pub hidden: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Headsets Earmuff can reach
    List {
        #[arg(long)]
        json: bool,
    },
    /// Battery and all settings
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Change a setting
    Set {
        #[command(subcommand)]
        setting: SetCommand,
    },
    /// Play one of the headset's notification sounds
    PlaySound { sound: u8 },
    /// One line of JSON for a Waybar custom module
    Waybar,
    /// Print changes as the headset reports them
    Monitor {
        /// Also print every report in hex (handy for decoding new ones)
        #[arg(long)]
        raw: bool,
    },
    /// Print every report any USB device sends, in hex, without sending it anything.
    /// For headsets Earmuff doesn't know yet; may need sudo.
    Sniff {
        #[arg(value_name = "VID:PID")]
        usb: UsbId,
    },
}

#[derive(Debug, Subcommand)]
pub enum SetCommand {
    /// Mute the microphone; `toggle` flips it, for a keyboard shortcut
    Mute { value: MuteValue },
    /// Mic monitoring (hearing your own voice): on, off, or a level from 0 to 100%
    #[command(alias = "mic-monitoring")]
    Sidetone {
        #[arg(value_parser = parse_sidetone)]
        value: Sidetone,
    },
    /// Minutes of no use before the headset turns off (0 = never)
    AutoOff { minutes: u8 },
    /// Spoken alerts from the headset
    VoicePrompts { value: OnOff },
    /// The headset's lights
    Lights { value: OnOff },
    /// Mute when the mic boom is raised
    RotateToMute { value: OnOff },
    /// Microphone volume, 0 to 100%
    MicVolume { level: Percent },
    /// Brightness of the mute light, 0 (off) to 3
    MuteLed {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=3))]
        level: u8,
    },
    /// Background noise filter on the mic: off, low or high
    NoiseFilter { level: NoiseFilter },
    /// Keep the volume from getting too loud
    VolumeLimiter { value: OnOff },
    /// Turn Bluetooth on together with the headset
    BluetoothOnPower { value: OnOff },
    /// Volume of Bluetooth calls, 0 to 100%
    BluetoothCallVolume { level: Percent },
    /// Equalizer preset, by number or name (`earmuff status` shows the current one)
    EqPreset { preset: String },
    /// Equalizer curve: one gain in dB per band, e.g. 3,2,0,-1,0,0,1,2,3,3
    Eq {
        #[arg(allow_hyphen_values = true)]
        curve: Curve,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum MuteValue {
    On,
    Off,
    Toggle,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

impl From<OnOff> for bool {
    fn from(value: OnOff) -> bool {
        matches!(value, OnOff::On)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sidetone {
    On,
    Off,
    Level(Percent),
}

pub fn parse_sidetone(value: &str) -> Result<Sidetone, String> {
    match value {
        "on" => Ok(Sidetone::On),
        "off" => Ok(Sidetone::Off),
        _ => value.parse().map(Sidetone::Level).map_err(|_| "use on, off, or a level from 0 to 100%".into()),
    }
}

/// Equalizer gains in dB, comma-separated
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Curve(pub Vec<Gain>);

impl FromStr for Curve {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.split(',').map(|gain| gain.parse::<Gain>().map_err(|e| e.to_string())).collect::<Result<_, _>>().map(Curve)
    }
}

/// The equalizer preset `wanted` names, by number or (case-insensitive) name
pub fn find_preset(info: &DeviceInfo, wanted: &str) -> Result<u8, String> {
    let presets = info.equalizer.as_ref().map(|eq| eq.presets.as_slice()).unwrap_or_default();
    if let Ok(index) = wanted.parse::<u8>() {
        return Ok(index);
    }
    presets
        .iter()
        .position(|preset| preset.name.eq_ignore_ascii_case(wanted.trim()))
        .and_then(|index| u8::try_from(index).ok())
        .ok_or_else(|| {
            let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
            if names.is_empty() {
                format!("this headset has no preset called {wanted:?}")
            } else {
                format!("no preset called {wanted:?}; this headset has: {}", names.join(", "))
            }
        })
}

/// The settings a `set` command changes, in the order they are sent
pub fn settings(command: &SetCommand, info: &DeviceInfo) -> Result<Vec<Setting>, String> {
    Ok(match command {
        SetCommand::Mute { value: MuteValue::On } => vec![Setting::MicMuted(true)],
        SetCommand::Mute { value: MuteValue::Off } => vec![Setting::MicMuted(false)],
        SetCommand::Mute { value: MuteValue::Toggle } => return Err("toggling depends on the headset's mute".into()),
        SetCommand::Sidetone { value: Sidetone::Off } => vec![Setting::Sidetone(false)],
        SetCommand::Sidetone { value: Sidetone::On } => vec![Setting::Sidetone(true)],
        SetCommand::Sidetone { value: Sidetone::Level(level) } => {
            vec![Setting::Sidetone(true), Setting::SidetoneLevel(*level)]
        }
        SetCommand::AutoOff { minutes } => vec![Setting::AutoOff(AutoOff::minutes(*minutes))],
        SetCommand::VoicePrompts { value } => vec![Setting::VoicePrompts((*value).into())],
        SetCommand::Lights { value } => vec![Setting::Lights((*value).into())],
        SetCommand::RotateToMute { value } => vec![Setting::RotateToMute((*value).into())],
        SetCommand::MicVolume { level } => vec![Setting::MicVolume(*level)],
        SetCommand::MuteLed { level } => vec![Setting::MicMuteLedBrightness(*level)],
        SetCommand::NoiseFilter { level } => vec![Setting::NoiseFilter(*level)],
        SetCommand::VolumeLimiter { value } => vec![Setting::VolumeLimiter((*value).into())],
        SetCommand::BluetoothOnPower { value } => vec![Setting::BtWhenPoweredOn((*value).into())],
        SetCommand::BluetoothCallVolume { level } => vec![Setting::BtCallVolume(*level)],
        SetCommand::EqPreset { preset } => vec![Setting::EqualizerPreset(find_preset(info, preset)?)],
        SetCommand::Eq { curve } => vec![Setting::Equalizer(curve.0.clone())],
    })
}

pub fn run(command: Command, device: Option<UsbId>, backends: &[Arc<dyn Backend>]) -> ExitCode {
    match command {
        Command::List { json } => return list(backends, json),
        Command::Sniff { usb } => return sniff(usb),
        Command::Waybar => {
            let status = open_device(backends, device).ok().and_then(|mut session| read_battery(session.as_mut()).ok());
            println!("{}", waybar_json(status.as_ref()));
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    let mut session = match open_device(backends, device) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("earmuff: {error}");
            return ExitCode::FAILURE;
        }
    };
    match execute(session.as_mut(), &command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("earmuff: {error}");
            ExitCode::FAILURE
        }
    }
}

fn list(backends: &[Arc<dyn Backend>], json: bool) -> ExitCode {
    let (found, problem) = discover(backends);
    let devices: Vec<DeviceInfo> = found.into_iter().map(|(_, info)| info).collect();
    if json {
        println!("{}", list_json(&devices));
    } else {
        print!("{}", list_text(&devices));
    }
    if let Some(problem) = problem {
        eprintln!("earmuff: {problem}");
    }
    ExitCode::SUCCESS
}

#[derive(Debug)]
enum Failure {
    Session(SessionError),
    Invalid(String),
    Output(io::Error),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Failure::Session(error) => error.fmt(f),
            Failure::Invalid(reason) => f.write_str(reason),
            Failure::Output(error) => error.fmt(f),
        }
    }
}

impl From<SessionError> for Failure {
    fn from(error: SessionError) -> Self {
        Failure::Session(error)
    }
}

fn execute(session: &mut dyn Session, command: &Command) -> Result<(), Failure> {
    match command {
        Command::Status { json } => {
            let status = read_status(session, TIMEOUT)?;
            if *json {
                println!("{}", status_json(session.info(), &status));
            } else {
                print!("{}", status_text(session.info(), &status));
            }
        }
        Command::Set { setting: SetCommand::Mute { value: MuteValue::Toggle } } => {
            let muted = toggle_mute(session, TIMEOUT).map_err(|error| match error {
                SessionError::Rejected(reason) if reason.contains("doesn't say") => {
                    Failure::Invalid(format!("{reason}; use `earmuff set mute on` or `off` instead"))
                }
                error => Failure::Session(error),
            })?;
            println!("{}", if muted { "Mic muted" } else { "Mic on" });
        }
        Command::Set { setting } => {
            let settings = settings(setting, session.info()).map_err(Failure::Invalid)?;
            apply_settings(session, &settings, TIMEOUT)?;
        }
        Command::PlaySound { sound } => {
            apply_settings(session, &[Setting::PlayNotificationSound(*sound)], TIMEOUT)?;
        }
        Command::Monitor { raw } => match monitor(session, *raw) {
            // the reader went away (e.g. piped into head)
            Err(Failure::Output(error)) if error.kind() == io::ErrorKind::BrokenPipe => {}
            result => result?,
        },
        Command::List { .. } | Command::Waybar | Command::Sniff { .. } => unreachable!("handled in run()"),
    }
    Ok(())
}

fn read_battery(session: &mut dyn Session) -> Result<earmuff_domain::Status, SessionError> {
    let mut status = earmuff_domain::Status::default();
    session.query(Query::Connection)?;
    session.query(Query::Battery)?;
    earmuff_application::settle(session, &mut status, TIMEOUT)?;
    Ok(status)
}

fn monitor(session: &mut dyn Session, raw: bool) -> Result<(), Failure> {
    let mut out = io::stdout().lock();
    session.set_raw(raw);
    session.query(Query::All)?;
    loop {
        for event in session.poll(Duration::from_secs(3600))? {
            writeln!(out, "{}  {}", clock(), event_line(&event)).map_err(Failure::Output)?;
        }
    }
}

fn sniff(usb: UsbId) -> ExitCode {
    let mut sniffer = match Sniffer::open(Path::new(SYSFS_HIDRAW), usb) {
        Ok(sniffer) => sniffer,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            eprintln!("earmuff: can't read {error}; try `sudo earmuff sniff {usb}`");
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("earmuff: {error}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("earmuff: reading {} (Ctrl+C stops)", sniffer.names().join(", "));
    let mut out = io::stdout().lock();
    loop {
        let reports = match sniffer.next(Duration::from_secs(3600)) {
            Ok(reports) => reports,
            Err(error) => {
                eprintln!("earmuff: {error}");
                return ExitCode::FAILURE;
            }
        };
        for (node, report) in reports {
            let hex: Vec<String> = report.iter().map(|b| format!("{b:02x}")).collect();
            if writeln!(out, "{}  {node}  {}", clock(), hex.join(" ")).is_err() {
                // the reader went away (e.g. piped into head)
                return ExitCode::SUCCESS;
            }
        }
    }
}

fn clock() -> String {
    glib::DateTime::now_local().and_then(|now| now.format("%H:%M:%S")).map(String::from).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::{BackendKind, Capability, DeviceId, EqPreset, EqualizerSpec};

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("earmuff").chain(args.iter().copied()))
    }

    fn set(args: &[&str]) -> SetCommand {
        let mut full = vec!["set"];
        full.extend_from_slice(args);
        match parse(&full).unwrap().command {
            Some(Command::Set { setting }) => setting,
            other => panic!("not a set command: {other:?}"),
        }
    }

    fn info() -> DeviceInfo {
        DeviceInfo {
            id: DeviceId { backend: BackendKind::Demo, usb: UsbId::new(1, 2), instance: "x".into() },
            name: "Test".into(),
            vendor: "Test".into(),
            capabilities: vec![Capability::EqualizerPreset],
            equalizer: Some(EqualizerSpec {
                bands: 2,
                min: Gain::tenths(-60),
                max: Gain::tenths(60),
                step: Gain::tenths(10),
                baseline: Gain::tenths(0),
                presets: vec![
                    EqPreset { name: "Flat".into(), curve: vec![Gain::tenths(0); 2] },
                    EqPreset { name: "Bass boost".into(), curve: vec![Gain::tenths(40), Gain::tenths(0)] },
                ],
            }),
        }
    }

    #[test]
    fn no_command_opens_the_window() {
        let cli = parse(&[]).unwrap();
        assert!(cli.command.is_none() && !cli.demo && cli.device.is_none());
    }

    #[test]
    fn hidden_starts_the_window_in_the_tray() {
        let cli = parse(&["--hidden"]).unwrap();
        assert!(cli.hidden && cli.command.is_none());
    }

    #[test]
    fn global_options_work_before_or_after_the_command() {
        assert!(parse(&["--demo", "status"]).unwrap().demo);
        assert!(parse(&["status", "--demo"]).unwrap().demo);
        assert_eq!(parse(&["-d", "03f0:098d", "status"]).unwrap().device, Some(UsbId::new(0x03F0, 0x098D)));
        assert_eq!(parse(&["status", "--device", "0x1038:0x12ad"]).unwrap().device, Some(UsbId::new(0x1038, 0x12AD)));
        assert!(parse(&["-d", "headset", "status"]).is_err());
    }

    #[test]
    fn sidetone_accepts_on_off_and_percentages() {
        assert_eq!(parse_sidetone("on"), Ok(Sidetone::On));
        assert_eq!(parse_sidetone("off"), Ok(Sidetone::Off));
        assert_eq!(parse_sidetone("40%"), Ok(Sidetone::Level(Percent::new(40).unwrap())));
        assert!(parse_sidetone("101").is_err());
        assert!(parse(&["set", "sidetone", "loud"]).is_err());
    }

    #[test]
    fn values_are_checked_while_parsing() {
        assert!(parse(&["set", "auto-off", "300"]).is_err());
        assert!(parse(&["set", "mute", "maybe"]).is_err());
        assert!(parse(&["set", "mute-led", "4"]).is_err());
        assert!(parse(&["set", "noise-filter", "max"]).is_err());
        assert!(parse(&["set", "mic-volume", "120%"]).is_err());
        assert!(parse(&["set", "eq", "1,loud"]).is_err());
    }

    #[test]
    fn set_commands_map_to_settings() {
        let info = info();
        let level = Percent::new(40).unwrap();
        let cases: Vec<(&[&str], Vec<Setting>)> = vec![
            (&["sidetone", "40%"], vec![Setting::Sidetone(true), Setting::SidetoneLevel(level)]),
            (&["sidetone", "off"], vec![Setting::Sidetone(false)]),
            (&["mic-monitoring", "off"], vec![Setting::Sidetone(false)]),
            (&["auto-off", "0"], vec![Setting::AutoOff(AutoOff::NEVER)]),
            (&["mute", "on"], vec![Setting::MicMuted(true)]),
            (&["mute", "off"], vec![Setting::MicMuted(false)]),
            (&["lights", "off"], vec![Setting::Lights(false)]),
            (&["rotate-to-mute", "on"], vec![Setting::RotateToMute(true)]),
            (&["mic-volume", "40"], vec![Setting::MicVolume(level)]),
            (&["mute-led", "2"], vec![Setting::MicMuteLedBrightness(2)]),
            (&["noise-filter", "high"], vec![Setting::NoiseFilter(NoiseFilter::High)]),
            (&["volume-limiter", "on"], vec![Setting::VolumeLimiter(true)]),
            (&["bluetooth-on-power", "off"], vec![Setting::BtWhenPoweredOn(false)]),
            (&["bluetooth-call-volume", "40%"], vec![Setting::BtCallVolume(level)]),
            (&["eq-preset", "1"], vec![Setting::EqualizerPreset(1)]),
            (&["eq-preset", "bass BOOST"], vec![Setting::EqualizerPreset(1)]),
            (&["eq", "-3,2.5"], vec![Setting::Equalizer(vec![Gain::tenths(-30), Gain::tenths(25)])]),
        ];
        for (args, expected) in cases {
            assert_eq!(settings(&set(args), &info), Ok(expected), "{args:?}");
        }
    }

    #[test]
    fn unknown_presets_list_the_real_ones() {
        assert_eq!(
            find_preset(&info(), "loud"),
            Err("no preset called \"loud\"; this headset has: Flat, Bass boost".into())
        );
    }
}
