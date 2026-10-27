//! Earmuff: battery and settings for your gaming headset.
//!
//! This crate is the presentation layer (window, tray icon, command line) and the
//! composition root that wires the adapters into the use cases.

pub mod cli;
pub mod tray;
pub mod ui;

use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use earmuff_application::Backend;
use earmuff_infrastructure::demo::DemoBackend;
use earmuff_infrastructure::native::NativeBackend;

pub fn run() -> ExitCode {
    let args = cli::Cli::parse();
    let backends = backends(args.demo);
    match args.command {
        None => ui::run(backends, ui::UiOptions { demo: args.demo, ..Default::default() }),
        Some(command) => cli::run(command, &backends),
    }
}

/// The backends in priority order. `demo` swaps real headsets for a simulated one.
pub fn backends(demo: bool) -> Vec<Arc<dyn Backend>> {
    if demo { vec![Arc::new(DemoBackend::new())] } else { vec![Arc::new(NativeBackend::new())] }
}
