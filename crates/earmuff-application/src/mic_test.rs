//! Testing a headset's mic: record a few seconds, watching the level, then hear it back.

use std::time::Duration;

use earmuff_domain::UsbId;

/// Mono 16-bit sound
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recording {
    pub rate: u32,
    pub samples: Vec<i16>,
}

impl Recording {
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(self.rate.max(1)))
    }
}

/// Records from a headset's mic and plays back on its speakers
pub trait MicRecorder: Send + Sync {
    /// Records for `duration`, passing on the level of each moment as it comes in (see [`meter`])
    fn record(&self, headset: UsbId, duration: Duration, level: &mut dyn FnMut(f64)) -> Result<Recording, String>;

    fn play(&self, headset: UsbId, recording: &Recording) -> Result<(), String>;
}

/// How long a mic test records
pub const MIC_TEST_LENGTH: Duration = Duration::from_secs(5);

/// The quietest level the meter shows, in dB below full scale
const FLOOR_DB: f64 = 60.0;

/// How loud some sound is, for a level meter: 0 for silence (or anything under -60 dB)
/// to 1 for full scale, evenly spread in decibels the way ears hear it
pub fn meter(samples: &[i16]) -> f64 {
    let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    if peak == 0 {
        return 0.0;
    }
    let db = 20.0 * (f64::from(peak) / 32768.0).log10();
    ((db + FLOOR_DB) / FLOOR_DB).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_spreads_levels_in_decibels() {
        assert_eq!(meter(&[]), 0.0);
        assert_eq!(meter(&[0, 0, 0]), 0.0);
        assert_eq!(meter(&[i16::MIN]), 1.0);
        // half of full scale is -6 dB: a tenth down the meter
        assert!((meter(&[0, -16384, 100]) - 0.9).abs() < 0.01);
        // -60 dB and below don't register
        assert_eq!(meter(&[30]), 0.0);
    }

    #[test]
    fn a_recording_knows_its_length() {
        let recording = Recording { rate: 48_000, samples: vec![0; 24_000] };
        assert_eq!(recording.duration(), Duration::from_millis(500));
    }
}
