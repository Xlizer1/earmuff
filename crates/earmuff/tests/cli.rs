//! End to end: the `earmuff` binary, run like a user would, against the simulated
//! headsets: a Cloud Alpha Wireless (03f0:098d) and the demo headset with every
//! feature (f00b:a00d).

use std::io::{BufRead, BufReader};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const FULL: &str = "f00b:a00d";

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
fn list_shows_every_headset() {
    let output = earmuff(&["list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "03f0:098d  Cloud Alpha Wireless (demo)  (demo)\nf00b:a00d  Earmuff Demo Headset  (demo)\n"
    );
    let json: serde_json::Value = serde_json::from_str(&stdout(&earmuff(&["list", "--json"]))).expect("valid JSON");
    assert_eq!(json[1]["usb_id"], FULL);
    assert!(json[1]["capabilities"].as_array().unwrap().iter().any(|c| c == "eq"));
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
fn status_of_the_headset_with_every_feature() {
    let output = earmuff(&["-d", FULL, "status"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    for line in [
        "Headset      Earmuff Demo Headset, connected (Graphite)",
        "Lights       on",
        "ChatMix      64 (even)",
        "Mic volume   80%",
        "Mute LED     2 of 3",
        "Noise filter low",
        "Call volume  70%",
        "EQ preset    Flat",
    ] {
        assert!(text.contains(line), "missing {line:?} in:\n{text}");
    }
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
    let full: serde_json::Value = serde_json::from_str(&stdout(&earmuff(&["-d", FULL, "status", "--json"]))).unwrap();
    assert_eq!(full["equalizer"].as_array().map(Vec::len), Some(10));
    assert_eq!(full["noise_filter"], "low");
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
    let cases: &[&[&str]] = &[
        &["set", "sidetone", "40%"],
        &["set", "sidetone", "off"],
        &["set", "auto-off", "30"],
        &["set", "mute", "on"],
        &["set", "voice-prompts", "off"],
        &["-d", FULL, "set", "lights", "off"],
        &["-d", FULL, "set", "rotate-to-mute", "off"],
        &["-d", FULL, "set", "mic-volume", "50%"],
        &["-d", FULL, "set", "mute-led", "1"],
        &["-d", FULL, "set", "noise-filter", "high"],
        &["-d", FULL, "set", "volume-limiter", "on"],
        &["-d", FULL, "set", "bluetooth-on-power", "off"],
        &["-d", FULL, "set", "bluetooth-call-volume", "30"],
        &["-d", FULL, "set", "eq-preset", "Bass boost"],
        &["-d", FULL, "set", "eq", "-3,-2,-1,0,1,2,3,2,1,0"],
        &["-d", FULL, "play-sound", "1"],
    ];
    for args in cases {
        let output = earmuff(args);
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        assert_eq!(stdout(&output), "", "{args:?} prints nothing");
    }
}

#[test]
fn refuses_what_the_headset_cannot_do() {
    for (args, reason) in [
        (&["set", "lights", "on"][..], "this headset has no lights setting"),
        (&["-d", FULL, "set", "eq", "1,2,3"], "this equalizer has 10 bands, not 3"),
        (&["-d", FULL, "set", "eq-preset", "Loud"], "no preset called \"Loud\""),
        (&["-d", "1234:5678", "status"], "headset 1234:5678 isn't plugged in"),
    ] {
        let output = earmuff(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(stderr(&output).contains(reason), "{args:?}: {}", stderr(&output));
    }
}

#[test]
fn invalid_values_are_rejected_with_a_reason() {
    for (args, reason) in [
        (&["set", "sidetone", "101"][..], "use on, off, or a level from 0 to 100%"),
        (&["set", "auto-off", "300"], "300 is not in 0..=255"),
        (&["set", "mute", "maybe"], "possible values: on, off"),
        (&["set", "noise-filter", "max"], "use off, low or high"),
        (&["set", "mute-led", "9"], "9 is not in 0..=3"),
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
    let help = stdout(&earmuff(&["--help"]));
    for command in ["list", "status", "set", "play-sound", "waybar", "monitor", "--device", "--demo"] {
        assert!(help.contains(command), "help mentions {command}");
    }
    let set_help = stdout(&earmuff(&["set", "--help"]));
    for setting in ["sidetone", "eq-preset", "noise-filter", "bluetooth-call-volume"] {
        assert!(set_help.contains(setting), "set help mentions {setting}");
    }
    assert_eq!(stdout(&earmuff(&["--version"])), format!("earmuff {}\n", env!("CARGO_PKG_VERSION")));
}
