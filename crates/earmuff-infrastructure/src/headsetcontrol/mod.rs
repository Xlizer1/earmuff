//! The backend for headsets without a native driver: it runs the HeadsetControl
//! program (https://github.com/Sapd/HeadsetControl) and reads its JSON output.
//!
//! HeadsetControl has no push events, so sessions poll it. Many of its settings can
//! only be written, never read back; those are reported as set once HeadsetControl
//! says the headset took them.

pub mod json;
mod runner;
mod session;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use earmuff_application::{Backend, BackendError, Session};
use earmuff_domain::{BackendKind, Capability, DeviceInfo, UsbId};

use crate::hidraw;

pub use runner::{Program, RunOutput, Runner};
pub use session::HeadsetControlSession;

#[derive(Default)]
struct Cache {
    /// The USB IDs of the hidraw devices when HeadsetControl last ran
    usb_ids: Option<Vec<UsbId>>,
    devices: Vec<DeviceInfo>,
}

pub struct HeadsetControlBackend {
    runner: Arc<dyn Runner>,
    /// Only runs HeadsetControl when the hidraw devices under this directory change
    sysfs: Option<PathBuf>,
    cache: Mutex<Cache>,
    poll_every: Duration,
}

impl HeadsetControlBackend {
    /// Uses the installed headsetcontrol, if there is one. HeadsetControl only runs
    /// again when a USB HID device is plugged in or out.
    pub fn installed() -> Option<Self> {
        Some(HeadsetControlBackend::new(Arc::new(Program::find()?)).watching(hidraw::SYSFS_HIDRAW))
    }

    /// Runs HeadsetControl through `runner` on every discovery
    pub fn new(runner: Arc<dyn Runner>) -> Self {
        HeadsetControlBackend { runner, sysfs: None, cache: Mutex::default(), poll_every: Duration::from_secs(5) }
    }

    /// Only runs HeadsetControl when the hidraw devices under `sysfs` change
    pub fn watching(mut self, sysfs: impl Into<PathBuf>) -> Self {
        self.sysfs = Some(sysfs.into());
        self
    }

    /// How often sessions ask HeadsetControl for the battery and connection
    pub fn polling_every(mut self, interval: Duration) -> Self {
        self.poll_every = interval;
        self
    }

    fn run_discovery(&self) -> Result<Vec<DeviceInfo>, BackendError> {
        let output = self
            .runner
            .run(&["-o".into(), "json".into()])
            .map_err(|e| BackendError::Failed(format!("headsetcontrol: {e}")))?;
        let report = json::parse(&output.stdout).map_err(BackendError::Failed)?;
        if !json::api_supported(&report.api_version) {
            return Err(BackendError::Failed(format!(
                "HeadsetControl speaks JSON API {}; Earmuff needs 1.x",
                report.api_version
            )));
        }
        Ok(report.devices.iter().map(json::device_info).collect())
    }
}

impl Backend for HeadsetControlBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::HeadsetControl
    }

    fn discover(&self) -> Result<Vec<DeviceInfo>, BackendError> {
        let Some(sysfs) = &self.sysfs else {
            return self.run_discovery();
        };
        let mut usb_ids: Vec<UsbId> = hidraw::scan(sysfs).into_iter().map(|node| node.usb).collect();
        usb_ids.sort();
        usb_ids.dedup();
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.usb_ids.as_ref() != Some(&usb_ids) {
            let devices = self.run_discovery()?;
            cache.devices = devices.into_iter().filter(|device| usb_ids.contains(&device.id.usb)).collect();
            cache.usb_ids = Some(usb_ids);
        }
        Ok(cache.devices.clone())
    }

    fn open(&self, device: &DeviceInfo) -> Result<Box<dyn Session>, BackendError> {
        // the game/chat dial changes by hand, so headsets with one are polled faster
        let poll_every = if device.supports(Capability::ChatMix) {
            self.poll_every.min(Duration::from_secs(1))
        } else {
            self.poll_every
        };
        Ok(Box::new(HeadsetControlSession::new(device.clone(), self.runner.clone(), poll_every)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Answers every run with the same output, and counts runs
    struct Canned {
        stdout: String,
        runs: AtomicUsize,
    }

    impl Runner for Canned {
        fn run(&self, _args: &[String]) -> std::io::Result<RunOutput> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(RunOutput { success: true, stdout: self.stdout.clone(), stderr: String::new() })
        }
    }

    fn canned(stdout: &str) -> Arc<Canned> {
        Arc::new(Canned { stdout: stdout.into(), runs: AtomicUsize::new(0) })
    }

    #[test]
    fn discovers_every_device_headsetcontrol_lists() {
        let runner = canned(json::tests::SAMPLE);
        let devices = HeadsetControlBackend::new(runner).discover().unwrap();
        let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["HeadsetControl Test device", "HyperX Cloud Alpha Wireless"]);
    }

    #[test]
    fn refuses_an_unknown_api_version() {
        let runner = canned(r#"{"api_version": "2.0", "devices": []}"#);
        let error = HeadsetControlBackend::new(runner).discover().unwrap_err();
        assert_eq!(error, BackendError::Failed("HeadsetControl speaks JSON API 2.0; Earmuff needs 1.x".into()));
    }

    #[test]
    fn runs_again_only_when_hid_devices_change() {
        let dir = tempfile::tempdir().unwrap();
        let sysfs = dir.path();
        let node = sysfs.join("hidraw3/device");
        std::fs::create_dir_all(&node).unwrap();
        std::fs::write(node.join("uevent"), "HID_ID=0003:000003F0:0000098D\n").unwrap();
        let runner = canned(json::tests::SAMPLE);
        let backend = HeadsetControlBackend::new(runner.clone()).watching(sysfs);
        let devices = backend.discover().unwrap();
        // only the plugged-in headset is listed, not HeadsetControl's test device
        assert_eq!(devices.iter().map(|d| d.id.usb).collect::<Vec<_>>(), [UsbId::new(0x03F0, 0x098D)]);
        backend.discover().unwrap();
        assert_eq!(runner.runs.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(sysfs.join("hidraw3")).unwrap();
        assert_eq!(backend.discover().unwrap(), []);
        assert_eq!(runner.runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn polls_headsets_with_a_chatmix_dial_faster() {
        let runner = canned(json::tests::SAMPLE);
        let backend = HeadsetControlBackend::new(runner).polling_every(Duration::from_secs(5));
        let devices = backend.discover().unwrap();
        assert!(backend.open(&devices[0]).is_ok());
    }
}
