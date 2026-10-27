//! Running the `headsetcontrol` program.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// What one run of headsetcontrol printed
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs headsetcontrol. A trait so tests can script its answers.
pub trait Runner: Send + Sync {
    fn run(&self, args: &[String]) -> io::Result<RunOutput>;
}

/// The real headsetcontrol program
#[derive(Clone, Debug)]
pub struct Program {
    path: PathBuf,
    extra: Vec<String>,
}

impl Program {
    /// HID read timeout per run, so a headset that doesn't answer can't stall a poll for long
    const TIMEOUT_MS: &str = "1500";

    /// headsetcontrol from PATH (or /usr/bin), if it is installed
    pub fn find() -> Option<Self> {
        let dirs: Vec<PathBuf> =
            std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
        dirs.into_iter()
            .chain([PathBuf::from("/usr/bin"), PathBuf::from("/usr/local/bin")])
            .map(|dir| dir.join("headsetcontrol"))
            .find(|path| path.is_file())
            .map(Program::at)
    }

    pub fn at(path: impl AsRef<Path>) -> Self {
        Program { path: path.as_ref().to_path_buf(), extra: Vec::new() }
    }

    /// Also lists HeadsetControl's built-in mock headset. `profile` picks its behavior:
    /// 0 normal, 1 HID error, 2 charging, 4 offline, 5 timeout, 6 full, 7 low battery.
    pub fn with_test_device(mut self, profile: u8) -> Self {
        self.extra = vec!["--test-device".into(), profile.to_string()];
        self
    }
}

impl Runner for Program {
    fn run(&self, args: &[String]) -> io::Result<RunOutput> {
        let output =
            Command::new(&self.path).args(&self.extra).args(args).args(["--timeout", Program::TIMEOUT_MS]).output()?;
        Ok(RunOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}
