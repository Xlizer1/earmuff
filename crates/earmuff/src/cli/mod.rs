//! Command line: `earmuff` alone opens the window, subcommands work from scripts.

mod output;

use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use earmuff_application::{Backend, Session, SessionError, apply_settings, open_first, read_status};
use earmuff_domain::{AutoOff, Percent, Query, Setting};
use gtk::glib;

pub use output::{event_line, status_json, status_text, waybar_json};

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

    /// Use a simulated headset instead of real ones
    #[arg(long, global = true, hide = true)]
    pub demo: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Battery and all settings
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Mic monitoring: on, off, or a level from 0 to 100%
    Sidetone {
        #[arg(value_parser = parse_sidetone)]
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

/// The settings a command changes, in the order they are sent
pub fn settings(command: &Command) -> Vec<Setting> {
    match *command {
        Command::Sidetone { value: Sidetone::Off } => vec![Setting::Sidetone(false)],
        Command::Sidetone { value: Sidetone::On } => vec![Setting::Sidetone(true)],
        Command::Sidetone { value: Sidetone::Level(level) } => {
            vec![Setting::Sidetone(true), Setting::SidetoneLevel(level)]
        }
        Command::AutoOff { minutes } => vec![Setting::AutoOff(AutoOff::minutes(minutes))],
        Command::VoicePrompts { value } => vec![Setting::VoicePrompts(value.into())],
        Command::Mute { value } => vec![Setting::MicMuted(value.into())],
        Command::Status { .. } | Command::Waybar | Command::Monitor { .. } => vec![],
    }
}

pub fn run(command: Command, backends: &[Arc<dyn Backend>]) -> ExitCode {
    if let Command::Waybar = command {
        let status = open_first(backends).ok().and_then(|mut session| read_battery(session.as_mut()).ok());
        println!("{}", waybar_json(status.as_ref()));
        return ExitCode::SUCCESS;
    }
    let mut session = match open_first(backends) {
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

#[derive(Debug)]
enum Failure {
    Session(SessionError),
    Output(io::Error),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Failure::Session(error) => error.fmt(f),
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
        Command::Monitor { raw } => match monitor(session, *raw) {
            // the reader went away (e.g. piped into head)
            Err(Failure::Output(error)) if error.kind() == io::ErrorKind::BrokenPipe => {}
            result => result?,
        },
        Command::Waybar => unreachable!("handled in run()"),
        _ => {
            apply_settings(session, &settings(command), TIMEOUT)?;
        }
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

fn clock() -> String {
    glib::DateTime::now_local().and_then(|now| now.format("%H:%M:%S")).map(String::from).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("earmuff").chain(args.iter().copied()))
    }

    #[test]
    fn no_command_opens_the_window() {
        let cli = parse(&[]).unwrap();
        assert!(cli.command.is_none() && !cli.demo);
    }

    #[test]
    fn demo_works_before_or_after_the_command() {
        assert!(parse(&["--demo", "status"]).unwrap().demo);
        assert!(parse(&["status", "--demo"]).unwrap().demo);
    }

    #[test]
    fn sidetone_accepts_on_off_and_percentages() {
        assert_eq!(parse_sidetone("on"), Ok(Sidetone::On));
        assert_eq!(parse_sidetone("off"), Ok(Sidetone::Off));
        assert_eq!(parse_sidetone("40"), Ok(Sidetone::Level(Percent::new(40).unwrap())));
        assert_eq!(parse_sidetone("40%"), Ok(Sidetone::Level(Percent::new(40).unwrap())));
        assert!(parse_sidetone("101").is_err());
        assert!(parse_sidetone("-4").is_err());
        assert!(parse(&["sidetone", "loud"]).is_err());
    }

    #[test]
    fn auto_off_and_switches_validate_their_values() {
        assert!(parse(&["auto-off", "300"]).is_err());
        assert!(parse(&["mute", "maybe"]).is_err());
        assert!(matches!(parse(&["auto-off", "30"]).unwrap().command, Some(Command::AutoOff { minutes: 30 })));
    }

    #[test]
    fn commands_map_to_settings() {
        let level = Percent::new(40).unwrap();
        assert_eq!(
            settings(&Command::Sidetone { value: Sidetone::Level(level) }),
            [Setting::Sidetone(true), Setting::SidetoneLevel(level)]
        );
        assert_eq!(settings(&Command::Sidetone { value: Sidetone::Off }), [Setting::Sidetone(false)]);
        assert_eq!(settings(&Command::AutoOff { minutes: 0 }), [Setting::AutoOff(AutoOff::NEVER)]);
        assert_eq!(settings(&Command::Mute { value: OnOff::On }), [Setting::MicMuted(true)]);
        assert_eq!(settings(&Command::VoicePrompts { value: OnOff::Off }), [Setting::VoicePrompts(false)]);
        assert!(settings(&Command::Status { json: false }).is_empty());
    }
}
