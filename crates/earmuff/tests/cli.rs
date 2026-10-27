//! End to end: the `earmuff` binary, run like a user would, against the simulated headset.

use std::io::{BufRead, BufReader};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn earmuff(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_earmuff")).arg("--demo").args(args).output().expect("earmuff runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn status_lists_every_setting() {
    let output = earmuff(&["status"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "Headset      Cloud Alpha Wireless (demo), connected (Black/Red)\n\
         Battery      80%, discharging\n\
         Mic          attached, not muted\n\
         Monitoring   off, level 0%\n\
         Auto-off     20 min\n\
         Voice        on\n"
    );
}

#[test]
fn status_json_is_machine_readable() {
    let output = earmuff(&["status", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let status: serde_json::Value = serde_json::from_str(&stdout(&output)).expect("valid JSON");
    assert_eq!(status["device"], "Cloud Alpha Wireless (demo)");
    assert_eq!(status["usb_id"], "03f0:098d");
    assert_eq!(status["connected"], true);
    assert_eq!(status["battery"], 80);
    assert_eq!(status["sidetone_level"], 0);
    assert_eq!(status["auto_off"], 20);
}

#[test]
fn waybar_prints_one_line_of_json() {
    let output = earmuff(&["waybar"]);
    assert!(output.status.success());
    let text = stdout(&output);
    assert_eq!(text.lines().count(), 1);
    let module: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(module["text"], "80%");
    assert_eq!(module["class"], "ok");
    assert_eq!(module["percentage"], 80);
}

#[test]
fn settings_commands_succeed() {
    for args in [
        &["sidetone", "40%"][..],
        &["sidetone", "40"],
        &["sidetone", "on"],
        &["sidetone", "off"],
        &["auto-off", "30"],
        &["auto-off", "0"],
        &["mute", "on"],
        &["voice-prompts", "off"],
    ] {
        let output = earmuff(args);
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        assert_eq!(stdout(&output), "", "{args:?} prints nothing");
    }
}

#[test]
fn invalid_values_are_rejected_with_a_reason() {
    for (args, reason) in [
        (&["sidetone", "101"][..], "use on, off, or a level from 0 to 100%"),
        (&["sidetone", "loud"], "use on, off, or a level from 0 to 100%"),
        (&["auto-off", "300"], "300 is not in 0..=255"),
        (&["mute", "maybe"], "possible values: on, off"),
    ] {
        let output = earmuff(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(stderr(&output).contains(reason), "{args:?}: {}", stderr(&output));
    }
}

#[test]
fn monitor_streams_events_and_raw_reports() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_earmuff"))
        .args(["--demo", "monitor", "--raw"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("earmuff runs");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let (mut saw_event, mut saw_raw) = (false, false);
    let end = Instant::now() + Duration::from_secs(10);
    while !(saw_event && saw_raw) && Instant::now() < end {
        let Some(Ok(line)) = lines.next() else { break };
        // lines look like "19:19:57  battery = 80%"
        let (time, rest) = line.split_once("  ").expect("time, then the event");
        assert_eq!(time.len(), 8, "{line}");
        saw_event |= rest == "battery = 80%";
        saw_raw |= rest.starts_with("raw  21 bb 0b 50");
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(saw_event && saw_raw, "monitor printed the battery as an event and as a raw report");
}

#[test]
fn help_and_version() {
    let output = earmuff(&["--help"]);
    let help = stdout(&output);
    for command in ["status", "sidetone", "auto-off", "voice-prompts", "mute", "waybar", "monitor"] {
        assert!(help.contains(command), "help mentions {command}");
    }
    assert!(!help.contains("--demo"), "the demo flag stays hidden");
    assert_eq!(stdout(&earmuff(&["--version"])), format!("earmuff {}\n", env!("CARGO_PKG_VERSION")));
}
