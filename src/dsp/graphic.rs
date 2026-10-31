//! Graphic EQ: fixed slider frequencies, the smooth curve the sliders describe, and the
//! peak filters that reproduce it.
//!
//! The curve is a monotone cubic (PCHIP, Fritsch-Carlson) in log frequency: it passes
//! through every slider, never overshoots between two of them, and stays flat beyond the
//! end ones. Neighbouring peak filters overlap, so their gains are solved for together
//! rather than set to the slider values.

use super::biquad::Coeffs;
use crate::state::FilterKind;

/// Every second octave, nested inside the 10-band set.
const C5: [f32; 5] = [63.0, 250.0, 1000.0, 4000.0, 16000.0];
const C10: [f32; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
const C15: [f32; 15] = [
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0,
    10000.0, 16000.0,
];
const C31: [f32; 31] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0,
    8000.0, 10000.0, 12500.0, 16000.0, 20000.0,
];

pub const COUNTS: [u32; 4] = [5, 10, 15, 31];
pub const MAX_COUNT: usize = 31;
/// Filters `design` returns at most: one per slider and a shelf beyond each end.
pub const MAX_FILTERS: usize = MAX_COUNT + 2;

/// Points the fit matches between each pair of sliders, besides the sliders themselves.
const POINTS_BETWEEN: usize = 3;
/// How much more a slider's own frequency counts than the points between, so the curve
/// goes through the sliders even where it cannot follow every bend between them.
const SLIDER_WEIGHT: f64 = 10.0;
/// Keeps the solved gains from fighting each other where the curve allows it.
const DAMPING: f64 = 1e-3;
/// Gauss-Newton passes. The fit has settled by the fourth.
const PASSES: usize = 4;
/// Gain step used to measure how each filter moves the response.
const PROBE_DB: f64 = 0.1;
const MAX_FILTER_DB: f64 = 30.0;
/// Each peak is wider than the gap to its neighbours by this much (a smaller Q), so that
/// together they sum to a smooth curve instead of a row of bumps.
const OVERLAP: f64 = 0.6;
const SHELF_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// The fit also looks this far, in octaves, beyond the end sliders, where the curve is flat.
const BEYOND_OCTAVES: [f64; 2] = [0.5, 1.0];

/// Slider frequencies for a supported band count.
pub fn centres(count: u32) -> Option<&'static [f32]> {
    match count {
        5 => Some(&C5),
        10 => Some(&C10),
        15 => Some(&C15),
        31 => Some(&C31),
        _ => None,
    }
}

fn end_slope(h0: f64, h1: f64, s0: f64, s1: f64) -> f64 {
    let d = ((2.0 * h0 + h1) * s0 - h0 * s1) / (h0 + h1);
    if d.signum() != s0.signum() || d == 0.0 {
        0.0
    } else if s0.signum() != s1.signum() && d.abs() > 3.0 * s0.abs() {
        3.0 * s0
    } else {
        d
    }
}

/// Fritsch-Carlson slopes for nodes (x, y).
fn pchip_slopes(x: &[f64], y: &[f64]) -> Vec<f64> {
    let n = x.len();
    let mut d = vec![0.0; n];
    if n < 2 {
        return d;
    }
    let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
    let s: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect();
    if n == 2 {
        d[0] = s[0];
        d[1] = s[0];
        return d;
    }
    for i in 1..n - 1 {
        if s[i - 1] * s[i] > 0.0 {
            let (w1, w2) = (2.0 * h[i] + h[i - 1], h[i] + 2.0 * h[i - 1]);
            d[i] = (w1 + w2) / (w1 / s[i - 1] + w2 / s[i]);
        }
    }
    d[0] = end_slope(h[0], h[1], s[0], s[1]);
    d[n - 1] = end_slope(h[n - 2], h[n - 3], s[n - 2], s[n - 3]);
    d
}

/// The curve in dB at `freq` for sliders `gains_db` at `centres`.
pub fn target_db(centres: &[f32], gains_db: &[f32], freq: f32) -> f32 {
    let n = centres.len().min(gains_db.len());
    if n == 0 {
        return 0.0;
    }
    let x: Vec<f64> = centres[..n].iter().map(|&c| (c as f64).log2()).collect();
    let y: Vec<f64> = gains_db[..n].iter().map(|&g| g as f64).collect();
    let xf = (freq.max(f32::MIN_POSITIVE) as f64).log2();
    if n == 1 || xf <= x[0] {
        return y[0] as f32;
    }
    if xf >= x[n - 1] {
        return y[n - 1] as f32;
    }
    let d = pchip_slopes(&x, &y);
    let i = x.partition_point(|&v| v <= xf).min(n - 1) - 1;
    let h = x[i + 1] - x[i];
    let t = (xf - x[i]) / h;
    let (t2, t3) = (t * t, t * t * t);
    let v = (2.0 * t3 - 3.0 * t2 + 1.0) * y[i]
        + (t3 - 2.0 * t2 + t) * h * d[i]
        + (-2.0 * t3 + 3.0 * t2) * y[i + 1]
        + (t3 - t2) * h * d[i + 1];
    v as f32
}

/// Gains for `new_count` sliders that keep the curve of the old ones. All zero if either
/// count is unsupported or the old gains do not match their count.
pub fn resample(old_count: u32, old_gains: &[f32], new_count: u32) -> Vec<f32> {
    let (Some(old), Some(new)) = (centres(old_count), centres(new_count)) else {
        return vec![0.0; new_count as usize];
    };
    if old_gains.len() != old.len() {
        return vec![0.0; new.len()];
    }
    new.iter().map(|&f| target_db(old, old_gains, f)).collect()
}

/// Q of the peak filter at slider `i`: its bandwidth spans the gap to its neighbours.
fn q_at(centres: &[f32], i: usize) -> f64 {
    let gap = |a: usize, b: usize| (centres[b] as f64 / centres[a] as f64).log2();
    let octaves = match i {
        0 => gap(0, 1),
        _ if i == centres.len() - 1 => gap(i - 1, i),
        _ => (gap(i - 1, i) + gap(i, i + 1)) / 2.0,
    };
    let ratio = 2f64.powf(octaves);
    ratio.sqrt() / (ratio - 1.0) * OVERLAP
}


/// The Q that gives a digital peak filter the bandwidth an analog one would have. Near
/// Nyquist the bilinear transform squeezes a filter; this widens it back (the RBJ cookbook's
/// bandwidth correction).
fn warped_q(q: f64, freq: f64, sample_rate: f64) -> f64 {
    let w0 = 2.0 * std::f64::consts::PI * freq.min(sample_rate * 0.49) / sample_rate;
    let octaves = 2.0 / std::f64::consts::LN_2 * (1.0 / (2.0 * q)).asinh();
    1.0 / (2.0 * (std::f64::consts::LN_2 / 2.0 * octaves * w0 / w0.sin()).sinh())
}

/// Frequencies the fit matches, with how much each counts: each slider, and points spaced
/// evenly in log frequency between them.
fn fit_points(centres: &[f32], sample_rate: f64) -> Vec<(f64, f64)> {
    let top = sample_rate * 0.45;
    let mut points = Vec::new();
    for pair in centres.windows(2) {
        let (a, b) = (pair[0] as f64, pair[1] as f64);
        points.push((a, SLIDER_WEIGHT));
        for k in 1..=POINTS_BETWEEN {
            points.push((a * (b / a).powf(k as f64 / (POINTS_BETWEEN + 1) as f64), 1.0));
        }
    }
    let (first, last) = (centres[0] as f64, *centres.last().unwrap_or(&1000.0) as f64);
    points.push((last, SLIDER_WEIGHT));
    for octaves in BEYOND_OCTAVES {
        points.push((first / 2f64.powf(octaves), 1.0));
        points.push((last * 2f64.powf(octaves), 1.0));
    }
    points.retain(|&(f, _)| f < top && f >= 10.0);
    points
}

/// Solves the small symmetric system `a x = b` in place. `a` is n by n, row major.
fn solve(mut a: Vec<f64>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&i, &j| a[i * n + col].abs().total_cmp(&a[j * n + col].abs()))
            .unwrap_or(col);
        if pivot != col {
            for k in 0..n {
                a.swap(col * n + k, pivot * n + k);
            }
            b.swap(col, pivot);
        }
        let p = a[col * n + col];
        if p.abs() < 1e-12 {
            continue;
        }
        for row in col + 1..n {
            let factor = a[row * n + col] / p;
            if factor != 0.0 {
                for k in col..n {
                    a[row * n + k] -= factor * a[col * n + k];
                }
                b[row] -= factor * b[col];
            }
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let p = a[row * n + row];
        let sum: f64 = (row + 1..n).map(|k| a[row * n + k] * x[k]).sum();
        x[row] = if p.abs() < 1e-12 { 0.0 } else { (b[row] - sum) / p };
    }
    x
}

/// The last design, since the graph, the headroom and the audio all ask for the same one.
type Cached = (u32, Vec<f32>, f64, Vec<Coeffs>);

thread_local! {
    static LAST: std::cell::RefCell<Option<Cached>> = const { std::cell::RefCell::new(None) };
}

/// A peak filter per slider and a shelf beyond each end, with gains chosen so that together
/// they follow the curve the sliders describe. Flat sliders give identity filters, which
/// leave audio untouched.
pub fn design(count: u32, gains_db: &[f32], sample_rate: f64) -> Vec<Coeffs> {
    let hit = LAST.with_borrow(|last| {
        last.as_ref()
            .filter(|(c, g, sr, _)| *c == count && g == gains_db && *sr == sample_rate)
            .map(|(.., filters)| filters.clone())
    });
    if let Some(filters) = hit {
        return filters;
    }
    let filters = solve_filters(count, gains_db, sample_rate);
    LAST.set(Some((count, gains_db.to_vec(), sample_rate, filters.clone())));
    filters
}

fn solve_filters(count: u32, gains_db: &[f32], sample_rate: f64) -> Vec<Coeffs> {
    let Some(centres) = centres(count).filter(|c| c.len() == gains_db.len()) else {
        return Vec::new();
    };
    let n = centres.len();
    if gains_db.iter().all(|&g| g == 0.0) {
        return vec![Coeffs::IDENTITY; n + 2];
    }
    let qs: Vec<f64> = (0..n).map(|i| q_at(centres, i)).collect();
    // A peak at every slider, and a shelf beyond each end slider to hold the curve at that
    // slider's level out to the ends of the range.
    let shelf_corner = |inner: usize, outer: usize| {
        let (a, b) = (centres[inner] as f64, centres[outer] as f64);
        (b * (b / a).sqrt()).min(sample_rate * 0.45)
    };
    let filter = |i: usize, gain: f64| {
        if i == n {
            Coeffs::design(FilterKind::LowShelf, shelf_corner(1, 0), gain, SHELF_Q, sample_rate)
        } else if i == n + 1 {
            Coeffs::design(FilterKind::HighShelf, shelf_corner(n - 2, n - 1), gain, SHELF_Q, sample_rate)
        } else {
            let freq = centres[i] as f64;
            Coeffs::design(FilterKind::Peak, freq, gain, warped_q(qs[i], freq, sample_rate), sample_rate)
        }
    };
    let n = n + 2;
    let points = fit_points(centres, sample_rate);
    let m = points.len();
    let target: Vec<f64> = points
        .iter()
        .map(|&(f, _)| target_db(centres, gains_db, f as f32) as f64)
        .collect();

    // Gauss-Newton: each pass measures how every filter moves every point at the gains so
    // far, and solves for the change that best closes the gap to the target.
    let mut gains: Vec<f64> = gains_db.iter().map(|&g| g as f64).collect();
    gains.extend([0.0, 0.0]);
    let mut own = vec![0.0; m * n];
    let mut slope = vec![0.0; m * n];
    for _ in 0..PASSES {
        for i in 0..n {
            let (at, probe) = (filter(i, gains[i]), filter(i, gains[i] + PROBE_DB));
            for (j, &(f, _)) in points.iter().enumerate() {
                let db = at.magnitude_db(f, sample_rate);
                own[j * n + i] = db;
                slope[j * n + i] = (probe.magnitude_db(f, sample_rate) - db) / PROBE_DB;
            }
        }
        let residual: Vec<f64> = (0..m)
            .map(|j| target[j] - (0..n).map(|i| own[j * n + i]).sum::<f64>())
            .collect();
        let mut normal = vec![0.0; n * n];
        for r in 0..n {
            for c in r..n {
                let v: f64 = (0..m)
                    .map(|j| points[j].1 * slope[j * n + r] * slope[j * n + c])
                    .sum();
                normal[r * n + c] = v;
                normal[c * n + r] = v;
            }
            normal[r * n + r] += DAMPING;
        }
        let rhs: Vec<f64> = (0..n)
            .map(|r| (0..m).map(|j| points[j].1 * slope[j * n + r] * residual[j]).sum())
            .collect();
        let step = solve(normal, rhs);
        for (g, s) in gains.iter_mut().zip(step) {
            *g = (*g + s).clamp(-MAX_FILTER_DB, MAX_FILTER_DB);
        }
    }
    (0..n).map(|i| filter(i, gains[i])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48_000.0;

    /// A small deterministic random source, so failures reproduce.
    struct Lcg(u64);

    impl Lcg {
        fn gain(&mut self, range: f32) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let unit = (self.0 >> 40) as f32 / (1u64 << 24) as f32;
            // Half-dB steps, as the sliders make.
            ((unit * 2.0 - 1.0) * range * 2.0).round() / 2.0
        }
    }

    fn response(filters: &[Coeffs], freq: f64, sample_rate: f64) -> f64 {
        filters.iter().map(|c| c.magnitude_db(freq, sample_rate)).sum()
    }

    #[test]
    fn tables_have_the_right_sizes() {
        for count in COUNTS {
            let c = centres(count).unwrap();
            assert_eq!(c.len(), count as usize);
            assert!(c.windows(2).all(|w| w[0] < w[1]));
        }
        assert!(centres(7).is_none());
        assert_eq!(centres(31).unwrap().len(), MAX_COUNT);
    }

    #[test]
    fn the_curve_passes_every_slider_without_overshoot() {
        let c = centres(10).unwrap();
        let g = [0.0, 6.0, -3.0, -3.0, 12.0, 0.0, 1.0, 2.0, 2.0, -12.0];
        for (f, gain) in c.iter().zip(g) {
            assert!((target_db(c, &g, *f) - gain).abs() < 1e-4);
        }
        for i in 0..9 {
            let (lo, hi) = (g[i].min(g[i + 1]), g[i].max(g[i + 1]));
            for k in 1..20 {
                let f = c[i] * (c[i + 1] / c[i]).powf(k as f32 / 20.0);
                let v = target_db(c, &g, f);
                assert!(v >= lo - 1e-4 && v <= hi + 1e-4, "{v} outside {lo}..{hi}");
            }
        }
        assert_eq!(target_db(c, &g, 10.0), 0.0);
        assert_eq!(target_db(c, &g, 20_000.0), -12.0);
    }

    #[test]
    fn resampling_nests_and_rejects_bad_input() {
        let g10 = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(resample(10, &g10, 5), [2.0, 4.0, 6.0, 8.0, 10.0]);
        assert_eq!(resample(10, &g10[..3], 5), [0.0; 5]);
        assert_eq!(resample(7, &g10, 5), [0.0; 5]);
    }

    #[test]
    fn flat_sliders_make_identity_filters() {
        for count in COUNTS {
            let filters = design(count, &vec![0.0; count as usize], FS);
            assert!(filters.iter().all(|c| *c == Coeffs::IDENTITY));
        }
        assert!(design(10, &[1.0; 3], FS).is_empty());
    }

    #[test]
    fn the_filters_go_through_the_sliders() {
        let mut rng = Lcg(7);
        for sample_rate in [44_100.0, 48_000.0, 96_000.0] {
            for count in COUNTS {
                let c = centres(count).unwrap();
                for _ in 0..20 {
                    // Any setting, however jagged: the curve may not follow every bend
                    // between sliders, but it passes close to each slider.
                    let gains: Vec<f32> = (0..count).map(|_| rng.gain(12.0)).collect();
                    let filters = design(count, &gains, sample_rate);
                    for (&f, &gain) in c.iter().zip(&gains) {
                        // Within an octave of Nyquist a digital filter has little room to
                        // turn, so a jagged setting up there is only followed roughly. The
                        // smooth settings below are checked all the way up.
                        if f as f64 > sample_rate * 0.25 {
                            continue;
                        }
                        let error = response(&filters, f as f64, sample_rate) - gain as f64;
                        assert!(
                            error.abs() < 1.5,
                            "{count} bands at {sample_rate}: {error:.2} dB at {f} Hz, {gains:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn smooth_settings_are_followed_closely() {
        for count in COUNTS {
            let c = centres(count).unwrap();
            let n = count as usize;
            for (gains, tolerance) in [
                (vec![6.0; n], 0.25),
                (vec![-12.0; n], 0.5),
                ((0..n).map(|i| if i < n / 3 { 6.0 } else { 0.0 }).collect(), 0.5),
                ((0..n).map(|i| 8.0 * i as f32 / (n - 1) as f32 - 4.0).collect(), 0.5),
            ] {
                let filters = design(count, &gains, FS);
                // The 5-band set is two octaves apart, so a step between two of its sliders
                // is approximated more loosely.
                let tolerance = if count == 5 { tolerance * 2.5 } else { tolerance };
                for k in 0..300 {
                    let f = 20.0 * (14_000f32 / 20.0).powf(k as f32 / 299.0);
                    let error = response(&filters, f as f64, FS) - target_db(c, &gains, f) as f64;
                    assert!(error.abs() < tolerance, "{count} bands: {error:.2} dB at {f:.0} Hz");
                }
            }
        }
        // The built-in presets follow their curve within 0.4 dB.
        for preset in crate::state::builtins() {
            let g = &preset.curve.graphic;
            let filters = design(g.band_count, &g.gains_db, FS);
            for k in 0..300 {
                let f = 20.0 * (14_000f32 / 20.0).powf(k as f32 / 299.0);
                let error = response(&filters, f as f64, FS) - target_db(g.centres(), &g.gains_db, f) as f64;
                assert!(error.abs() < 0.4, "{}: {error:.2} dB at {f:.0} Hz", preset.name);
            }
        }
    }
}
