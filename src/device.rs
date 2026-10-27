//! Talks to the USB dongle through its /dev/hidraw node.

use std::collections::VecDeque;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::protocol::{self as p, cmd, Report, State, Update};

/// The dongle drops commands sent back to back, so they go out one at a time
const REPLY_TIMEOUT: Duration = Duration::from_millis(400);

#[derive(Debug)]
pub enum OpenError {
    NotFound,
    Io(PathBuf, io::Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            OpenError::NotFound => f.write_str("USB dongle not found"),
            OpenError::Io(path, e) if e.kind() == io::ErrorKind::PermissionDenied => write!(
                f,
                "no permission to open {} (the udev rule from HeadsetControl grants it)",
                path.display()
            ),
            OpenError::Io(path, e) => write!(f, "{}: {e}", path.display()),
        }
    }
}

pub fn find_hidraw() -> Option<PathBuf> {
    let want = format!("HID_ID=0003:{:08X}:{:08X}", p::VENDOR_ID, p::PRODUCT_ID);
    let mut devices: Vec<PathBuf> = fs::read_dir("/sys/class/hidraw").ok()?.flatten().map(|e| e.path()).collect();
    devices.sort();
    devices.into_iter().find_map(|dev| {
        let uevent = fs::read_to_string(dev.join("device/uevent")).ok()?;
        if uevent.lines().any(|line| line == want) {
            Some(Path::new("/dev").join(dev.file_name()?))
        } else {
            None
        }
    })
}

/// Waits until `fd` has data to read; None waits forever. Returns false on timeout.
pub fn wait_readable(fd: RawFd, timeout: Option<Duration>) -> io::Result<bool> {
    let mut pollfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    let ms = timeout.map_or(-1, |t| t.as_millis().min(i32::MAX as u128) as i32);
    // SAFETY: pollfd is a valid, initialized array of length 1 for the duration of the call
    match unsafe { libc::poll(&mut pollfd, 1, ms) } {
        -1 => match io::Error::last_os_error() {
            e if e.kind() == io::ErrorKind::Interrupted => Ok(false),
            e => Err(e),
        },
        n => Ok(n > 0),
    }
}

/// The dongle. `state` holds the last value the headset reported for each setting.
///
/// Commands are queued by `request()` and sent by `pump()`; `read()` handles whatever
/// the headset sends back. The GUI calls these from its main loop, the command line
/// uses `flush()`. Every method fails with an io::Error once the dongle is unplugged.
pub struct Headset {
    file: File,
    pub state: State,
    queue: VecDeque<Report>,
    /// Reply code we are waiting for, and until when
    waiting: Option<(u8, Instant)>,
}

impl Headset {
    pub fn open() -> Result<Self, OpenError> {
        let path = find_hidraw().ok_or(OpenError::NotFound)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .map_err(|e| OpenError::Io(path, e))?;
        Ok(Headset { file, state: State::default(), queue: VecDeque::new(), waiting: None })
    }

    pub fn fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    pub fn request(&mut self, reports: impl IntoIterator<Item = Report>) -> io::Result<()> {
        self.queue.extend(reports);
        self.pump().map(drop)
    }

    /// Sends the next command once the last one was answered or timed out.
    /// Returns true while commands are still queued or waiting for an answer.
    pub fn pump(&mut self) -> io::Result<bool> {
        if let Some((_, deadline)) = self.waiting {
            if Instant::now() < deadline {
                return Ok(true);
            }
            self.waiting = None;
        }
        let Some(report) = self.queue.pop_front() else {
            return Ok(false);
        };
        self.file.write_all(&report)?;
        self.waiting = Some((p::reply_code(&report), Instant::now() + REPLY_TIMEOUT));
        Ok(true)
    }

    /// Handles every report waiting on the dongle; returns the updates that changed the state.
    pub fn read(&mut self) -> io::Result<Vec<Update>> {
        let mut changed = Vec::new();
        self.read_with(|_, update| changed.extend(update))?;
        Ok(changed)
    }

    /// Like `read()`, but calls `on_report` with every raw report and the update
    /// it caused (None if it changed nothing).
    pub fn read_with(&mut self, mut on_report: impl FnMut(&[u8], Option<Update>)) -> io::Result<()> {
        let mut buf = [0; 64];
        loop {
            let n = match self.file.read(&mut buf) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            };
            let report = &buf[..n];
            if let Some((code, _)) = self.waiting
                && n > 2
                && report[0] == p::REPORT_ID
                && report[2] == code
            {
                self.waiting = None;
            }
            let update = p::parse(report).filter(|&u| self.state.apply(u));
            on_report(report, update);
        }
        self.pump()?;
        Ok(())
    }

    /// Blocks until every queued command has been answered or timed out.
    pub fn flush(&mut self, timeout: Duration) -> io::Result<()> {
        let end = Instant::now() + timeout;
        while self.pump()? && Instant::now() < end {
            if wait_readable(self.fd(), Some(Duration::from_millis(50)))? {
                self.read()?;
            }
        }
        Ok(())
    }

    pub fn refresh(&mut self) -> io::Result<()> {
        self.request(p::GET_ALL.map(p::get))
    }

    pub fn set_sidetone(&mut self, on: bool) -> io::Result<()> {
        self.request([p::flag(cmd::SET_SIDETONE, on), p::get(cmd::GET_SIDETONE)])
    }

    pub fn set_sidetone_level(&mut self, level: i16) -> io::Result<()> {
        self.request([p::sidetone_level(level), p::get(cmd::GET_SIDETONE_LEVEL)])
    }

    pub fn set_auto_off(&mut self, minutes: u8) -> io::Result<()> {
        self.request([p::auto_off(minutes), p::get(cmd::GET_AUTO_OFF)])
    }

    pub fn set_voice_prompts(&mut self, on: bool) -> io::Result<()> {
        self.request([p::flag(cmd::SET_VOICE_PROMPTS, on), p::get(cmd::GET_VOICE_PROMPTS)])
    }

    pub fn set_mute(&mut self, on: bool) -> io::Result<()> {
        // the headset answers with a mute event, so no GET is needed
        self.request([p::flag(cmd::SET_MUTE, on)])
    }
}
