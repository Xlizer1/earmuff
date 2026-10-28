//! The PipeWire adapter against scripted stand-ins for `pw-dump` and `wpctl`, so the
//! tests never change the computer's real sound settings.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use earmuff_application::{AudioSystem, MicRecorder, Recording, SoftwareEqualizer, Switch};
use earmuff_domain::{Gain, UsbId};
use earmuff_infrastructure::pipewire::PipeWire;
use earmuff_infrastructure::pipewire::equalizer::PipeWireEqualizer;

const DUMP: &str = include_str!("data/pw-dump.json");
const CLOUD_ALPHA: UsbId = UsbId::new(0x03F0, 0x098D);

/// One test at a time: a script one test is still writing can't be started from
/// another test's fork of this process ("text file busy")
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A PipeWire whose pw-dump prints `dump` and whose wpctl logs its arguments
fn pipewire(dir: &Path, dump: &str) -> (PipeWire, PathBuf) {
    fs::write(dir.join("dump.json"), dump).unwrap();
    let log = dir.join("wpctl.log");
    let dump = script(dir, "pw-dump", &format!("cat '{}'", dir.join("dump.json").display()));
    let wpctl = script(dir, "wpctl", &format!("echo \"$@\" >> '{}'", log.display()));
    (PipeWire::with_programs(dump, wpctl), log)
}

#[test]
fn makes_the_headset_the_default_output_and_input() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let (pipewire, log) = pipewire(dir.path(), DUMP);
    assert_eq!(
        pipewire.use_headset(CLOUD_ALPHA),
        Ok(Switch::Switched("HyperX Cloud Alpha Wireless Analog Stereo".into()))
    );
    assert_eq!(fs::read_to_string(log).unwrap(), "set-default 57\nset-default 58\n");
}

#[test]
fn leaves_it_alone_when_it_already_is() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    // the same, with the headset already the default
    let mut objects: Vec<serde_json::Value> = serde_json::from_str(DUMP).unwrap();
    let metadata = objects.iter_mut().find(|o| o["type"] == "PipeWire:Interface:Metadata").unwrap();
    metadata["metadata"][0]["value"]["name"] =
        "alsa_output.usb-HP__Inc_HyperX_Cloud_Alpha_Wireless_00000001-00.analog-stereo".into();
    metadata["metadata"][1]["value"]["name"] =
        "alsa_input.usb-HP__Inc_HyperX_Cloud_Alpha_Wireless_00000001-00.mono-fallback".into();
    let dump = serde_json::to_string(&objects).unwrap();
    let (pipewire, log) = pipewire(dir.path(), &dump);
    assert_eq!(pipewire.use_headset(CLOUD_ALPHA), Ok(Switch::Already));
    assert!(!log.exists());
}

#[test]
fn a_headset_without_a_sound_card_is_not_found() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let (pipewire, log) = pipewire(dir.path(), DUMP);
    assert_eq!(pipewire.use_headset(UsbId::new(0x1234, 0x5678)), Ok(Switch::NotFound));
    assert!(!log.exists());
}

#[test]
fn says_so_when_pipewire_is_not_there() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let pipewire = PipeWire::with_programs(dir.path().join("no-pw-dump"), dir.path().join("no-wpctl"));
    assert!(pipewire.use_headset(CLOUD_ALPHA).unwrap_err().contains("pw-dump"));
}

/// A PipeWire whose pw-record "records" a steady tone of 'y\n' bytes and whose pw-play
/// keeps what it's given
fn recorder(dir: &Path) -> PipeWire {
    let (pipewire, _) = pipewire(dir, DUMP);
    let record =
        script(dir, "pw-record", &format!("echo \"$@\" > '{}'\nyes | head -c 9600", dir.join("record.args").display()));
    let play = script(
        dir,
        "pw-play",
        &format!("echo \"$@\" > '{}'\ncat > '{}'", dir.join("play.args").display(), dir.join("played.raw").display()),
    );
    pipewire.with_recorder(record, play)
}

#[test]
fn records_from_the_headsets_mic_with_a_live_level() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let pipewire = recorder(dir.path());
    let mut levels = Vec::new();
    let recording = pipewire.record(CLOUD_ALPHA, Duration::from_millis(100), &mut |level| levels.push(level)).unwrap();
    assert_eq!((recording.rate, recording.samples.len()), (48_000, 4800));
    // 'y' and '\n' make 0x0a79, about -22 dB
    assert!(recording.samples.iter().all(|s| *s == 0x0a79));
    assert!(!levels.is_empty() && levels.iter().all(|level| (0.6..0.7).contains(level)), "{levels:?}");
    let args = fs::read_to_string(dir.path().join("record.args")).unwrap();
    assert!(
        args.contains("--target alsa_input.usb-HP__Inc_HyperX_Cloud_Alpha_Wireless_00000001-00.mono-fallback"),
        "{args}"
    );
    assert!(args.contains("--sample-count 4800"), "{args}");
}

#[test]
fn plays_back_on_the_headsets_speakers() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let pipewire = recorder(dir.path());
    pipewire.play(CLOUD_ALPHA, &Recording { rate: 48_000, samples: vec![1, -1, 300] }).unwrap();
    assert_eq!(fs::read(dir.path().join("played.raw")).unwrap(), [1, 0, 0xFF, 0xFF, 0x2C, 0x01]);
    let args = fs::read_to_string(dir.path().join("play.args")).unwrap();
    assert!(
        args.contains("--target alsa_output.usb-HP__Inc_HyperX_Cloud_Alpha_Wireless_00000001-00.analog-stereo"),
        "{args}"
    );
}

#[test]
fn no_mic_test_without_the_headsets_sound_card() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let pipewire = recorder(dir.path());
    let error = pipewire.record(UsbId::new(0x1234, 0x5678), Duration::from_millis(100), &mut |_| {}).unwrap_err();
    assert_eq!(error, "the computer has no mic for this headset");
}

const HEADSET_SINK: &str = "alsa_output.usb-HP__Inc_HyperX_Cloud_Alpha_Wireless_00000001-00.analog-stereo";

/// The fixture with a different default sink
fn with_default_sink(objects: &mut [serde_json::Value], sink: &str) {
    let metadata = objects.iter_mut().find(|o| o["type"] == "PipeWire:Interface:Metadata").unwrap();
    metadata["metadata"][0]["value"]["name"] = sink.into();
}

/// Stand-ins for everything the equalizer runs. The "pipewire" it starts adds the
/// equalizer's sink to what pw-dump shows until it's told to stop, and the default
/// sink moves to it the way wpctl would move it.
fn equalizer(dir: &Path) -> PipeWireEqualizer {
    let mut before: Vec<serde_json::Value> = serde_json::from_str(DUMP).unwrap();
    with_default_sink(&mut before, HEADSET_SINK);
    let mut after = before.clone();
    after.push(serde_json::json!({"id": 200, "type": "PipeWire:Interface:Node", "info": {"props": {
        "media.class": "Audio/Sink", "node.name": "earmuff_eq_03f0_098d", "client.id": 300}}}));
    after.push(serde_json::json!({"id": 300, "type": "PipeWire:Interface:Client",
        "info": {"props": {"application.process.id": "DAEMON_PID"}}}));
    with_default_sink(&mut after, "earmuff_eq_03f0_098d");
    fs::write(dir.join("dump.json"), serde_json::to_string(&before).unwrap()).unwrap();
    fs::write(dir.join("dump-eq.template"), serde_json::to_string(&after).unwrap()).unwrap();
    let d = dir.display();
    let dump = script(
        dir,
        "pw-dump",
        &format!("if [ -e '{d}/running' ]; then cat '{d}/dump-eq.json'; else cat '{d}/dump.json'; fi"),
    );
    let wpctl = script(dir, "wpctl", &format!("echo \"$@\" >> '{d}/wpctl.log'"));
    let cli = script(dir, "pw-cli", &format!("echo \"$@\" >> '{d}/cli.log'"));
    let daemon = script(
        dir,
        "pipewire",
        &format!(
            "echo \"$@\" > '{d}/daemon.args'\n\
             sed 's/\"DAEMON_PID\"/'$$'/' '{d}/dump-eq.template' > '{d}/dump-eq.json'\n\
             touch '{d}/running'\n\
             trap \"rm -f '{d}/running'; exit 0\" TERM\n\
             while :; do sleep 0.05; done"
        ),
    );
    PipeWireEqualizer::with_programs(PipeWire::with_programs(dump, wpctl), daemon, cli, &dir.join("conf"))
}

#[test]
fn an_equalizer_starts_in_front_of_the_headset_retunes_and_stops() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let eq = equalizer(dir.path());
    let read = |name: &str| fs::read_to_string(dir.path().join(name)).unwrap_or_default();
    let flat = vec![Gain::tenths(0); 10];
    assert_eq!(eq.apply(CLOUD_ALPHA, "HyperX Cloud Alpha Wireless", &flat), Ok(true));
    let conf = dir.path().join("conf/eq-03f0-098d.conf");
    assert_eq!(read("daemon.args").trim(), format!("-c {}", conf.display()));
    assert!(fs::read_to_string(&conf).unwrap().contains(&format!("target.object = \"{HEADSET_SINK}\"")));
    // the headset was the computer's output, so its equalizer is now
    assert_eq!(read("wpctl.log"), "set-default 200\n");

    let bass: Vec<Gain> = [60, 40, 20, 0, 0, 0, 0, 0, 0, 0].map(Gain::tenths).to_vec();
    assert_eq!(eq.apply(CLOUD_ALPHA, "HyperX Cloud Alpha Wireless", &bass), Ok(true));
    assert!(
        read("cli.log").starts_with("set-param 200 Props { params = [ \"eq_band_1:Gain\" 6.0 "),
        "{}",
        read("cli.log")
    );

    assert_eq!(eq.stop(CLOUD_ALPHA), Ok(()));
    assert!(!dir.path().join("running").exists(), "the equalizer's process was stopped");
    assert_eq!(read("wpctl.log"), "set-default 200\nset-default 57\n");
}

#[test]
fn no_equalizer_without_the_headsets_speakers() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let eq = equalizer(dir.path());
    assert_eq!(eq.apply(UsbId::new(0x1234, 0x5678), "Other", &[Gain::tenths(0); 10]), Ok(false));
    assert!(!dir.path().join("daemon.args").exists());
}

/// The real PipeWire, with only wpctl replaced so the computer's sound output stays as it is
#[test]
#[ignore = "needs PipeWire and a Cloud Alpha Wireless"]
fn a_real_equalizer_runs_and_retunes() {
    let _serial = one_at_a_time();
    let dir = tempfile::tempdir().unwrap();
    let wpctl = script(dir.path(), "wpctl", "true");
    let pipewire = PipeWire::with_programs("pw-dump", wpctl);
    let eq = PipeWireEqualizer::with_programs(pipewire, "pipewire", "pw-cli", dir.path());
    let gains = |dump: &str| -> Vec<f64> {
        let objects: Vec<serde_json::Value> = serde_json::from_str(dump).unwrap();
        let node = objects.iter().find(|o| o["info"]["props"]["node.name"] == "earmuff_eq_03f0_098d").unwrap();
        let props = node["info"]["params"]["Props"].as_array().unwrap();
        // the adapter's own params come first; the filter's are further on
        let params: Vec<&serde_json::Value> = props.iter().filter_map(|p| p["params"].as_array()).flatten().collect();
        params
            .chunks(2)
            .filter(|kv| kv[0].as_str().is_some_and(|k| k.ends_with(":Gain")))
            .map(|kv| kv[1].as_f64().unwrap())
            .collect()
    };
    let dump = || String::from_utf8(std::process::Command::new("pw-dump").output().unwrap().stdout).unwrap();
    assert_eq!(eq.apply(CLOUD_ALPHA, "Cloud Alpha Wireless", &[Gain::tenths(0); 10]), Ok(true));
    let bass: Vec<Gain> = [60, 40, 20, 0, 0, 0, 0, 0, 0, -30].map(Gain::tenths).to_vec();
    assert_eq!(eq.apply(CLOUD_ALPHA, "Cloud Alpha Wireless", &bass), Ok(true));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(gains(&dump()), [6.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -3.0]);
    assert_eq!(eq.stop(CLOUD_ALPHA), Ok(()));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!dump().contains("earmuff_eq_03f0_098d"), "the equalizer is gone");
}
