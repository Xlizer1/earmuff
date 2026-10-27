//! A byte-level model of the HyperX Cloud Alpha Wireless and its dongle, built from
//! what the real headset was measured doing: it clamps the mic monitoring level,
//! echoes SET commands with the value sent, answers a mute change with the mute
//! event, and drops a report that arrives while it is still answering the last one.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use earmuff_protocols::hyperx::cloud_alpha_wireless::{MAGIC, REPORT_ID, SIDETONE_MAX, SIDETONE_MIN, cmd, packet};

#[derive(Debug)]
pub struct CloudAlphaSimulator {
    /// On and in range of its dongle
    pub powered: bool,
    pub battery: u8,
    /// 0 = on battery, 1 = charging, 2 = full
    pub charging: u8,
    pub muted: bool,
    pub mic_attached: bool,
    pub sidetone: bool,
    /// What the headset keeps: always within -4..=18
    pub sidetone_level: i16,
    pub auto_off: u8,
    pub voice_prompts: bool,
    /// 0 = black, 1 = white, 2 = black/red
    pub color: u8,
    /// How long the headset takes to answer
    pub latency: Duration,
    /// Reports the dongle dropped because they arrived while it was busy
    pub dropped: usize,
    /// Every report that reached the headset, in order
    pub received: Vec<Vec<u8>>,
    outbox: VecDeque<(Instant, Vec<u8>)>,
    busy_until: Option<Instant>,
}

impl Default for CloudAlphaSimulator {
    fn default() -> Self {
        CloudAlphaSimulator {
            powered: true,
            battery: 80,
            charging: 0,
            muted: false,
            mic_attached: true,
            sidetone: false,
            sidetone_level: -4,
            auto_off: 20,
            voice_prompts: true,
            color: 2,
            latency: Duration::from_millis(5),
            dropped: 0,
            received: Vec::new(),
            outbox: VecDeque::new(),
            busy_until: None,
        }
    }
}

impl CloudAlphaSimulator {
    /// The dongle receives a report from the computer
    pub fn receive(&mut self, report: &[u8], now: Instant) {
        if self.busy_until.is_some_and(|busy| now < busy) {
            self.dropped += 1;
            return;
        }
        if report.len() < 5 || report[0] != REPORT_ID || report[1] != MAGIC {
            return;
        }
        self.received.push(report.to_vec());
        self.busy_until = Some(now + self.latency);
        let (command, value) = (report[2], report[3]);
        if !self.powered {
            // with the headset off, only the dongle answers
            if command == cmd::GET_WIRELESS {
                self.reply(now, cmd::GET_WIRELESS, &[1]);
            }
            return;
        }
        match command {
            cmd::GET_WIRELESS => self.reply(now, command, &[2]),
            cmd::GET_PAIRING => self.reply(now, command, &[0xC8, 0x5A, 0xCF, 0x57, 0x02]),
            cmd::GET_SIDETONE => self.reply(now, command, &[u8::from(self.sidetone)]),
            cmd::GET_SIDETONE_LEVEL => self.reply(now, command, &self.sidetone_level.to_be_bytes()),
            cmd::GET_AUTO_OFF => self.reply(now, command, &[self.auto_off]),
            cmd::GET_MIC_ATTACHED => self.reply(now, command, &[u8::from(self.mic_attached)]),
            cmd::GET_VOICE_PROMPTS => self.reply(now, command, &[u8::from(self.voice_prompts)]),
            cmd::GET_MUTE => self.reply(now, command, &[u8::from(self.muted)]),
            cmd::GET_BATTERY => self.reply(now, command, &[self.battery, 0x10, 0x3A, 0x01]),
            cmd::GET_CHARGING => self.reply(now, command, &[self.charging]),
            cmd::GET_COLOR => self.reply(now, command, &[self.color]),
            cmd::SET_SIDETONE => {
                self.sidetone = value == 1;
                self.reply(now, command, &[value, 0x01]);
            }
            cmd::SET_SIDETONE_LEVEL => {
                let sent = i16::from_be_bytes([report[3], report[4]]);
                self.sidetone_level = sent.clamp(SIDETONE_MIN as i16, SIDETONE_MAX as i16);
                // the echo carries what was sent, not what was kept
                self.reply(now, command, &[report[3], report[4], 0x01]);
            }
            cmd::SET_AUTO_OFF => {
                self.auto_off = value;
                self.reply(now, command, &[value, 0x01]);
            }
            cmd::SET_VOICE_PROMPTS => {
                self.voice_prompts = value == 1;
                self.reply(now, command, &[value, 0x01]);
            }
            cmd::SET_MUTE => {
                self.muted = value == 1;
                self.reply(now, cmd::EVT_MUTE, &[value]);
            }
            _ => {} // unknown commands get no answer
        }
    }

    fn reply(&mut self, now: Instant, command: u8, value: &[u8]) {
        self.outbox.push_back((now + self.latency, packet(command, value)));
    }

    fn report(&mut self, now: Instant, event: u8, value: u8) {
        self.outbox.push_back((now, packet(event, &[value])));
    }

    /// Someone presses the mute button on the headset
    pub fn press_mute_button(&mut self, now: Instant) {
        if self.powered {
            self.muted = !self.muted;
            self.report(now, cmd::EVT_MUTE, u8::from(self.muted));
        }
    }

    /// The headset is switched on or off (or walks out of range)
    pub fn set_powered(&mut self, on: bool, now: Instant) {
        self.powered = on;
        self.report(now, cmd::EVT_WIRELESS, if on { 2 } else { 1 });
    }

    pub fn set_battery(&mut self, level: u8, now: Instant) {
        self.battery = level;
        self.report(now, cmd::EVT_BATTERY, level);
    }

    pub fn set_charging(&mut self, state: u8, now: Instant) {
        self.charging = state;
        self.report(now, cmd::EVT_CHARGING, state);
    }

    /// The next report the computer can read by `now`
    pub fn take_due(&mut self, now: Instant) -> Option<Vec<u8>> {
        if self.outbox.front().is_some_and(|(due, _)| *due <= now) {
            self.outbox.pop_front().map(|(_, report)| report)
        } else {
            None
        }
    }

    /// When the next report becomes readable
    pub fn next_due(&self) -> Option<Instant> {
        self.outbox.front().map(|(due, _)| *due)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn later(now: Instant) -> Instant {
        now + Duration::from_secs(1)
    }

    #[test]
    fn answers_after_its_latency() {
        let mut sim = CloudAlphaSimulator::default();
        let now = Instant::now();
        sim.receive(&packet(cmd::GET_BATTERY, &[]), now);
        assert_eq!(sim.take_due(now), None);
        assert_eq!(sim.take_due(later(now)).unwrap()[..7], [0x21, 0xBB, 0x0B, 80, 0x10, 0x3A, 0x01]);
    }

    #[test]
    fn drops_reports_sent_back_to_back() {
        let mut sim = CloudAlphaSimulator::default();
        let now = Instant::now();
        sim.receive(&packet(cmd::GET_BATTERY, &[]), now);
        sim.receive(&packet(cmd::GET_CHARGING, &[]), now);
        assert_eq!(sim.dropped, 1);
        assert_eq!(sim.received.len(), 1);
    }

    #[test]
    fn clamps_the_sidetone_level_but_echoes_what_was_sent() {
        let mut sim = CloudAlphaSimulator::default();
        let now = Instant::now();
        sim.receive(&packet(cmd::SET_SIDETONE_LEVEL, &40i16.to_be_bytes()), now);
        assert_eq!(sim.sidetone_level, 18);
        assert_eq!(sim.take_due(later(now)).unwrap()[2..6], [cmd::SET_SIDETONE_LEVEL, 0x00, 0x28, 0x01]);
    }

    #[test]
    fn answers_a_mute_change_with_the_mute_event() {
        let mut sim = CloudAlphaSimulator::default();
        let now = Instant::now();
        sim.receive(&packet(cmd::SET_MUTE, &[1]), now);
        assert!(sim.muted);
        assert_eq!(sim.take_due(later(now)).unwrap()[2..4], [cmd::EVT_MUTE, 1]);
    }

    #[test]
    fn only_the_dongle_answers_while_the_headset_is_off() {
        let mut sim = CloudAlphaSimulator::default();
        let now = Instant::now();
        sim.set_powered(false, now);
        assert_eq!(sim.take_due(now).unwrap()[2..4], [cmd::EVT_WIRELESS, 1]);
        sim.receive(&packet(cmd::GET_BATTERY, &[]), now);
        assert_eq!(sim.take_due(later(now)), None);
        sim.receive(&packet(cmd::GET_WIRELESS, &[]), later(now));
        assert_eq!(sim.take_due(later(later(now))).unwrap()[2..4], [cmd::GET_WIRELESS, 1]);
    }

    #[test]
    fn button_presses_are_reported_right_away() {
        let mut sim = CloudAlphaSimulator::default();
        let now = Instant::now();
        sim.press_mute_button(now);
        assert!(sim.muted);
        assert_eq!(sim.next_due(), Some(now));
        assert_eq!(sim.take_due(now).unwrap()[2..4], [cmd::EVT_MUTE, 1]);
    }

    #[test]
    fn ignores_reports_for_other_devices() {
        let mut sim = CloudAlphaSimulator::default();
        sim.receive(&[0x01, 0x02, 0x03, 0x04, 0x05], Instant::now());
        assert!(sim.received.is_empty());
    }
}
