//! Frequency sweep (chirp) generator and its mirror in `ChainParams`.
//!
//! The sweep is a diagnostic tone: a sine whose frequency runs from `start_freq` to
//! `end_freq` over `duration` seconds. The audio thread generates it instead of the tapped
//! system audio while it plays, then falls back to the tapped audio when it is done.

use crate::state::{Sweep, SweepType, MAX_FREQ, MIN_FREQ};
use std::f64::consts::PI;

/// The level the sweep plays at, as a linear gain.
pub fn sweep_level_linear(level_db: f32) -> f32 {
    10.0f32.powf(level_db / 20.0)
}

/// How the sweep should travel between the start and end frequency. Copyable, so it can
/// ride along inside [`super::chain::ChainParams`] to the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweepParams {
    pub sweep_type: SweepType,
    pub start_freq: f32,
    pub end_freq: f32,
    pub duration: f32,
    /// The tone's level against full scale, in dB. Negative.
    pub level: f32,
    /// Which sweep this is, so one played to its end is not played again.
    pub run: u32,
}

impl SweepParams {
    pub fn from_state(sweep: &Sweep) -> Self {
        SweepParams {
            sweep_type: sweep.sweep_type,
            start_freq: sweep.start_freq,
            end_freq: sweep.end_freq,
            duration: sweep.duration,
            level: sweep.level,
            run: sweep.run,
        }
    }

    /// The instantaneous frequency (Hz) at `t_seconds` seconds into the sweep.
    pub fn freq_at(&self, t_seconds: f32) -> f32 {
        let duration = self.duration.max(0.001);
        let t = (t_seconds / duration).clamp(0.0, 1.0);
        let f0 = self.start_freq.max(MIN_FREQ);
        let f1 = self.end_freq.max(MIN_FREQ);
        match self.sweep_type {
            SweepType::Logarithmic => {
                if f0 <= 0.0 || f1 <= 0.0 {
                    return f0;
                }
                f0 * (f1 / f0).powf(t)
            }
            SweepType::Linear => f0 + (f1 - f0) * t,
        }
    }

    /// The total phase (in radians) at `t_seconds` seconds into the sweep. In f64: it
    /// reaches hundreds of thousands of radians, where f32 steps would be audible as noise.
    fn phase_at(&self, t_seconds: f64) -> f64 {
        let duration = self.duration.max(0.001) as f64;
        let f0 = self.start_freq.max(MIN_FREQ) as f64;
        let f1 = self.end_freq.max(MIN_FREQ) as f64;
        match self.sweep_type {
            SweepType::Logarithmic => {
                if f0 <= 0.0 || f1 <= 0.0 || (f1 / f0 - 1.0).abs() <= 1e-6 {
                    // Degenerate but audible sweep: a tone at f0.
                    2.0 * PI * f0 * t_seconds
                } else {
                    let k = f1 / f0;
                    // Integral of f0 * k^(t/T) from 0 to t.
                    let ln_k = k.ln();
                    2.0 * PI * f0 * duration * (k.powf(t_seconds / duration) - 1.0) / ln_k
                }
            }
            SweepType::Linear => {
                // Integral of f0 + (f1 - f0) * t / T from 0 to t.
                let df = (f1 - f0) / duration;
                2.0 * PI * (f0 * t_seconds + 0.5 * df * t_seconds * t_seconds)
            }
        }
    }
}

/// A chirp played through the audio chain, one sample at a time.
pub struct SweepGenerator {
    params: SweepParams,
    gain: f32,
    /// Samples generated so far. The clock is a count, not a float, so the sweep is
    /// exactly as long as asked for and does not drift.
    sample: u64,
    /// Seconds over which the tone fades in and out, so it does not click.
    fade: f64,
    sample_rate: f64,
}

impl SweepGenerator {
    pub fn new(params: SweepParams, sample_rate: f64) -> Self {
        let params = SweepParams {
            start_freq: params.start_freq.clamp(MIN_FREQ, MAX_FREQ),
            end_freq: params.end_freq.clamp(MIN_FREQ, MAX_FREQ),
            duration: params.duration.max(0.05),
            ..params
        };
        SweepGenerator {
            gain: sweep_level_linear(params.level),
            fade: 0.010,
            sample_rate,
            params,
            sample: 0,
        }
    }

    /// Seconds into the sweep.
    fn t(&self) -> f64 {
        self.sample as f64 / self.sample_rate
    }

    /// Samples the sweep runs for in total.
    fn length(&self) -> u64 {
        (self.params.duration as f64 * self.sample_rate).round() as u64
    }

    /// The next sample of the chirp, or 0.0 once the sweep is over.
    pub fn advance_sample(&mut self) -> f32 {
        if self.sample >= self.length() {
            return 0.0;
        }
        let t = self.t();
        let phase = self.params.phase_at(t);
        let sample = (phase.sin() * self.fade_gain()) as f32 * self.gain;
        self.sample += 1;
        sample
    }

    /// The tone the generator makes, as passed to [`SweepGenerator::new`].
    pub fn params(&self) -> SweepParams {
        self.params
    }

    /// Whether the sweep has run to its end.
    pub fn is_complete(&self) -> bool {
        self.sample >= self.length()
    }

    /// How far through the sweep we are, 0 to 1.
    pub fn progress(&self) -> f32 {
        (self.sample as f32 / self.length().max(1) as f32).clamp(0.0, 1.0)
    }

    /// The instantaneous frequency, in Hz, at the current position.
    pub fn current_freq(&self) -> f32 {
        self.params.freq_at(self.t() as f32)
    }

    /// 1.0 in the middle of the sweep, ramping to 0 at both edges so the tone does not click.
    fn fade_gain(&self) -> f64 {
        let t = self.t();
        let fade = self.fade;
        let in_gain = (t / fade).min(1.0);
        let out_gain = ((self.params.duration as f64 - t) / fade).min(1.0);
        let gain = in_gain.min(out_gain);
        // Raised cosine: smooth at both ends of the fade.
        0.5 - 0.5 * (PI * gain).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SweepParams {
        SweepParams {
            sweep_type: SweepType::Logarithmic,
            start_freq: 20.0,
            end_freq: 20_000.0,
            duration: 4.0,
            level: -12.0,
            run: 1,
        }
    }

    #[test]
    fn chirp_starts_on_the_first_frequency() {
        let sweep = params();
        assert!((sweep.freq_at(0.0) - 20.0).abs() < 0.01);
    }

    #[test]
    fn chirp_ends_on_the_last_frequency() {
        let sweep = params();
        assert!((sweep.freq_at(4.0) - 20_000.0).abs() / 20_000.0 < 0.01);
    }

    #[test]
    fn linear_chirp_travels_in_a_straight_line() {
        let sweep = SweepParams {
            sweep_type: SweepType::Linear,
            start_freq: 100.0,
            end_freq: 1_000.0,
            duration: 3.0,
            level: -12.0,
            run: 1,
        };
        assert_eq!(sweep.freq_at(1.5), 550.0);
    }

    #[test]
    fn phase_is_continuous() {
        let sweep = params();
        // No jumps at the join points where the sample step is largest.
        let d = 1.0 / 48_000.0;
        let expected = sweep.phase_at(3.0 + d);
        let actual = sweep.phase_at(3.0);
        assert!((expected - actual).abs() > 0.0);
        // A large window sees exactly the elapsed phase.
        assert!(sweep.phase_at(0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn generator_completes_and_fades_to_silence() {
        let mut generator = SweepGenerator::new(params(), 48_000.0);
        assert!(!generator.is_complete());
        for _ in 0..(48_000 * 4 + 128) {
            generator.advance_sample();
        }
        assert!(generator.is_complete());
        assert_eq!(generator.advance_sample(), 0.0);
        assert_eq!(generator.progress(), 1.0);
    }

    #[test]
    fn a_long_sweep_stays_a_clean_tone_to_the_end() {
        let p = SweepParams { duration: 15.0, level: 0.0, ..params() };
        let fs = 48_000.0;
        let mut generator = SweepGenerator::new(p, fs);
        let (f0, k, d) = (20.0f64, 1_000.0f64, 15.0f64);
        for i in 0..(d * fs) as u64 {
            let y = generator.advance_sample() as f64;
            let t = i as f64 / fs;
            // Away from the fades, the samples are the exact chirp to f32 precision.
            if t > 0.05 && t < d - 0.05 {
                let phase = 2.0 * PI * f0 * d * (k.powf(t / d) - 1.0) / k.ln();
                assert!((y - phase.sin()).abs() < 1e-3, "{t:.3} s: {y} vs {}", phase.sin());
            }
        }
    }

    #[test]
    fn a_sweep_that_does_not_move_is_a_steady_tone() {
        for sweep_type in [SweepType::Logarithmic, SweepType::Linear] {
            let p = SweepParams { sweep_type, start_freq: 1_000.0, end_freq: 1_000.0, ..params() };
            let mut generator = SweepGenerator::new(p, 48_000.0);
            let peak = (0..48_000).map(|_| generator.advance_sample().abs()).fold(0.0, f32::max);
            assert!(peak > 0.2, "{sweep_type:?}: {peak}");
        }
    }

    #[test]
    fn a_sweep_can_run_downwards() {
        let p = SweepParams { start_freq: 8_000.0, end_freq: 100.0, ..params() };
        assert!((p.freq_at(0.0) - 8_000.0).abs() < 0.1 && (p.freq_at(4.0) - 100.0).abs() < 0.1);
        let mut generator = SweepGenerator::new(p, 48_000.0);
        assert!((0..48_000).all(|_| generator.advance_sample().is_finite()));
    }

    #[test]
    fn generator_never_exceeds_the_requested_level() {
        let mut generator = SweepGenerator::new(params(), 48_000.0);
        let limit = sweep_level_linear(-12.0) + 1e-6;
        for _ in 0..48_000 {
            assert!(generator.advance_sample().abs() <= limit);
        }
    }
}
