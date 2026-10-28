//! The GTK4/libadwaita window and the tray icon.

mod battery_graph;
mod battery_ring;
mod equalizer;
mod fonts;
pub mod view_model;
mod window;

use std::process::ExitCode;
use std::sync::Arc;

use adw::prelude::*;
use earmuff_application::{
    AudioSystem, Autostart, Backend, HistoryStore, ManagerOptions, MicRecorder, PreferencesStore, ProfileStore,
    SoftwareEqStore, SoftwareEqualizer,
};
use earmuff_infrastructure::config::{
    CsvHistoryStore, JsonPreferencesStore, JsonProfileStore, JsonSoftwareEqStore, XdgAutostart,
};
use earmuff_infrastructure::pipewire::PipeWire;
use earmuff_infrastructure::pipewire::equalizer::PipeWireEqualizer;
use gtk::gio;

pub const APP_ID: &str = "io.github.xlizer1.Earmuff";
/// Where the embedded icons and stylesheet live
pub const RESOURCE_PATH: &str = "/io/github/xlizer1/Earmuff";

/// Makes the icons and stylesheet compiled into the binary available
pub fn register_resources() {
    static REGISTER: std::sync::Once = std::sync::Once::new();
    REGISTER.call_once(|| gio::resources_register_include!("earmuff.gresource").expect("embedded resources are valid"));
}

#[derive(Clone)]
pub struct UiOptions {
    pub app_id: String,
    pub manager: ManagerOptions,
    /// Put an icon in the system tray (if there is one); closing the window then only hides it
    pub tray: bool,
    /// Running with a simulated headset: uses its own app ID so it doesn't reach a real instance
    pub demo: bool,
    /// Start in the tray without showing the window (when there is a tray)
    pub hidden: bool,
    pub preferences: Arc<dyn PreferencesStore>,
    pub autostart: Arc<dyn Autostart>,
    pub profiles: Arc<dyn ProfileStore>,
    pub history: Arc<dyn HistoryStore>,
    /// The sound system, for switching the computer's sound to a headset
    pub audio: Arc<dyn AudioSystem>,
    /// Records from a headset's mic and plays it back, for the mic test
    pub mic: Arc<dyn MicRecorder>,
    /// Runs equalizers on the computer for headsets without one of their own
    pub equalizer: Arc<dyn SoftwareEqualizer>,
    pub software_eq_store: Arc<dyn SoftwareEqStore>,
}

impl Default for UiOptions {
    fn default() -> Self {
        UiOptions {
            app_id: APP_ID.into(),
            manager: ManagerOptions::default(),
            tray: true,
            demo: false,
            hidden: false,
            preferences: Arc::new(JsonPreferencesStore::in_config_dir()),
            autostart: Arc::new(XdgAutostart::for_this_program()),
            profiles: Arc::new(JsonProfileStore::in_config_dir()),
            history: Arc::new(CsvHistoryStore::in_state_dir()),
            audio: Arc::new(PipeWire::default()),
            mic: Arc::new(PipeWire::default()),
            equalizer: Arc::new(PipeWireEqualizer::new(PipeWire::default())),
            software_eq_store: Arc::new(JsonSoftwareEqStore::in_config_dir()),
        }
    }
}

/// Builds the application. Running it opens the window, or brings an already running
/// Earmuff's window back.
pub fn application(backends: Vec<Arc<dyn Backend>>, options: UiOptions) -> adw::Application {
    let app_id = if options.demo { format!("{}.Demo", options.app_id) } else { options.app_id.clone() };
    register_resources();
    // GTK then finds the icons under RESOURCE_PATH/icons, whatever the app ID
    let app = adw::Application::builder().application_id(app_id).resource_base_path(RESOURCE_PATH).build();
    app.connect_activate(move |app| match app.active_window() {
        Some(window) => window.present(),
        None => window::open(app, backends.clone(), &options),
    });
    app
}

pub fn run(backends: Vec<Arc<dyn Backend>>, options: UiOptions) -> ExitCode {
    // clap already parsed the command line, so GTK gets no arguments
    application(backends, options).run_with_args::<&str>(&[]).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_compiled_in() {
        register_resources();
        for icon in [
            "icons/scalable/apps/io.github.xlizer1.Earmuff.svg",
            "icons/scalable/apps/io.github.xlizer1.Earmuff-symbolic.svg",
            "icons/scalable/actions/earmuff-headset-symbolic.svg",
        ] {
            let path = format!("{RESOURCE_PATH}/{icon}");
            let data = gio::resources_lookup_data(&path, gio::ResourceLookupFlags::NONE).expect("compiled in");
            assert!(data.windows(4).any(|w| w == b"<svg"), "{icon} is an SVG");
        }
    }
}
