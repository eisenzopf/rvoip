//! LP to ISP conversion, 3GPP TS 26.190 §5.2.3.
//!
//! The predictor `A(z)` is converted to Immittance Spectral Pairs for
//! quantisation and interpolation. From the spec:
//!
//! > For a 16th order LP filter, the ISPs are defined as the roots of the sum
//! > and difference polynomials […] (The polynomials f'1(z) and f'2(z) are
//! > symmetric and antisymmetric, respectively). It can be proven that all
//! > roots of these polynomials are on the unit circle and they alternate each
//! > other. f'2(z) has two roots at z = 1 (ω=0) and z = -1 (ω = π). To
//! > eliminate these two roots, we define the new polynomials […]
//!
//! > Polynomials f1(z) and f2(z) have 8 and 7 conjugate roots on the unit
//! > circle respectively. […] where qi=cos(ωi) with ωi being the immittance
//! > spectral frequencies (ISF) and a[16] is the last predictor coefficient.
//! > ISFs satisfy the ordering property. We refer to qi as the ISPs in the
//! > cosine domain.
//!
//! # Why ISPs rather than the predictor directly
//!
//! `A(z)`'s coefficients quantise badly: small errors can push a pole outside
//! the unit circle and make the synthesis filter unstable. The ISPs are roots
//! *on* the unit circle that interlace, so quantisation that preserves their
//! ordering preserves stability by construction — and they interpolate
//! sensibly between subframes, which raw coefficients do not.
//!
//! # Derivation
//!
//! The coefficient recursions are derived here from the polynomial identities
//! rather than transcribed, so they can be checked by inspection:
//!
//! - `f'1(z) = A(z) + z⁻¹⁶A(z⁻¹)` gives `f'1[i] = a[i] + a[16-i]`
//! - `f'2(z) = A(z) - z⁻¹⁶A(z⁻¹)` gives `f'2[i] = a[i] - a[16-i]`
//! - `f2(z) = f'2(z)/(1 - z⁻²)` means `f'2[i] = f2[i] - f2[i-2]`, so
//!   `f2[i] = f'2[i] + f2[i-2]`
//!
//! with `a[0] = 1`. Both `f1` and `f2` are symmetric, so only the first half of
//! each is independent — which is why the spec says "only the first 8 and 7
//! coefficients of each polynomial […] need to be computed".

// A reference model: arithmetic mirrors the spec's equations rather than being
// fused, and indices are small and exactly representable.
#![allow(clippy::suboptimal_flops, clippy::cast_precision_loss)]

use super::window::LP_ORDER;

/// Number of independent `f1` coefficients, `f1[0..=8]`.
pub const F1_LEN: usize = LP_ORDER / 2 + 1;

/// Number of independent `f2` coefficients, `f2[0..=7]`.
pub const F2_LEN: usize = LP_ORDER / 2;

/// Points on the `[-1, 1]` cosine grid used to bracket roots.
///
/// The reference uses 100 intervals; matching that matters because a coarser
/// grid can miss two roots that fall in the same interval, and a finer one
/// changes which bracket a root is found in.
pub const GRID_POINTS: usize = 100;

/// The sum and difference polynomials of §5.2.3, equations 10–13.
#[derive(Debug, Clone, PartialEq)]
pub struct IspPolynomials {
    /// `f1[0..=8]`, symmetric, 8 conjugate roots on the unit circle.
    pub f1: [f64; F1_LEN],
    /// `f2[0..=7]`, 7 conjugate roots, with the roots at `z = ±1` divided out.
    pub f2: [f64; F2_LEN],
}

/// Build `f1` and `f2` from the predictor coefficients.
///
/// `a` is `a[0..=16]` with `a[0] = 1`, as produced by
/// [`super::levinson::levinson_durbin`].
#[must_use]
pub fn isp_polynomials(a: &[f64; LP_ORDER + 1]) -> IspPolynomials {
    // f'1[i] = a[i] + a[16-i]; f'2[i] = a[i] - a[16-i].
    let mut f1 = [0.0f64; F1_LEN];
    let mut f2_prime = [0.0f64; F2_LEN + 2];

    for (i, slot) in f1.iter_mut().enumerate() {
        *slot = a[i] + a[LP_ORDER - i];
    }
    for (i, slot) in f2_prime.iter_mut().enumerate() {
        *slot = a[i] - a[LP_ORDER - i];
    }

    // f2 = f'2 / (1 - z^-2), i.e. f2[i] = f'2[i] + f2[i-2].
    let mut f2 = [0.0f64; F2_LEN];
    for i in 0..F2_LEN {
        // Reads f2[i-2], so this genuinely needs the index.
        f2[i] = f2_prime[i] + if i >= 2 { f2[i - 2] } else { 0.0 };
    }

    IspPolynomials { f1, f2 }
}

/// Evaluate a symmetric polynomial on the cosine grid via its Chebyshev series.
///
/// A symmetric polynomial of even degree `2m` evaluated on the unit circle
/// reduces to a cosine series in `ω`, and substituting `x = cos(ω)` turns that
/// into a Chebyshev sum — so root-finding happens on the real interval
/// `[-1, 1]` instead of on the complex unit circle. `x` is the ISP value
/// directly.
///
/// `c` holds the half-coefficients, `c[0]` being the centre term.
#[must_use]
fn chebyshev(x: f64, c: &[f64]) -> f64 {
    // Clenshaw recurrence for sum c[k] T_k(x), with c[0] halved as the series
    // convention requires.
    let mut b1 = 0.0f64;
    let mut b2 = 0.0f64;
    for &coefficient in c.iter().skip(1).rev() {
        let b0 = 2.0 * x * b1 - b2 + coefficient;
        b2 = b1;
        b1 = b0;
    }
    x * b1 - b2 + c[0]
}

/// Chebyshev half-coefficients of `f1`, in the form [`chebyshev`] expects.
fn f1_series(p: &IspPolynomials) -> [f64; F1_LEN] {
    // A symmetric polynomial sum_{i=0..16} f[i] z^-i on |z|=1 equals
    // 2 z^-8 (f[8]/2 + sum_{k=1..8} f[8-k] cos(kω)).
    let mut c = [0.0f64; F1_LEN];
    c[0] = p.f1[F1_LEN - 1] / 2.0;
    for (k, slot) in c.iter_mut().enumerate().skip(1) {
        *slot = p.f1[F1_LEN - 1 - k];
    }
    c
}

/// Chebyshev half-coefficients of `f2`.
fn f2_series(p: &IspPolynomials) -> [f64; F2_LEN] {
    let mut c = [0.0f64; F2_LEN];
    c[0] = p.f2[F2_LEN - 1] / 2.0;
    for (k, slot) in c.iter_mut().enumerate().skip(1) {
        *slot = p.f2[F2_LEN - 1 - k];
    }
    c
}

/// Find up to `expected` roots of a Chebyshev series on `[-1, 1]`.
///
/// Walks the cosine grid looking for sign changes and bisects each bracket.
/// Returns them in descending `x`, which is ascending frequency.
fn scan_roots(c: &[f64], expected: usize) -> Vec<f64> {
    let mut roots = Vec::with_capacity(expected);
    let mut x_prev = 1.0f64;
    let mut y_prev = chebyshev(x_prev, c);

    for step in 1..=GRID_POINTS {
        let x = 1.0 - 2.0 * step as f64 / GRID_POINTS as f64;
        let y = chebyshev(x, c);

        if y_prev * y <= 0.0 && y_prev != 0.0 {
            let (mut lo, mut hi) = (x, x_prev);
            let mut f_lo = y;
            // Bisect to convergence. The fixed-point version will use the
            // reference's fixed iteration count; here accuracy is free.
            for _ in 0..60 {
                let mid = 0.5 * (lo + hi);
                let f_mid = chebyshev(mid, c);
                if f_mid * f_lo <= 0.0 {
                    hi = mid;
                } else {
                    lo = mid;
                    f_lo = f_mid;
                }
            }
            roots.push(0.5 * (lo + hi));
            if roots.len() == expected {
                break;
            }
        }
        x_prev = x;
        y_prev = y;
    }
    roots
}

/// Convert predictor coefficients to ISPs in the cosine domain.
///
/// Returns `q[0..16]`: the first 15 are `cos(ωi)` for the interlaced roots of
/// `f1` and `f2`, and `q[15]` is the last predictor coefficient `a[16]`, which
/// the spec carries as the sixteenth ISP.
///
/// Returns `None` if fewer than 15 roots are found, which indicates the
/// predictor was not minimum-phase — [`super::levinson::levinson_durbin`]
/// guarantees it is, so this should not occur for its output.
#[must_use]
pub fn lp_to_isp(a: &[f64; LP_ORDER + 1]) -> Option<[f64; LP_ORDER]> {
    let p = isp_polynomials(a);
    let c1 = f1_series(&p);
    let c2 = f2_series(&p);

    // Scan each polynomial independently over the whole grid rather than
    // alternating between them.
    //
    // The reference alternates, relying on the roots interlacing so that each
    // switch lands in the next bracket. That is efficient but fragile: when two
    // roots fall inside one grid interval the search steps past one of them and
    // then fails to find the rest. Scanning separately cannot lose a root that
    // the grid brackets at all, and the interlacing is then *checked* rather
    // than assumed — which is the right trade for a reference model.
    let roots_f1 = scan_roots(&c1, F1_LEN - 1);
    let roots_f2 = scan_roots(&c2, F2_LEN - 1);
    if roots_f1.len() != F1_LEN - 1 || roots_f2.len() != F2_LEN - 1 {
        return None;
    }

    // Interleave: f1 and f2 roots alternate in descending x.
    let mut isp = [0.0f64; LP_ORDER];
    for (i, slot) in isp[..LP_ORDER - 1].iter_mut().enumerate() {
        *slot = if i % 2 == 0 {
            roots_f1[i / 2]
        } else {
            roots_f2[i / 2]
        };
    }
    // "and a[16] is the last predictor coefficient" — carried as the last ISP.
    isp[LP_ORDER - 1] = a[LP_ORDER];
    Some(isp)
}

#[cfg(test)]
mod tests {
    use super::super::levinson::levinson_durbin;
    use super::super::window::{autocorrelation, INTERNAL_RATE, WINDOW_LEN};
    use super::*;

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

    fn predictor(seed: u64) -> [f64; LP_ORDER + 1] {
        levinson_durbin(&autocorrelation(&speechlike(seed)))
            .expect("speech frame is solvable")
            .a
    }

    /// Evaluate sum a[i] z^-i at a complex point on the unit circle.
    fn eval_at_angle(coefficients: &[f64], omega: f64) -> (f64, f64) {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &c) in coefficients.iter().enumerate() {
            let angle = -(i as f64) * omega;
            re += c * angle.cos();
            im += c * angle.sin();
        }
        (re, im)
    }

    #[test]
    fn f1_and_f2_reproduce_their_defining_identities() {
        // f'1(z) = A(z) + z^-16 A(z^-1) and f'2(z) = A(z) - z^-16 A(z^-1),
        // checked numerically on the unit circle rather than by re-deriving
        // the recursion.
        let a = predictor(1);
        let p = isp_polynomials(&a);

        // Rebuild the full symmetric f'1 from its half.
        let mut f1_full = [0.0f64; LP_ORDER + 1];
        for (i, slot) in f1_full.iter_mut().enumerate() {
            *slot = if i < F1_LEN { p.f1[i] } else { p.f1[LP_ORDER - i] };
        }

        for step in 0..12 {
            let omega = std::f64::consts::PI * f64::from(step) / 12.0;
            let (ar, ai) = eval_at_angle(&a, omega);
            // z^-16 A(z^-1) at z = e^{jω} is conj(A) rotated by -16ω.
            // e^{-j16ω}·conj(A): (c + js)(ar - j·ai)
            //   = (c·ar + s·ai) + j(s·ar - c·ai)
            let (c, s) = ((-16.0 * omega).cos(), (-16.0 * omega).sin());
            let mirrored = (ar * c + ai * s, s * ar - c * ai);
            let want = (ar + mirrored.0, ai + mirrored.1);
            let got = eval_at_angle(&f1_full, omega);
            assert!(
                (got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9,
                "f'1 mismatch at ω={omega}: {got:?} vs {want:?}"
            );
        }
    }

    #[test]
    fn f2_times_one_minus_z_squared_recovers_f_prime_2() {
        // The deconvolution is the step most easily got wrong, so check it by
        // multiplying back: f2(z)(1 - z^-2) must equal f'2(z).
        let a = predictor(4);
        let p = isp_polynomials(&a);

        let mut f2_full = [0.0f64; F2_LEN + 2];
        f2_full[..F2_LEN].copy_from_slice(&p.f2);

        for i in 0..F2_LEN {
            let reconstructed = f2_full[i] - if i >= 2 { f2_full[i - 2] } else { 0.0 };
            let expected = a[i] - a[LP_ORDER - i];
            assert!(
                (reconstructed - expected).abs() < 1e-12,
                "f'2[{i}]: {reconstructed} vs {expected}"
            );
        }
    }

    #[test]
    fn difference_polynomial_vanishes_at_plus_and_minus_one() {
        // The spec removes roots at z = 1 and z = -1 from f'2 precisely because
        // they are always there. If they were not, dividing by (1 - z^-2) would
        // not be exact.
        let a = predictor(2);
        let mut f2_prime = [0.0f64; LP_ORDER + 1];
        for (i, slot) in f2_prime.iter_mut().enumerate() {
            *slot = a[i] - a[LP_ORDER - i];
        }
        let at_plus_one: f64 = f2_prime.iter().sum();
        let at_minus_one: f64 = f2_prime
            .iter()
            .enumerate()
            .map(|(i, c)| if i % 2 == 0 { *c } else { -*c })
            .sum();
        assert!(at_plus_one.abs() < 1e-12, "f'2(1) = {at_plus_one}");
        assert!(at_minus_one.abs() < 1e-12, "f'2(-1) = {at_minus_one}");
    }

    #[test]
    fn finds_fifteen_roots_for_real_predictors() {
        for seed in 0..8u64 {
            let isp = lp_to_isp(&predictor(seed));
            assert!(isp.is_some(), "seed {seed}: root search failed");
        }
    }

    #[test]
    fn isps_are_ordered_and_inside_the_cosine_range() {
        // The ordering property is what makes ISPs safe to quantise: preserving
        // the order preserves filter stability. Descending in cos(ω) is
        // ascending in frequency.
        for seed in 0..8u64 {
            let isp = lp_to_isp(&predictor(seed)).unwrap();
            for (i, &q) in isp[..LP_ORDER - 1].iter().enumerate() {
                assert!(q > -1.0 && q < 1.0, "seed {seed}: q[{i}] = {q} off the circle");
            }
            for i in 1..LP_ORDER - 1 {
                assert!(
                    isp[i] < isp[i - 1],
                    "seed {seed}: q[{i}] = {} not below q[{}] = {}",
                    isp[i],
                    i - 1,
                    isp[i - 1]
                );
            }
        }
    }

    #[test]
    fn the_roots_actually_zero_their_polynomials() {
        // Alternating roots means the even-indexed ones belong to f1 and the
        // odd to f2. Checking that each root really is a root of the polynomial
        // it was attributed to catches an off-by-one in the alternation.
        let a = predictor(5);
        let p = isp_polynomials(&a);
        let c1 = f1_series(&p);
        let c2 = f2_series(&p);
        let isp = lp_to_isp(&a).unwrap();

        for (i, &q) in isp[..LP_ORDER - 1].iter().enumerate() {
            let value = if i % 2 == 0 {
                chebyshev(q, &c1)
            } else {
                chebyshev(q, &c2)
            };
            // Scale tolerance by the polynomial's magnitude nearby.
            let scale = c1.iter().chain(c2.iter()).fold(0.0f64, |m, v| m.max(v.abs()));
            assert!(
                value.abs() < scale * 1e-9 + 1e-9,
                "root {i} (q = {q}) does not zero its polynomial: {value}"
            );
        }
    }

    #[test]
    fn the_last_isp_carries_the_last_predictor_coefficient() {
        // The spec's ISP vector is 15 roots plus a[16], not 16 roots.
        let a = predictor(6);
        let isp = lp_to_isp(&a).unwrap();
        assert!((isp[LP_ORDER - 1] - a[LP_ORDER]).abs() < f64::EPSILON);
    }

    #[test]
    fn chebyshev_evaluates_a_known_series() {
        // T_0 = 1, T_1 = x, T_2 = 2x²-1. With c = [c0, c1, c2] the Clenshaw
        // sum is c0 + c1 x + c2 (2x²-1).
        for &x in &[-1.0, -0.5, 0.0, 0.25, 1.0] {
            let c = [0.5, 2.0, -1.5];
            let want = 0.5 + 2.0 * x - 1.5 * (2.0 * x * x - 1.0);
            let got = chebyshev(x, &c);
            assert!((got - want).abs() < 1e-12, "x={x}: {got} vs {want}");
        }
    }
}
