use crate::state::{Band, FilterKind};
use std::f64::consts::PI;

/// Normalized biquad coefficients (a0 == 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coeffs {
    pub const IDENTITY: Coeffs = Coeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// RBJ Audio EQ Cookbook designs.
    pub fn design(kind: FilterKind, freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Coeffs {
        let freq = freq.clamp(1.0, sample_rate * 0.49);
        let q = q.max(0.01);
        let a = 10f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);

        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            FilterKind::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - s),
                    (a + 1.0) + (a - 1.0) * cos + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - s,
                )
            }
            FilterKind::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - s),
                    (a + 1.0) - (a - 1.0) * cos + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - s,
                )
            }
            FilterKind::LowPass => (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterKind::HighPass => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
        };

        Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    pub fn for_band(band: &Band, sample_rate: f64) -> Coeffs {
        if !band.enabled {
            return Coeffs::IDENTITY;
        }
        Coeffs::design(
            band.kind,
            band.freq as f64,
            band.gain_db as f64,
            band.q as f64,
            sample_rate,
        )
    }

    /// Magnitude response in dB at `freq`.
    pub fn magnitude_db(&self, freq: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * PI * freq / sample_rate;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = -(self.b1 * s1 + self.b2 * s2);
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = -(self.a1 * s1 + self.a2 * s2);
        let num = num_re * num_re + num_im * num_im;
        let den = den_re * den_re + den_im * den_im;
        10.0 * (num / den).max(1e-30).log10()
    }

    /// Moves each coefficient a fraction `k` of the way to `target`. The stable region of
    /// (a1, a2) is convex, so every intermediate filter is stable.
    pub fn approach(&mut self, target: &Coeffs, k: f64) -> f64 {
        let mut max_diff = 0f64;
        for (cur, tgt) in [
            (&mut self.b0, target.b0),
            (&mut self.b1, target.b1),
            (&mut self.b2, target.b2),
            (&mut self.a1, target.a1),
            (&mut self.a2, target.a2),
        ] {
            let diff = tgt - *cur;
            *cur += diff * k;
            max_diff = max_diff.max(diff.abs());
        }
        max_diff
    }
}

/// Transposed direct form II state for one channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct BiquadState {
    s1: f64,
    s2: f64,
}

impl BiquadState {
    #[inline]
    pub fn tick(&mut self, c: &Coeffs, x: f64) -> f64 {
        let y = c.b0 * x + self.s1;
        self.s1 = c.b1 * x - c.a1 * y + self.s2;
        self.s2 = c.b2 * x - c.a2 * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48_000.0;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn butterworth_lowpass_at_quarter_rate() {
        let c = Coeffs::design(FilterKind::LowPass, FS / 4.0, 0.0, 0.5f64.sqrt(), FS);
        assert!(close(c.b0, 0.292_893_2, 1e-6));
        assert!(close(c.b1, 0.585_786_4, 1e-6));
        assert!(close(c.b2, 0.292_893_2, 1e-6));
        assert!(close(c.a1, 0.0, 1e-9));
        assert!(close(c.a2, 0.171_572_9, 1e-6));
    }

    #[test]
    fn peak_hits_gain_at_center_only() {
        let c = Coeffs::design(FilterKind::Peak, 1_000.0, 6.0, 2.0, FS);
        assert!(close(c.magnitude_db(1_000.0, FS), 6.0, 1e-6));
        assert!(close(c.magnitude_db(20.0, FS), 0.0, 0.05));
        assert!(close(c.magnitude_db(20_000.0, FS), 0.0, 0.05));
    }

    #[test]
    fn shelves_reach_gain_on_their_side() {
        let lo = Coeffs::design(FilterKind::LowShelf, 200.0, -9.0, 0.707, FS);
        assert!(close(lo.magnitude_db(5.0, FS), -9.0, 0.05));
        assert!(close(lo.magnitude_db(15_000.0, FS), 0.0, 0.05));
        let hi = Coeffs::design(FilterKind::HighShelf, 4_000.0, 5.0, 0.707, FS);
        assert!(close(hi.magnitude_db(23_000.0, FS), 5.0, 0.05));
        assert!(close(hi.magnitude_db(30.0, FS), 0.0, 0.05));
    }

    #[test]
    fn pass_filters_are_3db_down_at_cutoff() {
        let q = 0.5f64.sqrt();
        let lp = Coeffs::design(FilterKind::LowPass, 1_000.0, 0.0, q, FS);
        let hp = Coeffs::design(FilterKind::HighPass, 1_000.0, 0.0, q, FS);
        assert!(close(lp.magnitude_db(1_000.0, FS), -3.0103, 1e-3));
        assert!(close(hp.magnitude_db(1_000.0, FS), -3.0103, 1e-3));
    }

    #[test]
    fn zero_gain_peak_is_identity() {
        let c = Coeffs::design(FilterKind::Peak, 1_000.0, 0.0, 1.0, FS);
        for f in [20.0, 1_000.0, 18_000.0] {
            assert!(close(c.magnitude_db(f, FS), 0.0, 1e-9));
        }
    }
}
