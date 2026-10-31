use super::biquad::{BiquadState, Coeffs};
use super::stages::{MAX_STAGES, Stages, balance_db};
use crate::state::EqState;

/// How often (in frames) filter coefficients step toward their targets.
const CONTROL_INTERVAL: u32 = 32;
/// Smoothing time constant for coefficients, gains and bypass.
const SMOOTH_SECONDS: f64 = 0.010;
/// The limiter keeps peaks at or under this, 0.1 dB below full scale.
const LIMIT_CEILING: f64 = 0.988_553;
/// How quickly the limiter lets go once the signal drops back under the ceiling.
const LIMIT_RELEASE_SECONDS: f64 = 0.100;

/// Everything the audio thread needs, precomputed on the UI thread.
#[derive(Clone, Copy, Debug)]
pub struct ChainParams {
    pub stages: Stages,
    /// Linear gain for each channel: the preamp, with the balance cut on one side.
    pub gain: [f64; 2],
    pub bypass: bool,
    /// Turn the output down for as long as it would otherwise exceed full scale.
    pub limit: bool,
}

impl ChainParams {
    pub fn from_state(state: &EqState, sample_rate: f64) -> Self {
        let balance = balance_db(state.balance);
        let gain = |channel: usize| 10f64.powf((state.preamp_db + balance[channel]) as f64 / 20.0);
        ChainParams {
            stages: Stages::new(state, sample_rate),
            gain: [gain(0), gain(1)],
            bypass: state.bypass,
            limit: state.clip_protection,
        }
    }
}

/// Stereo biquad cascade with click-free parameter changes. Allocation-free, so it is safe
/// to run on the audio thread.
pub struct Chain {
    current: [Coeffs; MAX_STAGES],
    target: [Coeffs; MAX_STAGES],
    state: [[BiquadState; MAX_STAGES]; 2],
    /// Stages being run. While a change settles this covers both the old and new stages,
    /// so filters that are going away fade out instead of stopping.
    active: usize,
    /// Stages the target uses.
    target_len: usize,
    gain: [f64; 2],
    gain_target: [f64; 2],
    /// 1.0 = fully processed, 0.0 = bypassed.
    wet: f64,
    wet_target: f64,
    limit: bool,
    /// Gain the limiter is applying, 1.0 when it is idle.
    limit_gain: f64,
    k_release: f64,
    settled: bool,
    countdown: u32,
    k_control: f64,
    k_sample: f64,
}

impl Chain {
    pub fn new(sample_rate: f64, params: &ChainParams) -> Self {
        let wet = if params.bypass { 0.0 } else { 1.0 };
        Chain {
            current: params.stages.coeffs,
            target: params.stages.coeffs,
            state: [[BiquadState::default(); MAX_STAGES]; 2],
            active: params.stages.len,
            target_len: params.stages.len,
            gain: params.gain,
            gain_target: params.gain,
            wet,
            wet_target: wet,
            limit: params.limit,
            limit_gain: 1.0,
            k_release: 1.0 - (-1.0 / (LIMIT_RELEASE_SECONDS * sample_rate)).exp(),
            settled: true,
            countdown: 0,
            k_control: 1.0 - (-(CONTROL_INTERVAL as f64) / (SMOOTH_SECONDS * sample_rate)).exp(),
            k_sample: 1.0 - (-1.0 / (SMOOTH_SECONDS * sample_rate)).exp(),
        }
    }

    pub fn set_params(&mut self, params: &ChainParams) {
        self.target = params.stages.coeffs;
        self.target_len = params.stages.len;
        self.active = self.active.max(params.stages.len);
        self.gain_target = params.gain;
        self.wet_target = if params.bypass { 0.0 } else { 1.0 };
        self.limit = params.limit;
        self.settled = false;
    }

    #[inline]
    pub fn tick(&mut self, input: [f32; 2]) -> [f32; 2] {
        if !self.settled {
            self.smooth();
        }
        let mut out = [0f64; 2];
        for ch in 0..2 {
            let dry = input[ch] as f64;
            let mut x = dry * self.gain[ch];
            for (state, coeffs) in self.state[ch][..self.active]
                .iter_mut()
                .zip(&self.current[..self.active])
            {
                x = state.tick(coeffs, x);
            }
            out[ch] = dry + self.wet * (x - dry);
        }
        if self.limit || self.limit_gain < 1.0 {
            self.limit(&mut out);
        }
        [out[0] as f32, out[1] as f32]
    }

    /// Turns both channels down together the instant a peak would pass the ceiling, then
    /// eases back. Leaves samples untouched while idle, so quiet audio passes bit for bit.
    #[inline]
    fn limit(&mut self, out: &mut [f64; 2]) {
        let peak = out[0].abs().max(out[1].abs());
        if self.limit && peak * self.limit_gain > LIMIT_CEILING {
            self.limit_gain = LIMIT_CEILING / peak;
        } else if self.limit_gain < 1.0 {
            self.limit_gain += (1.0 - self.limit_gain) * self.k_release;
            if self.limit_gain > 1.0 - 1e-6 {
                self.limit_gain = 1.0;
            }
        }
        if self.limit_gain < 1.0 {
            out[0] *= self.limit_gain;
            out[1] *= self.limit_gain;
        }
    }

    fn smooth(&mut self) {
        for ch in 0..2 {
            self.gain[ch] += (self.gain_target[ch] - self.gain[ch]) * self.k_sample;
        }
        self.wet += (self.wet_target - self.wet) * self.k_sample;

        if self.countdown > 0 {
            self.countdown -= 1;
            return;
        }
        self.countdown = CONTROL_INTERVAL - 1;

        let mut max_diff = 0f64;
        for (cur, tgt) in self.current[..self.active].iter_mut().zip(&self.target) {
            max_diff = max_diff.max(cur.approach(tgt, self.k_control));
        }
        for ch in 0..2 {
            max_diff = max_diff.max((self.gain_target[ch] - self.gain[ch]).abs());
        }
        max_diff = max_diff.max((self.wet_target - self.wet).abs());
        if max_diff < 1e-7 {
            self.current = self.target;
            self.gain = self.gain_target;
            self.wet = self.wet_target;
            // Stages no longer in use are identity filters by now. Clear them so they
            // start from silence when they come back.
            for state in &mut self.state {
                state[self.target_len..self.active].fill(BiquadState::default());
            }
            self.active = self.target_len;
            self.settled = true;
            self.countdown = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Band, FilterKind, Mode};

    const FS: f64 = 48_000.0;

    fn sine(i: usize, freq: f64) -> f32 {
        (2.0 * std::f64::consts::PI * freq * i as f64 / FS).sin() as f32 * 0.25
    }

    /// Peak output level over the last `measure` frames of a `total`-frame sine run.
    fn steady_peak(chain: &mut Chain, freq: f64, total: usize, measure: usize) -> f32 {
        let mut peak = 0f32;
        for i in 0..total {
            let x = sine(i, freq);
            let y = chain.tick([x, x]);
            if i >= total - measure {
                peak = peak.max(y[0].abs());
            }
        }
        peak
    }

    fn state_with_peak(gain_db: f32) -> EqState {
        let mut band = Band::new(FilterKind::Peak, 1_000.0);
        band.gain_db = gain_db;
        band.q = 1.0;
        EqState {
            bands: vec![band],
            clip_protection: false,
            ..EqState::default()
        }
    }

    #[test]
    fn clip_protection_holds_peaks_under_full_scale() {
        // +24 dB on a 0.25 sine would peak near 4.0.
        let mut state = state_with_peak(24.0);
        let mut chain = Chain::new(FS, &ChainParams::from_state(&state, FS));
        assert!(steady_peak(&mut chain, 1_000.0, 48_000, 48_000) > 3.0);

        state.clip_protection = true;
        let mut chain = Chain::new(FS, &ChainParams::from_state(&state, FS));
        let peak = steady_peak(&mut chain, 1_000.0, 48_000, 48_000);
        assert!(peak < 1.0 && peak > 0.9, "{peak}");

        // Once the boost is gone the limiter lets go completely.
        chain.set_params(&ChainParams::from_state(&EqState::default(), FS));
        steady_peak(&mut chain, 1_000.0, 96_000, 1);
        assert_eq!(chain.tick([0.25, -0.25]), [0.25, -0.25]);
    }

    #[test]
    fn flat_chain_is_bit_transparent() {
        let params = ChainParams::from_state(&EqState::default(), FS);
        let mut chain = Chain::new(FS, &params);
        for i in 0..4_800 {
            let x = [sine(i, 441.0), sine(i, 3_000.0)];
            assert_eq!(chain.tick(x), x);
        }
    }

    #[test]
    fn peak_band_boosts_by_its_gain() {
        let params = ChainParams::from_state(&state_with_peak(6.0), FS);
        let mut chain = Chain::new(FS, &params);
        let peak = steady_peak(&mut chain, 1_000.0, 48_000, 4_800);
        let expected = 0.25 * 10f32.powf(6.0 / 20.0);
        assert!((peak - expected).abs() < 1e-3, "{peak} vs {expected}");
    }

    #[test]
    fn parameter_change_converges_without_jumps() {
        let mut chain = Chain::new(FS, &ChainParams::from_state(&state_with_peak(0.0), FS));
        chain.set_params(&ChainParams::from_state(&state_with_peak(12.0), FS));
        let mut prev = 0f32;
        let mut max_step = 0f32;
        for i in 0..48_000 {
            let x = sine(i, 1_000.0);
            let y = chain.tick([x, x])[0];
            max_step = max_step.max((y - prev).abs());
            prev = y;
        }
        assert!(chain.settled);
        // A 1 kHz sine at +12 dB moves at most ~0.13 per sample; a click would exceed that.
        assert!(max_step < 0.14, "max step {max_step}");
        let peak = steady_peak(&mut chain, 1_000.0, 4_800, 2_400);
        assert!((peak - 0.25 * 10f32.powf(12.0 / 20.0)).abs() < 1e-2);
    }

    #[test]
    fn balance_turns_down_one_channel() {
        let state = EqState {
            balance: 0.5,
            ..EqState::default()
        };
        let mut chain = Chain::new(FS, &ChainParams::from_state(&state, FS));
        let out = chain.tick([0.5, 0.5]);
        assert!((out[0] - 0.25).abs() < 1e-3, "{out:?}");
        assert_eq!(out[1], 0.5);
    }

    #[test]
    fn graphic_mode_plays_its_sliders() {
        let mut state = EqState {
            mode: Mode::Graphic,
            clip_protection: false,
            ..EqState::default()
        };
        state.graphic.set_band_count(31);
        state.graphic.gains_db.fill(6.0);
        let mut chain = Chain::new(FS, &ChainParams::from_state(&state, FS));
        let peak = steady_peak(&mut chain, 1_000.0, 48_000, 4_800);
        let expected = 0.25 * 10f32.powf(6.0 / 20.0);
        assert!((peak - expected).abs() < 0.01, "{peak} vs {expected}");

        // Back to parametric: the graphic filters fade away and the chain shrinks.
        state.mode = Mode::Parametric;
        chain.set_params(&ChainParams::from_state(&state, FS));
        steady_peak(&mut chain, 1_000.0, 48_000, 1);
        assert!(chain.settled);
        assert_eq!(chain.active, 2 + state.bands.len());
    }

    #[test]
    fn bypass_and_preamp() {
        let mut state = state_with_peak(12.0);
        state.preamp_db = -6.0;
        let mut chain = Chain::new(FS, &ChainParams::from_state(&state, FS));
        let peak = steady_peak(&mut chain, 1_000.0, 48_000, 4_800);
        assert!((peak - 0.25 * 10f32.powf(6.0 / 20.0)).abs() < 1e-3);

        state.bypass = true;
        chain.set_params(&ChainParams::from_state(&state, FS));
        let peak = steady_peak(&mut chain, 1_000.0, 48_000, 4_800);
        assert!((peak - 0.25).abs() < 1e-4, "{peak}");
    }
}
