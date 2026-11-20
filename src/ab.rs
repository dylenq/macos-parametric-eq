use crate::dsp::chain::ChainParams;
use crate::dsp::response::loudness_db;
use crate::state::EqState;

/// A before-and-after comparison. A is the sound remembered when it was set, B is the
/// equalizer as it is now. The louder of the two is turned down to the quieter's level, so
/// a change cannot win, or lose, for being louder.
#[derive(Clone, Debug)]
pub struct Ab {
    /// The sound remembered as A.
    pub reference: EqState,
    /// A is playing rather than the current settings.
    pub hearing_a: bool,
}

impl Ab {
    /// Remembers `current` as A, and keeps playing B.
    pub fn new(current: &EqState) -> Ab {
        Ab {
            reference: current.clone(),
            hearing_a: false,
        }
    }

    pub fn toggle(&mut self) {
        self.hearing_a = !self.hearing_a;
    }

    /// Call when the current settings are edited: you hear what you are changing.
    pub fn edited(&mut self) {
        self.hearing_a = false;
    }

    /// How far A and B are each turned down, in dB, to play at the same loudness. The
    /// quieter one is left alone, so neither is ever boosted.
    pub fn trims(&self, current: &EqState, sample_rate: f64) -> (f32, f32) {
        let a = loudness_db(&self.reference, sample_rate);
        let b = loudness_db(current, sample_rate);
        let target = a.min(b);
        (target - a, target - b)
    }

    /// What the audio thread should play: A's sound or the current one, matched. Bypass,
    /// clip protection and the sweep are ways of listening, so they always come from the
    /// current settings.
    pub fn params(&self, current: &EqState, sample_rate: f64) -> ChainParams {
        let (trim_a, trim_b) = self.trims(current, sample_rate);
        if !self.hearing_a {
            return ChainParams::from_state(current, sample_rate).trimmed(trim_b);
        }
        let mut a = self.reference.clone();
        a.bypass = current.bypass;
        a.clip_protection = current.clip_protection;
        a.sweep = current.sweep;
        ChainParams::from_state(&a, sample_rate).trimmed(trim_a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::stages::Stages;
    use crate::state::{Band, FilterKind};

    const FS: f64 = 48_000.0;

    fn with_cut(gain_db: f32) -> EqState {
        let mut band = Band::new(FilterKind::Peak, 2_800.0);
        band.gain_db = gain_db;
        band.q = 0.7;
        EqState { bands: vec![band], ..EqState::default() }
    }

    #[test]
    fn the_louder_side_is_turned_down_and_the_quieter_left_alone() {
        let before = EqState::default();
        let after = with_cut(-6.0);
        let ab = Ab::new(&before);
        let (trim_a, trim_b) = ab.trims(&after, FS);
        // The cut made B quieter, so A comes down to meet it.
        assert!(trim_a < -0.5, "{trim_a}");
        assert_eq!(trim_b, 0.0);
        assert_eq!(Ab::new(&after).trims(&after, FS), (0.0, 0.0));
        // A boost the other way round.
        let (trim_a, trim_b) = ab.trims(&with_cut(6.0), FS);
        assert_eq!(trim_a, 0.0);
        assert!(trim_b < -0.5);
    }

    #[test]
    fn a_plays_its_own_filters_with_the_current_way_of_listening() {
        let before = EqState::default();
        let mut after = with_cut(-6.0);
        after.clip_protection = false;
        after.bypass = true;
        let mut ab = Ab::new(&before);
        let b = ab.params(&after, FS);
        assert_eq!(b.stages, Stages::new(&after, FS));
        ab.toggle();
        let a = ab.params(&after, FS);
        assert_eq!(a.stages, Stages::new(&before, FS));
        assert!(a.bypass && !a.limit);
        // A, the louder, is turned down; its preamp is 0 dB, so the gain is the trim alone.
        let (trim_a, _) = ab.trims(&after, FS);
        assert!((a.gain[0] - 10f64.powf(trim_a as f64 / 20.0)).abs() < 1e-9);
        ab.edited();
        assert!(!ab.hearing_a);
    }
}
