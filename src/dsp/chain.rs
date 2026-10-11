use super::biquad::{BiquadState, Coeffs};
use crate::state::{EqState, MAX_BANDS};

/// How often (in frames) filter coefficients step toward their targets.
const CONTROL_INTERVAL: u32 = 32;
/// Smoothing time constant for coefficients, preamp and bypass.
const SMOOTH_SECONDS: f64 = 0.010;
/// The limiter keeps peaks at or under this, 0.1 dB below full scale.
const LIMIT_CEILING: f64 = 0.988_553;
/// How quickly the limiter lets go once the signal drops back under the ceiling.
const LIMIT_RELEASE_SECONDS: f64 = 0.100;

/// Everything the audio thread needs, precomputed on the UI thread.
#[derive(Clone, Copy, Debug)]
pub struct ChainParams {
    pub coeffs: [Coeffs; MAX_BANDS],
    /// Linear preamp gain.
    pub preamp: f64,
    pub bypass: bool,
    /// Turn the output down for as long as it would otherwise exceed full scale.
    pub limit: bool,
}

impl ChainParams {
    pub fn from_state(state: &EqState, sample_rate: f64) -> Self {
        let mut coeffs = [Coeffs::IDENTITY; MAX_BANDS];
        for (slot, band) in coeffs.iter_mut().zip(&state.bands) {
            *slot = Coeffs::for_band(band, sample_rate);
        }
        ChainParams {
            coeffs,
            preamp: 10f64.powf(state.preamp_db as f64 / 20.0),
            bypass: state.bypass,
            limit: state.clip_protection,
        }
    }
}

/// Stereo biquad cascade with click-free parameter changes. Allocation-free, so it is safe
/// to run on the audio thread.
pub struct Chain {
    current: [Coeffs; MAX_BANDS],
    target: [Coeffs; MAX_BANDS],
    state: [[BiquadState; MAX_BANDS]; 2],
    preamp: f64,
    preamp_target: f64,
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
            current: params.coeffs,
            target: params.coeffs,
            state: [[BiquadState::default(); MAX_BANDS]; 2],
            preamp: params.preamp,
            preamp_target: params.preamp,
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
        self.target = params.coeffs;
        self.preamp_target = params.preamp;
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
            let mut x = dry * self.preamp;
            for (state, coeffs) in self.state[ch].iter_mut().zip(&self.current) {
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
        self.preamp += (self.preamp_target - self.preamp) * self.k_sample;
        self.wet += (self.wet_target - self.wet) * self.k_sample;

        if self.countdown > 0 {
            self.countdown -= 1;
            return;
        }
        self.countdown = CONTROL_INTERVAL - 1;

        let mut max_diff = 0f64;
        for (cur, tgt) in self.current.iter_mut().zip(&self.target) {
            max_diff = max_diff.max(cur.approach(tgt, self.k_control));
        }
        max_diff = max_diff
            .max((self.preamp_target - self.preamp).abs())
            .max((self.wet_target - self.wet).abs());
        if max_diff < 1e-7 {
            self.current = self.target;
            self.preamp = self.preamp_target;
            self.wet = self.wet_target;
            self.settled = true;
            self.countdown = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Band, FilterKind};

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
