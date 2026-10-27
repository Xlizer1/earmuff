//! Command line: `earmuff` alone opens the window, subcommands work from scripts.

use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use gtk::glib;
use serde_json::json;

use crate::device::{wait_readable, Headset};
use crate::protocol::{self as p, cmd, ChargeState, State};

const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(name = "earmuff", about = "HyperX Cloud Alpha Wireless settings. Without a command, opens the window.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Battery and all settings
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Mic monitoring: on, off or a level -4..18
    Sidetone {
        #[arg(allow_negative_numbers = true, value_parser = parse_sidetone)]
        value: Sidetone,
    },
    /// Minutes of no use before the headset turns off (0 = never)
    AutoOff { minutes: u8 },
    /// Spoken alerts from the headset
    VoicePrompts { value: OnOff },
    /// Mute the microphone
    Mute { value: OnOff },
    /// One line of JSON for a Waybar custom module
    Waybar,
    /// Print changes as the headset reports them
    Monitor {
        /// Also print every report in hex (handy for decoding new ones)
        #[arg(long)]
        raw: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

impl From<OnOff> for bool {
    fn from(value: OnOff) -> bool {
        matches!(value, OnOff::On)
    }
}

#[derive(Clone, Copy)]
pub enum Sidetone {
    On,
    Off,
    Level(i16),
}

fn parse_sidetone(value: &str) -> Result<Sidetone, String> {
    match value {
        "on" => Ok(Sidetone::On),
        "off" => Ok(Sidetone::Off),
        _ => match value.parse::<i16>() {
            Ok(level) if (p::SIDETONE_MIN..=p::SIDETONE_MAX).contains(&level) => Ok(Sidetone::Level(level)),
            _ => Err(format!("use on, off or a level {}..{}", p::SIDETONE_MIN, p::SIDETONE_MAX)),
        },
    }
}

pub fn run(command: Command) -> ExitCode {
    if let Command::Waybar = command {
        println!("{}", waybar());
        return ExitCode::SUCCESS;
    }
    let mut headset = match Headset::open() {
        Ok(headset) => headset,
        Err(e) => {
            eprintln!("earmuff: {e}");
            return ExitCode::FAILURE;
        }
    };
    match execute(&mut headset, command) {
        Ok(()) => ExitCode::SUCCESS,
        // the reader went away (e.g. piped into head)
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("earmuff: {e}");
            ExitCode::FAILURE
        }
    }
}

fn execute(headset: &mut Headset, command: Command) -> io::Result<()> {
    match command {
        Command::Status { json } => {
            read_status(headset)?;
            if json {
                println!("{}", status_json(&headset.state));
            } else {
                print_status(&headset.state);
            }
        }
        Command::Sidetone { value } => match value {
            Sidetone::Off => headset.set_sidetone(false)?,
            Sidetone::On => headset.set_sidetone(true)?,
            Sidetone::Level(level) => {
                headset.set_sidetone(true)?;
                headset.set_sidetone_level(level)?;
            }
        },
        Command::AutoOff { minutes } => headset.set_auto_off(minutes)?,
        Command::VoicePrompts { value } => headset.set_voice_prompts(value.into())?,
        Command::Mute { value } => headset.set_mute(value.into())?,
        Command::Monitor { raw } => monitor(headset, raw)?,
        Command::Waybar => unreachable!("handled in run()"),
    }
    headset.flush(FLUSH_TIMEOUT)
}

fn read_status(headset: &mut Headset) -> io::Result<()> {
    headset.request([p::get(cmd::GET_WIRELESS)])?;
    headset.flush(FLUSH_TIMEOUT)?;
    if headset.state.connected == Some(true) {
        headset.refresh()?;
        headset.flush(FLUSH_TIMEOUT)?;
    }
    Ok(())
}

fn print_status(s: &State) {
    if s.connected != Some(true) {
        println!("Headset      off (dongle plugged in)");
        return;
    }
    let show = |value: Option<String>| value.unwrap_or_else(|| "?".into());
    let yes_no = |value: Option<bool>, yes: &str, no: &str| show(value.map(|v| if v { yes } else { no }.into()));
    println!("Headset      connected ({})", show(s.color.map(|c| c.to_string())));
    println!("Battery      {}%, {}", show(s.battery.map(|b| b.to_string())), show(s.charging.map(|c| c.to_string())));
    println!("Mic          {}, {}", yes_no(s.mic_attached, "attached", "detached"), yes_no(s.muted, "muted", "not muted"));
    println!(
        "Monitoring   {}, level {} ({}..{})",
        yes_no(s.sidetone, "on", "off"),
        show(s.sidetone_level.map(|l| l.to_string())),
        p::SIDETONE_MIN,
        p::SIDETONE_MAX
    );
    println!(
        "Auto-off     {}",
        show(s.auto_off.map(|m| if m == 0 { "never".into() } else { format!("{m} min") }))
    );
    println!("Voice        {}", yes_no(s.voice_prompts, "on", "off"));
}

fn status_json(s: &State) -> serde_json::Value {
    json!({
        "connected": s.connected,
        "battery": s.battery,
        "charging": s.charging.map(|c| c.to_string()),
        "muted": s.muted,
        "mic_attached": s.mic_attached,
        "sidetone": s.sidetone,
        "sidetone_level": s.sidetone_level,
        "auto_off": s.auto_off,
        "voice_prompts": s.voice_prompts,
        "color": s.color.map(|c| c.to_string()),
    })
}

fn waybar() -> serde_json::Value {
    let hidden = json!({ "text": "" }); // empty text hides the module
    let Ok(mut headset) = Headset::open() else {
        return hidden;
    };
    let queried = headset
        .request([cmd::GET_WIRELESS, cmd::GET_BATTERY, cmd::GET_CHARGING].map(p::get))
        .and_then(|()| headset.flush(FLUSH_TIMEOUT));
    if queried.is_err() {
        return hidden;
    }
    let s = headset.state;
    if s.connected != Some(true) {
        return json!({ "text": "off", "tooltip": "Headset is off", "class": "off" });
    }
    let level = s.battery.unwrap_or(0);
    let class = match s.charging {
        Some(ChargeState::Charging | ChargeState::Full) => "charging",
        _ if level <= 15 => "low",
        _ => "ok",
    };
    let charging = s.charging.map_or("unknown".into(), |c| c.to_string());
    json!({
        "text": format!("{level}%"),
        "tooltip": format!("Cloud Alpha Wireless: {level}%, {charging}"),
        "class": class,
        "percentage": level,
    })
}

fn monitor(headset: &mut Headset, raw: bool) -> io::Result<()> {
    let mut out = io::stdout().lock();
    headset.refresh()?;
    loop {
        let busy = headset.pump()?;
        if !wait_readable(headset.fd(), busy.then_some(Duration::from_millis(50)))? {
            continue;
        }
        let mut lines = Vec::new();
        headset.read_with(|report, update| {
            let time = clock();
            if raw {
                let hex: Vec<String> = report.iter().map(|b| format!("{b:02x}")).collect();
                lines.push(format!("{time}  raw  {}", hex.join(" ")));
            }
            if let Some(update) = update {
                lines.push(format!("{time}  {update}"));
            }
        })?;
        for line in lines {
            writeln!(out, "{line}")?;
        }
    }
}

fn clock() -> String {
    glib::DateTime::now_local()
        .and_then(|now| now.format("%H:%M:%S"))
        .map(String::from)
        .unwrap_or_default()
}
