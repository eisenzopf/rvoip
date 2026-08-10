//! Encoder LP analysis: Levinson-Durbin, A(z) → ISP, and the weighted speech.
//!
//! Implements the analysis half of TS 26.190 §5.2 in the arithmetic of
//! TS 26.173: `Levinson` (`levinson.c`), `Az_isp` + `Chebps2` (`az_isp.c`),
//! `Weight_a` (`weight_a.c`), `Residu` (`residu.c`), `Deemph2` (`deemph.c`),
//! `LP_Decim2` (`lp_dec2.c`), and the frame-level ordering of `coder()`
//! (`cod_main.c` 468–541) that ties them to
//! [`super::preproc`].
//!
//! The pieces the decoder already carries are reused rather than rewritten:
//! [`autocorrelation`] (which **already applies the lag window** — calling
//! `lag_window` again squares it, producing a stable filter and a wrong
//! bitstream), [`isp_to_isf`], [`interpolate_isp`] and
//! [`scale_sig`].
//!
//! Validated against `testdata/wb_enc_trace.txt` — the `window`, `r_h`/`r_l`,
//! `A`, `rc`, `ispnew`, `A_interp`, `isf_unq46`, `wsp` and `wsp_shift` rows —
//! replayed across all three committed frames from
//! `testdata/amrwb_enc_input.pcm`, so the carried memories, the `Q_max`
//! history and the previous frame's ISPs are all exercised. Checked once,
//! off-tree, against the full 50-frame trace `tools/trace-amrwb-encoder.sh`
//! produces: every row of every frame matched.
//!
//! # Q-formats
//!
//! Autocorrelations arrive as normalised double-precision pairs. Predictor
//! coefficients `a[]` and the weighted `ap[]` are Q12 with `a[0] = 1.0 =
//! 4096`; reflection coefficients are Q15; ISPs are Q15 cosines; ISFs are Q15
//! normalised frequencies over `0..=0.5`. `wsp` inherits the frame's `Q_new`
//! plus its own `wsp_shift`.

// A transcription of reference fixed-point arithmetic: the magic constants are
// the specification, the index arithmetic is deliberately unchecked, and the
// root search has the shape the reference gives it rather than the shape a
// numerical analyst would choose.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::unreadable_literal
)]

use super::super::lp::autocorr::{autocorrelation, Autocorrelation, LP_ORDER, WINDOW_LEN};
use super::super::lp::isf::{interpolate_isp, isp_to_isf, NB_SUBFR};
use super::super::math::scale_sig;
use super::preproc::{Preprocessor, Scaling, L_FRAME, L_FRAME16K, L_TOTAL, NEW_SPEECH};
use crate::fixed_point::arith::{abs_s, add, extract_h, extract_l, negate, round, sub};
use crate::fixed_point::arith32::{l_abs, l_add, l_mac, l_msu, l_mult, l_negate, l_sub};
use crate::fixed_point::div::div_s;
use crate::fixed_point::oper32::{div_32, l_comp, l_extract, mpy_32, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, norm_s, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Subframe length at 12.8 kHz.
pub const L_SUBFR: usize = 64;

/// Lookahead the LP analysis window reaches past the present frame.
pub const L_NEXT: usize = 64;

/// Offset of the "present frame" within the total speech buffer.
///
/// `L_TOTAL - L_FRAME - L_NEXT`. The subframe loop works on
/// `window[SPEECH .. SPEECH + L_FRAME]`, which is 52 samples *behind* the
/// region the LP analysis looks at — the analysis deliberately runs ahead.
pub const SPEECH: usize = L_TOTAL - L_FRAME - L_NEXT;

/// The whole speech buffer is the analysis window; the two lengths must agree
/// or `autocorrelation` would be fed the wrong span.
const _: () = assert!(L_TOTAL == WINDOW_LEN);

/// Decimation factor of the open-loop pitch analysis.
pub const OPL_DECIM: usize = 2;

/// Maximum pitch lag.
pub const PIT_MAX: usize = 231;

/// Length of the decimated weighted-speech history the pitch search needs.
pub const WSP_HISTORY: usize = PIT_MAX / OPL_DECIM;

/// Weighting factor for the perceptual filter `A(z/γ₁)`, 0.92 in Q15.
pub const GAMMA1: Word16 = Word16(30147);

/// Spectral tilt compensation for the weighted speech, 0.68 in Q15.
pub const TILT_FAC: Word16 = Word16(22282);

/// Half the LP order — the number of roots each polynomial contributes.
const NC: usize = LP_ORDER / 2;

/// Number of intervals in the Chebyshev evaluation grid.
const GRID_POINTS: usize = 100;

/// Cold-start ISPs (`isp_init`), Q15.
///
/// A flat spectrum: `cos(iπ/16)` for the fifteen roots, plus a small last
/// predictor coefficient.
pub const ISP_INIT: [Word16; LP_ORDER] = [
    Word16(32138),
    Word16(30274),
    Word16(27246),
    Word16(23170),
    Word16(18205),
    Word16(12540),
    Word16(6393),
    Word16(0),
    Word16(-6393),
    Word16(-12540),
    Word16(-18205),
    Word16(-23170),
    Word16(-27246),
    Word16(-30274),
    Word16(-32138),
    Word16(1475),
];

/// Evaluation grid for the root search (`grid100.tab`), Q15.
///
/// `grid[0] = 1.0`, `grid[100] = -1.0` (as `-32760`), and the interior points
/// are `cos(πi/100)`. **Not** interchangeable with G.729's 51-point grid: a
/// coarser scan merges close roots and the search fails.
const GRID: [Word16; GRID_POINTS + 1] = {
    const fn w(v: i16) -> Word16 {
        Word16(v)
    }
    [
        w(32767), w(32751), w(32703), w(32622), w(32509), w(32364),
        w(32187), w(31978), w(31738), w(31466), w(31164), w(30830),
        w(30466), w(30072), w(29649), w(29196), w(28714), w(28204),
        w(27666), w(27101), w(26509), w(25891), w(25248), w(24579),
        w(23886), w(23170), w(22431), w(21669), w(20887), w(20083),
        w(19260), w(18418), w(17557), w(16680), w(15786), w(14876),
        w(13951), w(13013), w(12062), w(11099), w(10125), w(9141),
        w(8149), w(7148), w(6140), w(5126), w(4106), w(3083),
        w(2057), w(1029), w(0), w(-1029), w(-2057), w(-3083),
        w(-4106), w(-5126), w(-6140), w(-7148), w(-8149), w(-9141),
        w(-10125), w(-11099), w(-12062), w(-13013), w(-13951), w(-14876),
        w(-15786), w(-16680), w(-17557), w(-18418), w(-19260), w(-20083),
        w(-20887), w(-21669), w(-22431), w(-23170), w(-23886), w(-24579),
        w(-25248), w(-25891), w(-26509), w(-27101), w(-27666), w(-28204),
        w(-28714), w(-29196), w(-29649), w(-30072), w(-30466), w(-30830),
        w(-31164), w(-31466), w(-31738), w(-31978), w(-32187), w(-32364),
        w(-32509), w(-32622), w(-32703), w(-32751), w(-32760),
    ]
};

/// Decimation-by-two FIR for the weighted speech, Q15.
///
/// Sums to 32767 rather than 32768 so a DC input cannot overflow.
const H_FIR: [Word16; 5] = [
    Word16(4260),
    Word16(7536),
    Word16(9175),
    Word16(7536),
    Word16(4260),
];

// ---------------------------------------------------------------------------
// Levinson-Durbin
// ---------------------------------------------------------------------------

/// The Levinson-Durbin recursion's carried state (`mem_levinson`).
///
/// Only ever read on the unstable-filter path, where the previous frame's
/// filter is emitted again rather than an unstable new one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevinsonMemory {
    /// `old_A`: the previous frame's `a[1..=16]`, Q12.
    previous_a: [Word16; LP_ORDER],
    /// `old_rc`: the previous frame's first two reflection coefficients, Q15.
    previous_rc: [Word16; 2],
}

impl LevinsonMemory {
    /// Cold-start state: all zero, as `Init_Levinson`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            previous_a: [Word16(0); LP_ORDER],
            previous_rc: [Word16(0); 2],
        }
    }
}

/// Solve the normal equations for the order-16 predictor.
///
/// Returns `(a, rc)`: seventeen Q12 coefficients with `a[0] = 4096`, and
/// sixteen Q15 reflection coefficients. `rc` is written but never read by
/// `coder()` — it is produced here because the reference trace records it, and
/// because it is the cheapest independent check on the recursion.
///
/// Everything but the output runs in double precision: reflection
/// coefficients and the prediction gain in Q31, the working coefficients in
/// Q27, each as a `(high, low)` pair.
///
/// If the recursion produces a reflection coefficient at magnitude 1 — an
/// unstable filter — the previous frame's coefficients are returned unchanged
/// and `mem` is **not** updated, so a run of unstable frames all emit the same
/// last-good filter.
pub fn levinson(
    r: &Autocorrelation,
    mem: &mut LevinsonMemory,
) -> ([Word16; LP_ORDER + 1], [Word16; LP_ORDER]) {
    let mut ctx = DspContext::default();
    let mut a = [Word16(0); LP_ORDER + 1];
    let mut rc = [Word16(0); LP_ORDER];

    // Working coefficients in Q27 DPF.
    let mut a_hi = [Word16(0); LP_ORDER + 1];
    let mut a_lo = [Word16(0); LP_ORDER + 1];

    // --- order 1: K = -r[1] / r[0] ---
    let r1 = l_comp(r.high[1], r.low[1]);
    let magnitude = l_abs(&mut ctx, r1);
    let mut k = div_32(magnitude, r.high[0], r.low[0]);
    // Negate when r[1] is *positive*: the recursion wants -r[1]/r[0], and the
    // division was done on the magnitude.
    if r1.0 > 0 {
        k = l_negate(&mut ctx, k);
    }
    let (mut k_hi, mut k_lo) = l_extract(k);
    rc[0] = k_hi;
    let k_q27 = l_shr(&mut ctx, k, 4);
    let (hi, lo) = l_extract(k_q27);
    a_hi[1] = hi;
    a_lo[1] = lo;

    // Alpha = r[0] · (1 - K²), normalised.
    let (mut alpha_hi, mut alpha_lo, mut alpha_exp) =
        prediction_gain(&mut ctx, r.high[0], r.low[0], k_hi, k_lo);

    // --- orders 2..=16 ---
    for i in 2..=LP_ORDER {
        let mut acc = Word32(0);
        for j in 1..i {
            let term = mpy_32(r.high[j], r.low[j], a_hi[i - j], a_lo[i - j]);
            acc = l_add(&mut ctx, acc, term);
        }
        // The sum is Q27 against Q31 autocorrelations; lift it to Q31 before
        // adding r[i]. No overflow is possible — both factors are below 1.
        acc = l_shl(&mut ctx, acc, 4);
        let ri = l_comp(r.high[i], r.low[i]);
        let error = l_add(&mut ctx, acc, ri);

        let magnitude = l_abs(&mut ctx, error);
        let mut k = div_32(magnitude, alpha_hi, alpha_lo);
        if error.0 > 0 {
            k = l_negate(&mut ctx, k);
        }
        // Undo Alpha's normalisation so K is back on Alpha's own scale.
        let mut k = l_shl(&mut ctx, k, alpha_exp);
        let (hi, lo) = l_extract(k);
        k_hi = hi;
        k_lo = lo;
        rc[i - 1] = k_hi;

        // Unstable filter: keep the last good A(z). The test is on the high
        // word alone and is strict, and neither `previous_a` nor
        // `previous_rc` is updated on the way out.
        if abs_s(&mut ctx, k_hi).0 > 32750 {
            a[0] = Word16(4096);
            a[1..=LP_ORDER].copy_from_slice(&mem.previous_a);
            rc[0] = mem.previous_rc[0];
            rc[1] = mem.previous_rc[1];
            return (a, rc);
        }

        // an[j] = a[j] + K·a[i-j]. The scratch is mandatory: the update reads
        // a[i-j] while writing a[j], so an in-place version would fold in
        // values it has already overwritten.
        let mut an_hi = [Word16(0); LP_ORDER + 1];
        let mut an_lo = [Word16(0); LP_ORDER + 1];
        for j in 1..i {
            let mut term = mpy_32(k_hi, k_lo, a_hi[i - j], a_lo[i - j]);
            term = l_add(&mut ctx, term, l_comp(a_hi[j], a_lo[j]));
            let (hi, lo) = l_extract(term);
            an_hi[j] = hi;
            an_lo[j] = lo;
        }
        k = l_shr(&mut ctx, k, 4);
        let (hi, lo) = l_extract(k);
        an_hi[i] = hi;
        an_lo[i] = lo;

        let (hi, lo, exp) = prediction_gain(&mut ctx, alpha_hi, alpha_lo, k_hi, k_lo);
        alpha_hi = hi;
        alpha_lo = lo;
        alpha_exp = add(&mut ctx, Word16(alpha_exp), Word16(exp)).0;

        a_hi[1..=i].copy_from_slice(&an_hi[1..=i]);
        a_lo[1..=i].copy_from_slice(&an_lo[1..=i]);
    }

    a[0] = Word16(4096);
    for i in 1..=LP_ORDER {
        let value = l_comp(a_hi[i], a_lo[i]);
        let lifted = l_shl(&mut ctx, value, 1);
        a[i] = round(&mut ctx, lifted);
        mem.previous_a[i - 1] = a[i];
    }
    mem.previous_rc[0] = rc[0];
    mem.previous_rc[1] = rc[1];

    (a, rc)
}

/// `gain · (1 - K²)`, renormalised, as `(high, low, exponent)`.
///
/// The `L_abs` is not defensive noise. `Mpy_32` drops the low·low product, so
/// for `K` near ±1 it can return a small *negative* K², and subtracting that
/// from 0x7FFFFFFF would push the result past Q31.
fn prediction_gain(
    ctx: &mut DspContext,
    gain_hi: Word16,
    gain_lo: Word16,
    k_hi: Word16,
    k_lo: Word16,
) -> (Word16, Word16, i16) {
    let squared = mpy_32(k_hi, k_lo, k_hi, k_lo);
    let squared = l_abs(ctx, squared);
    let complement = l_sub(ctx, Word32(0x7fff_ffff), squared);
    let (hi, lo) = l_extract(complement);
    let gain = mpy_32(gain_hi, gain_lo, hi, lo);
    let exp = norm_l(gain);
    let gain = l_shl(ctx, gain, exp);
    let (hi, lo) = l_extract(gain);
    (hi, lo, exp)
}

// ---------------------------------------------------------------------------
// A(z) -> ISP
// ---------------------------------------------------------------------------

/// What the ISP root search found, beyond the ISPs themselves.
///
/// The extra fields exist because a root search that lands on a
/// different-but-plausible root set still produces speech-shaped output: they
/// let a test assert *which* candidate was chosen, not only that the answer
/// looks reasonable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IspSearch {
    /// The sixteen ISPs, Q15 — or `old_isp` unchanged if the search failed.
    pub isp: [Word16; LP_ORDER],
    /// Grid index `j` whose interval `[grid[j], grid[j-1]]` bracketed each
    /// accepted root. Only the first [`Self::roots_found`] entries are set.
    pub accepted_at: [usize; LP_ORDER - 1],
    /// How many roots the scan located before it stopped.
    pub roots_found: usize,
    /// Whether the scan gave up and fell back to `old_isp`.
    pub fell_back: bool,
}

/// Convert predictor coefficients to immittance spectral pairs.
///
/// `a` is Q12; the result is fifteen Q15 root cosines followed by `a[16]`
/// rescaled to Q15. `old_isp` is the previous frame's answer, returned
/// unchanged — **all sixteen entries, including the last** — if fewer than
/// fifteen roots are found.
#[must_use]
pub fn az_isp(a: &[Word16; LP_ORDER + 1], old_isp: &[Word16; LP_ORDER]) -> [Word16; LP_ORDER] {
    az_isp_detail(a, old_isp).isp
}

/// [`az_isp`], keeping the search's own record of what it did.
#[must_use]
pub fn az_isp_detail(
    a: &[Word16; LP_ORDER + 1],
    old_isp: &[Word16; LP_ORDER],
) -> IspSearch {
    let mut ctx = DspContext::default();
    let (f1, f2) = sum_difference_polynomials(&mut ctx, a);

    let mut isp = [Word16(0); LP_ORDER];
    let mut accepted_at = [0usize; LP_ORDER - 1];
    let mut found = 0usize;

    // The two polynomials' roots interlace, so they are searched alternately —
    // starting on F1 — and each accepted root becomes the upper end of the
    // next interval. F1 has NC roots on the unit circle, F2 has NC-1 after its
    // roots at z = ±1 have been divided out, which is why the orders alternate
    // 8, 7, 8, 7.
    let mut on_f1 = true;
    let mut xlow = GRID[0];
    let mut ylow = evaluate(xlow, &f1, &f2, on_f1);

    let mut j = 0usize;
    while found < LP_ORDER - 1 && j < GRID_POINTS {
        j += 1;
        let mut xhigh = xlow;
        let mut yhigh = ylow;
        xlow = GRID[j];
        ylow = evaluate(xlow, &f1, &f2, on_f1);

        // A 32-bit product, and **zero counts as a sign change**: an exact
        // zero at a grid point is a root, and a predicate that skipped it
        // would find a different root set.
        if l_mult(&mut ctx, ylow, yhigh).0 > 0 {
            continue;
        }

        // Exactly two bisections. This is specified depth, not an
        // approximation to improve on — the linear interpolation that follows
        // is calibrated to the interval width two halvings leave.
        for _ in 0..2 {
            // Two independent floors and then an add, not one shift of the
            // sum: for odd endpoints the two differ by one, which moves the
            // root and every root after it.
            let half_low = shr(&mut ctx, xlow, 1);
            let half_high = shr(&mut ctx, xhigh, 1);
            let xmid = add(&mut ctx, half_low, half_high);
            let ymid = evaluate(xmid, &f1, &f2, on_f1);
            if l_mult(&mut ctx, ylow, ymid).0 <= 0 {
                yhigh = ymid;
                xhigh = xmid;
            } else {
                ylow = ymid;
                xlow = xmid;
            }
        }

        let root = interpolate_root(&mut ctx, xlow, xhigh, ylow, yhigh);
        isp[found] = root;
        accepted_at[found] = j;
        found += 1;

        // Only `xlow` carries the root forward. `xhigh`/`yhigh` are left where
        // the bisection put them and are overwritten at the top of the next
        // iteration by `xhigh = xlow; yhigh = ylow` — so the next interval
        // runs from the accepted root down to `grid[j+1]`, not from `grid[j]`.
        xlow = root;
        on_f1 = !on_f1;
        // Mandatory, not an optimisation: the bisection wrote `ylow` for the
        // *previous* polynomial, and nothing else discards it.
        ylow = evaluate(xlow, &f1, &f2, on_f1);
    }

    if found < LP_ORDER - 1 {
        return IspSearch {
            isp: *old_isp,
            accepted_at,
            roots_found: found,
            fell_back: true,
        };
    }

    // The last "ISP" is not a root at all: it is a[16] carried alongside them,
    // Q12 to Q15 with saturation.
    isp[LP_ORDER - 1] = shl(&mut ctx, a[LP_ORDER], 3);
    IspSearch {
        isp,
        accepted_at,
        roots_found: found,
        fell_back: false,
    }
}

/// Build `F1(z) = [A(z) + z⁻¹⁶A(z⁻¹)]` and `F2(z) = [A(z) - z⁻¹⁶A(z⁻¹)]/(1-z⁻²)`.
///
/// Both are stored at half scale, in Q11 — which is why `f1[NC]` is `a[NC]`
/// and not the `2·a[NC]` the reference's own header pseudocode shows. The two
/// agree; the code is right and the comment is loose.
fn sum_difference_polynomials(
    ctx: &mut DspContext,
    a: &[Word16; LP_ORDER + 1],
) -> ([Word16; NC + 1], [Word16; NC]) {
    let mut f1 = [Word16(0); NC + 1];
    let mut f2 = [Word16(0); NC];

    for i in 0..NC {
        let base = l_mult(ctx, a[i], Word16(16384));
        let sum = l_mac(ctx, base, a[LP_ORDER - i], Word16(16384));
        f1[i] = round(ctx, sum);
        let difference = l_msu(ctx, base, a[LP_ORDER - i], Word16(16384));
        f2[i] = round(ctx, difference);
    }
    f1[NC] = a[NC];

    // Divide F2 by (1 - z⁻²). A genuinely self-referencing recurrence: the
    // updated f2[i] feeds f2[i+2], so ascending order is required.
    for i in 2..NC {
        f2[i] = add(ctx, f2[i], f2[i - 2]);
    }

    (f1, f2)
}

/// Evaluate whichever polynomial the search is currently on.
///
/// A free function rather than a closure so the two borrows stay local to the
/// call.
fn evaluate(x: Word16, f1: &[Word16; NC + 1], f2: &[Word16; NC], on_f1: bool) -> Word16 {
    if on_f1 {
        chebps2(x, f1, NC)
    } else {
        chebps2(x, f2, NC - 1)
    }
}

/// Locate a root inside its bracketing interval by linear interpolation.
///
/// `xint = xlow - ylow·(xhigh-xlow)/(yhigh-ylow)`, computed through a
/// reciprocal in Q11.
fn interpolate_root(
    ctx: &mut DspContext,
    xlow: Word16,
    xhigh: Word16,
    ylow: Word16,
    yhigh: Word16,
) -> Word16 {
    let dx = sub(ctx, xhigh, xlow);
    let dy = sub(ctx, yhigh, ylow);
    if dy.0 == 0 {
        return xlow;
    }

    let sign = dy;
    let magnitude = abs_s(ctx, dy);
    let exp = norm_s(magnitude);
    let normalised = shl(ctx, magnitude, exp);
    // 16383, not 16384: the numerator is one below half scale so `div_s`
    // cannot be handed a quotient of exactly 1.0.
    let reciprocal = div_s(Word16(16383), normalised);

    let scaled = l_mult(ctx, dx, reciprocal);
    let shift = sub(ctx, Word16(20), Word16(exp));
    let slope = l_shr(ctx, scaled, shift.0);
    let mut slope = extract_l(slope);
    if sign.0 < 0 {
        slope = negate(ctx, slope);
    }

    let step = l_mult(ctx, ylow, slope);
    let step = l_shr(ctx, step, 11);
    sub(ctx, xlow, extract_l(step))
}

/// Evaluate the Chebyshev series `C(x) = f(0)T_n(x) + … + f(n)/2`.
///
/// `x` is a Q15 cosine, `f` is Q11 and read up to index `n` inclusive; the
/// result is Q14. All intermediates are Q24 in double precision.
fn chebps2(x: Word16, f: &[Word16], n: usize) -> Word16 {
    let mut ctx = DspContext::default();

    let seed = l_mult(&mut ctx, f[0], Word16(4096));
    let (mut b2_hi, mut b2_lo) = l_extract(seed);

    let mut acc = mpy_32_16(b2_hi, b2_lo, x);
    acc = l_shl(&mut ctx, acc, 1);
    acc = l_mac(&mut ctx, acc, f[1], Word16(4096));
    let (mut b1_hi, mut b1_lo) = l_extract(acc);

    for &coefficient in f.iter().take(n).skip(2) {
        // b0 = 2x·b1 - b2 + f[i]. The doubling is folded into the shift, so
        // b2 is subtracted at half weight before it and its low half after.
        let mut acc = mpy_32_16(b1_hi, b1_lo, x);
        acc = l_mac(&mut ctx, acc, b2_hi, Word16(-16384));
        acc = l_mac(&mut ctx, acc, coefficient, Word16(2048));
        acc = l_shl(&mut ctx, acc, 1);
        acc = l_msu(&mut ctx, acc, b2_lo, Word16(1));
        let (b0_hi, b0_lo) = l_extract(acc);

        b2_hi = b1_hi;
        b2_lo = b1_lo;
        b1_hi = b0_hi;
        b1_lo = b0_lo;
    }

    // The tail is *not* the loop body with a different index: it computes
    // x·b1 - b2 + f[n]/2, so there is no doubling, `b2` comes in at full
    // weight through -32768 (which relies on `L_mult(-32768,-32768)`
    // saturating to 0x7FFFFFFF), and the low half is subtracted *before* the
    // constant term rather than after it.
    let mut acc = mpy_32_16(b1_hi, b1_lo, x);
    acc = l_mac(&mut ctx, acc, b2_hi, Word16(-32768));
    acc = l_msu(&mut ctx, acc, b2_lo, Word16(1));
    acc = l_mac(&mut ctx, acc, f[n], Word16(2048));
    let acc = l_shl(&mut ctx, acc, 6);

    let value = extract_h(acc);
    // Clamped so the caller's `L_mult(ylow, yhigh)` cannot saturate to a
    // product whose sign misreads the bracket.
    if value.0 == -32768 {
        Word16(-32767)
    } else {
        value
    }
}

// ---------------------------------------------------------------------------
// Weighted speech
// ---------------------------------------------------------------------------

/// Bandwidth-expand a predictor: `ap[i] = a[i]·γⁱ`, Q12 in and out.
///
/// The power of γ is built by **iterated rounding** — `fac ← round(fac·γ)` —
/// so its error accumulates. Computing γⁱ exactly and rounding once gives
/// different coefficients.
#[must_use]
pub fn weight_a(a: &[Word16; LP_ORDER + 1], gamma: Word16) -> [Word16; LP_ORDER + 1] {
    let mut ctx = DspContext::default();
    let mut ap = [Word16(0); LP_ORDER + 1];

    ap[0] = a[0];
    let mut fac = gamma;
    for i in 1..LP_ORDER {
        let scaled = l_mult(&mut ctx, a[i], fac);
        ap[i] = round(&mut ctx, scaled);
        // Updated *after* use, and only m-1 times, so the last tap below sees
        // γ^m.
        let next = l_mult(&mut ctx, fac, gamma);
        fac = round(&mut ctx, next);
    }
    let scaled = l_mult(&mut ctx, a[LP_ORDER], fac);
    ap[LP_ORDER] = round(&mut ctx, scaled);
    ap
}

/// Filter a signal through `A(z)` to get its residual, at **twice** the
/// mathematical value.
///
/// `signal` must carry `a.len() - 1` samples of history in front of the block
/// to be filtered; `out` receives one sample per block sample. The `<< 4`
/// against Q12 coefficients is what doubles the output, and it saturates
/// before rounding.
///
/// # Panics
///
/// If `signal` is not `history + out.len()` long.
pub fn residu(a: &[Word16], signal: &[Word16], out: &mut [Word16]) {
    let m = a.len() - 1;
    assert_eq!(
        signal.len(),
        m + out.len(),
        "residu needs `m` samples of history in front of the block"
    );
    let mut ctx = DspContext::default();

    for (i, slot) in out.iter_mut().enumerate() {
        // `window[m]` is sample i; `window[m - j]` is sample i-j.
        let window = &signal[i..=i + m];
        let mut acc = l_mult(&mut ctx, window[m], a[0]);
        for j in 1..=m {
            acc = l_mac(&mut ctx, acc, a[j], window[m - j]);
        }
        let acc = l_shl(&mut ctx, acc, 4);
        *slot = round(&mut ctx, acc);
    }
}

/// De-emphasise in place through `1/(1 - μz⁻¹)`, halving as it goes.
///
/// The mirror image of the encoder's input pre-emphasis: ascending, and each
/// step reads the previous *output*. The halving cancels [`residu`]'s
/// doubling, so the weighted speech comes out on the same scale as the speech
/// it was derived from.
pub fn deemph2(x: &mut [Word16], mu: Word16, memory: &mut Word16) {
    let mut ctx = DspContext::default();
    let mut previous = *memory;

    for slot in x.iter_mut() {
        let mut acc = l_mult(&mut ctx, *slot, Word16(16384));
        // Saturation here is expected; the reference says so.
        acc = l_mac(&mut ctx, acc, previous, mu);
        *slot = round(&mut ctx, acc);
        previous = *slot;
    }

    *memory = previous;
}

/// Decimate by two through a 5-tap FIR, in place.
///
/// `x[0..n]` becomes `x[0..n/2]`. `memory` carries the three samples that
/// precede the block, and is updated from the last three **input** samples
/// before any output is written — so it lives in the un-`wsp_shift`-scaled
/// domain, and is rescaled by the frame's `exp` rather than by `wsp_shift`.
///
/// # Panics
///
/// If `x` is longer than a frame or has an odd length.
pub fn lp_decim2(x: &mut [Word16], memory: &mut [Word16; 3]) {
    let n = x.len();
    assert!(n <= L_FRAME && n.is_multiple_of(2), "decimation needs an even block");
    let mut ctx = DspContext::default();

    let mut buffer = [Word16(0); 3 + L_FRAME];
    buffer[..3].copy_from_slice(memory);
    buffer[3..3 + n].copy_from_slice(x);
    memory.copy_from_slice(&x[n - 3..n]);

    for j in 0..n / 2 {
        let mut acc = Word32(0);
        for (k, &tap) in H_FIR.iter().enumerate() {
            acc = l_mac(&mut ctx, acc, buffer[2 * j + k], tap);
        }
        x[j] = round(&mut ctx, acc);
    }
}

// ---------------------------------------------------------------------------
// Frame-level orchestration
// ---------------------------------------------------------------------------

/// Everything one frame of the front end produces.
///
/// The signal path here is rate-independent: nothing in it differs between the
/// nine ACELP modes.
#[derive(Clone, Debug)]
pub struct FrontEndFrame {
    /// `p_window` / `old_speech`: the whole 384-word speech buffer, in the
    /// frame's `Q_new` domain.
    ///
    /// The subframe loop's "present frame" is `window[64..320]`, and the 16
    /// samples of history a `Residu` call needs sit at `window[48..64]`.
    pub window: [Word16; L_TOTAL],
    /// The scaling every carried buffer must be brought onto.
    pub scaling: Scaling,
    /// Extra shift applied to the decimated weighted speech, in `-3..=0`.
    pub wsp_shift: i16,
    /// `exp` after §4.14's mutation: what `old_hp_wsp` and the `Hp_wsp` state
    /// must be rescaled by. **Not** the same as `scaling.exp`, which is what
    /// `old_exc`, `mem_syn` and `mem_w0` take.
    pub wsp_exp: i16,
    /// Lag-windowed autocorrelations of the analysis window.
    pub autocorr: Autocorrelation,
    /// Unquantised predictor for the fourth subframe, Q12.
    pub a: [Word16; LP_ORDER + 1],
    /// Reflection coefficients, Q15. Unused downstream; recorded for tests.
    pub rc: [Word16; LP_ORDER],
    /// This frame's ISPs, Q15.
    pub isp: [Word16; LP_ORDER],
    /// Unquantised ISFs, Q15 — the quantiser's input, and what the pitch-gain
    /// clipping test reads.
    pub isf: [Word16; LP_ORDER],
    /// Unquantised interpolated predictors, one per subframe, Q12.
    pub a_interp: [[Word16; LP_ORDER + 1]; NB_SUBFR],
    /// Weighted speech, decimated by two: the open-loop pitch search's input.
    pub wsp: [Word16; L_FRAME / OPL_DECIM],
    /// The pitch search's history: the 115 decimated samples before `wsp`,
    /// already rescaled by [`Self::wsp_exp`].
    pub wsp_history: [Word16; WSP_HISTORY],
}

/// The encoder front end: everything from 16 kHz input to the weighted speech
/// the open-loop pitch search runs on.
///
/// Owns the state `coder()` carries in `old_speech`, `old_wsp`,
/// `mem_levinson`, `ispold`, `mem_wsp`, `mem_decim2`, `old_wsp_max` and
/// `old_wsp_shift`, plus [`Preprocessor`]'s. It deliberately does **not** own
/// `old_exc`, `mem_syn`, `mem_w0`, `old_hp_wsp` or the `Hp_wsp` memory: those
/// belong to the stages after it, which must rescale them themselves using
/// [`FrontEndFrame::scaling`] and [`FrontEndFrame::wsp_exp`].
#[derive(Clone, Debug)]
pub struct FrontEnd {
    preproc: Preprocessor,
    /// `st->old_speech`: the 128 words that precede the next frame.
    old_speech: [Word16; L_TOTAL - L_FRAME],
    /// `st->old_wsp`: decimated weighted-speech history.
    old_wsp: [Word16; WSP_HISTORY],
    levinson: LevinsonMemory,
    /// `st->ispold`: the previous frame's ISPs.
    isp_old: [Word16; LP_ORDER],
    /// `st->mem_wsp`: the de-emphasis memory.
    wsp_memory: Word16,
    /// `st->mem_decim2`: the decimation FIR's three carried samples.
    decim2_memory: [Word16; 3],
    /// `st->old_wsp_max`: the previous frame's peak weighted speech.
    old_wsp_max: Word16,
    /// `st->old_wsp_shift`: the previous frame's extra `wsp` shift.
    old_wsp_shift: i16,
}

impl Default for FrontEnd {
    fn default() -> Self {
        Self::new()
    }
}

impl FrontEnd {
    /// A cold-start front end, matching `Reset_encoder(st, 1)`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            preproc: Preprocessor::new(),
            old_speech: [Word16(0); L_TOTAL - L_FRAME],
            old_wsp: [Word16(0); WSP_HISTORY],
            levinson: LevinsonMemory::new(),
            isp_old: ISP_INIT,
            wsp_memory: Word16(0),
            decim2_memory: [Word16(0); 3],
            old_wsp_max: Word16(0),
            old_wsp_shift: 0,
        }
    }

    /// Run one 20 ms frame of analysis.
    ///
    /// `speech16k` must already have passed through
    /// [`super::preproc::restrict_to_14_bit`] — the reference applies that at
    /// its caller, not inside `coder()`.
    pub fn process_frame(&mut self, speech16k: &[Word16; L_FRAME16K]) -> FrontEndFrame {
        let mut ctx = DspContext::default();

        // The 384-word buffer, with last frame's tail 128 words in front of
        // the region the decimator is about to fill.
        let mut window = [Word16(0); L_TOTAL];
        window[..L_TOTAL - L_FRAME].copy_from_slice(&self.old_speech);

        self.preproc.band_limit(speech16k, &mut window[NEW_SPEECH..]);
        let scaling = self.preproc.preemphasise(&mut window[NEW_SPEECH..]);

        // Bring the carried history onto the new frame's scale. The bound is
        // NEW_SPEECH — 116 — precisely because the pre-emphasis has already
        // shifted everything from there upward. The 12 stale tail words above
        // it are never read before the decimator overwrites them.
        scale_sig(&mut ctx, &mut window[..NEW_SPEECH], scaling.exp);
        scale_sig(&mut ctx, &mut self.decim2_memory, scaling.exp);
        scale_sig(&mut ctx, core::slice::from_mut(&mut self.wsp_memory), scaling.exp);

        // LP analysis, centred on the fourth subframe. `autocorrelation`
        // already applies the lag window on its way out.
        let autocorr = autocorrelation(&window);
        let (a, rc) = levinson(&autocorr, &mut self.levinson);
        let isp = az_isp(&a, &self.isp_old);

        // Both of these read the *previous* frame's ISPs, so the update comes
        // after the interpolation.
        let a_interp = interpolate_isp(&self.isp_old, &isp);
        self.isp_old = isp;
        let isf = isp_to_isf(&isp);

        let (wsp, wsp_history, wsp_shift, wsp_exp) = self.weighted_speech(&window, &a_interp, scaling);

        self.old_speech.copy_from_slice(&window[L_FRAME..]);

        FrontEndFrame {
            window,
            scaling,
            wsp_shift,
            wsp_exp,
            autocorr,
            a,
            rc,
            isp,
            isf,
            a_interp,
            wsp,
            wsp_history,
        }
    }

    /// The weighted speech chain: four `Weight_a`/`Residu` pairs, de-emphasis,
    /// the level-dependent shift, decimation by two, and the history rescale.
    ///
    /// Returns `(wsp, history, wsp_shift, wsp_exp)`.
    fn weighted_speech(
        &mut self,
        window: &[Word16; L_TOTAL],
        a_interp: &[[Word16; LP_ORDER + 1]; NB_SUBFR],
        scaling: Scaling,
    ) -> (
        [Word16; L_FRAME / OPL_DECIM],
        [Word16; WSP_HISTORY],
        i16,
        i16,
    ) {
        let mut ctx = DspContext::default();

        // The reference's `old_wsp[371]`: history, then this frame's block.
        let mut buffer = [Word16(0); L_FRAME + WSP_HISTORY];
        buffer[..WSP_HISTORY].copy_from_slice(&self.old_wsp);

        // `Residu` reaches `LP_ORDER` samples further back than the present
        // frame's start, which is why the buffer keeps 64 words of genuine
        // history in front of it.
        for (k, a) in a_interp.iter().enumerate() {
            let ap = weight_a(a, GAMMA1);
            let start = k * L_SUBFR;
            let source = &window[SPEECH + start - LP_ORDER..SPEECH + start + L_SUBFR];
            residu(
                &ap,
                source,
                &mut buffer[WSP_HISTORY + start..WSP_HISTORY + start + L_SUBFR],
            );
        }
        deemph2(
            &mut buffer[WSP_HISTORY..WSP_HISTORY + L_FRAME],
            TILT_FAC,
            &mut self.wsp_memory,
        );

        // The peak is measured on the 256 undecimated samples but the shift is
        // applied to the 128 decimated ones. Measuring after decimation gives
        // a different shift on some frames.
        let mut peak = Word16(0);
        for i in 0..L_FRAME {
            let magnitude = abs_s(&mut ctx, buffer[WSP_HISTORY + i]);
            if magnitude.0 > peak.0 {
                peak = magnitude;
            }
        }
        // One frame of hysteresis: the comparison uses both frames' peaks, but
        // only this frame's is remembered.
        let reference = peak.max(self.old_wsp_max);
        self.old_wsp_max = peak;
        // `norm_s(0) = 0`, so a silent frame yields -3 rather than 0.
        let wsp_shift = sub(&mut ctx, Word16(norm_s(reference)), Word16(3)).0.min(0);

        lp_decim2(
            &mut buffer[WSP_HISTORY..WSP_HISTORY + L_FRAME],
            &mut self.decim2_memory,
        );
        let decimated = L_FRAME / OPL_DECIM;
        scale_sig(
            &mut ctx,
            &mut buffer[WSP_HISTORY..WSP_HISTORY + decimated],
            wsp_shift,
        );

        // `exp` is reused and mutated here: it entered as Q_new - Q_old and
        // leaves carrying the change in `wsp_shift` too. The history rescale
        // below — and the pitch search's own buffers — see the mutated value;
        // everything earlier saw the original.
        let change = sub(&mut ctx, Word16(wsp_shift), Word16(self.old_wsp_shift));
        let wsp_exp = add(&mut ctx, Word16(scaling.exp), change).0;
        self.old_wsp_shift = wsp_shift;
        scale_sig(&mut ctx, &mut buffer[..WSP_HISTORY], wsp_exp);

        let mut wsp = [Word16(0); L_FRAME / OPL_DECIM];
        wsp.copy_from_slice(&buffer[WSP_HISTORY..WSP_HISTORY + decimated]);
        let mut history = [Word16(0); WSP_HISTORY];
        history.copy_from_slice(&buffer[..WSP_HISTORY]);

        self.old_wsp
            .copy_from_slice(&buffer[decimated..decimated + WSP_HISTORY]);

        (wsp, history, wsp_shift, wsp_exp)
    }
}

#[cfg(test)]
mod tests {
    use super::super::preproc::restrict_to_14_bit;
    use super::super::preproc::trace_support::{frames, input_frame, scalar, words};
    use super::*;

    /// Replay the whole front end across every committed frame.
    ///
    /// Returns the number of frames replayed so callers can assert it: a
    /// harness that silently yields nothing reads as agreement.
    fn replay(mut check: impl FnMut(usize, &FrontEndFrame)) -> usize {
        let mut front = FrontEnd::new();
        let total = frames();
        for frame in 0..total {
            let mut input = input_frame(frame);
            restrict_to_14_bit(&mut input);
            let result = front.process_frame(&input);
            check(frame, &result);
        }
        total
    }

    fn compare(frame: usize, label: &str, got: &[Word16], want: &[Word16]) -> usize {
        assert_eq!(got.len(), want.len(), "frame {frame}: {label} length");
        for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                g.0, w.0,
                "frame {frame}: {label}[{i}] = {} but the reference gives {}",
                g.0, w.0
            );
        }
        got.len()
    }

    #[test]
    fn analysis_window_is_bit_exact_against_ts26173() {
        // The 384-word buffer the LP analysis sees: the freshly pre-emphasised
        // frame *and* the 116 words of carried history that `Scale_sig` had to
        // bring onto the new frame's scale. Frame 0's history is zero, so this
        // only becomes a real test from frame 1 onward.
        let mut compared = 0usize;
        let count = replay(|frame, got| {
            compared += compare(frame, "window", &got.window, &words(frame, "window", L_TOTAL));
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * L_TOTAL, "1152 window samples compared");
    }

    #[test]
    fn autocorrelation_is_bit_exact_against_ts26173() {
        // `r_h_pre`/`r_l_pre` and `r_h`/`r_l` are identical in the committed
        // trace: the instrumentation's "pre" point sits *after* `Lag_window`,
        // not before it. Both are compared anyway, and their equality is
        // asserted, so a future trace that fixes the instrumentation fails
        // here loudly instead of silently comparing the wrong thing.
        let mut compared = 0usize;
        let count = replay(|frame, got| {
            for label in ["r_h", "r_h_pre"] {
                compared += compare(
                    frame,
                    label,
                    &got.autocorr.high,
                    &words(frame, label, LP_ORDER + 1),
                );
            }
            for label in ["r_l", "r_l_pre"] {
                compared += compare(
                    frame,
                    label,
                    &got.autocorr.low,
                    &words(frame, label, LP_ORDER + 1),
                );
            }
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * 4 * (LP_ORDER + 1), "204 lag values compared");
    }

    #[test]
    fn levinson_is_bit_exact_against_ts26173() {
        let mut compared = 0usize;
        let count = replay(|frame, got| {
            compared += compare(frame, "A", &got.a, &words(frame, "A", LP_ORDER + 1));
            compared += compare(frame, "rc", &got.rc, &words(frame, "rc", LP_ORDER));
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * (LP_ORDER + 1 + LP_ORDER), "99 values compared");
    }

    #[test]
    fn az_isp_is_bit_exact_against_ts26173() {
        let mut compared = 0usize;
        let count = replay(|frame, got| {
            compared += compare(frame, "ispnew", &got.isp, &words(frame, "ispnew", LP_ORDER));
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * LP_ORDER, "48 ISPs compared");
    }

    #[test]
    fn unquantised_isf_is_bit_exact_against_ts26173() {
        // `isf_unq46` is what the 46-bit quantiser is handed. It is a pure
        // function of `ispnew`, so agreement here is a second, independent
        // reading of the root search.
        let mut compared = 0usize;
        let count = replay(|frame, got| {
            compared += compare(
                frame,
                "isf_unq46",
                &got.isf,
                &words(frame, "isf_unq46", LP_ORDER),
            );
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * LP_ORDER, "48 ISFs compared");
    }

    #[test]
    fn interpolated_predictors_are_bit_exact_against_ts26173() {
        let mut compared = 0usize;
        let count = replay(|frame, got| {
            let want = words(frame, "A_interp", NB_SUBFR * (LP_ORDER + 1));
            let flat: Vec<Word16> = got.a_interp.iter().flatten().copied().collect();
            compared += compare(frame, "A_interp", &flat, &want);
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * NB_SUBFR * (LP_ORDER + 1), "204 values compared");
    }

    #[test]
    fn weighted_speech_is_bit_exact_against_ts26173() {
        let mut compared = 0usize;
        let mut shifts = Vec::new();
        let count = replay(|frame, got| {
            compared += compare(
                frame,
                "wsp",
                &got.wsp,
                &words(frame, "wsp", L_FRAME / OPL_DECIM),
            );
            assert_eq!(
                i32::from(got.wsp_shift),
                scalar(frame, "wsp_shift"),
                "frame {frame}: wsp_shift"
            );
            shifts.push(got.wsp_shift);
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * L_FRAME / OPL_DECIM, "384 samples compared");
        // The shift is not constant across the trace, so the peak hysteresis
        // and the `exp` mutation that depends on it are genuinely exercised.
        assert_eq!(shifts, vec![-1, -1, 0], "wsp_shift must vary across the trace");
    }

    // ---- the root search's decisions ----

    #[test]
    fn the_root_search_finds_fifteen_roots_inside_their_own_grid_intervals() {
        // The count and the placement, not just the values: each accepted root
        // must lie in the interval the scan says bracketed it, the grid
        // indices must strictly increase (the scan never revisits), and the
        // roots must descend (F1 and F2 interlace).
        let count = replay(|frame, got| {
            let search = az_isp_detail(&got.a, &ISP_INIT);
            assert_eq!(search.roots_found, LP_ORDER - 1, "frame {frame}: root count");
            assert!(!search.fell_back, "frame {frame}: fell back to old ISPs");

            let mut previous_index = 0usize;
            for (n, &j) in search.accepted_at.iter().enumerate() {
                assert!(
                    j > previous_index,
                    "frame {frame}: root {n} accepted at grid {j}, not after {previous_index}"
                );
                // The root is bracketed below by grid[j]; its upper bound is
                // either the previous root or grid[j-1].
                let upper = if n == 0 {
                    GRID[j - 1]
                } else {
                    search.isp[n - 1].max(GRID[j - 1])
                };
                assert!(
                    search.isp[n].0 >= GRID[j].0 && search.isp[n].0 <= upper.0,
                    "frame {frame}: root {n} = {} outside [{}, {}]",
                    search.isp[n].0,
                    GRID[j].0,
                    upper.0
                );
                previous_index = j;
            }
            for n in 1..LP_ORDER - 1 {
                assert!(
                    search.isp[n].0 < search.isp[n - 1].0,
                    "frame {frame}: roots {} and {n} are not descending",
                    n - 1
                );
            }
            // The last entry is a[16], not a root.
            let mut ctx = DspContext::default();
            assert_eq!(search.isp[LP_ORDER - 1], shl(&mut ctx, got.a[LP_ORDER], 3));
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
    }

    /// Plausible alternatives to the reference's search decisions.
    ///
    /// Each is something an implementer might write believing it equivalent.
    /// The test below requires every one of them to change the answer, so a
    /// future edit that "tidies" the search cannot pass unnoticed.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Variant {
        /// The reference.
        Reference,
        /// `(xlow + xhigh) >> 1` instead of two independent floors.
        MidpointRoundsTogether,
        /// A strict sign-change test, so an exact zero is not a root.
        StrictSignChange,
        /// Continue from the grid point rather than from the accepted root.
        NoRootInheritance,
        /// Skip re-evaluating `ylow` on the new polynomial.
        NoReevaluation,
        /// "Improve" the bisection by halving four times instead of twice.
        FourBisections,
    }

    /// The search loop, parameterised so the alternatives can be run against
    /// the same trace data. Deliberately a separate copy: the production
    /// search must not grow a knob to make this test possible.
    fn search(a: &[Word16; LP_ORDER + 1], variant: Variant) -> [Word16; LP_ORDER] {
        let mut ctx = DspContext::default();
        let (f1, f2) = sum_difference_polynomials(&mut ctx, a);
        let mut isp = [Word16(0); LP_ORDER];
        let mut found = 0usize;
        let mut on_f1 = true;
        let mut xlow = GRID[0];
        let mut ylow = evaluate(xlow, &f1, &f2, on_f1);
        let mut j = 0usize;

        while found < LP_ORDER - 1 && j < GRID_POINTS {
            j += 1;
            let mut xhigh = xlow;
            let mut yhigh = ylow;
            xlow = GRID[j];
            ylow = evaluate(xlow, &f1, &f2, on_f1);

            let product = l_mult(&mut ctx, ylow, yhigh).0;
            let bracketed = if variant == Variant::StrictSignChange {
                product < 0
            } else {
                product <= 0
            };
            if !bracketed {
                continue;
            }

            let halvings = if variant == Variant::FourBisections { 4 } else { 2 };
            for _ in 0..halvings {
                let xmid = if variant == Variant::MidpointRoundsTogether {
                    Word16(((i32::from(xlow.0) + i32::from(xhigh.0)) >> 1) as i16)
                } else {
                    let half_low = shr(&mut ctx, xlow, 1);
                    let half_high = shr(&mut ctx, xhigh, 1);
                    add(&mut ctx, half_low, half_high)
                };
                let ymid = evaluate(xmid, &f1, &f2, on_f1);
                if l_mult(&mut ctx, ylow, ymid).0 <= 0 {
                    yhigh = ymid;
                    xhigh = xmid;
                } else {
                    ylow = ymid;
                    xlow = xmid;
                }
            }

            let root = interpolate_root(&mut ctx, xlow, xhigh, ylow, yhigh);
            isp[found] = root;
            found += 1;
            if variant == Variant::NoRootInheritance {
                xlow = GRID[j];
            } else {
                xlow = root;
            }
            on_f1 = !on_f1;
            if variant != Variant::NoReevaluation {
                ylow = evaluate(xlow, &f1, &f2, on_f1);
            }
        }

        if found < LP_ORDER - 1 {
            return ISP_INIT;
        }
        isp[LP_ORDER - 1] = shl(&mut ctx, a[LP_ORDER], 3);
        isp
    }

    #[test]
    fn the_search_decisions_are_the_ones_the_trace_pins_down() {
        // Each alternative is run against the same A(z) the reference
        // produced. Everything the trace *can* distinguish must come out
        // different; anything it cannot is listed explicitly below, so the
        // gap is recorded rather than mistaken for coverage.
        let alternatives = [
            Variant::MidpointRoundsTogether,
            Variant::StrictSignChange,
            Variant::NoRootInheritance,
            Variant::NoReevaluation,
            Variant::FourBisections,
        ];
        let mut distinguished = vec![false; alternatives.len()];

        let count = replay(|frame, got| {
            assert_eq!(
                search(&got.a, Variant::Reference),
                got.isp,
                "frame {frame}: the parameterised copy must agree with az_isp"
            );
            for (n, &variant) in alternatives.iter().enumerate() {
                if search(&got.a, variant) != got.isp {
                    distinguished[n] = true;
                }
            }
        });
        assert_eq!(count, 3, "the committed trace covers three frames");

        let undistinguished: Vec<Variant> = alternatives
            .iter()
            .zip(distinguished.iter())
            .filter_map(|(&v, &d)| (!d).then_some(v))
            .collect();
        // `StrictSignChange` only bites when a Chebyshev evaluation lands on
        // exactly zero at a grid point, which none of the three committed
        // frames does. It is covered instead by
        // `an_exact_zero_at_a_grid_point_counts_as_a_sign_change`, which tests
        // the predicate directly. Every other alternative must change the
        // answer here.
        assert_eq!(
            undistinguished,
            vec![Variant::StrictSignChange],
            "the set of search decisions this trace cannot distinguish changed"
        );
    }

    #[test]
    fn an_exact_zero_at_a_grid_point_counts_as_a_sign_change() {
        // The tie-break the `<= 0` predicate encodes, tested directly because
        // a zero evaluation is rare enough that the three committed frames may
        // never produce one.
        let mut ctx = DspContext::default();
        for (ylow, yhigh) in [(0i16, 5i16), (5, 0), (0, 0), (0, -5), (-5, 0)] {
            assert!(
                l_mult(&mut ctx, Word16(ylow), Word16(yhigh)).0 <= 0,
                "({ylow}, {yhigh}) must bracket a root"
            );
        }
        for (ylow, yhigh) in [(5i16, 7i16), (-5, -7)] {
            assert!(
                l_mult(&mut ctx, Word16(ylow), Word16(yhigh)).0 > 0,
                "({ylow}, {yhigh}) must not bracket a root"
            );
        }
    }

    #[test]
    fn a_failed_search_returns_every_old_isp_including_the_last() {
        // Chosen so `f1` collapses to its constant term: a[i] = -a[16-i] for
        // i < 8 makes every f1[i] zero, leaving C(x) = f1[8]/2, which never
        // changes sign. The scan therefore reaches the end of the grid having
        // found nothing, and *all sixteen* values — the a[16] term included —
        // come from the previous frame.
        let mut a = [Word16(0); LP_ORDER + 1];
        a[0] = Word16(4096);
        a[NC] = Word16(4096);
        a[LP_ORDER] = Word16(-4096);
        let old: [Word16; LP_ORDER] = ISP_INIT;

        let search = az_isp_detail(&a, &old);
        assert!(search.fell_back, "this predictor must fail the search");
        assert_eq!(search.roots_found, 0, "F1 has no sign change to find");
        assert_eq!(search.isp, old, "all sixteen ISPs come from old_isp");
        // In particular the a[16] term is discarded, not carried through.
        let mut ctx = DspContext::default();
        assert_ne!(search.isp[LP_ORDER - 1], shl(&mut ctx, a[LP_ORDER], 3));
    }

    #[test]
    fn the_grid_matches_its_generating_formula() {
        // Catches a transcription slip in the 101-entry table without claiming
        // the formula is normative: the endpoints are pinned to 1.0 and -1.0
        // by hand in the reference and are excluded.
        assert_eq!(GRID[0].0, 32767);
        assert_eq!(GRID[GRID_POINTS].0, -32760);
        #[allow(clippy::cast_precision_loss)]
        for (i, point) in GRID.iter().enumerate().take(GRID_POINTS).skip(1) {
            let want =
                ((std::f64::consts::PI * i as f64 / GRID_POINTS as f64).cos() * 32768.0).round();
            assert!(
                (f64::from(point.0) - want).abs() <= 1.0,
                "grid[{i}] = {} but cos gives {want}",
                point.0
            );
        }
        for (i, pair) in GRID.windows(2).enumerate() {
            assert!(pair[1].0 < pair[0].0, "grid must descend at {}", i + 1);
        }
    }

    // ---- Levinson's decision ----

    #[test]
    fn an_unstable_reflection_coefficient_keeps_the_previous_filter() {
        // r[1] = 0 leaves the first reflection coefficient at zero, so Alpha
        // is still r[0]; r[2] = -r[0] then asks for |K| = 1 at the second
        // order. The reference must emit the previous frame's A(z) unchanged,
        // write a[0] = 4096 anyway, and leave its own memory untouched so a
        // run of unstable frames all emit the same last-good filter.
        let mut mem = LevinsonMemory::new();
        for i in 0..LP_ORDER {
            mem.previous_a[i] = Word16(100 + i as i16);
        }
        mem.previous_rc = [Word16(-1000), Word16(2000)];
        let before = mem.clone();

        let mut r = Autocorrelation::default();
        let (hi, lo) = l_extract(Word32(0x7fff_ffff));
        r.high[0] = hi;
        r.low[0] = lo;
        let (hi, lo) = l_extract(Word32(-0x7fff_ffff));
        r.high[2] = hi;
        r.low[2] = lo;

        let (a, rc) = levinson(&r, &mut mem);
        assert_eq!(a[0].0, 4096, "a[0] is written even on the unstable path");
        assert_eq!(&a[1..], &before.previous_a[..], "the old filter is re-emitted");
        assert_eq!(rc[0], before.previous_rc[0]);
        assert_eq!(rc[1], before.previous_rc[1]);
        assert_eq!(mem, before, "the unstable path must not update the memory");
    }

    #[test]
    fn levinson_updates_its_memory_on_the_stable_path() {
        // The other half of the contract above: a normal frame must leave the
        // memory holding exactly what it returned, or the fallback would
        // re-emit a stale filter.
        let count = replay(|frame, got| {
            let mut mem = LevinsonMemory::new();
            let (a, rc) = levinson(&got.autocorr, &mut mem);
            assert_eq!(&mem.previous_a[..], &a[1..], "frame {frame}: stored A");
            assert_eq!(mem.previous_rc, [rc[0], rc[1]], "frame {frame}: stored rc");
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
    }

    #[test]
    fn weighting_builds_its_gamma_powers_by_iterated_rounding() {
        // `ap[m]` uses gamma^m, and that power is accumulated through a round
        // at every step rather than computed once. Reproducing the ladder is
        // mandatory: it drifts from the exactly-computed power by an LSB from
        // the sixth term onward.
        let count = replay(|frame, got| {
            let ap = weight_a(&got.a_interp[0], GAMMA1);
            assert_eq!(ap[0], got.a_interp[0][0], "frame {frame}: ap[0] is a copy");

            let mut ctx = DspContext::default();
            let mut fac = GAMMA1;
            for (i, (&weighted, &plain)) in ap
                .iter()
                .zip(got.a_interp[0].iter())
                .enumerate()
                .skip(1)
            {
                let scaled = l_mult(&mut ctx, plain, fac);
                let want = round(&mut ctx, scaled);
                assert_eq!(weighted, want, "frame {frame}: ap[{i}]");
                if i < LP_ORDER {
                    let next = l_mult(&mut ctx, fac, GAMMA1);
                    fac = round(&mut ctx, next);
                }
            }
        });
        assert_eq!(count, 3, "the committed trace covers three frames");

        // And the ladder is not the same sequence as gamma^i rounded once.
        let mut ctx = DspContext::default();
        let mut fac = GAMMA1;
        let mut exact = f64::from(GAMMA1.0) / 32768.0;
        let mut diverged = false;
        for _ in 1..LP_ORDER {
            let next = l_mult(&mut ctx, fac, GAMMA1);
            fac = round(&mut ctx, next);
            exact *= f64::from(GAMMA1.0) / 32768.0;
            #[allow(clippy::cast_possible_truncation)]
            let single_rounding = (exact * 32768.0).round() as i16;
            if single_rounding != fac.0 {
                diverged = true;
            }
        }
        assert!(diverged, "the iterated ladder must drift from gamma^i");
    }
}
