//! Small validated values shared by every layer.

use std::fmt;
use std::str::FromStr;

/// A whole percentage from 0 to 100.
///
/// Levels such as mic monitoring are percentages in the domain, so the UI and the
/// command line work the same for every headset; each adapter maps them onto its
/// headset's own range with [`Percent::to_range`] and [`Percent::from_range`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Percent(u8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PercentError(String);

impl fmt::Display for PercentError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} is not a percentage from 0 to 100", self.0)
    }
}

impl std::error::Error for PercentError {}

impl Percent {
    pub const ZERO: Percent = Percent(0);
    pub const FULL: Percent = Percent(100);

    pub fn new(value: u8) -> Result<Self, PercentError> {
        if value <= 100 { Ok(Percent(value)) } else { Err(PercentError(value.to_string())) }
    }

    /// Clamps any number into 0..=100
    pub fn saturating(value: i64) -> Self {
        Percent(value.clamp(0, 100) as u8)
    }

    pub fn get(self) -> u8 {
        self.0
    }

    /// The step of `min..=max` closest to this percentage
    pub fn to_range(self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        min + ((max - min) * i32::from(self.0) + 50) / 100
    }

    /// The percentage that `value` stands for within `min..=max` (clamped into the range)
    pub fn from_range(value: i32, min: i32, max: i32) -> Self {
        if max <= min {
            return Percent::ZERO;
        }
        let span = max - min;
        let offset = value.clamp(min, max) - min;
        Percent::saturating(i64::from((offset * 100 + span / 2) / span))
    }
}

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}%", self.0)
    }
}

/// Accepts "40" and "40%"
impl FromStr for Percent {
    type Err = PercentError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let digits = s.trim().trim_end_matches('%');
        digits.parse::<u8>().map_err(|_| PercentError(s.to_string())).and_then(Percent::new)
    }
}

/// How long the headset waits, unused, before turning itself off
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AutoOff {
    minutes: u8,
}

impl AutoOff {
    pub const NEVER: AutoOff = AutoOff { minutes: 0 };

    pub fn minutes(minutes: u8) -> Self {
        AutoOff { minutes }
    }

    /// Minutes before turning off; 0 means never
    pub fn get(self) -> u8 {
        self.minutes
    }

    pub fn is_never(self) -> bool {
        self.minutes == 0
    }
}

impl fmt::Display for AutoOff {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.is_never() { f.write_str("never") } else { write!(f, "{} min", self.minutes) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChargeState {
    Discharging,
    Charging,
    Full,
    /// The headset sent a state Earmuff doesn't know
    Unknown,
}

impl ChargeState {
    /// Plugged in, whether still charging or already full
    pub fn is_plugged_in(self) -> bool {
        matches!(self, ChargeState::Charging | ChargeState::Full)
    }
}

impl fmt::Display for ChargeState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            ChargeState::Discharging => "discharging",
            ChargeState::Charging => "charging",
            ChargeState::Full => "full",
            ChargeState::Unknown => "unknown",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_rejects_values_over_100() {
        assert_eq!(Percent::new(100).map(Percent::get), Ok(100));
        assert!(Percent::new(101).is_err());
    }

    #[test]
    fn percent_saturates() {
        assert_eq!(Percent::saturating(-5), Percent::ZERO);
        assert_eq!(Percent::saturating(250), Percent::FULL);
        assert_eq!(Percent::saturating(42).get(), 42);
    }

    #[test]
    fn percent_parses_with_or_without_sign() {
        assert_eq!("40".parse::<Percent>().map(Percent::get), Ok(40));
        assert_eq!(" 40% ".parse::<Percent>().map(Percent::get), Ok(40));
        assert!("-1".parse::<Percent>().is_err());
        assert!("101".parse::<Percent>().is_err());
        assert!("loud".parse::<Percent>().is_err());
        assert_eq!("101".parse::<Percent>().unwrap_err().to_string(), "101 is not a percentage from 0 to 100");
    }

    #[test]
    fn percent_maps_onto_a_range_and_back() {
        // the Cloud Alpha Wireless keeps mic monitoring as -4..18; every step must survive a round trip
        for raw in -4..=18 {
            assert_eq!(Percent::from_range(raw, -4, 18).to_range(-4, 18), raw, "step {raw}");
        }
        assert_eq!(Percent::ZERO.to_range(-4, 18), -4);
        assert_eq!(Percent::FULL.to_range(-4, 18), 18);
        assert_eq!(Percent::new(50).unwrap().to_range(-4, 18), 7);
        assert_eq!(Percent::from_range(7, -4, 18).get(), 50);
    }

    #[test]
    fn percent_range_mapping_clamps_and_handles_empty_ranges() {
        assert_eq!(Percent::from_range(99, 0, 10), Percent::FULL);
        assert_eq!(Percent::from_range(-99, 0, 10), Percent::ZERO);
        assert_eq!(Percent::FULL.to_range(5, 5), 5);
        assert_eq!(Percent::from_range(5, 5, 5), Percent::ZERO);
    }

    #[test]
    fn percent_displays_with_sign() {
        assert_eq!(Percent::new(7).unwrap().to_string(), "7%");
    }

    #[test]
    fn auto_off_zero_means_never() {
        assert!(AutoOff::NEVER.is_never());
        assert_eq!(AutoOff::minutes(0), AutoOff::NEVER);
        assert_eq!(AutoOff::NEVER.to_string(), "never");
        assert_eq!(AutoOff::minutes(20).to_string(), "20 min");
        assert_eq!(AutoOff::minutes(20).get(), 20);
    }

    #[test]
    fn charge_state_plugged_in() {
        assert!(ChargeState::Charging.is_plugged_in());
        assert!(ChargeState::Full.is_plugged_in());
        assert!(!ChargeState::Discharging.is_plugged_in());
        assert!(!ChargeState::Unknown.is_plugged_in());
        assert_eq!(ChargeState::Full.to_string(), "full");
    }
}
