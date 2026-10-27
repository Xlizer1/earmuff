//! Battery and settings for the HyperX Cloud Alpha Wireless, in place of HyperX NGENUITY.

mod cli;
mod device;
mod gui;
mod protocol;
mod tray;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    match cli::Cli::parse().command {
        None => gui::run(),
        Some(command) => cli::run(command),
    }
}
