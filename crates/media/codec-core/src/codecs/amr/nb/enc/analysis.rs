//! Encoder LP analysis, from the analysis window to the pitch search's target.
//!
//! The autocorrelation and its lag window, Levinson-Durbin, the `A(z) → LSP`
//! root search, the weighted speech, and the per-subframe pre-processing.
//!
//! Implements TS 26.073 `lpc` (`lpc.c`), `Autocorr` (`autocorr.c`),
//! `Lag_window` (`lag_wind.c`), `Levinson` (`levinson.c`), `Az_lsp` + `Chebps`
//! (`az_lsp.c`), `pre_big` (`pre_big.c`) and `subframePreProc` (`spreproc.c`),
//! together with the `old_speech` addressing from `cod_amr_reset`
//! (`cod_amr.c` 167–180) that decides which samples each of them sees.
//!
//! The filters the decoder already carries are reused rather than rewritten:
//! [`expand_bandwidth`] (`Weight_Ai`), [`lp_residual`] (`Residu`) and
//! [`synthesis_filter`] / [`synthesis_filter_in_place`] (`Syn_filt`, whose
//! copy-out through scratch is what makes the two in-place calls below safe).
//!
//! Validated against `testdata/nb_enc_trace.txt` — the `A_t`, `lsp_new`, `xn`,
//! `h1` and `res` rows — replayed across all three committed frames from
//! `testdata/amrnb_enc_input.pcm`, so the carried `old_speech` history, the
//! Levinson state and the previous frame's LSPs are exercised rather than
//! assumed. `Aq` is not in the trace and is reconstructed by replaying the
//! committed bitstream through the decoder's LSF dequantiser, which is the
//! same spectrum by construction.
//!
//! Checked off-tree against the full 50-frame traces
//! `tools/trace-amrnb-encoder.sh` produces: 36 200 values at 7.40 kbit/s and
//! 34 200 at 10.2 (which takes the other weighting numerator), plus 4 200 at
//! 12.2, where the two analyses per frame and their shared Levinson state are
//! the thing under test. All matched.
//!
//! # One stage with no committed fixture
//!
//! [`WeightedSpeech`] has none: the committed trace carries no `wsp` row, and
//! the open-loop pitch lag it feeds is only traced at the two rates that search
//! over a whole frame. It was checked off-tree against a one-line extra trace
//! point in the reference — 8 000 samples at 7.40 and 8 000 at 10.2, all
//! matching — but the tests here can only pin its structure: which `A_t` slot
//! each subframe takes, and that the filter memory advances once per subframe
//! rather than once per call. Treat a `wsp` regression as something the
//! open-loop pitch tests must catch.
//!
//! # Rate dependence
//!
//! 12.2 kbit/s runs **two** LP analyses per frame, over two different windows
//! on the *same* 240 samples, and fills `A_t` slots 1 and 3. Every other rate
//! runs one, over a window on a span shifted 40 samples later, and fills slot
//! 3 alone. The remaining slots come from the LSP interpolation in `lsp()`,
//! which is not this module.
//!
//! # Q-formats
//!
//! Speech is Q0. Autocorrelations are normalised double-precision pairs with
//! no fixed Q. Predictor coefficients `a[]` and the weighted `ap[]` are Q12
//! with `a[0] = 4096`; reflection coefficients are Q15; LSPs are Q15 cosines.
//! The impulse response `h1` is Q12; `xn`, `res` and the weighted speech are
//! Q0.

// A transcription of reference fixed-point arithmetic: the magic constants are
// the specification, the index arithmetic is deliberately unchecked, and the
// root search has the shape the reference gives it rather than the shape a
// numerical analyst would choose.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::unreadable_literal
)]

use super::super::decoder_tables::{
    GAMMA1, GAMMA1_12K2, GAMMA2, LAG_WINDOW_H, LAG_WINDOW_L, LP_WINDOW_160_80, LP_WINDOW_200_40,
    LP_WINDOW_232_8, LSP_GRID,
};
use super::super::lsp::{AZ_SIZE, M, MP1};
use super::super::synthesis::{
    expand_bandwidth, lp_residual, synthesis_filter, synthesis_filter_in_place,
};
use super::super::{L_FRAME, L_SUBFR};
use crate::fixed_point::arith::{abs_s, add, extract_h, extract_l, negate, round, sub};
use crate::fixed_point::arith32::{l_abs, l_add, l_mac, l_msu, l_mult, l_negate, l_sub, mult_r};
use crate::fixed_point::div::div_s;
use crate::fixed_point::oper32::{div_32, l_comp, l_extract, mpy_32, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, norm_s, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32, MAX_32};

// ------------------------------------------------------------------ modes --

/// 4.75 kbit/s. Codes two subframes' gains jointly.
pub const MR475: u8 = 0;
/// 5.15 kbit/s.
pub const MR515: u8 = 1;
/// 7.95 kbit/s — the boundary `pre_big` tests against.
pub const MR795: u8 = 5;
/// 10.2 kbit/s.
pub const MR102: u8 = 6;
/// 12.2 kbit/s. The only rate with two LP analyses per frame.
pub const MR122: u8 = 7;
/// The DTX pseudo-mode. Never a *requested* mode, but it reaches `pre_big`.
pub const MRDTX: u8 = 8;

// -------------------------------------------------------------- constants --

/// LP analysis window length; also the span of speech each analysis sees.
pub const L_WINDOW: usize = 240;

/// The whole speech history the encoder keeps: two frames' worth.
pub const L_TOTAL: usize = 320;

/// Lookahead: how far past the coded frame the analysis window reaches.
///
/// Equivalently, how far *behind* `new_speech` the coded frame sits. The
/// subframe loop works on `old_speech[120..280]`, which is the last 40 samples
/// of the previous frame followed by the first 120 of this one.
pub const L_NEXT: usize = 40;

/// Where the coded frame starts inside `old_speech`.
///
/// `L_TOTAL - L_FRAME - L_NEXT`. The subframe loop indexes from here, so its
/// sample 0 is 40 samples *older* than the frame just pushed — the encoder
/// always codes one lookahead behind its input.
const CODED_BASE: usize = L_TOTAL - L_FRAME - L_NEXT;

/// Half the LP order — the order of each Chebyshev series in [`az_lsp`].
const NC: usize = M / 2;

/// Intervals in the [`LSP_GRID`] cosine grid; the grid has one more point.
const GRID_POINTS: usize = 60;

/// `Levinson`'s instability threshold, on the high word of `K` alone.
///
/// Strictly greater than this trips it, so `abs(Kh) == 32750` is still stable.
const K_UNSTABLE: Word16 = Word16(32750);

/// The flat filter `A(z) = 1` in Q12 — `Levinson_reset`'s `old_A`.
const FLAT_FILTER: Word16 = Word16(4096);

// ---------------------------------------------------------- speech buffer --

/// The encoder's `old_speech`: 320 samples, two frames deep.
///
/// Every window into it is a fixed offset, and the offsets are the whole
/// reason this is a type rather than an array. `p_window` starts at 80,
/// `p_window_12k2` at 40, and the *coded* frame — what the subframe loop calls
/// `speech` — starts at 120, which is 40 samples behind the frame just pushed.
/// Getting any of those wrong produces a perfectly plausible encoder that
/// analyses the wrong 240 samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechBuffer {
    samples: [Word16; L_TOTAL],
}

impl Default for SpeechBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechBuffer {
    /// A buffer in the state `cod_amr_reset` leaves behind: all 320 zero.
    ///
    /// The reference never primes the lookahead. `Speech_Encode_Frame_First`
    /// exists, would fill `old_speech[120..160]` with the first 40 samples, and
    /// is *never called* by `coder.c` — the published vectors are produced with
    /// that region left at zero, so a port that primes it diverges on frame 1.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            samples: [Word16(0); L_TOTAL],
        }
    }

    /// Place a conditioned frame at `old_speech[160..320]`.
    ///
    /// `frame` is Q0, already masked and high-passed by
    /// [`super::preproc::Preprocessor::condition`] — `cod_amr` copies the
    /// *processed* samples, never the raw PCM.
    pub fn push(&mut self, frame: &[Word16; L_FRAME]) {
        self.samples[L_TOTAL - L_FRAME..].copy_from_slice(frame);
    }

    /// Slide the buffer down one frame, the last thing `cod_amr` does.
    ///
    /// Runs on every frame, including DTX frames where the subframe loop was
    /// skipped entirely.
    pub fn shift(&mut self) {
        self.samples.copy_within(L_FRAME.., 0);
    }

    /// The LP analysis window for every rate except 12.2: `old_speech[80..320]`.
    ///
    /// 200 samples of past-and-current speech plus the 40-sample lookahead.
    ///
    /// # Panics
    ///
    /// Never: the span is a compile-time constant of the right length.
    #[must_use]
    pub fn analysis_window(&self) -> &[Word16; L_WINDOW] {
        self.samples[L_TOTAL - L_WINDOW..]
            .try_into()
            .expect("the window is the tail of the buffer")
    }

    /// The LP analysis window at 12.2 kbit/s: `old_speech[40..280]`.
    ///
    /// The same length, 40 samples earlier. **Both** of 12.2's analyses read
    /// this span; they differ only in which window multiplies it.
    ///
    /// # Panics
    ///
    /// Never: the span is a compile-time constant of the right length.
    #[must_use]
    pub fn analysis_window_12k2(&self) -> &[Word16; L_WINDOW] {
        self.samples[L_TOTAL - L_WINDOW - L_NEXT..L_TOTAL - L_NEXT]
            .try_into()
            .expect("the 12.2 window is one lookahead earlier")
    }

    /// `st->new_speech`: the newest frame, which is *not* the coded one.
    ///
    /// The coded frame sits one lookahead earlier — see
    /// [`coded`](Self::coded). Both the VAD and the DTX energy ring measure
    /// this window instead, so a stream's comfort noise describes the audio
    /// forty samples ahead of the frame it is transmitted with. Passing
    /// `coded()` to either is a plausible-looking substitution that shifts
    /// every energy and every detector decision by half a subframe.
    ///
    /// # Panics
    /// Never: the slice is a fixed tail of a fixed-size array.
    #[must_use]
    pub fn newest(&self) -> &[Word16; L_FRAME] {
        self.samples[L_TOTAL - L_FRAME..]
            .try_into()
            .expect("the newest frame is the tail of the buffer")
    }

    /// The 200 samples `vad1` reads: forty of history, then [`newest`](Self::newest).
    ///
    /// # Panics
    /// Never, as above.
    #[must_use]
    pub fn vad_window(&self) -> &[Word16; L_NEXT + L_FRAME] {
        self.samples[L_TOTAL - L_FRAME - L_NEXT..]
            .try_into()
            .expect("the detector window is the tail of the buffer")
    }

    /// The coded frame: `old_speech[120..280]`, what the subframe loop indexes
    /// from zero.
    ///
    /// # Panics
    ///
    /// Never: the span is a compile-time constant of the right length.
    #[must_use]
    pub fn coded(&self) -> &[Word16; L_FRAME] {
        self.samples[CODED_BASE..CODED_BASE + L_FRAME]
            .try_into()
            .expect("the coded frame sits one lookahead into the buffer")
    }

    /// A window of the coded frame with the `M` samples of history that
    /// `Residu` reads back into.
    ///
    /// Returns `speech[offset - M .. offset + len]` in the subframe loop's own
    /// indexing, which is `old_speech[120 + offset - M .. 120 + offset + len]`.
    /// Both `pre_big` and `subframePreProc` depend on that negative reach: at
    /// `offset == 0` it is the tail of the *previous* frame.
    ///
    /// # Panics
    ///
    /// If the requested window runs past the end of the coded frame.
    #[must_use]
    pub fn with_history(&self, offset: usize, len: usize) -> &[Word16] {
        assert!(offset + len <= L_FRAME, "window runs past the coded frame");
        let base = CODED_BASE + offset - M;
        &self.samples[base..base + M + len]
    }
}

// ------------------------------------------------------------ correlation --

/// One frame's autocorrelation sequence in the reference's double-precision
/// format, with the shift that normalised it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Autocorrelation {
    /// High words of `r[0..=M]`.
    pub r_h: [Word16; MP1],
    /// Low words of `r[0..=M]`.
    pub r_l: [Word16; MP1],
    /// `norm_l(r[0])` less four per overflow rescale — the reference returns
    /// this and `lpc()` discards it. Kept because the VAD's own analysis wants
    /// it and because it is the only visible evidence a rescale happened.
    pub norm: i16,
}

/// Windowed autocorrelation of one analysis window, TS 26.073 `Autocorr`.
///
/// `x` is Q0 speech, `window` is Q15. The result is normalised so that `r[0]`
/// uses the full 32-bit range; every lag shares that one normalisation.
///
/// # Overflow is detected by saturation, not by a wider accumulator
///
/// `r[0]` is accumulated with saturating `L_mac` and compared for *equality*
/// with `0x7fffffff`. A port that accumulates in `i64` never sees that value,
/// never rescales, and silently produces a different `r[]` on loud frames.
/// Each rescale divides the windowed signal by four and adds **4** to the
/// exponent, and the loop can run more than twice.
///
/// The `+1` bias applies to `r[0]` only; the lag terms are neither biased nor
/// re-checked for overflow, and their `L_shl` by the shared `norm` saturates.
#[must_use]
pub fn autocorrelate(
    ctx: &mut DspContext,
    x: &[Word16; L_WINDOW],
    window: &[i16; L_WINDOW],
) -> Autocorrelation {
    let mut y = [Word16(0); L_WINDOW];
    for (slot, (&sample, &w)) in y.iter_mut().zip(x.iter().zip(window.iter())) {
        // `mult_r`, the rounding variant. Plain `mult` here biases the whole
        // window low by up to one LSB per sample.
        *slot = mult_r(ctx, sample, Word16(w));
    }

    let mut overfl_shft = Word16(0);
    let mut sum;
    loop {
        sum = Word32(0);
        for &v in &y {
            sum = l_mac(ctx, sum, v, v);
        }
        if l_sub(ctx, sum, Word32(MAX_32)).0 != 0 {
            break;
        }
        overfl_shft = add(ctx, overfl_shft, Word16(4));
        for v in &mut y {
            *v = shr(ctx, *v, 2);
        }
    }

    // "Avoid the case of all zeros": without it a silent window normalises to
    // nothing and Levinson divides by zero.
    sum = l_add(ctx, sum, Word32(1));

    let norm = norm_l(sum);
    sum = l_shl(ctx, sum, norm);
    let mut r_h = [Word16(0); MP1];
    let mut r_l = [Word16(0); MP1];
    (r_h[0], r_l[0]) = l_extract(sum);

    for lag in 1..=M {
        let mut sum = Word32(0);
        for j in 0..L_WINDOW - lag {
            sum = l_mac(ctx, sum, y[j], y[j + lag]);
        }
        sum = l_shl(ctx, sum, norm);
        (r_h[lag], r_l[lag]) = l_extract(sum);
    }

    Autocorrelation {
        r_h,
        r_l,
        norm: sub(ctx, Word16(norm), overfl_shft).0,
    }
}

/// Apply the 60 Hz lag window, TS 26.073 `Lag_window`, in place.
///
/// Multiplies `r[i]` by `lag[i - 1]` for `i = 1..=M` in double precision.
/// `r[0]` is deliberately untouched — the off-by-one in the table index is the
/// reference's, and shifting it by one would expand the wrong bandwidth.
///
/// Needs no [`DspContext`]: `Mpy_32` and `L_Extract` are the only operators
/// involved and neither can set the overflow flag.
pub fn lag_window(r: &mut Autocorrelation) {
    for i in 1..=M {
        let x = mpy_32(
            r.r_h[i],
            r.r_l[i],
            Word16(LAG_WINDOW_H[i - 1]),
            Word16(LAG_WINDOW_L[i - 1]),
        );
        (r.r_h[i], r.r_l[i]) = l_extract(x);
    }
}

// -------------------------------------------------------------- levinson ---

/// Levinson-Durbin with the reference's stability fallback, TS 26.073
/// `Levinson`.
///
/// Carries one frame of state: the last *stable* filter it produced. An
/// unstable recursion re-emits that filter and, critically, does **not**
/// update it — so a run of unstable frames keeps re-emitting the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levinson {
    old_a: [Word16; MP1],
}

impl Default for Levinson {
    fn default() -> Self {
        Self::new()
    }
}

impl Levinson {
    /// The state `Levinson_reset` leaves behind: `A(z) = 1` in Q12.
    ///
    /// `old_a[0]` is written here and never again — the update loop at the end
    /// of [`solve`](Self::solve) starts at index 1 — so it stays 4096 for the
    /// life of the encoder.
    #[must_use]
    pub const fn new() -> Self {
        let mut old_a = [Word16(0); MP1];
        old_a[0] = FLAT_FILTER;
        Self { old_a }
    }

    /// Solve for the order-`M` predictor, Q12, and the first four reflection
    /// coefficients, Q15.
    ///
    /// `r` must already be lag-windowed. On an unstable recursion the returned
    /// filter is the previous stable one and all four reflection coefficients
    /// are zero — which is how a caller tells the two apart, since the
    /// reference discards the `int` return value everywhere.
    pub fn solve(
        &mut self,
        ctx: &mut DspContext,
        r: &Autocorrelation,
    ) -> ([Word16; MP1], [Word16; 4]) {
        let (rh, rl) = (&r.r_h, &r.r_l);
        let mut rc = [Word16(0); 4];
        // `Ah`/`Al` and `Anh`/`Anl` must stay separate arrays. The update loop
        // reads `Ah[i - j]` *and* `Ah[j]` while writing `An*[j]`; merged, the
        // second half of the loop would read slots the first half overwrote.
        let mut ah = [Word16(0); MP1];
        let mut al = [Word16(0); MP1];
        let mut anh = [Word16(0); MP1];
        let mut anl = [Word16(0); MP1];

        // Order 1: K = -r[1] / r[0].
        let t1 = l_comp(rh[1], rl[1]);
        let t2 = l_abs(ctx, t1);
        let mut t0 = div_32(t2, rh[0], rl[0]);
        // Plain C `>` on the composed 32-bit value: `t1 == 0` leaves `t0`
        // positive rather than negating it.
        if t1.0 > 0 {
            t0 = l_negate(ctx, t0);
        }
        let (mut kh, mut kl) = l_extract(t0);
        rc[0] = round(ctx, t0);
        t0 = l_shr(ctx, t0, 4);
        (ah[1], al[1]) = l_extract(t0);

        // Alpha = r[0] * (1 - K²). The `L_abs` is the reference's "some case
        // <0 !!" guard: `Mpy_32` can return a small negative for K near ±1,
        // and without it Alpha comes out slightly too large.
        let mut t0 = mpy_32(kh, kl, kh, kl);
        t0 = l_abs(ctx, t0);
        t0 = l_sub(ctx, Word32(MAX_32), t0);
        let (hi, lo) = l_extract(t0);
        t0 = mpy_32(rh[0], rl[0], hi, lo);
        let mut alp_exp = norm_l(t0);
        t0 = l_shl(ctx, t0, alp_exp);
        let (mut alp_h, mut alp_l) = l_extract(t0);

        for i in 2..=M {
            let mut t0 = Word32(0);
            for j in 1..i {
                t0 = l_add(ctx, t0, mpy_32(rh[j], rl[j], ah[i - j], al[i - j]));
            }
            // The shift is applied to the *sum*, not term by term, and it
            // saturates.
            t0 = l_shl(ctx, t0, 4);
            t0 = l_add(ctx, t0, l_comp(rh[i], rl[i]));

            let t1 = l_abs(ctx, t0);
            let mut t2 = div_32(t1, alp_h, alp_l);
            if t0.0 > 0 {
                t2 = l_negate(ctx, t2);
            }
            t2 = l_shl(ctx, t2, alp_exp);
            (kh, kl) = l_extract(t2);

            // Only orders 1..4 are captured; `rc` is four long.
            if i < 5 {
                rc[i - 1] = round(ctx, t2);
            }

            let magnitude = abs_s(ctx, kh);
            if sub(ctx, magnitude, K_UNSTABLE).0 > 0 {
                // Unstable. Re-emit the previous filter and return before
                // `old_a` is touched.
                return (self.old_a, [Word16(0); 4]);
            }

            for j in 1..i {
                let mut t = mpy_32(kh, kl, ah[i - j], al[i - j]);
                t = l_add(ctx, t, l_comp(ah[j], al[j]));
                (anh[j], anl[j]) = l_extract(t);
            }
            t2 = l_shr(ctx, t2, 4);
            (anh[i], anl[i]) = l_extract(t2);

            let mut t = mpy_32(kh, kl, kh, kl);
            t = l_abs(ctx, t);
            t = l_sub(ctx, Word32(MAX_32), t);
            let (hi, lo) = l_extract(t);
            t = mpy_32(alp_h, alp_l, hi, lo);
            let shift = norm_l(t);
            t = l_shl(ctx, t, shift);
            (alp_h, alp_l) = l_extract(t);
            alp_exp = add(ctx, Word16(alp_exp), Word16(shift)).0;

            ah[1..=i].copy_from_slice(&anh[1..=i]);
            al[1..=i].copy_from_slice(&anl[1..=i]);
        }

        let mut a = [Word16(0); MP1];
        a[0] = FLAT_FILTER;
        for i in 1..=M {
            // The `<< 1` saturates, and `old_a` receives the *same* saturated
            // value the caller does.
            let t0 = l_shl(ctx, l_comp(ah[i], al[i]), 1);
            a[i] = round(ctx, t0);
            self.old_a[i] = a[i];
        }
        (a, rc)
    }
}

// ----------------------------------------------------------- lpc() driver --

/// The frame-level LP analysis, TS 26.073 `lpc`.
///
/// Owns the single [`Levinson`] state that 12.2's *two* analyses share — an
/// unstable first analysis leaves `old_A` untouched and the second can then
/// inherit it, which is only reproducible if the two share one state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LpAnalysis {
    levinson: Levinson,
}

impl LpAnalysis {
    /// A fresh analysis state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            levinson: Levinson::new(),
        }
    }

    /// Fill the slots of `az` that come from a direct analysis, Q12.
    ///
    /// At 12.2 kbit/s that is slots **1 and 3**, from two windows over
    /// `old_speech[40..280]`. At every other rate it is slot **3** alone, from
    /// one window over `old_speech[80..320]`. The remaining slots are filled by
    /// the LSP interpolation in `lsp()`, which is deliberately not here: which
    /// slots it fills is the complement of this, and the two halves are easy to
    /// get consistently wrong together.
    ///
    /// Takes the *requested* mode. `lpc` is one of the stages that never sees
    /// `usedMode`, so a DTX frame is still analysed at its requested rate.
    pub fn analyse(
        &mut self,
        ctx: &mut DspContext,
        mode: u8,
        speech: &SpeechBuffer,
        az: &mut [Word16; AZ_SIZE],
    ) {
        if mode == MR122 {
            let window = speech.analysis_window_12k2();
            let mid = self.one_analysis(ctx, window, &LP_WINDOW_160_80);
            az[MP1..2 * MP1].copy_from_slice(&mid);
            let end = self.one_analysis(ctx, window, &LP_WINDOW_232_8);
            az[3 * MP1..].copy_from_slice(&end);
        } else {
            let window = speech.analysis_window();
            let end = self.one_analysis(ctx, window, &LP_WINDOW_200_40);
            az[3 * MP1..].copy_from_slice(&end);
        }
    }

    /// Autocorrelate, lag-window, and solve — the three steps every branch of
    /// `lpc` runs, in that order.
    fn one_analysis(
        &mut self,
        ctx: &mut DspContext,
        x: &[Word16; L_WINDOW],
        window: &[i16; L_WINDOW],
    ) -> [Word16; MP1] {
        let mut r = autocorrelate(ctx, x, window);
        lag_window(&mut r);
        let (a, _rc) = self.levinson.solve(ctx, &r);
        a
    }
}

// -------------------------------------------------------------- A(z) → LSP --

/// One step of the Chebyshev series evaluation, TS 26.073 `Chebps`.
///
/// Returns `C(x)` scaled by `2^6` and truncated to 16 bits. The final
/// `L_shl(t0, 6)` **saturates**, which is load-bearing: near a root where the
/// unscaled series is large, the saturated value keeps its sign and loses its
/// magnitude, and the linear interpolation below then uses the clamped number.
/// Evaluating this in wider precision and scaling afterwards places the root at
/// a different integer.
fn chebps(ctx: &mut DspContext, x: Word16, f: &[Word16; NC + 1]) -> Word16 {
    // b2 = 1.0 in the reference's double-precision format.
    let mut b2_h = Word16(256);
    let mut b2_l = Word16(0);

    let mut t0 = l_mult(ctx, x, Word16(512));
    t0 = l_mac(ctx, t0, f[1], Word16(8192));
    let (mut b1_h, mut b1_l) = l_extract(t0);

    for &coefficient in &f[2..NC] {
        let mut t = mpy_32_16(b1_h, b1_l, x);
        t = l_shl(ctx, t, 1);
        // `-b2` in double precision: `L_mac(t, b2_h, -32768)` is
        // `t + 2·b2_h·(-32768)`, which *saturates* when `b2_h == -32768`, and
        // `L_msu(t, b2_l, 1)` takes off the remaining `2·b2_l`.
        t = l_mac(ctx, t, b2_h, Word16(-32768));
        t = l_msu(ctx, t, b2_l, Word16(1));
        t = l_mac(ctx, t, coefficient, Word16(8192));
        let (b0_h, b0_l) = l_extract(t);
        b2_l = b1_l;
        b2_h = b1_h;
        b1_l = b0_l;
        b1_h = b0_h;
    }

    let mut t = mpy_32_16(b1_h, b1_l, x);
    t = l_mac(ctx, t, b2_h, Word16(-32768));
    t = l_msu(ctx, t, b2_l, Word16(1));
    // The reference reaches this line with its loop variable left at `n`, so
    // the coefficient used here is `f[NC]` — the same index the loop stopped
    // *before* — and its weight is 4096 rather than 8192, which is the
    // `f(n)/2` term of the series.
    t = l_mac(ctx, t, f[NC], Word16(4096));

    t = l_shl(ctx, t, 6);
    extract_h(t)
}

/// The sign test the grid scan and the bisection both apply.
///
/// `L_mult(a, b) <= 0`, exactly as written. Two things a port gets wrong here:
///
/// * the product is formed with the **saturating** `L_mult`, not a wider
///   multiply — `L_mult(-32768, -32768)` clamps to `0x7fffffff` rather than
///   overflowing to a negative value, so a wrapping port would see a sign
///   change where the reference sees none;
/// * the comparison is `<=`, so **an exact zero at either endpoint counts**. In
///   the grid scan that means a series value landing precisely on a grid point
///   is a root; in the bisection it means a tie takes the `xhigh = xmid` arm,
///   collapsing the interval toward the low end and keeping `xlow`.
///
/// Testing `a * b < 0`, or comparing signs, gets both of those wrong at once.
fn brackets_a_root(ctx: &mut DspContext, a: Word16, b: Word16) -> bool {
    l_mult(ctx, a, b).0 <= 0
}

/// Predictor coefficients to line spectral pairs, TS 26.073 `Az_lsp`.
///
/// `a` is Q12; the result is ten Q15 cosines in **descending** order. `old_lsp`
/// is the fallback used when the search finds fewer than ten roots.
///
/// # The search
///
/// The sum and difference polynomials `F1`/`F2` are evaluated as Chebyshev
/// series over a fixed 61-point cosine grid descending from +32760 to −32760,
/// and the search alternates between them: `F1` supplies the odd-numbered
/// roots, `F2` the even, which is the interlacing property that makes the
/// alternation legitimate.
///
/// - **Visit order** is strictly monotone over the grid, `j = 1..=60`. The scan
///   is *not* restarted when a root is found; `j` carries on.
/// - **Sign change** is `L_mult(ylow, yhigh) <= 0`. The `<=` matters: a series
///   value that lands exactly on zero at a grid point counts as a crossing.
///   Testing `< 0`, or comparing signs, drops those roots and falls through to
///   the fallback below.
/// - **Refinement** is exactly four bisections, never convergence-based, and
///   the midpoint is `add(shr(xlow, 1), shr(xhigh, 1))` — each half floored
///   *before* the add, so it is not `(xlow + xhigh) / 2` for odd sums.
/// - **Ties** in the bisection (`ylow · ymid == 0`) take the `xhigh = xmid`
///   arm, collapsing the interval toward the low end and keeping `xlow`.
/// - **After a root** the scan resumes from the interpolated root rather than
///   from the grid point: `xlow = xint`, so the next interval is
///   `[grid[j + 1], xint]`, and `yhigh` for it is the *new* polynomial's value
///   at `xint`.
/// - **Fallback**: fewer than ten roots replaces the **whole** output with
///   `old_lsp`. The roots already found are discarded, not kept.
///
/// [`super::super::lsp::lsp_to_lp`] is the inverse and is bit-exact, which
/// makes a round trip a cheap sanity check — but only a sanity check: it cannot
/// tell a correctly placed root from one the interpolation put one LSB away.
#[must_use]
pub fn az_lsp(ctx: &mut DspContext, a: &[Word16; MP1], old_lsp: &[Word16; M]) -> [Word16; M] {
    // F1(z) = F1(z)/(1+z⁻¹), F2(z) = F2(z)/(1−z⁻¹), both scaled by 1/4 so the
    // Chebyshev recursion cannot overflow. The `f[i]` read on each line was
    // written by the previous iteration: this is a recurrence, not a map.
    let mut f1 = [Word16(0); NC + 1];
    let mut f2 = [Word16(0); NC + 1];
    f1[0] = Word16(1024);
    f2[0] = Word16(1024);
    for i in 0..NC {
        let mut t0 = l_mult(ctx, a[i + 1], Word16(8192));
        t0 = l_mac(ctx, t0, a[M - i], Word16(8192));
        f1[i + 1] = sub(ctx, extract_h(t0), f1[i]);

        let mut t0 = l_mult(ctx, a[i + 1], Word16(8192));
        t0 = l_msu(ctx, t0, a[M - i], Word16(8192));
        f2[i + 1] = add(ctx, extract_h(t0), f2[i]);
    }

    let mut lsp = [Word16(0); M];
    let mut found = 0usize;
    let mut on_f2 = false;
    let mut xlow = Word16(LSP_GRID[0]);
    let mut ylow = chebps(ctx, xlow, &f1);
    let mut j = 0usize;

    while found < M && j < GRID_POINTS {
        j += 1;
        let mut xhigh = xlow;
        let mut yhigh = ylow;
        xlow = Word16(LSP_GRID[j]);
        let coef = if on_f2 { &f2 } else { &f1 };
        ylow = chebps(ctx, xlow, coef);

        if !brackets_a_root(ctx, ylow, yhigh) {
            continue;
        }

        for _ in 0..4 {
            // Each half is floored *before* the add: not `(xlow + xhigh) / 2`
            // when the sum is odd.
            let half_low = shr(ctx, xlow, 1);
            let half_high = shr(ctx, xhigh, 1);
            let xmid = add(ctx, half_low, half_high);
            let ymid = chebps(ctx, xmid, coef);
            if brackets_a_root(ctx, ylow, ymid) {
                yhigh = ymid;
                xhigh = xmid;
            } else {
                ylow = ymid;
                xlow = xmid;
            }
        }

        lsp[found] = interpolate_root(ctx, xlow, xhigh, ylow, yhigh);
        xlow = lsp[found];
        found += 1;

        on_f2 = !on_f2;
        let coef = if on_f2 { &f2 } else { &f1 };
        ylow = chebps(ctx, xlow, coef);
    }

    if found < M {
        lsp.copy_from_slice(old_lsp);
    }
    lsp
}

/// `xint = xlow − ylow·(xhigh − xlow)/(yhigh − ylow)`, in the reference's
/// arithmetic.
///
/// The reciprocal's numerator is **16383**, not 16384 and not 32767, and both
/// `extract_l` calls truncate to 16 bits *without* saturating — a plain cast of
/// the low half. The reference relies on neither intermediate exceeding 16
/// bits; reproducing the truncation rather than clamping is what keeps a port
/// on the same integer when one of them does.
fn interpolate_root(
    ctx: &mut DspContext,
    xlow: Word16,
    xhigh: Word16,
    ylow: Word16,
    yhigh: Word16,
) -> Word16 {
    let x = sub(ctx, xhigh, xlow);
    let y = sub(ctx, yhigh, ylow);
    if y.0 == 0 {
        return xlow;
    }

    let sign = y;
    let magnitude = abs_s(ctx, y);
    let exp = norm_s(magnitude);
    let normalised = shl(ctx, magnitude, exp);
    let reciprocal = div_s(Word16(16383), normalised);

    let mut t0 = l_mult(ctx, x, reciprocal);
    let shift = sub(ctx, Word16(20), Word16(exp)).0;
    t0 = l_shr(ctx, t0, shift);
    let mut slope = extract_l(t0);
    if sign.0 < 0 {
        slope = negate(ctx, slope);
    }

    let scaled = l_mult(ctx, ylow, slope);
    let t0 = l_shr(ctx, scaled, 11);
    let correction = extract_l(t0);
    sub(ctx, xlow, correction)
}

// -------------------------------------------------------- weighted speech --

/// Which weighting numerator `pre_big` uses.
///
/// `sub(mode, MR795) <= 0`, so 7.95 itself takes `gamma1`. **This is not the
/// same test [`subframe_targets`] applies.** That one asks
/// `mode == MR122 || mode == MR102`. The two agree on every speech rate and
/// disagree on [`MRDTX`], which reaches `pre_big` but never the subframe loop.
/// Collapsing them into one predicate is a change of behaviour, not a
/// simplification.
const fn pre_big_gamma1(mode: u8) -> &'static [i16; M] {
    if mode <= MR795 {
        &GAMMA1
    } else {
        &GAMMA1_12K2
    }
}

/// The perceptually weighted speech the open-loop pitch search runs on,
/// TS 26.073 `pre_big`.
///
/// Called twice per frame, on half-frames of 80 samples, and carries the only
/// state in the file: `mem_w`, the weighting synthesis filter's memory, which
/// is updated **four times per frame** — once per subframe, across both calls —
/// and is read by nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WeightedSpeech {
    mem_w: [Word16; M],
}

impl WeightedSpeech {
    /// A filter memory in the state `cod_amr_reset` leaves behind: silent.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mem_w: [Word16(0); M],
        }
    }

    /// Weight one half-frame in place, writing `wsp[offset..offset + 80]`.
    ///
    /// `offset` is 0 or 80 in the coded frame's own indexing. The unquantized
    /// `az` slot each subframe uses is derived from it: half-frame 1 starts at
    /// slot 0, half-frame 2 at slot **2** — the test is `frameOffset > 0`, so
    /// this is a two-way choice and not `offset / L_SUBFR`.
    ///
    /// # Panics
    ///
    /// If `offset` is not a subframe boundary with two subframes left in the
    /// frame.
    pub fn half_frame(
        &mut self,
        ctx: &mut DspContext,
        mode: u8,
        az: &[Word16; AZ_SIZE],
        offset: usize,
        speech: &SpeechBuffer,
        wsp: &mut [Word16; L_FRAME],
    ) {
        assert!(
            offset.is_multiple_of(L_SUBFR) && offset + 2 * L_SUBFR <= L_FRAME,
            "pre_big works on two whole subframes inside the frame"
        );
        let g1 = pre_big_gamma1(mode);
        let mut slot = usize::from(offset > 0) * 2;
        let mut at = offset;

        for _ in 0..2 {
            let a = &az[slot * MP1..(slot + 1) * MP1];
            let ap1 = expand_bandwidth(ctx, a, g1);
            let ap2 = expand_bandwidth(ctx, a, &GAMMA2);

            lp_residual(
                ctx,
                &ap1,
                speech.with_history(at, L_SUBFR),
                &mut wsp[at..at + L_SUBFR],
            );
            // Deliberately in-place, and the one `Syn_filt` in the front end
            // that updates its memory.
            self.mem_w =
                synthesis_filter_in_place(ctx, &ap2, &mut wsp[at..at + L_SUBFR], &self.mem_w);

            slot += 1;
            at += L_SUBFR;
        }
    }
}

// -------------------------------------------------- per-subframe targets ---

/// What `subframePreProc` produces for one subframe.
///
/// `exc` is not among them: the reference seeds `exc[i_subfr..]` with a copy of
/// [`res`](Self::res) and the closed-loop pitch search immediately overwrites
/// it, so the copy belongs to the caller that owns the excitation buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubframeTargets {
    /// The pitch search's target signal, Q0.
    pub xn: [Word16; L_SUBFR],
    /// Impulse response of `Ap1(z) / (Aq(z)·Ap2(z))`, Q12.
    pub h1: [Word16; L_SUBFR],
    /// The LP residual, Q0. The reference keeps this pristine for the gain
    /// quantiser and hands the pitch search a *copy* it is free to destroy.
    pub res: [Word16; L_SUBFR],
}

/// Which weighting numerator `subframePreProc` uses — see [`pre_big_gamma1`],
/// whose test is deliberately different.
const fn subframe_gamma1(mode: u8) -> &'static [i16; M] {
    if mode == MR122 || mode == MR102 {
        &GAMMA1_12K2
    } else {
        &GAMMA1
    }
}

/// Per-subframe pre-processing, TS 26.073 `subframePreProc`.
///
/// `a` is this subframe's **unquantized** `A(z)` and `aq` its **quantized**
/// one, both Q12; mixing them up produces a target that is subtly wrong
/// everywhere. `speech` is `speech[i_subfr - M .. i_subfr + L_SUBFR]` — the
/// residual filter reads ten samples back, which at subframe 0 reaches into the
/// previous frame. `mem_err` and `mem_w0` are read-only here: both `Syn_filt`
/// calls that use them discard their memory update.
///
/// Takes the *used* mode, not the requested one.
///
/// # The two coupled memories
///
/// `error[]` is the reference's `mem_err + M`, so the `Residu` that produces
/// `xn` reads backwards out of the real synthesis-error memory. That negative
/// reach is the only thing connecting this subframe's target to the previous
/// subframe's coding error, and dropping it gives an encoder that tracks
/// nothing and still sounds fine.
///
/// # Panics
///
/// If `speech` is not exactly `M + L_SUBFR` samples, or if either filter is
/// shorter than `M + 1`.
#[must_use]
pub fn subframe_targets(
    ctx: &mut DspContext,
    mode: u8,
    a: &[Word16],
    aq: &[Word16],
    speech: &[Word16],
    mem_err: &[Word16; M],
    mem_w0: &[Word16; M],
) -> SubframeTargets {
    assert_eq!(
        speech.len(),
        M + L_SUBFR,
        "subframePreProc needs M samples of speech history"
    );

    let ap1 = expand_bandwidth(ctx, a, subframe_gamma1(mode));
    let ap2 = expand_bandwidth(ctx, a, &GAMMA2);

    // `ai_zero` is `Ap1` followed by 29 zeros: the reference builds it by
    // writing MP1 words over the head of a buffer whose tail is the permanently
    // zero `zero[]` region. That region doubles as the filter memory for both
    // calls below, which is why both pass a zero memory and neither updates it.
    let mut ai_zero = [Word16(0); L_SUBFR];
    ai_zero[..MP1].copy_from_slice(&ap1);
    let silence = [Word16(0); M];
    let mut h1 = [Word16(0); L_SUBFR];
    let _ = synthesis_filter(ctx, aq, &ai_zero, &mut h1, &silence);
    let _ = synthesis_filter_in_place(ctx, &ap2, &mut h1, &silence);

    let mut res = [Word16(0); L_SUBFR];
    lp_residual(ctx, aq, speech, &mut res);

    // `error[0..M]` is the real error memory and `error[M..]` the freshly
    // filtered subframe; the `Residu` below indexes the whole thing.
    let mut error = [Word16(0); M + L_SUBFR];
    error[..M].copy_from_slice(mem_err);
    let mut filtered = [Word16(0); L_SUBFR];
    let _ = synthesis_filter(ctx, aq, &res, &mut filtered, mem_err);
    error[M..].copy_from_slice(&filtered);

    let mut xn = [Word16(0); L_SUBFR];
    lp_residual(ctx, &ap1, &error, &mut xn);
    let _ = synthesis_filter_in_place(ctx, &ap2, &mut xn, mem_w0);

    SubframeTargets { xn, h1, res }
}

#[cfg(test)]
mod tests {
    use super::super::preproc::trace_support::{frames, input_frame, scalar, words};
    use super::super::preproc::Preprocessor;
    use super::*;
    use crate::codecs::amr::nb::lsp::{interpolate_lsp, lsp_to_lp, LsfDecoder};
    use crate::codecs::amr::nb::{bitstream, L_FRAME as FRAME};

    /// The rate the committed trace was produced at: 7.40 kbit/s.
    const TRACE_MODE: u8 = 4;

    /// Four subframes to a frame.
    const NB_SUBFR: usize = 4;

    /// Quantised `A(z)` for one frame, recovered from the encoder's own output.
    ///
    /// The trace does not carry `Aq_t`, and `subframePreProc` cannot be
    /// evaluated without it. The bitstream the reference encoder produced is
    /// committed next to the trace, and the LSF *dequantiser* next door is
    /// bit-exact, so replaying the committed frames through it reconstructs
    /// exactly the quantised spectrum the encoder used — that equality is the
    /// definition of an analysis-by-synthesis coder, not an approximation of
    /// it. If it were wrong, every `xn` and `res` comparison below would fail.
    struct QuantisedSpectrum {
        lsf: LsfDecoder,
        lsp_old: [Word16; M],
        payloads: Vec<Vec<u8>>,
    }

    impl QuantisedSpectrum {
        fn new() -> Self {
            const AMR: &[u8] = include_bytes!("../../testdata/amrnb_enc_mode4.amr");
            // Storage format: the magic header, then one octet-aligned frame
            // each with a leading ToC byte.
            let magic = b"#!AMR\n";
            assert_eq!(&AMR[..magic.len()], magic, "unexpected .amr magic");
            let mut payloads = Vec::new();
            let mut at = magic.len();
            while at < AMR.len() {
                let toc = AMR[at];
                let frame_type = (toc >> 3) & 0x0f;
                assert_eq!(
                    frame_type, TRACE_MODE,
                    "the committed .amr is not at the trace's rate"
                );
                // 7.40 kbit/s: 148 bits, 19 payload octets after the ToC.
                let len = 19usize;
                // `parse` wants the speech bits alone; the ToC byte is not
                // part of them.
                payloads.push(AMR[at + 1..at + 1 + len].to_vec());
                at += 1 + len;
            }
            Self {
                // `Q_plsf_reset` and `D_plsf_reset` leave the predictor in the
                // same state, which is what makes this replay legitimate.
                lsf: LsfDecoder::at_reset(),
                lsp_old: crate::codecs::amr::nb::lsp::initial_lsp(),
                payloads,
            }
        }

        /// `Aq_t` for one frame, Q12, four `MP1` blocks.
        fn next(&mut self, frame: usize) -> [Word16; AZ_SIZE] {
            let params = bitstream::parse(TRACE_MODE, &self.payloads[frame]).expect("frame parses");
            let lsp_new = self.lsf.decode(TRACE_MODE, &params[..3], false);
            let mut ctx = DspContext::default();
            let az = interpolate_lsp(&mut ctx, &self.lsp_old, &lsp_new);
            self.lsp_old = lsp_new;
            az
        }
    }

    /// Reproduce `subframePostProc`'s two memory updates from trace rows.
    ///
    /// Not this module's code — it is reproduced here because `xn` and `res`
    /// for subframes 1 to 3 are functions of the memories the *previous*
    /// subframe's post-processing left behind, and the trace records that
    /// subframe's gains and filtered vectors rather than the memories
    /// themselves. Every input is a committed trace row or the `Aq` above.
    // The three names share a prefix because the reference's do; renaming
    // them would make the transcription harder to check.
    #[allow(clippy::struct_field_names)]
    struct PostProc {
        mem_syn: [Word16; M],
        mem_err: [Word16; M],
        mem_w0: [Word16; M],
    }

    impl PostProc {
        const fn new() -> Self {
            Self {
                mem_syn: [Word16(0); M],
                mem_err: [Word16(0); M],
                mem_w0: [Word16(0); M],
            }
        }

        fn advance(
            &mut self,
            ctx: &mut DspContext,
            aq: &[Word16],
            speech: &[Word16],
            frame: usize,
            subfr: i32,
        ) {
            let adapt = words(frame, subfr, "adapt", L_SUBFR);
            let code = words(frame, subfr, "code", L_SUBFR);
            let y1 = words(frame, subfr, "y1", L_SUBFR);
            let y2 = words(frame, subfr, "y2", L_SUBFR);
            let xn = words(frame, subfr, "xn", L_SUBFR);
            let gain_pit = Word16(scalar(frame, subfr, "gain_pit") as i16);
            let gain_code = Word16(scalar(frame, subfr, "gain_code") as i16);
            // 7.40 is not 12.2, so the shifts are the common ones.
            let (temp_shift, k_shift, pitch_fac) = (1, 2, gain_pit);

            let mut exc = [Word16(0); L_SUBFR];
            for i in 0..L_SUBFR {
                let mut acc = l_mult(ctx, adapt[i], pitch_fac);
                acc = l_mac(ctx, acc, code[i], gain_code);
                acc = l_shl(ctx, acc, temp_shift);
                exc[i] = round(ctx, acc);
            }

            let mut synth = [Word16(0); L_SUBFR];
            self.mem_syn = synthesis_filter(ctx, aq, &exc, &mut synth, &self.mem_syn);

            for (j, i) in (L_SUBFR - M..L_SUBFR).enumerate() {
                self.mem_err[j] = sub(ctx, speech[i], synth[i]);
                let scaled = l_mult(ctx, y1[i], gain_pit);
                let temp = extract_h(l_shl(ctx, scaled, 1));
                let scaled = l_mult(ctx, y2[i], gain_code);
                let k = extract_h(l_shl(ctx, scaled, k_shift));
                let together = add(ctx, temp, k);
                self.mem_w0[j] = sub(ctx, xn[i], together);
            }
        }
    }

    /// Everything the front end produces for one frame.
    struct FrameOutput {
        az: [Word16; AZ_SIZE],
        lsp_new: [Word16; M],
        targets: Vec<SubframeTargets>,
    }

    /// Replay the whole front end across every committed frame, keeping all
    /// state, and hand each frame's output to `check`.
    ///
    /// Returns the number of frames replayed.
    fn replay(mut check: impl FnMut(usize, &FrameOutput)) -> usize {
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let mut buffer = SpeechBuffer::new();
        let mut analysis = LpAnalysis::new();
        let mut lsp_old = crate::codecs::amr::nb::lsp::initial_lsp();
        let mut quantised = QuantisedSpectrum::new();
        let mut post = PostProc::new();

        let total = frames();
        for frame in 0..total {
            let mut samples = input_frame(frame);
            pre.condition(&mut ctx, &mut samples);
            buffer.push(&samples);

            let mut az = [Word16(0); AZ_SIZE];
            analysis.analyse(&mut ctx, TRACE_MODE, &buffer, &mut az);

            let a_end: [Word16; MP1] = az[3 * MP1..].try_into().expect("slot 3");
            let lsp_new = az_lsp(&mut ctx, &a_end, &lsp_old);

            // `lsp()`'s unquantized interpolation, which fills slots 0..2 for
            // every rate but 12.2 and is another module's job; taken from the
            // decoder's interpolator, which is the identical computation.
            let interpolated = interpolate_lsp(&mut ctx, &lsp_old, &lsp_new);
            az[..3 * MP1].copy_from_slice(&interpolated[..3 * MP1]);
            lsp_old = lsp_new;

            let aq = quantised.next(frame);
            let mut targets = Vec::new();
            for subfr in 0..NB_SUBFR {
                let at = subfr * L_SUBFR;
                let t = subframe_targets(
                    &mut ctx,
                    TRACE_MODE,
                    &az[subfr * MP1..(subfr + 1) * MP1],
                    &aq[subfr * MP1..(subfr + 1) * MP1],
                    buffer.with_history(at, L_SUBFR),
                    &post.mem_err,
                    &post.mem_w0,
                );
                targets.push(t);
                post.advance(
                    &mut ctx,
                    &aq[subfr * MP1..(subfr + 1) * MP1],
                    &buffer.coded()[at..at + L_SUBFR],
                    frame,
                    subfr as i32,
                );
            }

            check(
                frame,
                &FrameOutput {
                    az,
                    lsp_new,
                    targets,
                },
            );
            buffer.shift();
        }
        total
    }

    #[test]
    fn interpolated_lp_coefficients_are_bit_exact_against_ts26073() {
        // `A_t` is traced after `lsp()`, so it carries the analysis result in
        // slot 3 and the interpolation in slots 0..2. Comparing all 44 words
        // checks the Levinson output *and* that the right slot received it.
        let mut compared = 0usize;
        let count = replay(|frame, out| {
            let want = words(frame, -1, "A_t", AZ_SIZE);
            for (i, &got) in out.az.iter().enumerate() {
                assert_eq!(
                    got.0,
                    want[i].0,
                    "frame {frame}: A_t[{i}] (subframe {}, tap {}) differs",
                    i / MP1,
                    i % MP1
                );
                compared += 1;
            }
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * AZ_SIZE, "132 coefficients compared");
    }

    #[test]
    fn levinson_output_reaches_the_fourth_subframe_slot() {
        // The slot assignment is rate-dependent and silent when wrong: a port
        // that writes slot 0 produces a filter that is merely interpolated
        // differently. Pin it directly rather than only through `A_t`.
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let mut buffer = SpeechBuffer::new();
        let mut analysis = LpAnalysis::new();
        let mut samples = input_frame(0);
        pre.condition(&mut ctx, &mut samples);
        buffer.push(&samples);

        let mut az = [Word16(0); AZ_SIZE];
        analysis.analyse(&mut ctx, TRACE_MODE, &buffer, &mut az);
        let want = words(0, -1, "A_t", AZ_SIZE);
        for i in 0..MP1 {
            assert_eq!(az[3 * MP1 + i].0, want[3 * MP1 + i].0, "slot 3 tap {i}");
        }
        assert!(
            az[..3 * MP1].iter().all(|w| w.0 == 0),
            "a single-analysis rate must leave slots 0..2 for the interpolation"
        );
    }

    #[test]
    fn lsps_are_bit_exact_against_ts26073() {
        let mut compared = 0usize;
        let count = replay(|frame, out| {
            // The trace records `lsp_new` once per subframe; the value is the
            // frame's, so every subframe row must agree with it.
            for subfr in 0..NB_SUBFR {
                let want = words(frame, subfr as i32, "lsp_new", M);
                for (i, &got) in out.lsp_new.iter().enumerate() {
                    assert_eq!(got.0, want[i].0, "frame {frame}: lsp_new[{i}] differs");
                    compared += 1;
                }
            }
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(
            compared,
            3 * NB_SUBFR * M,
            "120 line spectral pairs compared"
        );
    }

    #[test]
    fn subframe_targets_are_bit_exact_against_ts26073() {
        let mut compared = 0usize;
        let count = replay(|frame, out| {
            for (subfr, t) in out.targets.iter().enumerate() {
                let s = subfr as i32;
                for (name, got) in [("xn", &t.xn), ("h1", &t.h1), ("res", &t.res)] {
                    let want = words(frame, s, name, L_SUBFR);
                    for (i, &value) in got.iter().enumerate() {
                        assert_eq!(
                            value.0, want[i].0,
                            "frame {frame} subframe {subfr}: {name}[{i}] differs"
                        );
                        compared += 1;
                    }
                }
            }
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(
            compared,
            3 * NB_SUBFR * 3 * L_SUBFR,
            "1440 target samples compared"
        );
    }

    #[test]
    fn the_lookahead_is_never_primed() {
        // `Speech_Encode_Frame_First` would fill `old_speech[120..160]` with the
        // first 40 input samples. The reference driver never calls it, and the
        // published vectors assume it was not called — so frame 0's first
        // subframe must be built from silence.
        let mut buffer = SpeechBuffer::new();
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let mut samples = input_frame(0);
        pre.condition(&mut ctx, &mut samples);
        buffer.push(&samples);
        assert!(
            buffer.coded()[..L_NEXT].iter().all(|w| w.0 == 0),
            "the first 40 coded samples are the unprimed lookahead"
        );
        assert_eq!(
            buffer.coded()[L_NEXT].0,
            samples[0].0,
            "the coded frame lags the pushed frame by exactly one lookahead"
        );
    }

    #[test]
    fn windows_address_the_spans_the_reference_uses() {
        let mut buffer = SpeechBuffer::new();
        let mut marked = [Word16(0); FRAME];
        for (i, slot) in marked.iter_mut().enumerate() {
            *slot = Word16(i as i16 + 1);
        }
        buffer.push(&marked);
        // p_window = old_speech[80..320]: 80 zeros of history, then the frame.
        let w = buffer.analysis_window();
        assert!(w[..80].iter().all(|v| v.0 == 0));
        assert_eq!(w[80].0, 1, "the frame starts 80 samples into p_window");
        assert_eq!(w[L_WINDOW - 1].0, FRAME as i16);
        // p_window_12k2 = old_speech[40..280]: 40 more history, 40 fewer of the
        // frame.
        let w = buffer.analysis_window_12k2();
        assert!(w[..120].iter().all(|v| v.0 == 0));
        assert_eq!(w[120].0, 1, "the 12.2 window starts one lookahead earlier");
        assert_eq!(w[L_WINDOW - 1].0, (FRAME - L_NEXT) as i16);
        // `Residu`'s ten-sample reach at subframe 0 lands in the previous
        // frame, and so does the whole of subframe 0 on a fresh buffer: the
        // coded frame starts one lookahead behind what was just pushed.
        let s = buffer.with_history(0, L_SUBFR);
        assert_eq!(s.len(), M + L_SUBFR);
        assert!(
            s.iter().all(|v| v.0 == 0),
            "subframe 0 of a fresh buffer is entirely history"
        );
        // Subframe 1 straddles the boundary: ten samples of history that are
        // still the previous frame, then the pushed frame's first sample.
        let s = buffer.with_history(L_NEXT, L_SUBFR);
        assert!(
            s[..M].iter().all(|v| v.0 == 0),
            "the history is still silent"
        );
        assert_eq!(
            s[M].0, 1,
            "the pushed frame starts exactly one lookahead in"
        );
    }

    #[test]
    fn shifting_moves_the_frame_into_the_history() {
        let mut buffer = SpeechBuffer::new();
        let mut marked = [Word16(0); FRAME];
        for (i, slot) in marked.iter_mut().enumerate() {
            *slot = Word16(i as i16 + 1);
        }
        buffer.push(&marked);
        buffer.shift();
        // After the shift the frame occupies old_speech[0..160], so
        // p_window[0..80] is its second half.
        assert_eq!(buffer.analysis_window()[0].0, 81);
        assert_eq!(buffer.analysis_window()[79].0, FRAME as i16);
    }

    #[test]
    fn az_lsp_round_trips_through_the_decoders_inverse() {
        // A cheap consistency check alongside the fixture comparison, never in
        // place of it: `lsp_to_lp` is bit-exact and is `Az_lsp`'s inverse, so a
        // filter that survives the round trip within a coefficient or two is
        // evidence the roots are the right ten. It cannot detect a root the
        // interpolation placed one LSB away, which is why the trace comparison
        // above is the test that counts.
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let mut buffer = SpeechBuffer::new();
        let mut analysis = LpAnalysis::new();
        let mut lsp_old = crate::codecs::amr::nb::lsp::initial_lsp();
        let mut checked = 0usize;

        for frame in 0..frames() {
            let mut samples = input_frame(frame);
            pre.condition(&mut ctx, &mut samples);
            buffer.push(&samples);
            let mut az = [Word16(0); AZ_SIZE];
            analysis.analyse(&mut ctx, TRACE_MODE, &buffer, &mut az);
            let a: [Word16; MP1] = az[3 * MP1..].try_into().expect("slot 3");
            let lsp = az_lsp(&mut ctx, &a, &lsp_old);
            let back = lsp_to_lp(&mut ctx, &lsp);
            for i in 0..MP1 {
                let delta = i32::from(a[i].0) - i32::from(back[i].0);
                assert!(
                    delta.abs() <= 4,
                    "frame {frame}: round trip moved a[{i}] by {delta}"
                );
                checked += 1;
            }
            lsp_old = lsp;
            buffer.shift();
        }
        assert_eq!(checked, 3 * MP1, "33 coefficients round-tripped");
    }

    #[test]
    fn the_root_bracket_test_counts_an_exact_zero_as_a_crossing() {
        // The tie-break direction of the only search in this module. A root
        // that lands exactly on a grid point is a root; a bisection midpoint
        // that evaluates to exactly zero collapses the interval downward and
        // keeps `xlow`. Both follow from the single `<=`.
        let mut ctx = DspContext::default();
        assert!(
            brackets_a_root(&mut ctx, Word16(0), Word16(12345)),
            "a zero low endpoint must count as a crossing"
        );
        assert!(
            brackets_a_root(&mut ctx, Word16(12345), Word16(0)),
            "a zero high endpoint must count as a crossing"
        );
        assert!(brackets_a_root(&mut ctx, Word16(-1), Word16(1)));
        assert!(brackets_a_root(&mut ctx, Word16(1), Word16(-1)));
        assert!(!brackets_a_root(&mut ctx, Word16(1), Word16(1)));
        assert!(!brackets_a_root(&mut ctx, Word16(-1), Word16(-1)));
        // The saturating product: two most-negative values clamp to the largest
        // positive, i.e. still no crossing. A wrapping multiply gives the same
        // sign here, but `L_mult` is what the reference wrote and the clamp is
        // observable in the magnitude the caller never looks at.
        assert!(!brackets_a_root(&mut ctx, Word16(-32768), Word16(-32768)));
    }

    #[test]
    fn az_lsp_returns_ten_descending_roots() {
        // Interlacing: the ten roots alternate between the two polynomials and
        // must come out strictly descending in the cosine domain. A search that
        // failed to alternate would still return ten numbers.
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let mut buffer = SpeechBuffer::new();
        let mut analysis = LpAnalysis::new();
        let mut lsp_old = crate::codecs::amr::nb::lsp::initial_lsp();
        let mut checked = 0usize;
        for frame in 0..frames() {
            let mut samples = input_frame(frame);
            pre.condition(&mut ctx, &mut samples);
            buffer.push(&samples);
            let mut az = [Word16(0); AZ_SIZE];
            analysis.analyse(&mut ctx, TRACE_MODE, &buffer, &mut az);
            let a: [Word16; MP1] = az[3 * MP1..].try_into().expect("slot 3");
            let lsp = az_lsp(&mut ctx, &a, &lsp_old);
            for pair in lsp.windows(2) {
                assert!(pair[0].0 > pair[1].0, "frame {frame}: roots not descending");
                checked += 1;
            }
            lsp_old = lsp;
            buffer.shift();
        }
        assert_eq!(checked, 3 * (M - 1), "27 adjacent pairs checked");
    }

    #[test]
    fn az_lsp_falls_back_to_the_previous_set_when_roots_are_missing() {
        // `A(z) = 1 + 2z^-1` has its only root at z = -2, outside the unit
        // circle, so F1 and F2 have no interlacing roots on it and the scan
        // reaches grid point 60 with fewer than ten sign changes. The whole
        // output must then be `old_lsp` — not the roots that *were* found with
        // the remainder left over from the previous call, which is what a port
        // that fills `lsp[]` as it goes and forgets the check produces.
        let mut ctx = DspContext::default();
        let mut a = [Word16(0); MP1];
        a[0] = Word16(4096);
        a[1] = Word16(8192);
        // Deliberately not a plausible LSP set, so a partial fill would show.
        let old = [
            Word16(31000),
            Word16(30000),
            Word16(29000),
            Word16(28000),
            Word16(27000),
            Word16(26000),
            Word16(25000),
            Word16(24000),
            Word16(23000),
            Word16(22000),
        ];
        let lsp = az_lsp(&mut ctx, &a, &old);
        assert_eq!(lsp, old, "the fallback replaces the whole vector");

        // And the complement: a filter the reference's own Levinson produced
        // does find its ten roots, so the fallback is not simply always taken.
        let mut pre = Preprocessor::new();
        let mut buffer = SpeechBuffer::new();
        let mut analysis = LpAnalysis::new();
        let mut samples = input_frame(0);
        pre.condition(&mut ctx, &mut samples);
        buffer.push(&samples);
        let mut az = [Word16(0); AZ_SIZE];
        analysis.analyse(&mut ctx, TRACE_MODE, &buffer, &mut az);
        let good: [Word16; MP1] = az[3 * MP1..].try_into().expect("slot 3");
        assert_ne!(
            az_lsp(&mut ctx, &good, &old),
            old,
            "a well-behaved filter must not take the fallback"
        );
    }

    #[test]
    fn levinson_reemits_the_previous_filter_when_unstable() {
        // Solve one real frame first, so the stored filter is something other
        // than reset's flat one and re-emitting it is visible.
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let mut buffer = SpeechBuffer::new();
        let mut samples = input_frame(0);
        pre.condition(&mut ctx, &mut samples);
        buffer.push(&samples);
        let mut levinson = Levinson::new();
        let mut r = autocorrelate(&mut ctx, buffer.analysis_window(), &LP_WINDOW_200_40);
        lag_window(&mut r);
        let (stable, rc) = levinson.solve(&mut ctx, &r);
        assert!(
            stable[1..].iter().any(|w| w.0 != 0) && rc.iter().any(|w| w.0 != 0),
            "the stable solve produced nothing to re-emit"
        );

        // `r[1]` one LSB short of `r[0]` with every higher lag zero is not a
        // realisable autocorrelation: the order-1 predictor is already almost
        // exact, so Alpha collapses and the order-2 reflection coefficient
        // saturates well past 32750.
        let mut r_h = [Word16(0); MP1];
        let mut r_l = [Word16(0); MP1];
        r_h[0] = Word16(0x4000);
        r_h[1] = Word16(0x3fff);
        r_l[1] = Word16(0x7fff);
        let unstable = Autocorrelation { r_h, r_l, norm: 0 };
        let (a, rc) = levinson.solve(&mut ctx, &unstable);
        assert_eq!(a, stable, "the unstable path re-emits the previous filter");
        assert!(
            rc.iter().all(|w| w.0 == 0),
            "unstable zeroes all four reflection coefficients"
        );
        let (again, _) = levinson.solve(&mut ctx, &unstable);
        assert_eq!(
            again, stable,
            "an unstable frame must not update the stored filter"
        );
    }

    #[test]
    fn the_stability_threshold_is_strictly_greater_than_32750() {
        // `abs(Kh) == 32750` is stable. The test is on the DPF high word alone,
        // so this pins the comparison rather than the value of K.
        assert_eq!(K_UNSTABLE.0, 32750, "the threshold moved");
        let mut ctx = DspContext::default();
        assert!(
            {
                let m = abs_s(&mut ctx, K_UNSTABLE);
                sub(&mut ctx, m, K_UNSTABLE).0 <= 0
            },
            "equality must not trip the instability test"
        );
    }

    #[test]
    fn the_two_gamma1_tests_disagree_on_the_dtx_pseudo_mode() {
        // The one place the two weighting-factor selections differ. Neither
        // disagreement is reachable from the reference driver, which is exactly
        // why a port is tempted to merge them.
        // Compared by value: the two tables differ in every entry, which the
        // generator asserts, so equality here means "the same table".
        assert_ne!(GAMMA1, GAMMA1_12K2, "the two numerators must be different");
        for mode in 0..=MR122 {
            assert_eq!(
                pre_big_gamma1(mode),
                subframe_gamma1(mode),
                "mode {mode}: the two tests must agree on every speech rate"
            );
        }
        assert_eq!(pre_big_gamma1(MRDTX), &GAMMA1_12K2);
        assert_eq!(subframe_gamma1(MRDTX), &GAMMA1);
        // And the boundary itself: 7.95 takes gamma1, 10.2 does not.
        assert_eq!(pre_big_gamma1(MR795), &GAMMA1);
        assert_eq!(pre_big_gamma1(MR102), &GAMMA1_12K2);
        assert_eq!(pre_big_gamma1(MR475), &GAMMA1);
        assert_eq!(pre_big_gamma1(MR515), &GAMMA1);
    }

    #[test]
    fn the_weighted_speech_half_frames_use_slots_zero_and_two() {
        // `pre_big`'s A-slot choice is `frameOffset > 0`, not `offset /
        // L_SUBFR`: the second half-frame starts at slot 2, and subframes 1 and
        // 3 take slots 1 and 3. Detect a wrong mapping by making the four slots
        // distinguishable.
        let mut ctx = DspContext::default();
        let mut az = [Word16(0); AZ_SIZE];
        for slot in 0..4 {
            az[slot * MP1] = Word16(4096);
            az[slot * MP1 + 1] = Word16(-1000 * (slot as i16 + 1));
        }
        let mut buffer = SpeechBuffer::new();
        let mut marked = [Word16(0); FRAME];
        for (i, s) in marked.iter_mut().enumerate() {
            *s = Word16(((i % 32) as i16) - 16);
        }
        buffer.push(&marked);

        let mut wsp = [Word16(0); FRAME];
        let mut weighted = WeightedSpeech::new();
        weighted.half_frame(&mut ctx, TRACE_MODE, &az, 0, &buffer, &mut wsp);
        let first = weighted;
        weighted.half_frame(&mut ctx, TRACE_MODE, &az, 80, &buffer, &mut wsp);

        // Swapping slots 2 and 3 must change the second half-frame and leave
        // the first alone; that is only true if the mapping is 0,1 then 2,3.
        let mut swapped = az;
        swapped.swap(2 * MP1 + 1, 3 * MP1 + 1);
        let mut other = [Word16(0); FRAME];
        let mut w2 = WeightedSpeech::new();
        w2.half_frame(&mut ctx, TRACE_MODE, &swapped, 0, &buffer, &mut other);
        assert_eq!(w2, first, "the first half-frame does not read slots 2 or 3");
        assert_eq!(other[..80], wsp[..80], "the first half-frame is unchanged");
        w2.half_frame(&mut ctx, TRACE_MODE, &swapped, 80, &buffer, &mut other);
        assert_ne!(
            other[80..],
            wsp[80..],
            "the second half-frame reads slots 2 and 3"
        );
    }

    #[test]
    fn the_weighting_memory_advances_once_per_subframe() {
        // Four updates per frame, two per `pre_big` call. A port that updates
        // once per half-frame produces weighted speech that is right for the
        // first subframe of each pair and wrong for the second.
        let mut ctx = DspContext::default();
        let mut az = [Word16(0); AZ_SIZE];
        for slot in 0..4 {
            az[slot * MP1] = Word16(4096);
            az[slot * MP1 + 1] = Word16(-2000);
        }
        let mut buffer = SpeechBuffer::new();
        let mut marked = [Word16(0); FRAME];
        for (i, s) in marked.iter_mut().enumerate() {
            *s = Word16(((i % 32) as i16) - 16);
        }
        buffer.push(&marked);

        let mut wsp = [Word16(0); FRAME];
        let mut weighted = WeightedSpeech::new();
        let states: Vec<_> = (0..2)
            .map(|half| {
                weighted.half_frame(&mut ctx, TRACE_MODE, &az, half * 80, &buffer, &mut wsp);
                weighted
            })
            .collect();
        assert_ne!(states[0], WeightedSpeech::new(), "the memory advanced");
        assert_ne!(states[0], states[1], "and advanced again");
        assert!(
            wsp.iter().any(|w| w.0 != 0),
            "the weighted speech is not identically zero"
        );
    }

    #[test]
    fn the_impulse_response_is_the_weighted_synthesis_filters() {
        // `h1` is the response of Ap1(z)/(Aq(z)·Ap2(z)) and its first sample is
        // therefore Ap1[0] scaled by nothing at all: 4096 in Q12. A port that
        // filtered the wrong way round, or that forgot the second `Syn_filt`,
        // still produces a plausible response but not this one.
        let count = replay(|frame, out| {
            for (subfr, t) in out.targets.iter().enumerate() {
                assert_eq!(
                    t.h1[0].0, 4096,
                    "frame {frame} subframe {subfr}: h1[0] is not the monic lead"
                );
            }
        });
        assert_eq!(count, 3);
    }

    #[test]
    fn autocorrelation_rescales_on_saturation_rather_than_widening() {
        // A window loud enough to saturate `r[0]` must come back with a
        // negative `norm` — `norm_l` of a saturated sum is 0 and each rescale
        // subtracts 4. An `i64` accumulator never gets here.
        let mut ctx = DspContext::default();
        let loud = [Word16(32760); L_WINDOW];
        let flat = [32767i16; L_WINDOW];
        let r = autocorrelate(&mut ctx, &loud, &flat);
        assert!(
            r.norm < 0,
            "a saturating window did not trigger the rescale (norm = {})",
            r.norm
        );
        assert_eq!(r.norm % 4, 0, "the rescale exponent moves in steps of four");
        // Quiet input: no rescale, and the +1 bias keeps a silent window from
        // normalising to nothing.
        let silent = [Word16(0); L_WINDOW];
        let r = autocorrelate(&mut ctx, &silent, &flat);
        assert_eq!(r.norm, 30, "silence normalises the +1 bias to the top");
    }

    #[test]
    fn the_lag_window_leaves_r_zero_alone() {
        let mut r = Autocorrelation {
            r_h: [Word16(0x4000); MP1],
            r_l: [Word16(0); MP1],
            norm: 0,
        };
        let before = r.r_h[0];
        lag_window(&mut r);
        assert_eq!(r.r_h[0].0, before.0, "r[0] must not be windowed");
        // And the off-by-one: r[1] gets lag[0], the largest factor, so it moves
        // least of all the windowed lags.
        assert!(
            r.r_h[1].0 > r.r_h[M].0,
            "the lag window is applied from r[1]"
        );
    }
}
