//! Shared helpers for the window and tray end-to-end tests.
//!
//! Each test binary re-runs itself as a child process with its own headless display
//! (GTK's Broadway backend) and its own private D-Bus session, so no window appears on
//! the desktop and the user's real session bus is never touched. The child runs the
//! real app against a simulated headset and plays a scenario on it.

#![allow(dead_code)] // each test binary uses a different part of this module

use std::cell::RefCell;
use std::fs;
use std::future::Future;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use adw::prelude::*;
use earmuff::ui::UiOptions;
use earmuff_application::{
    AudioSystem, Backend, EqAction, ManagerOptions, MicRecorder, Recording, SoftwareEqualizer, Switch,
};
use earmuff_domain::{Gain, UsbId};
use earmuff_infrastructure::config::{
    CsvHistoryStore, JsonPreferencesStore, JsonProfileStore, JsonSoftwareEqStore, XdgAutostart,
};
use gtk::glib;

const CHILD: &str = "EARMUFF_E2E_CHILD";
const CHILD_TIMEOUT: Duration = Duration::from_secs(120);
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

pub type Outcome = Result<(), String>;

fn find_program(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?.to_str()?.split(':').map(|dir| Path::new(dir).join(name)).find(|path| path.is_file())
}

/// Kills a helper process when the test ends
struct Helper(Child);

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_broadway(runtime: &Path) -> Result<(Helper, u32), String> {
    let program = find_program("gtk4-broadwayd").ok_or("gtk4-broadwayd not found")?;
    let display = 50 + std::process::id() % 900;
    let child = Command::new(program)
        .arg(format!(":{display}"))
        .args(["--address", "127.0.0.1", "--port", &(20000 + display).to_string()])
        .env("XDG_RUNTIME_DIR", runtime)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("gtk4-broadwayd: {e}"))?;
    let helper = Helper(child);
    let end = Instant::now() + Duration::from_secs(10);
    while !fs::read_dir(runtime)
        .map_err(|e| e.to_string())?
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("broadway"))
    {
        if Instant::now() > end {
            return Err("gtk4-broadwayd did not start".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok((helper, display))
}

fn start_bus(dir: &Path) -> Result<(Helper, String), String> {
    let program = find_program("dbus-daemon").ok_or("dbus-daemon not found")?;
    let config = dir.join("bus.conf");
    fs::write(
        &config,
        format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}</listen>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
            dir.join("bus").display()
        ),
    )
    .map_err(|e| e.to_string())?;
    let mut child = Command::new(program)
        .arg(format!("--config-file={}", config.display()))
        .args(["--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("dbus-daemon: {e}"))?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    let helper = Helper(child);
    let mut address = String::new();
    BufReader::new(stdout).read_line(&mut address).map_err(|e| e.to_string())?;
    Ok((helper, address.trim().to_string()))
}

/// Runs `scenario` in an isolated copy of this process and reports its result
pub fn isolated(name: &str, scenario: impl FnOnce() -> Outcome) -> ExitCode {
    if std::env::var_os(CHILD).is_some() {
        return match scenario() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{name}: FAILED: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match run_child(name) {
        Ok(true) => {
            println!("{name}: ok");
            ExitCode::SUCCESS
        }
        Ok(false) => ExitCode::FAILURE,
        Err(skip) => {
            // a missing tool is a broken test machine, not a passing test
            eprintln!("{name}: cannot run: {skip}");
            ExitCode::FAILURE
        }
    }
}

fn run_child(name: &str) -> Result<bool, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let runtime = dir.path().join("runtime");
    fs::create_dir(&runtime).map_err(|e| e.to_string())?;
    fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700)).map_err(|e| e.to_string())?;
    let (_broadway, display) = start_broadway(&runtime)?;
    let (_bus, address) = start_bus(dir.path())?;
    let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .env(CHILD, "1")
        .env("GDK_BACKEND", "broadway")
        .env("BROADWAY_DISPLAY", format!(":{display}"))
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env("XDG_STATE_HOME", dir.path().join("state"))
        // nor the icons and fonts installed for the real Earmuff, nor the user's cache
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .env("XDG_CACHE_HOME", dir.path().join("cache"))
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .env("GSETTINGS_BACKEND", "memory")
        .env("GTK_A11Y", "none")
        .env("ADW_DISABLE_PORTAL", "1")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .spawn()
        .map_err(|e| e.to_string())?;
    let end = Instant::now() + CHILD_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status.success());
        }
        if Instant::now() > end {
            let _ = child.kill();
            eprintln!("{name}: FAILED: timed out");
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Stands in for the sound system, so tests never change the computer's sound settings
#[derive(Default)]
pub struct RecordingAudio(Mutex<Vec<UsbId>>);

impl AudioSystem for RecordingAudio {
    fn use_headset(&self, headset: UsbId) -> Result<Switch, String> {
        self.0.lock().unwrap().push(headset);
        Ok(Switch::Switched("Demo speakers".into()))
    }
}

static AUDIO: LazyLock<Arc<RecordingAudio>> = LazyLock::new(Arc::default);

/// The headsets the app has switched the sound to so far
pub fn audio_switches() -> Vec<UsbId> {
    AUDIO.0.lock().unwrap().clone()
}

/// Stands in for the mic: "records" a short sound with a rising level, and keeps track
/// of what it was asked to play
#[derive(Default)]
pub struct FakeMic(Mutex<Vec<(UsbId, Recording)>>);

impl MicRecorder for FakeMic {
    fn record(&self, _: UsbId, _: Duration, level: &mut dyn FnMut(f64)) -> Result<Recording, String> {
        for step in 1..=5 {
            level(f64::from(step) / 5.0);
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(Recording { rate: 48_000, samples: vec![100, -100, 50] })
    }

    fn play(&self, headset: UsbId, recording: &Recording) -> Result<(), String> {
        self.0.lock().unwrap().push((headset, recording.clone()));
        Ok(())
    }
}

static MIC: LazyLock<Arc<FakeMic>> = LazyLock::new(Arc::default);

/// What the mic test has played back so far, and on which headset
pub fn mic_playbacks() -> Vec<(UsbId, Recording)> {
    MIC.0.lock().unwrap().clone()
}

/// Stands in for PipeWire's equalizers, keeping track of what it was asked to do
#[derive(Default)]
pub struct FakeEqualizer(Mutex<Vec<EqAction>>);

impl SoftwareEqualizer for FakeEqualizer {
    fn apply(&self, headset: UsbId, name: &str, curve: &[Gain]) -> Result<bool, String> {
        self.0.lock().unwrap().push(EqAction::Apply { headset, name: name.into(), curve: curve.to_vec() });
        Ok(true)
    }

    fn stop(&self, headset: UsbId) -> Result<(), String> {
        self.0.lock().unwrap().push(EqAction::Stop(headset));
        Ok(())
    }
}

static EQUALIZER: LazyLock<Arc<FakeEqualizer>> = LazyLock::new(Arc::default);

/// What the app has asked of the software equalizer so far
pub fn equalizer_actions() -> Vec<EqAction> {
    EQUALIZER.0.lock().unwrap().clone()
}

/// Settings that make the app react fast enough for tests
pub fn options(tray: bool) -> UiOptions {
    UiOptions {
        app_id: "io.github.xlizer1.Earmuff.Tests".into(),
        manager: ManagerOptions { rescan: Duration::from_millis(50), battery_refresh: Duration::from_secs(60) },
        tray,
        demo: false,
        hidden: false,
        preferences: Arc::new(JsonPreferencesStore::in_config_dir()),
        autostart: Arc::new(XdgAutostart::for_this_program()),
        profiles: Arc::new(JsonProfileStore::in_config_dir()),
        history: Arc::new(CsvHistoryStore::in_state_dir()),
        audio: AUDIO.clone(),
        mic: MIC.clone(),
        equalizer: EQUALIZER.clone(),
        software_eq_store: Arc::new(JsonSoftwareEqStore::in_config_dir()),
    }
}

/// Runs the real app and plays `scenario` once its window is up. The scenario's last
/// step must make the app quit by itself (closing the window, or Quit in the tray).
pub fn run_app<F, Fut>(backends: Vec<Arc<dyn Backend>>, tray: bool, scenario: F) -> Outcome
where
    F: FnOnce(adw::Application) -> Fut + 'static,
    Fut: Future<Output = Outcome> + 'static,
{
    let app = earmuff::ui::application(backends, options(tray));
    let result: Rc<RefCell<Option<Outcome>>> = Rc::default();
    let scenario = RefCell::new(Some(scenario));
    {
        let result = result.clone();
        app.connect_activate(move |app| {
            let Some(scenario) = scenario.take() else { return };
            let (app, result) = (app.clone(), result.clone());
            glib::spawn_future_local(async move {
                let outcome = scenario(app.clone()).await;
                let failed = outcome.is_err();
                result.replace(Some(outcome));
                if failed {
                    app.quit();
                    return;
                }
                // the scenario ended with something that should quit the app
                glib::timeout_future(Duration::from_secs(10)).await;
                result.replace(Some(Err("the app did not quit".into())));
                app.quit();
            });
        });
    }
    let watchdog = {
        let (app, result) = (app.clone(), result.clone());
        glib::timeout_add_local_once(Duration::from_secs(90), move || {
            result.replace(Some(Err("the scenario took too long".into())));
            app.quit();
        })
    };
    app.run_with_args::<&str>(&[]);
    let outcome = result.take().unwrap_or_else(|| Err("the app quit before the scenario finished".into()));
    if outcome.is_ok() {
        watchdog.remove();
    }
    outcome
}

/// Waits until `check` passes, letting the app run meanwhile
pub async fn until(what: &str, check: impl Fn() -> bool) -> Outcome {
    let end = Instant::now() + STEP_TIMEOUT;
    while !check() {
        if Instant::now() > end {
            return Err(format!("timed out waiting for {what}"));
        }
        glib::timeout_future(Duration::from_millis(10)).await;
    }
    Ok(())
}

pub fn ensure(condition: bool, what: impl Into<String>) -> Outcome {
    if condition { Ok(()) } else { Err(what.into()) }
}

/// The app's window, once it exists
pub async fn window(app: &adw::Application) -> Result<gtk::Window, String> {
    until("the window", || !app.windows().is_empty()).await?;
    Ok(app.windows()[0].clone())
}

/// The widget named `name` inside `root`, as type `W`
pub fn find<W: IsA<gtk::Widget>>(root: &impl IsA<gtk::Widget>, name: &str) -> Result<W, String> {
    fn walk(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
        if widget.widget_name() == name {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = walk(&current, name) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    walk(root.upcast_ref(), name)
        .ok_or_else(|| format!("no widget named {name}"))?
        .downcast::<W>()
        .map_err(|_| format!("widget {name} has another type"))
}

/// The page the window shows: "missing", "off" or "settings"
pub fn page(window: &gtk::Window) -> String {
    find::<gtk::Stack>(window, "pages")
        .ok()
        .and_then(|stack| stack.visible_child_name())
        .map(|name| name.to_string())
        .unwrap_or_default()
}
