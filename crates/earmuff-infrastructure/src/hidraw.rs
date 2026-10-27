//! Linux hidraw: finding USB HID devices and moving their reports.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use earmuff_application::Waker;
use earmuff_domain::UsbId;
use rustix::event::{EventfdFlags, PollFd, PollFlags, Timespec, eventfd, poll};

/// Where the kernel lists hidraw devices
pub const SYSFS_HIDRAW: &str = "/sys/class/hidraw";

/// A hidraw device node and the USB device behind it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HidrawNode {
    pub path: PathBuf,
    pub usb: UsbId,
}

/// The USB ID in a hidraw device's `uevent`, from a line like `HID_ID=0003:000003F0:0000098D`
pub fn parse_hid_id(uevent: &str) -> Option<UsbId> {
    let id = uevent.lines().find_map(|line| line.strip_prefix("HID_ID="))?;
    let mut parts = id.split(':');
    let bus = u32::from_str_radix(parts.next()?, 16).ok()?;
    let vendor = u32::from_str_radix(parts.next()?, 16).ok()?;
    let product = u32::from_str_radix(parts.next()?, 16).ok()?;
    const BUS_USB: u32 = 0x03;
    (bus == BUS_USB).then_some(UsbId::new(u16::try_from(vendor).ok()?, u16::try_from(product).ok()?))
}

/// Every USB hidraw node listed under `sysfs` (normally [`SYSFS_HIDRAW`]), sorted by path
pub fn scan(sysfs: &Path) -> Vec<HidrawNode> {
    let Ok(entries) = fs::read_dir(sysfs) else {
        return vec![];
    };
    let mut nodes: Vec<HidrawNode> = entries
        .flatten()
        .filter_map(|entry| {
            let uevent = fs::read_to_string(entry.path().join("device/uevent")).ok()?;
            Some(HidrawNode { path: Path::new("/dev").join(entry.file_name()), usb: parse_hid_id(&uevent)? })
        })
        .collect();
    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    nodes
}

/// Why [`Transport::wait`] returned
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wakeup {
    /// A report may be waiting
    Ready,
    /// The waker fired
    Woken,
    TimedOut,
}

/// Moves reports to and from one device
pub trait Transport: Send {
    fn send(&mut self, report: &[u8]) -> io::Result<()>;

    /// The next report, if one is waiting; never blocks
    fn receive(&mut self) -> io::Result<Option<Vec<u8>>>;

    /// Blocks until a report may be waiting, the waker fires, or `timeout` passes
    fn wait(&mut self, timeout: Duration) -> io::Result<Wakeup>;

    fn waker(&self) -> Waker;
}

/// A `/dev/hidraw*` node, plus an eventfd so another thread can interrupt `wait`
#[derive(Debug)]
pub struct HidrawTransport {
    file: File,
    wake: Arc<OwnedFd>,
}

impl HidrawTransport {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        rustix::io::ioctl_fionbio(&file, true)?;
        let wake = eventfd(0, EventfdFlags::NONBLOCK | EventfdFlags::CLOEXEC)?;
        Ok(HidrawTransport { file, wake: Arc::new(wake) })
    }
}

impl Transport for HidrawTransport {
    fn send(&mut self, report: &[u8]) -> io::Result<()> {
        self.file.write_all(report)
    }

    fn receive(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut buf = [0; 64];
        match self.file.read(&mut buf) {
            Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => Ok(Some(buf[..n].to_vec())),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn wait(&mut self, timeout: Duration) -> io::Result<Wakeup> {
        let timeout = Timespec::try_from(timeout).unwrap_or(Timespec { tv_sec: i64::MAX, tv_nsec: 0 });
        let mut fds = [PollFd::new(&self.file, PollFlags::IN), PollFd::new(&*self.wake, PollFlags::IN)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => return Ok(Wakeup::TimedOut),
            Err(e) => return Err(e.into()),
        }
        let (device, wake) = (fds[0].revents(), fds[1].revents());
        if device.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "device unplugged"));
        }
        if wake.contains(PollFlags::IN) {
            let mut counter = [0; 8];
            let _ = rustix::io::read(self.wake.as_fd(), &mut counter);
            return Ok(Wakeup::Woken);
        }
        Ok(if device.contains(PollFlags::IN) { Wakeup::Ready } else { Wakeup::TimedOut })
    }

    fn waker(&self) -> Waker {
        let wake = self.wake.clone();
        Waker::new(move || {
            let _ = rustix::io::write(wake.as_fd(), &1u64.to_ne_bytes());
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usb_hid_ids() {
        let uevent = "DRIVER=hid-generic\nHID_ID=0003:000003F0:0000098D\nHID_NAME=HP, Inc HyperX\n";
        assert_eq!(parse_hid_id(uevent), Some(UsbId::new(0x03F0, 0x098D)));
    }

    #[test]
    fn ignores_non_usb_and_malformed_ids() {
        assert_eq!(parse_hid_id("HID_ID=0005:0000046D:0000B023"), None); // Bluetooth
        assert_eq!(parse_hid_id("HID_ID=0003:XYZ:0000098D"), None);
        assert_eq!(parse_hid_id("HID_ID=0003:000103F0:0000098D"), None); // too big for a USB ID
        assert_eq!(parse_hid_id("HID_NAME=nothing"), None);
    }
}
