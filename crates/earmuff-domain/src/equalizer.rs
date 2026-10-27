//! Hardware equalizers: gains, curves and what a headset's equalizer can do.

use std::fmt;
use std::str::FromStr;

/// A gain in tenths of a decibel, so curves compare exactly (no float rounding)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Gain(i16);

impl Gain {
    pub const fn tenths(tenths: i16) -> Self {
        Gain(tenths)
    }

    /// The nearest tenth of a decibel
    pub fn from_db(db: f64) -> Self {
        Gain((db * 10.0).round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16)
    }

    pub fn get_tenths(self) -> i16 {
        self.0
    }

    pub fn db(self) -> f64 {
        f64::from(self.0) / 10.0
    }
}

/// "2.5 dB", "-3 dB", "0 dB"
impl fmt::Display for Gain {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} dB", self.db())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GainError(String);

impl fmt::Display for GainError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} is not a gain in dB", self.0)
    }
}

impl std::error::Error for GainError {}

/// Accepts "2.5", "-3" and "2.5dB"
impl FromStr for Gain {
    type Err = GainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let number = s.trim().trim_end_matches("dB").trim_end_matches("db").trim();
        match number.parse::<f64>() {
            Ok(db) if db.is_finite() && db.abs() < 1000.0 => Ok(Gain::from_db(db)),
            _ => Err(GainError(s.to_string())),
        }
    }
}

/// A named curve the headset ships with
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EqPreset {
    pub name: String,
    pub curve: Vec<Gain>,
}

/// What a headset's equalizer can do
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EqualizerSpec {
    pub bands: usize,
    pub min: Gain,
    pub max: Gain,
    pub step: Gain,
    /// The gain of a flat curve
    pub baseline: Gain,
    pub presets: Vec<EqPreset>,
}

impl EqualizerSpec {
    /// Why a curve doesn't fit this equalizer, if it doesn't
    pub fn check(&self, curve: &[Gain]) -> Result<(), String> {
        if curve.len() != self.bands {
            return Err(format!("this equalizer has {} bands, not {}", self.bands, curve.len()));
        }
        match curve.iter().find(|g| **g < self.min || **g > self.max) {
            Some(gain) => Err(format!("{gain} is outside {} to {}", self.min, self.max)),
            None => Ok(()),
        }
    }

    pub fn flat(&self) -> Vec<Gain> {
        vec![self.baseline; self.bands]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> EqualizerSpec {
        EqualizerSpec {
            bands: 3,
            min: Gain::tenths(-120),
            max: Gain::tenths(120),
            step: Gain::tenths(5),
            baseline: Gain::tenths(0),
            presets: vec![],
        }
    }

    #[test]
    fn gains_parse_and_display_in_decibels() {
        assert_eq!("2.5".parse(), Ok(Gain::tenths(25)));
        assert_eq!("-3dB".parse(), Ok(Gain::tenths(-30)));
        assert_eq!(" 1.04 dB ".parse(), Ok(Gain::tenths(10)));
        assert!("loud".parse::<Gain>().is_err());
        assert!("inf".parse::<Gain>().is_err());
        assert_eq!(Gain::tenths(25).to_string(), "2.5 dB");
        assert_eq!(Gain::tenths(-30).to_string(), "-3 dB");
        assert_eq!(Gain::from_db(-12.0).get_tenths(), -120);
    }

    #[test]
    fn curves_must_fit_the_equalizer() {
        let spec = spec();
        assert_eq!(spec.check(&[Gain::tenths(0); 3]), Ok(()));
        assert_eq!(spec.check(&[Gain::tenths(0); 2]), Err("this equalizer has 3 bands, not 2".into()));
        assert_eq!(
            spec.check(&[Gain::tenths(0), Gain::tenths(130), Gain::tenths(0)]),
            Err("13 dB is outside -12 dB to 12 dB".into())
        );
        assert_eq!(spec.flat(), [Gain::tenths(0); 3]);
    }
}
