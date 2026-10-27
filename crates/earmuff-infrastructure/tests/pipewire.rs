//! The PipeWire adapter against scripted stand-ins for `pw-dump` and `wpctl`, so the
//! tests never change the computer's real sound settings.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use earmuff_application::{AudioSystem, Switch};
use earmuff_domain::UsbId;
use earmuff_infrastructure::pipewire::PipeWire;

const DUMP: &str = include_str!("data/pw-dump.json");
const CLOUD_ALPHA: UsbId = UsbId::new(0x03F0, 0x098D);

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
    let dir = tempfile::tempdir().unwrap();
    let (pipewire, log) = pipewire(dir.path(), DUMP);
    assert_eq!(pipewire.use_headset(UsbId::new(0x1234, 0x5678)), Ok(Switch::NotFound));
    assert!(!log.exists());
}

#[test]
fn says_so_when_pipewire_is_not_there() {
    let dir = tempfile::tempdir().unwrap();
    let pipewire = PipeWire::with_programs(dir.path().join("no-pw-dump"), dir.path().join("no-wpctl"));
    assert!(pipewire.use_headset(CLOUD_ALPHA).unwrap_err().contains("pw-dump"));
}
