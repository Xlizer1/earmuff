//! The GTK4/libadwaita window and the tray icon.

pub mod view_model;
mod window;

use std::process::ExitCode;
use std::sync::Arc;

use adw::prelude::*;
use earmuff_application::{Backend, ManagerOptions};

pub const APP_ID: &str = "io.github.xlizer1.Earmuff";

#[derive(Clone, Debug)]
pub struct UiOptions {
    pub app_id: String,
    pub manager: ManagerOptions,
    /// Put an icon in the system tray (if there is one); closing the window then only hides it
    pub tray: bool,
    /// Running with a simulated headset: uses its own app ID so it doesn't reach a real instance
    pub demo: bool,
}

impl Default for UiOptions {
    fn default() -> Self {
        UiOptions { app_id: APP_ID.into(), manager: ManagerOptions::default(), tray: true, demo: false }
    }
}

/// Builds the application. Running it opens the window, or brings an already running
/// Earmuff's window back.
pub fn application(backends: Vec<Arc<dyn Backend>>, options: UiOptions) -> adw::Application {
    let app_id = if options.demo { format!("{}.Demo", options.app_id) } else { options.app_id.clone() };
    let app = adw::Application::builder().application_id(app_id).build();
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
