//! Earmuff's own files: preferences under $XDG_CONFIG_HOME/earmuff.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use earmuff_application::{Autostart, HistoryStore, Preferences, PreferencesStore, ProfileStore, Sample};
use earmuff_domain::{AutoOff, Gain, NoiseFilter, Percent, Setting, UsbId};
use serde_json::{Map, Value, json};

/// $XDG_CONFIG_HOME/earmuff, or ~/.config/earmuff
pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config").join("earmuff")
}

/// $XDG_STATE_HOME/earmuff, or ~/.local/state/earmuff
pub fn state_dir() -> PathBuf {
    xdg_dir("XDG_STATE_HOME", ".local/state").join("earmuff")
}

fn xdg_dir(variable: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(variable) {
        Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir),
        _ => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp")).join(fallback),
    }
}

/// Writes a file whole or not at all: a crash mid-write never leaves half a file
pub fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path)
}

/// Preferences as a JSON file. Unknown keys are kept, so an older Earmuff doesn't
/// erase what a newer one saved.
#[derive(Debug)]
pub struct JsonPreferencesStore {
    path: PathBuf,
}

impl JsonPreferencesStore {
    /// $XDG_CONFIG_HOME/earmuff/preferences.json
    pub fn in_config_dir() -> Self {
        JsonPreferencesStore::at(config_dir().join("preferences.json"))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        JsonPreferencesStore { path: path.into() }
    }

    fn read(&self) -> Map<String, Value> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()
    }
}

impl PreferencesStore for JsonPreferencesStore {
    fn load(&self) -> Preferences {
        let saved = self.read();
        let defaults = Preferences::default();
        let text = |key: &str, default: String| saved.get(key).and_then(Value::as_str).map_or(default, String::from);
        let flag = |key: &str, default: bool| saved.get(key).and_then(Value::as_bool).unwrap_or(default);
        let number = |key: &str, default: u8| {
            saved.get(key).and_then(Value::as_u64).and_then(|n| u8::try_from(n).ok()).unwrap_or(default)
        };
        Preferences {
            accent: text("accent", defaults.accent),
            battery_alert: number("battery_alert", defaults.battery_alert).min(100),
            notify_charged: flag("notify_charged", defaults.notify_charged),
            remember_settings: flag("remember_settings", defaults.remember_settings),
        }
    }

    fn save(&self, preferences: &Preferences) -> Result<(), String> {
        let mut saved = self.read();
        let current = json!({
            "accent": preferences.accent,
            "battery_alert": preferences.battery_alert,
            "notify_charged": preferences.notify_charged,
            "remember_settings": preferences.remember_settings,
        });
        saved.extend(current.as_object().cloned().unwrap_or_default());
        let text = serde_json::to_string_pretty(&Value::Object(saved)).map_err(|e| e.to_string())?;
        write_atomically(&self.path, text.as_bytes()).map_err(|e| format!("{}: {e}", self.path.display()))
    }
}

/// A remembered setting as a JSON key and value
fn setting_to_json(setting: &Setting) -> Option<(&'static str, Value)> {
    Some(match setting {
        Setting::Sidetone(v) => ("sidetone", json!(v)),
        Setting::SidetoneLevel(v) => ("sidetone_level", json!(v.get())),
        Setting::AutoOff(v) => ("auto_off", json!(v.get())),
        Setting::VoicePrompts(v) => ("voice_prompts", json!(v)),
        Setting::Lights(v) => ("lights", json!(v)),
        Setting::RotateToMute(v) => ("rotate_to_mute", json!(v)),
        Setting::MicVolume(v) => ("mic_volume", json!(v.get())),
        Setting::MicMuteLedBrightness(v) => ("mute_led", json!(v)),
        Setting::NoiseFilter(v) => ("noise_filter", json!(v.to_string())),
        Setting::VolumeLimiter(v) => ("volume_limiter", json!(v)),
        Setting::BtWhenPoweredOn(v) => ("bluetooth_on_power", json!(v)),
        Setting::BtCallVolume(v) => ("bluetooth_call_volume", json!(v.get())),
        Setting::EqualizerPreset(v) => ("eq_preset", json!(v)),
        Setting::Equalizer(curve) => ("eq", json!(curve.iter().map(|g| g.db()).collect::<Vec<_>>())),
        Setting::MicMuted(_) | Setting::PlayNotificationSound(_) => return None,
    })
}

/// The setting a JSON key and value stand for; None for anything unknown or invalid
fn setting_from_json(key: &str, value: &Value) -> Option<Setting> {
    let flag = || value.as_bool();
    let byte = || value.as_u64().and_then(|n| u8::try_from(n).ok());
    let percent = || byte().and_then(|n| Percent::new(n).ok());
    Some(match key {
        "sidetone" => Setting::Sidetone(flag()?),
        "sidetone_level" => Setting::SidetoneLevel(percent()?),
        "auto_off" => Setting::AutoOff(AutoOff::minutes(byte()?)),
        "voice_prompts" => Setting::VoicePrompts(flag()?),
        "lights" => Setting::Lights(flag()?),
        "rotate_to_mute" => Setting::RotateToMute(flag()?),
        "mic_volume" => Setting::MicVolume(percent()?),
        "mute_led" => Setting::MicMuteLedBrightness(byte().filter(|n| *n <= 3)?),
        "noise_filter" => Setting::NoiseFilter(value.as_str()?.parse::<NoiseFilter>().ok()?),
        "volume_limiter" => Setting::VolumeLimiter(flag()?),
        "bluetooth_on_power" => Setting::BtWhenPoweredOn(flag()?),
        "bluetooth_call_volume" => Setting::BtCallVolume(percent()?),
        "eq_preset" => Setting::EqualizerPreset(byte()?),
        "eq" => {
            Setting::Equalizer(value.as_array()?.iter().map(|g| g.as_f64().map(Gain::from_db)).collect::<Option<_>>()?)
        }
        _ => return None,
    })
}

/// Each headset model's remembered settings, keyed by USB ID
#[derive(Debug)]
pub struct JsonProfileStore {
    path: PathBuf,
}

impl JsonProfileStore {
    /// $XDG_CONFIG_HOME/earmuff/headsets.json
    pub fn in_config_dir() -> Self {
        JsonProfileStore::at(config_dir().join("headsets.json"))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        JsonProfileStore { path: path.into() }
    }

    fn read(&self) -> Map<String, Value> {
        fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()
    }
}

impl ProfileStore for JsonProfileStore {
    fn load(&self, headset: UsbId) -> Vec<Setting> {
        let all = self.read();
        let Some(saved) = all.get(&headset.to_string()).and_then(Value::as_object) else {
            return Vec::new();
        };
        saved.iter().filter_map(|(key, value)| setting_from_json(key, value)).collect()
    }

    fn save(&self, headset: UsbId, settings: &[Setting]) -> Result<(), String> {
        let mut all = self.read();
        let saved: Map<String, Value> =
            settings.iter().filter_map(setting_to_json).map(|(key, value)| (key.to_string(), value)).collect();
        all.insert(headset.to_string(), Value::Object(saved));
        let text = serde_json::to_string_pretty(&Value::Object(all)).map_err(|e| e.to_string())?;
        write_atomically(&self.path, text.as_bytes()).map_err(|e| format!("{}: {e}", self.path.display()))
    }
}

/// Battery readings as one CSV file per headset model (time,level,charging), under
/// $XDG_STATE_HOME/earmuff
#[derive(Debug)]
pub struct CsvHistoryStore {
    dir: PathBuf,
}

impl CsvHistoryStore {
    pub fn in_state_dir() -> Self {
        CsvHistoryStore::at(state_dir())
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        CsvHistoryStore { dir: dir.into() }
    }

    fn file(&self, headset: UsbId) -> PathBuf {
        self.dir.join(format!("battery-{:04x}-{:04x}.csv", headset.vendor, headset.product))
    }
}

fn parse_sample(line: &str) -> Option<Sample> {
    let mut fields = line.split(',');
    let sample = Sample {
        at: fields.next()?.trim().parse().ok()?,
        level: fields.next()?.trim().parse::<u8>().ok().filter(|level| *level <= 100)?,
        charging: fields.next()?.trim() == "1",
    };
    fields.next().is_none().then_some(sample)
}

fn sample_line(sample: &Sample) -> String {
    format!("{},{},{}\n", sample.at, sample.level, u8::from(sample.charging))
}

impl HistoryStore for CsvHistoryStore {
    fn load(&self, headset: UsbId, since: u64) -> Vec<Sample> {
        let path = self.file(headset);
        let Ok(text) = fs::read_to_string(&path) else { return Vec::new() };
        let all: Vec<Sample> = text.lines().filter_map(parse_sample).collect();
        let kept: Vec<Sample> = all.iter().filter(|s| s.at >= since).copied().collect();
        // drop old readings from the file once they make up half of it
        if kept.len() * 2 < all.len() {
            let text: String = kept.iter().map(sample_line).collect();
            let _ = write_atomically(&path, text.as_bytes());
        }
        kept
    }

    fn append(&self, headset: UsbId, sample: Sample) -> Result<(), String> {
        use std::io::Write as _;
        let path = self.file(headset);
        let error = |e: io::Error| format!("{}: {e}", path.display());
        fs::create_dir_all(&self.dir).map_err(error)?;
        let mut file = fs::OpenOptions::new().create(true).append(true).open(&path).map_err(error)?;
        file.write_all(sample_line(&sample).as_bytes()).map_err(error)
    }
}

/// Start-on-login through an XDG autostart entry, which GNOME, KDE and most desktops run
#[derive(Debug)]
pub struct XdgAutostart {
    entry: PathBuf,
    program: PathBuf,
}

impl XdgAutostart {
    /// $XDG_CONFIG_HOME/autostart, starting this very program
    pub fn for_this_program() -> Self {
        let program = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("earmuff"));
        XdgAutostart::new(xdg_dir("XDG_CONFIG_HOME", ".config").join("autostart"), program)
    }

    pub fn new(dir: impl Into<PathBuf>, program: impl Into<PathBuf>) -> Self {
        XdgAutostart { entry: dir.into().join("io.github.xlizer1.Earmuff.desktop"), program: program.into() }
    }

    fn entry_text(&self) -> String {
        // quoted, as the desktop entry spec allows, in case the path has spaces
        let program = self.program.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            "[Desktop Entry]\nType=Application\nName=Earmuff\nComment=Battery and settings for your gaming headset\n\
             Exec=\"{program}\" --hidden\nIcon=io.github.xlizer1.Earmuff\nTerminal=false\nNoDisplay=true\n\
             X-GNOME-Autostart-enabled=true\n"
        )
    }
}

impl Autostart for XdgAutostart {
    fn is_enabled(&self) -> bool {
        self.entry.is_file()
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let result = if enabled {
            write_atomically(&self.entry, self.entry_text().as_bytes())
        } else {
            match fs::remove_file(&self.entry) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        };
        result.map_err(|e| format!("{}: {e}", self.entry.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, JsonPreferencesStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonPreferencesStore::at(dir.path().join("earmuff/preferences.json"));
        (dir, store)
    }

    #[test]
    fn defaults_until_something_is_saved() {
        let (_dir, store) = store();
        assert_eq!(store.load(), Preferences::default());
    }

    #[test]
    fn saves_and_loads() {
        let (_dir, store) = store();
        let saved =
            Preferences { accent: "cyan".into(), battery_alert: 15, notify_charged: false, remember_settings: false };
        store.save(&saved).unwrap();
        assert_eq!(store.load(), saved);
    }

    #[test]
    fn unreadable_files_fall_back_to_defaults() {
        let (_dir, store) = store();
        write_atomically(&store.path, b"{ not json").unwrap();
        assert_eq!(store.load(), Preferences::default());
        write_atomically(&store.path, b"{\"accent\": 7}").unwrap();
        assert_eq!(store.load(), Preferences::default());
    }

    #[test]
    fn keeps_keys_it_does_not_know() {
        let (_dir, store) = store();
        write_atomically(&store.path, b"{\"from_the_future\": true}").unwrap();
        store.save(&Preferences::default()).unwrap();
        let text = fs::read_to_string(&store.path).unwrap();
        assert!(text.contains("from_the_future") && text.contains("accent"));
    }

    #[test]
    fn writes_leave_no_temporary_files() {
        let (dir, store) = store();
        store.save(&Preferences::default()).unwrap();
        let files: Vec<_> =
            fs::read_dir(dir.path().join("earmuff")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(files, ["preferences.json"]);
    }

    #[test]
    fn autostart_writes_and_removes_its_entry() {
        let dir = tempfile::tempdir().unwrap();
        let autostart = XdgAutostart::new(dir.path().join("autostart"), "/home/me/My Apps/earmuff");
        assert!(!autostart.is_enabled());
        autostart.set_enabled(true).unwrap();
        assert!(autostart.is_enabled());
        let entry = fs::read_to_string(dir.path().join("autostart/io.github.xlizer1.Earmuff.desktop")).unwrap();
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("Exec=\"/home/me/My Apps/earmuff\" --hidden\n"), "{entry}");
        autostart.set_enabled(false).unwrap();
        assert!(!autostart.is_enabled());
        autostart.set_enabled(false).unwrap(); // already off
    }

    fn every_remembered_setting() -> Vec<Setting> {
        vec![
            Setting::Sidetone(true),
            Setting::SidetoneLevel(Percent::saturating(40)),
            Setting::AutoOff(AutoOff::minutes(45)),
            Setting::VoicePrompts(false),
            Setting::Lights(true),
            Setting::RotateToMute(false),
            Setting::MicVolume(Percent::saturating(70)),
            Setting::MicMuteLedBrightness(2),
            Setting::NoiseFilter(NoiseFilter::High),
            Setting::VolumeLimiter(true),
            Setting::BtWhenPoweredOn(false),
            Setting::BtCallVolume(Percent::saturating(55)),
            Setting::EqualizerPreset(3),
            Setting::Equalizer(vec![Gain::tenths(25), Gain::tenths(-30), Gain::tenths(0)]),
        ]
    }

    #[test]
    fn profiles_keep_every_remembered_setting() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonProfileStore::at(dir.path().join("headsets.json"));
        let headset = UsbId::new(0x03F0, 0x098D);
        store.save(headset, &every_remembered_setting()).unwrap();
        assert_eq!(store.load(headset), every_remembered_setting());
        // momentary things aren't kept
        store.save(headset, &[Setting::MicMuted(true), Setting::Lights(false)]).unwrap();
        assert_eq!(store.load(headset), [Setting::Lights(false)]);
    }

    #[test]
    fn profiles_are_kept_per_headset() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonProfileStore::at(dir.path().join("headsets.json"));
        let (a, b) = (UsbId::new(1, 1), UsbId::new(2, 2));
        store.save(a, &[Setting::Lights(true)]).unwrap();
        store.save(b, &[Setting::Lights(false)]).unwrap();
        assert_eq!(store.load(a), [Setting::Lights(true)]);
        assert_eq!(store.load(b), [Setting::Lights(false)]);
        assert_eq!(store.load(UsbId::new(3, 3)), []);
    }

    #[test]
    fn profiles_skip_unknown_or_invalid_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("headsets.json");
        let text =
            r#"{"0001:0002": {"lights": true, "mic_volume": 200, "mute_led": 9, "noise_filter": "max", "rgb": 1}}"#;
        write_atomically(&path, text.as_bytes()).unwrap();
        assert_eq!(JsonProfileStore::at(path).load(UsbId::new(1, 2)), [Setting::Lights(true)]);
    }

    #[test]
    fn history_appends_and_loads_readings() {
        let dir = tempfile::tempdir().unwrap();
        let store = CsvHistoryStore::at(dir.path().join("state"));
        let headset = UsbId::new(0x03F0, 0x098D);
        let samples = [Sample { at: 100, level: 80, charging: false }, Sample { at: 400, level: 79, charging: true }];
        for sample in samples {
            store.append(headset, sample).unwrap();
        }
        assert_eq!(store.load(headset, 0), samples);
        assert_eq!(store.load(headset, 200), samples[1..]);
        assert!(dir.path().join("state/battery-03f0-098d.csv").is_file());
        assert_eq!(store.load(UsbId::new(1, 1), 0), []);
    }

    #[test]
    fn history_drops_old_readings_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = CsvHistoryStore::at(dir.path());
        let headset = UsbId::new(1, 2);
        for at in 0..10 {
            store.append(headset, Sample { at, level: 50, charging: false }).unwrap();
        }
        assert_eq!(store.load(headset, 8).len(), 2);
        let lines = fs::read_to_string(dir.path().join("battery-0001-0002.csv")).unwrap().lines().count();
        assert_eq!(lines, 2);
    }

    #[test]
    fn history_skips_broken_lines() {
        let dir = tempfile::tempdir().unwrap();
        let text = "100,80,0\ngarbage\n200,180,0\n300,70,1,extra\n400,60,1\n";
        fs::write(dir.path().join("battery-0001-0002.csv"), text).unwrap();
        let loaded = CsvHistoryStore::at(dir.path()).load(UsbId::new(1, 2), 0);
        let levels: Vec<u8> = loaded.iter().map(|s| s.level).collect();
        assert_eq!(levels, [80, 60]);
    }
}
