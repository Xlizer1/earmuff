//! Earmuff's own files: preferences under $XDG_CONFIG_HOME/earmuff.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use earmuff_application::{Preferences, PreferencesStore};
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
        Preferences { accent: saved.get("accent").and_then(Value::as_str).map_or(defaults.accent, String::from) }
    }

    fn save(&self, preferences: &Preferences) -> Result<(), String> {
        let mut saved = self.read();
        let current = json!({ "accent": preferences.accent });
        saved.extend(current.as_object().cloned().unwrap_or_default());
        let text = serde_json::to_string_pretty(&Value::Object(saved)).map_err(|e| e.to_string())?;
        write_atomically(&self.path, text.as_bytes()).map_err(|e| format!("{}: {e}", self.path.display()))
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
        store.save(&Preferences { accent: "cyan".into() }).unwrap();
        assert_eq!(store.load().accent, "cyan");
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
}
