use super::biquad::Coeffs;
use super::stages::Stages;
use crate::state::{Band, EqState, MAX_FREQ, MAX_GAIN_DB, MIN_FREQ, Mode};
use std::cell::RefCell;

/// Points the peak search samples between 20 Hz and 20 kHz, besides the band centres.
const PEAK_POINTS: usize = 512;

/// A result with the filters, rate and frequencies it was worked out for.
type Cached<T> = Option<(Stages, f64, Vec<f32>, T)>;

thread_local! {
    /// The last curve and peak worked out. The window asks for the same ones every frame
    /// until something changes.
    static CURVE: RefCell<Cached<Vec<f32>>> = const { RefCell::new(None) };
    static PEAK: RefCell<Cached<f32>> = const { RefCell::new(None) };
}

/// Response of a single band in dB at `freq`.
pub fn band_db(band: &Band, freq: f32, sample_rate: f64) -> f32 {
    Coeffs::for_band(band, sample_rate).magnitude_db(freq as f64, sample_rate) as f32
}

/// Response of everything that shapes the sound, in dB at each of `freqs`: the bands or
/// sliders, plus bass and treble. Preamp and balance are not included.
pub fn total_db(state: &EqState, freqs: &[f32], sample_rate: f64) -> Vec<f32> {
    let stages = Stages::new(state, sample_rate);
    CURVE.with_borrow_mut(|last| {
        if let Some((s, rate, f, dbs)) = last
            && *s == stages
            && *rate == sample_rate
            && f == freqs
        {
            return dbs.clone();
        }
        let dbs: Vec<f32> = freqs
            .iter()
            .map(|&f| stages.magnitude_db(f as f64, sample_rate) as f32)
            .collect();
        *last = Some((stages, sample_rate, freqs.to_vec(), dbs.clone()));
        dbs
    })
}

/// The loudest point of the response in dB, before the preamp.
pub fn peak_db(state: &EqState, sample_rate: f64) -> f32 {
    // A narrow peak can fall between the points, so look at each band's own frequency.
    let own: Vec<f32> = match state.mode {
        Mode::Parametric => state.bands.iter().map(|b| b.freq).collect(),
        Mode::Graphic => state.graphic.centres().to_vec(),
    };
    let stages = Stages::new(state, sample_rate);
    PEAK.with_borrow_mut(|last| {
        if let Some((s, rate, f, peak)) = last
            && *s == stages
            && *rate == sample_rate
            && *f == own
        {
            return *peak;
        }
        let peak = (0..PEAK_POINTS)
            .map(|i| MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(i as f32 / (PEAK_POINTS - 1) as f32))
            .chain(own.iter().copied())
            .filter(|&f| (f as f64) < sample_rate / 2.0)
            .map(|f| stages.magnitude_db(f as f64, sample_rate) as f32)
            .fold(f32::MIN, f32::max);
        let peak = if peak.is_finite() { peak } else { 0.0 };
        *last = Some((stages, sample_rate, own, peak));
        peak
    })
}

/// A preamp that keeps a response peaking at `peak_db` from going over full scale: the peak
/// rounded up to a tenth of a dB, turned down, and never a boost.
pub fn suggested_preamp_db(peak_db: f32) -> f32 {
    if !peak_db.is_finite() || peak_db <= 0.0 {
        return 0.0;
    }
    let tenths = (peak_db as f64 * 10.0 - 1e-6).ceil();
    ((-tenths / 10.0) as f32).max(-MAX_GAIN_DB)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::FilterKind;

    #[test]
    fn suggested_preamp_rounds_up_and_never_boosts() {
        assert_eq!(suggested_preamp_db(5.01), -5.1);
        assert_eq!(suggested_preamp_db(5.1), -5.1);
        assert_eq!(suggested_preamp_db(0.01), -0.1);
        assert_eq!(suggested_preamp_db(11.99), -12.0);
        assert_eq!(suggested_preamp_db(40.0), -24.0);
        assert_eq!(suggested_preamp_db(0.0), 0.0);
        assert_eq!(suggested_preamp_db(-3.0), 0.0);
        assert_eq!(suggested_preamp_db(f32::NAN), 0.0);
    }

    #[test]
    fn the_peak_includes_tone_and_narrow_bands() {
        let mut band = Band::new(FilterKind::Peak, 1_234.0);
        band.gain_db = 9.0;
        band.q = 18.0;
        let mut state = EqState {
            bands: vec![band],
            ..EqState::default()
        };
        assert!((peak_db(&state, 48_000.0) - 9.0).abs() < 0.01);
        state.bands.clear();
        state.bass_db = 4.0;
        assert!((peak_db(&state, 48_000.0) - 4.0).abs() < 0.1);
        state.bass_db = -4.0;
        assert!(peak_db(&state, 48_000.0).abs() < 0.01);
    }

    #[test]
    fn repeated_questions_get_the_same_answer_until_something_changes() {
        let mut band = Band::new(FilterKind::Peak, 1_000.0);
        band.gain_db = 6.0;
        let mut state = EqState {
            bands: vec![band],
            ..EqState::default()
        };
        let freqs = [100.0, 1_000.0];
        let first = (peak_db(&state, 48_000.0), total_db(&state, &freqs, 48_000.0));
        assert_eq!((peak_db(&state, 48_000.0), total_db(&state, &freqs, 48_000.0)), first);
        state.bands[0].gain_db = 3.0;
        assert!((peak_db(&state, 48_000.0) - 3.0).abs() < 0.01);
        assert!((total_db(&state, &freqs, 48_000.0)[1] - 3.0).abs() < 0.01);
        // Another rate or other frequencies are asked afresh.
        assert_eq!(total_db(&state, &[1_000.0], 48_000.0).len(), 1);
        let at_96k: Vec<f32> = freqs.iter().map(|&f| band_db(&state.bands[0], f, 96_000.0)).collect();
        assert_eq!(total_db(&state, &freqs, 96_000.0), at_96k);
    }

    #[test]
    fn bands_sum_in_db() {
        let mut a = Band::new(FilterKind::Peak, 1_000.0);
        a.gain_db = 4.0;
        let mut b = a;
        b.gain_db = -1.5;
        let mut off = a;
        off.gain_db = 10.0;
        off.enabled = false;
        let state = EqState {
            bands: vec![a, b, off],
            preamp_db: 3.0,
            ..EqState::default()
        };
        let db = total_db(&state, &[1_000.0], 48_000.0)[0];
        assert!((db - 2.5).abs() < 1e-4, "{db}");
        assert!((band_db(&a, 1_000.0, 48_000.0) - 4.0).abs() < 1e-4);
    }
}
