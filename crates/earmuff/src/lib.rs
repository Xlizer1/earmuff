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
use earmuff_infrastructure::headsetcontrol::HeadsetControlBackend;
use earmuff_infrastructure::native::NativeBackend;

pub fn run() -> ExitCode {
    let args = cli::Cli::parse();
    let backends = backends(args.demo);
    match args.command {
        None => ui::run(backends, ui::UiOptions { demo: args.demo, hidden: args.hidden, ..Default::default() }),
        Some(command) => cli::run(command, args.device, &backends),
    }
}

/// The backends in priority order: Earmuff's own drivers first, then HeadsetControl
/// (when it's installed) for everything else. `demo` swaps real headsets for simulated ones.
pub fn backends(demo: bool) -> Vec<Arc<dyn Backend>> {
    if demo {
        return vec![Arc::new(DemoBackend::new())];
    }
    let mut backends: Vec<Arc<dyn Backend>> = vec![Arc::new(NativeBackend::new())];
    if let Some(headsetcontrol) = HeadsetControlBackend::installed() {
        backends.push(Arc::new(headsetcontrol));
    }
    backends
}

#[cfg(test)]
mod tests {
    use super::*;
    use earmuff_domain::BackendKind;

    #[test]
    fn native_drivers_come_first() {
        let kinds: Vec<BackendKind> = backends(false).iter().map(|b| b.kind()).collect();
        assert_eq!(kinds[0], BackendKind::Native);
        assert!(kinds[1..].iter().all(|kind| *kind == BackendKind::HeadsetControl));
    }

    #[test]
    fn demo_uses_only_simulated_headsets() {
        let kinds: Vec<BackendKind> = backends(true).iter().map(|b| b.kind()).collect();
        assert_eq!(kinds, [BackendKind::Demo]);
    }
}
