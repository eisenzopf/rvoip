//! Levinson-Durbin recursion, 3GPP TS 26.190 §5.2.2.
//!
//! Solves the Yule-Walker system for the order-16 LP predictor:
//!
//! ```text
//! sum_{k=1..16} a_k · r(|i - k|) = -r(i),   i = 1..16
//! ```
//!
//! The spec defers the algorithm itself to a standard reference, so the
//! recursion below is the textbook one rather than anything AMR-specific. What
//! *is* AMR-specific is the fixed-point formulation — extended-precision
//! accumulation and Q-format bookkeeping — which will be written against this
//! float version.
//!
//! # Why this algorithm rather than a general solve
//!
//! The system is Toeplitz, so Levinson-Durbin solves it in O(M²) instead of
//! O(M³). More importantly it produces the **reflection coefficients** as a
//! by-product, and their magnitudes being below 1 is exactly the condition for
//! the resulting synthesis filter `1/A(z)` to be stable. A general matrix solve
//! gives the same coefficients without that guarantee, which is why every
//! speech codec uses this recursion.

// This is a reference model: the arithmetic is written to mirror the spec's
// equations term by term rather than to be fused. `mul_add` would round
// differently from the separate multiply and add the specification describes,
// and the fixed-point implementation will be checked against this.
// Sample indices are small and exactly representable in f64, same as in
// `window`.
#![allow(clippy::suboptimal_flops, clippy::cast_precision_loss)]

use super::window::LP_ORDER;

/// Result of the recursion.
#[derive(Debug, Clone, PartialEq)]
pub struct LpAnalysis {
    /// Predictor coefficients `a[1..=LP_ORDER]`, with `a[0] = 1.0` implied and
    /// stored so indices match the spec's `a_k`.
    pub a: [f64; LP_ORDER + 1],
    /// Reflection coefficients `k[1..=LP_ORDER]`, index 0 unused.
    ///
    /// Also called PARCOR coefficients. `|k_i| < 1` for every `i` iff the
    /// synthesis filter is stable.
    pub reflection: [f64; LP_ORDER + 1],
    /// Residual energy after prediction, i.e. `E(LP_ORDER)`.
    ///
    /// Monotonically non-increasing through the recursion: each order can only
    /// explain more of the signal.
    pub residual_energy: f64,
}

impl LpAnalysis {
    /// Whether every reflection coefficient is inside the unit circle, which is
    /// equivalent to `1/A(z)` being stable.
    #[must_use]
    pub fn is_stable(&self) -> bool {
        self.reflection[1..=LP_ORDER].iter().all(|k| k.abs() < 1.0)
    }
}

/// Run the Levinson-Durbin recursion on lag-windowed autocorrelations.
///
/// `r` is `r[0..=LP_ORDER]` as produced by
/// [`super::window::autocorrelation`].
///
/// Returns `None` when `r[0]` is not positive — an all-zero frame has no
/// predictor, and the recursion would divide by zero. Callers handle that by
/// reusing the previous frame's filter, which is what the spec's error
/// concealment does anyway.
#[must_use]
pub fn levinson_durbin(r: &[f64; LP_ORDER + 1]) -> Option<LpAnalysis> {
    // Written as a positive test so NaN is rejected too: a NaN r(0) would
    // otherwise pass a `<= 0.0` check and poison every coefficient.
    if !matches!(r[0].partial_cmp(&0.0), Some(std::cmp::Ordering::Greater)) {
        return None;
    }

    let mut a = [0.0f64; LP_ORDER + 1];
    let mut reflection = [0.0f64; LP_ORDER + 1];
    a[0] = 1.0;
    let mut energy = r[0];

    for i in 1..=LP_ORDER {
        // Prediction error for this order: r(i) plus the previous predictor
        // applied to the intervening lags.
        let mut acc = r[i];
        for j in 1..i {
            acc += a[j] * r[i - j];
        }
        let k = -acc / energy;
        reflection[i] = k;

        // Update the predictor in place, working outwards from the middle so
        // the symmetric pair a[j] / a[i-j] both read pre-update values.
        let previous = a;
        for j in 1..i {
            a[j] = previous[j] + k * previous[i - j];
        }
        a[i] = k;

        energy *= 1.0 - k * k;
        if energy <= 0.0 {
            // A non-positive-definite autocorrelation sequence, which the lag
            // window and white-noise correction exist to prevent. Stop rather
            // than produce coefficients from a negative energy.
            return None;
        }
    }

    Some(LpAnalysis {
        a,
        reflection,
        residual_energy: energy,
    })
}

#[cfg(test)]
mod tests {
    use super::super::window::{autocorrelation, INTERNAL_RATE, WINDOW_LEN};
    use super::*;

    /// A deterministic speech-like frame: two formants plus an envelope.
    fn speechlike(seed: u64) -> [f64; WINDOW_LEN] {
        let mut s = [0.0f64; WINDOW_LEN];
        let f1 = 300.0 + f64::from(u32::try_from(seed % 7).unwrap_or(0)) * 40.0;
        let f2 = 1100.0 + f64::from(u32::try_from(seed % 5).unwrap_or(0)) * 90.0;
        for (n, slot) in s.iter_mut().enumerate() {
            let t = n as f64 / INTERNAL_RATE;
            let env = 0.5 + 0.5 * (2.0 * std::f64::consts::PI * 3.0 * t).sin();
            *slot = env
                * (0.6 * (2.0 * std::f64::consts::PI * f1 * t).sin()
                    + 0.3 * (2.0 * std::f64::consts::PI * f2 * t).sin())
                * 12000.0;
        }
        s
    }

    /// Solve the Yule-Walker system directly by Gaussian elimination.
    ///
    /// Deliberately a different algorithm from the one under test — an O(M³)
    /// general solve that knows nothing about the Toeplitz structure — so
    /// agreement is evidence rather than a restatement.
    // The row reduction reads the pivot row while writing another, so it needs
    // indexed access to two disjoint rows; iterating one of them is not enough.
    #[allow(clippy::needless_range_loop)]
    fn solve_yule_walker_directly(r: &[f64; LP_ORDER + 1]) -> [f64; LP_ORDER] {
        const N: usize = LP_ORDER;
        // Augmented matrix: the Toeplitz system with -r(i) as the right side.
        let mut aug = vec![vec![0.0f64; N + 1]; N];
        for (row, line) in aug.iter_mut().enumerate() {
            for (col, cell) in line.iter_mut().take(N).enumerate() {
                *cell = r[row.abs_diff(col)];
            }
            line[N] = -r[row + 1];
        }

        for col in 0..N {
            let pivot = (col..N)
                .max_by(|&x, &y| aug[x][col].abs().total_cmp(&aug[y][col].abs()))
                .expect("range is non-empty");
            aug.swap(col, pivot);
            let divisor = aug[col][col];
            for value in &mut aug[col][col..=N] {
                *value /= divisor;
            }
            for row in 0..N {
                if row != col && aug[row][col] != 0.0 {
                    let factor = aug[row][col];
                    for idx in col..=N {
                        aug[row][idx] -= factor * aug[col][idx];
                    }
                }
            }
        }

        let mut coefficients = [0.0f64; LP_ORDER];
        for (slot, line) in coefficients.iter_mut().zip(aug.iter()) {
            *slot = line[N];
        }
        coefficients
    }

    #[test]
    fn agrees_with_a_direct_solve_of_the_same_system() {
        for seed in 0..6u64 {
            let r = autocorrelation(&speechlike(seed));
            let lp = levinson_durbin(&r).expect("speech frame should be solvable");
            let direct = solve_yule_walker_directly(&r);
            for k in 1..=LP_ORDER {
                let tol = direct[k - 1].abs() * 1e-6 + 1e-9;
                assert!(
                    (lp.a[k] - direct[k - 1]).abs() <= tol,
                    "seed {seed} a[{k}]: recursion {} vs direct solve {}",
                    lp.a[k],
                    direct[k - 1]
                );
            }
        }
    }

    #[test]
    fn satisfies_the_normal_equations_it_claims_to_solve() {
        // The definition, checked directly: sum_k a_k r(|i-k|) = -r(i).
        let r = autocorrelation(&speechlike(3));
        let lp = levinson_durbin(&r).unwrap();
        for i in 1..=LP_ORDER {
            let mut acc = 0.0;
            for k in 1..=LP_ORDER {
                acc += lp.a[k] * r[i.abs_diff(k)];
            }
            let tol = r[i].abs() * 1e-6 + r[0] * 1e-12;
            assert!(
                (acc + r[i]).abs() <= tol,
                "equation {i} unsatisfied: {acc} vs {}",
                -r[i]
            );
        }
    }

    #[test]
    fn produces_a_stable_synthesis_filter() {
        // The property that makes this recursion worth using: |k_i| < 1 for
        // every order, which is exactly the condition for 1/A(z) to be stable.
        for seed in 0..8u64 {
            let lp = levinson_durbin(&autocorrelation(&speechlike(seed))).unwrap();
            assert!(lp.is_stable(), "seed {seed}: unstable filter");
            for i in 1..=LP_ORDER {
                assert!(
                    lp.reflection[i].abs() < 1.0,
                    "seed {seed}: |k[{i}]| = {}",
                    lp.reflection[i].abs()
                );
            }
        }
    }

    #[test]
    fn residual_energy_never_increases_with_order() {
        // Each additional pole can only explain more of the signal, so the
        // energy sequence E(i) = E(i-1)(1 - k_i²) is non-increasing. A rise
        // would mean a sign error in the recursion.
        let r = autocorrelation(&speechlike(2));
        let lp = levinson_durbin(&r).unwrap();
        assert!(lp.residual_energy > 0.0);
        assert!(
            lp.residual_energy <= r[0],
            "residual {} exceeded r(0) {}",
            lp.residual_energy,
            r[0]
        );

        // Reconstruct the sequence from the reflection coefficients.
        let mut e = r[0];
        for i in 1..=LP_ORDER {
            let next = e * (1.0 - lp.reflection[i] * lp.reflection[i]);
            assert!(next <= e * (1.0 + 1e-12), "energy rose at order {i}");
            e = next;
        }
        assert!((e - lp.residual_energy).abs() <= lp.residual_energy * 1e-9);
    }

    #[test]
    fn a_strongly_predictable_signal_leaves_little_residual() {
        // A single sinusoid is nearly perfectly predictable by a low-order
        // filter, so the residual should collapse. This catches a recursion
        // that runs but does not actually fit anything.
        let mut s = [0.0f64; WINDOW_LEN];
        for (n, slot) in s.iter_mut().enumerate() {
            *slot = (2.0 * std::f64::consts::PI * 400.0 * n as f64 / INTERNAL_RATE).sin() * 10000.0;
        }
        let r = autocorrelation(&s);
        let lp = levinson_durbin(&r).unwrap();
        assert!(
            lp.residual_energy < r[0] * 1e-3,
            "prediction gain too low: {} vs r(0) {}",
            lp.residual_energy,
            r[0]
        );
    }

    /// Zero-mean pseudo-random noise. Centring matters: a DC offset is a
    /// constant, and a constant is perfectly predictable, so uncentred "noise"
    /// would look highly structured to the predictor.
    fn white_noise() -> [f64; WINDOW_LEN] {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut s = [0.0f64; WINDOW_LEN];
        for slot in &mut s {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *slot = f64::from((state >> 48) as u16) - 32768.0;
        }
        s
    }

    #[test]
    fn white_noise_resists_prediction_far_more_than_speech() {
        // Prediction gain is the point of LP analysis, so it should be large
        // for structured input and small for unstructured. Comparing the two
        // is more robust than asserting an absolute threshold, which depends
        // on the analysis window's own spectral shape.
        let noise = autocorrelation(&white_noise());
        let noise_lp = levinson_durbin(&noise).unwrap();
        let noise_gain = noise[0] / noise_lp.residual_energy;

        let tone = {
            let mut s = [0.0f64; WINDOW_LEN];
            for (n, slot) in s.iter_mut().enumerate() {
                *slot = (2.0 * std::f64::consts::PI * 400.0 * n as f64 / INTERNAL_RATE).sin()
                    * 10000.0;
            }
            s
        };
        let tone_r = autocorrelation(&tone);
        let tone_lp = levinson_durbin(&tone_r).unwrap();
        let tone_gain = tone_r[0] / tone_lp.residual_energy;

        assert!(
            tone_gain > noise_gain * 100.0,
            "a tone should be far more predictable than noise: {tone_gain:.1} vs {noise_gain:.1}"
        );
        assert!(noise_lp.is_stable());
    }

    #[test]
    fn silence_has_no_predictor() {
        // r(0) = 0 would divide by zero. Returning None is the honest answer;
        // the caller reuses the previous frame's filter.
        let r = autocorrelation(&[0.0f64; WINDOW_LEN]);
        assert!(levinson_durbin(&r).is_none());
    }

    #[test]
    fn a_zeroth_order_predictor_is_the_identity() {
        // a[0] is 1 by definition and must not be overwritten by the recursion.
        let lp = levinson_durbin(&autocorrelation(&speechlike(1))).unwrap();
        assert!((lp.a[0] - 1.0).abs() < f64::EPSILON);
    }
}
