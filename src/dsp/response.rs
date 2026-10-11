use super::biquad::Coeffs;
use crate::state::{Band, EqState};

/// Response of a single band in dB at `freq`.
pub fn band_db(band: &Band, freq: f32, sample_rate: f64) -> f32 {
    Coeffs::for_band(band, sample_rate).magnitude_db(freq as f64, sample_rate) as f32
}

/// Summed response of every enabled band in dB at each of `freqs`. Preamp is not included.
pub fn total_db(state: &EqState, freqs: &[f32], sample_rate: f64) -> Vec<f32> {
    let coeffs: Vec<Coeffs> = state
        .bands
        .iter()
        .filter(|b| b.enabled)
        .map(|b| Coeffs::for_band(b, sample_rate))
        .collect();
    freqs
        .iter()
        .map(|&f| {
            coeffs
                .iter()
                .map(|c| c.magnitude_db(f as f64, sample_rate))
                .sum::<f64>() as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::FilterKind;

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
            bypass: false,
        };
        let db = total_db(&state, &[1_000.0], 48_000.0)[0];
        assert!((db - 2.5).abs() < 1e-4, "{db}");
        assert!((band_db(&a, 1_000.0, 48_000.0) - 4.0).abs() < 1e-4);
    }
}
