use super::biquad::Coeffs;
use super::graphic;
use crate::state::{EqState, FilterKind, MAX_BANDS, Mode};

pub const BASS_HZ: f64 = 100.0;
pub const TREBLE_HZ: f64 = 10_000.0;
const TONE_Q: f64 = 0.707;
/// Balance never turns a channel down further than this.
pub const MAX_BALANCE_CUT_DB: f32 = -60.0;
/// Bass, treble, and the most filters either mode uses.
pub const MAX_STAGES: usize =
    2 + if graphic::MAX_FILTERS > MAX_BANDS { graphic::MAX_FILTERS } else { MAX_BANDS };

/// The filters a state runs through, in order. The first two are bass and treble; each
/// keeps its slot from one call to the next, so a change glides instead of jumping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stages {
    pub coeffs: [Coeffs; MAX_STAGES],
    /// Slots in use. The rest are identity filters.
    pub len: usize,
}

impl Stages {
    pub fn new(state: &EqState, sample_rate: f64) -> Stages {
        let mut coeffs = [Coeffs::IDENTITY; MAX_STAGES];
        let shelf = |kind, freq, gain_db: f32| {
            if gain_db == 0.0 {
                Coeffs::IDENTITY
            } else {
                Coeffs::design(kind, freq, gain_db as f64, TONE_Q, sample_rate)
            }
        };
        coeffs[0] = shelf(FilterKind::LowShelf, BASS_HZ, state.bass_db);
        coeffs[1] = shelf(FilterKind::HighShelf, TREBLE_HZ, state.treble_db);
        let editor: Vec<Coeffs> = match state.mode {
            Mode::Parametric => state
                .bands
                .iter()
                .take(MAX_BANDS)
                .map(|b| Coeffs::for_band(b, sample_rate))
                .collect(),
            Mode::Graphic => {
                graphic::design(state.graphic.band_count, &state.graphic.gains_db, sample_rate)
            }
        };
        for (slot, c) in coeffs[2..].iter_mut().zip(&editor) {
            *slot = *c;
        }
        Stages {
            coeffs,
            len: 2 + editor.len().min(MAX_STAGES - 2),
        }
    }

    /// Summed response of every stage in dB at `freq`.
    pub fn magnitude_db(&self, freq: f64, sample_rate: f64) -> f64 {
        self.coeffs[..self.len]
            .iter()
            .filter(|c| **c != Coeffs::IDENTITY)
            .map(|c| c.magnitude_db(freq, sample_rate))
            .sum()
    }
}

/// How far balance turns each channel down, in dB, as [left, right]. Only the channel
/// away from the side it leans to is turned down.
pub fn balance_db(balance: f32) -> [f32; 2] {
    if balance == 0.0 {
        return [0.0, 0.0];
    }
    let rest = 1.0 - balance.abs().min(1.0);
    let cut = if rest <= 0.0 {
        MAX_BALANCE_CUT_DB
    } else {
        (20.0 * rest.log10()).max(MAX_BALANCE_CUT_DB)
    };
    if balance > 0.0 { [cut, 0.0] } else { [0.0, cut] }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48_000.0;

    #[test]
    fn balance_turns_down_the_other_side() {
        let [l, r] = balance_db(0.5);
        assert!((l + 6.0206).abs() < 1e-3 && r == 0.0, "{l} {r}");
        assert_eq!(balance_db(-1.0), [0.0, MAX_BALANCE_CUT_DB]);
        assert_eq!(balance_db(0.0), [0.0, 0.0]);
    }

    #[test]
    fn tone_shelves_reach_their_gain() {
        let state = EqState {
            bass_db: 6.0,
            treble_db: -4.0,
            bands: Vec::new(),
            ..EqState::default()
        };
        let stages = Stages::new(&state, FS);
        assert_eq!(stages.len, 2);
        assert!((stages.magnitude_db(25.0, FS) - 6.0).abs() < 0.3);
        assert!((stages.magnitude_db(19_000.0, FS) + 4.0).abs() < 0.3);
        assert!(stages.magnitude_db(1_000.0, FS).abs() < 0.3);
    }

    #[test]
    fn flat_settings_are_all_identity() {
        let mut state = EqState::default();
        assert!(Stages::new(&state, FS).coeffs[..2].iter().all(|c| *c == Coeffs::IDENTITY));
        state.mode = Mode::Graphic;
        state.graphic.set_band_count(31);
        let stages = Stages::new(&state, FS);
        assert_eq!(stages.len, 2 + 33);
        assert!(stages.coeffs.iter().all(|c| *c == Coeffs::IDENTITY));
    }
}
