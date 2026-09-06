//! Pitch analysis for the AMR-WB encoder, 3GPP TS 26.190 §5.5–5.7.
//!
//! Implements, from the TS 26.173 fixed-point reference: `Hp_wsp` and
//! `scale_mem_Hp_wsp` (`hp_wsp.c`), `Pitch_med_ol`, `median5` and `Med_olag`
//! (`p_med_ol.c`) together with the open-loop driver and adaptive weighting of
//! `cod_main.c` 543–608, the per-subframe lag window of `cod_main.c` 751–765 /
//! 925–998, `Pitch_fr4` with its `Norm_Corr` and `Interpol_4` helpers
//! (`pitch_f4.c`), the long-term-prediction low-pass and its `select` decision
//! (`cod_main.c` 1026–1109), `G_pitch` (`g_pitch.c`), `Convolve`
//! (`convolve.c`), `Updt_tar` (`updt_tar.c`) and the gain-clipping tracker
//! (`gpclip.c`).
//!
//! The pitch *index* encoding of `cod_main.c` 902–1011 is deliberately not
//! here: it turns these outputs into bits, which is the bitstream layer's job.
//!
//! # What validated it
//!
//! `testdata/wb_enc_trace.txt`, produced by driving the TS 26.173 encoder
//! itself at 12.65 kbit/s. The committed tests compare, over its three frames
//! and twelve subframes, the rows `T_op_med`, `T0`, `T0_frac`, `T0_min`,
//! `T0_max`, `adapt`, `y1`, `gain1`, `gain2`, `select` and `cn`, driven from
//! `wsp`, `wsp_shift`, `Q_new`, `xn`, `h1`, `Aq`, `isf_unq46`, `decimated`,
//! `exc_total` and `gain_pit`. Each test states how many subframes it compared,
//! so a harness that silently produced nothing would fail rather than pass.
//!
//! During development the same harness was run against the full 50-frame trace
//! (200 subframes) at 6.60, 8.85, 12.65 and 14.25 kbit/s and agreed on every
//! row, which is what exercises the branches the committed rate does not: the
//! whole-frame open-loop search, the single-absolute-subframe window, the
//! half-sample-only resolution, the absent unfiltered candidate, and the
//! interoperable gain-clipping constants. Only the three committed frames are
//! used by the tests that ship.
//!
//! # Three searches, three tie-breaks
//!
//! Every stage here picks a best candidate, and each picks it a different way.
//! An implementation that maximises the same objective but breaks ties the
//! other way produces speech that sounds fine and fails conformance, so the
//! handedness is spelled out at each site and tested on its own:
//!
//! * open loop — visits lags **descending** and compares non-strictly, so a tie
//!   goes to the **smallest** lag;
//! * closed-loop integer — visits **ascending**, compares non-strictly, so a
//!   tie goes to the **largest** lag;
//! * closed-loop fraction — visits **ascending**, compares **strictly**, so a
//!   tie keeps the incumbent, i.e. the **smallest** fraction.
//!
//! # Q-formats
//!
//! The weighted speech, the pitch target `xn` and the excitation share the
//! frame's scaling: `Q_new` from the pre-emphasis, plus the frame-level `shift`
//! derived from the maximum of `wsp[]`. This module never computes either — it
//! takes signals already in that scaling and states per function what it
//! expects. Gains are Q14, correlations Q15, the impulse response `h1` Q15.

use super::super::codebook::L_SUBFR;
use super::super::ltp::{low_pass, predict};
use super::super::math::{dot_product12, isqrt_n, median5, scale_sig};
use crate::fixed_point::arith::{add, extract_h, extract_l, mult, negate, round, sub};
use crate::fixed_point::arith32::{l_deposit_h, l_mac, l_msu, l_mult, l_sub};
use crate::fixed_point::div::div_s;
use crate::fixed_point::oper32::{l_comp, l_extract, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Shortest pitch lag, in 12.8 kHz samples.
pub const PIT_MIN: i16 = 34;
/// Longest pitch lag, in 12.8 kHz samples.
pub const PIT_MAX: i16 = 231;
/// Lag at or above which the 9-bit index drops to half-sample resolution.
const PIT_FR2: i16 = 128;
/// Lag at or above which the 9-bit index drops to whole-sample resolution.
const PIT_FR1_9B: i16 = 160;
/// Lag at or above which the 8-bit index drops to whole-sample resolution.
const PIT_FR1_8B: i16 = 92;

/// Hard ceiling on an unquantised pitch gain when clipping is active, 0.95 Q14.
const GP_CLIP: i16 = 15565;

/// Decimation factor of the open-loop search.
const OPL_DECIM: i16 = 2;
/// Shortest open-loop lag, in decimated samples. Never *returned* — the search
/// loop excludes its own lower bound.
const OL_MIN_LAG: usize = 17;
/// Longest open-loop lag, in decimated samples.
const OL_MAX_LAG: usize = 115;
/// Decimated weighted speech per frame.
const WSP_FRAME: usize = 128;
/// Decimated weighted-speech history the search reaches back over.
const WSP_HISTORY: usize = OL_MAX_LAG;

/// Frame bit counts that the reference's mode tests are written against
/// (`bits.h`). Only the two smallest are ever compared for equality.
const BITS_6K60: u16 = 132;
const BITS_8K85: u16 = 177;

/// Half-length of the correlation interpolation filter, in whole samples.
const L_INTERPOL1: i16 = 4;
/// Upsampling factor of both quarter-sample interpolators.
const UP_SAMP: i16 = 4;

/// Open-loop correlation weighting, from `p_med_ol.tab`.
///
/// Two windows are read out of one table: a monotone taper that favours short
/// lags, indexed `198 - L_max + lag`, and a triangular bump centred on the
/// previous lag, indexed `98 + lag - L_0`. The flat 16384 plateau at indices
/// 95..=101 is the centre of the second.
const CORR_WEIGHT: [i16; 199] = [
    10772, 10794, 10816, 10839, 10862, 10885, 10908, 10932, 10955, 10980, 11004, 11029, 11054,
    11079, 11105, 11131, 11157, 11183, 11210, 11238, 11265, 11293, 11322, 11350, 11379, 11409,
    11439, 11469, 11500, 11531, 11563, 11595, 11628, 11661, 11694, 11728, 11763, 11798, 11834,
    11870, 11907, 11945, 11983, 12022, 12061, 12101, 12142, 12184, 12226, 12270, 12314, 12358,
    12404, 12451, 12498, 12547, 12596, 12647, 12699, 12751, 12805, 12861, 12917, 12975, 13034,
    13095, 13157, 13221, 13286, 13353, 13422, 13493, 13566, 13641, 13719, 13798, 13880, 13965,
    14053, 14143, 14237, 14334, 14435, 14539, 14648, 14761, 14879, 15002, 15130, 15265, 15406,
    15554, 15710, 15874, 16056, 16384, 16384, 16384, 16384, 16384, 16384, 16384, 16056, 15874,
    15710, 15554, 15406, 15265, 15130, 15002, 14879, 14761, 14648, 14539, 14435, 14334, 14237,
    14143, 14053, 13965, 13880, 13798, 13719, 13641, 13566, 13493, 13422, 13353, 13286, 13221,
    13157, 13095, 13034, 12975, 12917, 12861, 12805, 12751, 12699, 12647, 12596, 12547, 12498,
    12451, 12404, 12358, 12314, 12270, 12226, 12184, 12142, 12101, 12061, 12022, 11983, 11945,
    11907, 11870, 11834, 11798, 11763, 11728, 11694, 11661, 11628, 11595, 11563, 11531, 11500,
    11469, 11439, 11409, 11379, 11350, 11322, 11293, 11265, 11238, 11210, 11183, 11157, 11131,
    11105, 11079, 11054, 11029, 11004, 10980, 10955, 10932, 10908, 10885, 10862, 10839, 10816,
    10794, 10772, 10750, 10728,
];

/// Quarter-sample interpolation of the normalised correlation, Q14, −3 dB at
/// `0.791·fs/2` (`inter4_1`, file-static in `pitch_f4.c`).
///
/// Stored interleaved by phase, so the read strides by [`UP_SAMP`]. This is a
/// *different* filter from the adaptive-codebook interpolator in
/// [`super::super::ltp`]: shorter, and tuned for a correlation rather than a
/// waveform.
const INTER4_1: [i16; 32] = [
    -12, -26, 32, 206, 420, 455, 73, -766, -1732, -2142, -1242, 1376, 5429, 9910, 13418, 14746,
    13418, 9910, 5429, 1376, -1242, -2142, -1732, -766, 73, 455, 420, 206, 32, -26, -12, 0,
];

/// Feedback taps of the weighted-speech high-pass.
///
/// Q13 of the *negated* denominator, not Q12: `21663 / 8192 = 2.6444` is
/// `-a_coef[0]`. The extra bit and the sign flip are compensated by the double
/// precision split of the recursion, so these integers must be used verbatim —
/// re-deriving them from the float coefficients in `hp_wsp.c`'s header with one
/// common Q gives a filter that is wrong by a factor of two. Index 0 is never
/// read; it is kept so the tap numbering matches the difference equation.
const HP_FEEDBACK: [i16; 4] = [8192, 21663, -19258, 5734];

/// Feed-forward taps of the weighted-speech high-pass, Q12.
const HP_FORWARD: [i16; 4] = [-3432, 10280, -10280, 3432];

/// The frame's bit budget, which is what every mode test in the reference's
/// pitch path keys on.
///
/// Not the codec mode: a comfort-noise frame carries 35 bits whatever mode was
/// requested, and that budget routes it down the wideband branches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PitchMode {
    bits: u16,
}

impl PitchMode {
    /// Build from a frame's bit count — `AmrMode::bits()`, or 35 for SID.
    #[must_use]
    pub const fn from_frame_bits(bits: u16) -> Self {
        Self { bits }
    }

    /// Whether the open-loop search runs once over the whole frame.
    ///
    /// Only 6.60 kbit/s does; every other budget, comfort noise included,
    /// searches the two halves separately and codes two open-loop lags.
    #[must_use]
    pub const fn open_loop_spans_frame(self) -> bool {
        self.bits == BITS_6K60
    }

    /// Whether the unfiltered long-term candidate exists and costs a bit.
    ///
    /// Below 12.65 kbit/s there is no bit to spend on the choice, so the
    /// low-pass-filtered candidate is always used.
    #[must_use]
    pub const fn has_sharp_candidate(self) -> bool {
        self.bits > BITS_8K85
    }

    /// Whether the third subframe restarts the lag window from `T_op2`.
    ///
    /// Same condition as [`Self::has_sharp_candidate`] in the reference, but a
    /// separate question, so it gets its own name rather than a shared one.
    #[must_use]
    pub const fn third_subframe_is_absolute(self) -> bool {
        self.bits > BITS_6K60
    }

    /// The fractional resolution schedule for the closed-loop search.
    #[must_use]
    pub const fn lag_resolution(self) -> LagResolution {
        if self.bits > BITS_8K85 {
            LagResolution::NINE_BIT
        } else {
            LagResolution::EIGHT_BIT
        }
    }

    /// Whether the gain-clipping tracker uses its interoperable-mode constants.
    ///
    /// The two lowest budgets clamp the smoothed ISF gap to 384 rather than 307
    /// and average the pitch gain far more slowly.
    #[must_use]
    pub const fn interoperable_clipping(self) -> bool {
        self.bits == BITS_6K60 || self.bits == BITS_8K85
    }
}

/// Where the closed-loop search drops from quarter- to half- to whole-sample
/// resolution, which the pitch index width decides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LagResolution {
    /// Lag at or above which the step becomes 1/2 sample.
    half_from: i16,
    /// Lag at or above which the fraction is dropped entirely.
    whole_from: i16,
}

impl LagResolution {
    /// 6.60 and 8.85 kbit/s, where the pitch index is 8 bits.
    ///
    /// `half_from == PIT_MIN` makes the half-sample test vacuously true, so
    /// *every* subframe searches at half-sample resolution — the reference
    /// expresses "this mode has no quarter-sample lags" through that identity
    /// rather than through a separate flag.
    pub const EIGHT_BIT: Self = Self {
        half_from: PIT_MIN,
        whole_from: PIT_FR1_8B,
    };

    /// 12.65 kbit/s and above, where the pitch index is 9 bits.
    pub const NINE_BIT: Self = Self {
        half_from: PIT_FR2,
        whole_from: PIT_FR1_9B,
    };
}

/// The 16-lag window a closed-loop search runs over.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LagWindow {
    /// Lowest integer lag searched.
    pub min: i16,
    /// Highest integer lag searched.
    pub max: i16,
}

impl LagWindow {
    /// The window centred on `lag`, clamped into `[PIT_MIN, PIT_MAX]`.
    ///
    /// Used twice with the same code: once per open-loop lag to open a
    /// subframe that carries an absolute pitch index, and once again on that
    /// subframe's chosen lag to seed the relative subframes that follow. The
    /// span is always exactly 16 integer lags, which is what makes the 5- and
    /// 6-bit relative indices fit.
    #[must_use]
    pub fn around(ctx: &mut DspContext, lag: i16) -> Self {
        let mut min = sub(ctx, Word16(lag), Word16(8)).0;
        if sub(ctx, Word16(min), Word16(PIT_MIN)).0 < 0 {
            min = PIT_MIN;
        }
        let mut max = add(ctx, Word16(min), Word16(15)).0;
        if sub(ctx, Word16(max), Word16(PIT_MAX)).0 > 0 {
            max = PIT_MAX;
            min = sub(ctx, Word16(max), Word16(15)).0;
        }
        Self { min, max }
    }
}

/// Third-order 180 Hz high-pass applied to the weighted speech before the
/// open-loop *gain* is measured (`Hp_wsp`).
///
/// The lag itself is chosen on the unfiltered signal; this filter exists only
/// so the normalised correlation that gates the median smoothing is not
/// dominated by the low-frequency tilt of voiced speech.
#[derive(Clone, Copy, Debug, Default)]
pub struct WeightedSpeechHighPass {
    /// `y[i-3]`, `y[i-2]`, `y[i-1]` as (hi, lo) double-precision pairs.
    feedback: [(Word16, Word16); 3],
    /// `x[i-1]`, `x[i-2]`, `x[i-3]`.
    inputs: [Word16; 3],
}

impl WeightedSpeechHighPass {
    /// Filter `input` into `output`, advancing the state.
    ///
    /// Both signals are in the weighted speech's own Q.
    ///
    /// # Panics
    ///
    /// If `output` is shorter than `input`.
    pub fn filter(&mut self, ctx: &mut DspContext, input: &[Word16], output: &mut [Word16]) {
        assert!(
            output.len() >= input.len(),
            "the high-pass writes one output sample per input sample"
        );
        let [mut y3, mut y2, mut y1] = self.feedback;
        let [mut x0, mut x1, mut x2] = self.inputs;

        for (&sample, slot) in input.iter().zip(output.iter_mut()) {
            let x3 = x2;
            x2 = x1;
            x1 = x0;
            x0 = sample;

            // The seed is a rounding constant for the `L_shr(.., 15)` below,
            // not an offset: the low halves are accumulated first, rounded down
            // into the same scale as the high halves, and only then added to
            // them.
            let mut acc = Word32(16384);
            acc = l_mac(ctx, acc, y1.1, Word16(HP_FEEDBACK[1]));
            acc = l_mac(ctx, acc, y2.1, Word16(HP_FEEDBACK[2]));
            acc = l_mac(ctx, acc, y3.1, Word16(HP_FEEDBACK[3]));
            acc = l_shr(ctx, acc, 15);
            acc = l_mac(ctx, acc, y1.0, Word16(HP_FEEDBACK[1]));
            acc = l_mac(ctx, acc, y2.0, Word16(HP_FEEDBACK[2]));
            acc = l_mac(ctx, acc, y3.0, Word16(HP_FEEDBACK[3]));
            acc = l_mac(ctx, acc, x0, Word16(HP_FORWARD[0]));
            acc = l_mac(ctx, acc, x1, Word16(HP_FORWARD[1]));
            acc = l_mac(ctx, acc, x2, Word16(HP_FORWARD[2]));
            acc = l_mac(ctx, acc, x3, Word16(HP_FORWARD[3]));
            acc = l_shl(ctx, acc, 2);

            y3 = y2;
            y2 = y1;
            // The recursion carries the value *before* the final doubling. The
            // output is doubled, the state is not; feeding the doubled value
            // back gives a filter that drifts slowly instead of failing loudly.
            y1 = l_extract(acc);

            let doubled = l_shl(ctx, acc, 1);
            *slot = round(ctx, doubled);
        }

        self.feedback = [y3, y2, y1];
        self.inputs = [x0, x1, x2];
    }

    /// Rescale the state by `exp` bits, to follow a change in the frame's Q.
    pub fn rescale(&mut self, ctx: &mut DspContext, exp: i16) {
        for pair in &mut self.feedback {
            let widened = l_shl(ctx, l_comp(pair.0, pair.1), exp);
            *pair = l_extract(widened);
        }
        for sample in &mut self.inputs {
            let widened = l_shl(ctx, l_deposit_h(*sample), exp);
            *sample = round(ctx, widened);
        }
    }
}

/// What one frame's open-loop analysis produced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpenLoopLags {
    /// `T_op`, in 12.8 kHz samples: the lag for the frame's first half (or, at
    /// 6.60 kbit/s, for the whole frame).
    pub first_half: i16,
    /// `T_op2`. Equal to [`Self::first_half`] at 6.60 kbit/s, where it is never
    /// used.
    pub second_half: i16,
    /// The median-smoothed lag after the first half, in *decimated* samples,
    /// and `None` when the half's normalised correlation was too weak to
    /// refresh it.
    ///
    /// The distinction is not cosmetic: the reference only shifts its five-lag
    /// history when the correlation clears 0.6, so a frame that fails the test
    /// leaves the history — and therefore the next frame's weighting — alone.
    pub smoothed_after_first_half: Option<i16>,
}

/// Open-loop pitch state, carried across frames (`cod_main.c` 543–608).
#[derive(Clone, Debug)]
pub struct OpenLoopPitch {
    /// Decimated weighted speech from previous frames, oldest first.
    history: [Word16; WSP_HISTORY],
    /// High-passed weighted speech: history then the current half.
    high_passed: [Word16; WSP_HISTORY + WSP_FRAME],
    /// State of the high-pass itself.
    high_pass: WeightedSpeechHighPass,
    /// `old_T0_med`, decimated.
    smoothed_lag: i16,
    /// `old_ol_lag`, the five most recent open-loop lags, newest first.
    recent_lags: [Word16; 5],
    /// `ol_gain`, the normalised correlation of the last half analysed, Q15.
    gain: Word16,
    /// `ada_w`, Q15: how confident the recent lag history is.
    confidence: Word16,
    /// `ol_wght_flg`: whether to weight candidates towards the smoothed lag.
    weight_towards_history: bool,
    /// `old_wsp_shift`, the frame-level `shift` the history is scaled by.
    history_shift: i16,
}

impl Default for OpenLoopPitch {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenLoopPitch {
    /// The reset state of `Reset_encoder(st, 1)`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            history: [Word16(0); WSP_HISTORY],
            high_passed: [Word16(0); WSP_HISTORY + WSP_FRAME],
            high_pass: WeightedSpeechHighPass {
                feedback: [(Word16(0), Word16(0)); 3],
                inputs: [Word16(0); 3],
            },
            smoothed_lag: 40,
            recent_lags: [Word16(40); 5],
            gain: Word16(0),
            confidence: Word16(0),
            weight_towards_history: false,
            history_shift: 0,
        }
    }

    /// The median-smoothed open-loop lag, in decimated samples.
    #[must_use]
    pub const fn smoothed_lag(&self) -> i16 {
        self.smoothed_lag
    }

    /// Follow the frame's change of scaling, before [`Self::analyse`].
    ///
    /// `q_exp` is `Q_new - Q_old` from the pre-emphasis; `shift` is the
    /// frame-level exponent derived from the maximum of `wsp[]`, which the new
    /// samples have *already* been scaled by. The history has not, so it is
    /// brought to the new scale by the sum of the two differences.
    pub fn rescale(&mut self, ctx: &mut DspContext, q_exp: i16, shift: i16) {
        let regained = sub(ctx, Word16(shift), Word16(self.history_shift));
        let exp = add(ctx, Word16(q_exp), regained).0;
        self.history_shift = shift;
        scale_sig(ctx, &mut self.history, exp);
        scale_sig(ctx, &mut self.high_passed[..WSP_HISTORY], exp);
        self.high_pass.rescale(ctx, exp);
    }

    /// Search one frame of decimated weighted speech for its open-loop lags.
    ///
    /// `weighted` is `wsp[]` after decimation and scaling: `WSP_FRAME`
    /// samples at 6.4 kHz.
    ///
    /// # Panics
    ///
    /// If `weighted` is not exactly `WSP_FRAME` long.
    pub fn analyse(
        &mut self,
        ctx: &mut DspContext,
        weighted: &[Word16],
        mode: PitchMode,
    ) -> OpenLoopLags {
        assert_eq!(
            weighted.len(),
            WSP_FRAME,
            "the open-loop search wants one frame of decimated weighted speech"
        );
        let mut buffer = [Word16(0); WSP_HISTORY + WSP_FRAME];
        buffer[..WSP_HISTORY].copy_from_slice(&self.history);
        buffer[WSP_HISTORY..].copy_from_slice(weighted);

        let span = if mode.open_loop_spans_frame() {
            WSP_FRAME
        } else {
            WSP_FRAME / 2
        };

        let first = self.scan(ctx, &buffer, WSP_HISTORY, span);
        let smoothed_after_first_half = self.absorb(ctx, first);
        let second_half = if mode.open_loop_spans_frame() {
            // Never read at 6.60 kbit/s; the reference still assigns it.
            first
        } else {
            let second = self.scan(ctx, &buffer, WSP_HISTORY + span, span);
            self.absorb(ctx, second);
            second
        };

        self.history.copy_from_slice(&buffer[WSP_FRAME..]);

        OpenLoopLags {
            // Plain multiplies, as in the reference: `T_op *= OPL_DECIM` is C
            // arithmetic, not `shl`, and 230 cannot overflow.
            first_half: first * OPL_DECIM,
            second_half: second_half * OPL_DECIM,
            smoothed_after_first_half,
        }
    }

    /// One `Pitch_med_ol` call: pick the lag, then measure its gain.
    fn scan(&mut self, ctx: &mut DspContext, buffer: &[Word16], base: usize, span: usize) -> i16 {
        let lag = self.best_lag(ctx, buffer, base, span);
        self.gain = self.correlation_at(ctx, &buffer[base..base + span], lag);

        // Overlapping forward copy when the span is half a frame: the ranges
        // are `[64, 178] -> [0, 114]`, so this must be a memmove.
        self.high_passed.copy_within(span..span + WSP_HISTORY, 0);

        lag
    }

    /// The weighted-autocorrelation search itself, in decimated samples.
    fn best_lag(&self, ctx: &mut DspContext, buffer: &[Word16], base: usize, span: usize) -> i16 {
        // `L_sub` saturates, so even a candidate equal to MIN_32 wins the first
        // comparison and the returned lag is always a real one.
        let mut best = Word32(i32::MIN);
        let mut best_lag = 0usize;

        let mut taper = 198usize;
        let towards_history = self.smoothed_lag > 0 && self.weight_towards_history;
        // Proven in range by the reference: `old_T0_med` is always a decimated
        // lag, so the bump index stays inside [0, 198]. Deliberately not
        // clamped — an out-of-range index here means the caller corrupted the
        // smoothed lag, and that should be loud.
        let mut bump = 98 + OL_MAX_LAG
            - usize::try_from(self.smoothed_lag)
                .expect("the smoothed open-loop lag is a decimated lag");

        // Descending, and the lower bound is excluded: decimated lag 17 — real
        // lag 34 — is not reachable from the open-loop search at all.
        for lag in (OL_MIN_LAG + 1..=OL_MAX_LAG).rev() {
            let mut acc = Word32(0);
            for j in 0..span {
                acc = l_mac(ctx, acc, buffer[base + j], buffer[base + j - lag]);
            }

            // The taper index advances every iteration whether or not the
            // second weighting applies; the bump index advances only inside it.
            let (hi, lo) = l_extract(acc);
            acc = mpy_32_16(hi, lo, Word16(CORR_WEIGHT[taper]));
            taper -= 1;

            if towards_history {
                let (hi, lo) = l_extract(acc);
                acc = mpy_32_16(hi, lo, Word16(CORR_WEIGHT[bump]));
                bump -= 1;
            }

            // Non-strict, and the visit order is descending, so a tie is
            // awarded to the *smaller* lag.
            if l_sub(ctx, acc, best).0 >= 0 {
                best = acc;
                best_lag = lag;
            }
        }

        i16::try_from(best_lag).expect("a decimated lag fits in 16 bits")
    }

    /// The normalised correlation of the high-passed signal at `lag`, Q15.
    ///
    /// Computed after the lag is chosen and never fed back into the choice: it
    /// gates the median smoothing and the tone detector, nothing else.
    fn correlation_at(&mut self, ctx: &mut DspContext, input: &[Word16], lag: i16) -> Word16 {
        let span = input.len();
        let lag = usize::try_from(lag).expect("the open-loop lag is positive");
        self.high_pass
            .filter(ctx, input, &mut self.high_passed[WSP_HISTORY..]);

        // The energies start at 1, not 0, so silence still normalises.
        let mut cross = Word32(0);
        let mut lagged_energy = Word32(1);
        let mut energy = Word32(1);
        for j in 0..span {
            let here = self.high_passed[WSP_HISTORY + j];
            let back = self.high_passed[WSP_HISTORY + j - lag];
            cross = l_mac(ctx, cross, here, back);
            lagged_energy = l_mac(ctx, lagged_energy, back, back);
            energy = l_mac(ctx, energy, here, here);
        }

        let cross_exp = norm_l(cross);
        let cross = l_shl(ctx, cross, cross_exp);
        let lagged_exp = norm_l(lagged_energy);
        let lagged_energy = l_shl(ctx, lagged_energy, lagged_exp);
        let energy_exp = norm_l(energy);
        let energy = l_shl(ctx, energy, energy_exp);

        let lagged_rounded = round(ctx, lagged_energy);
        let energy_rounded = round(ctx, energy);
        let mut product = l_mult(ctx, lagged_rounded, energy_rounded);
        let renorm = norm_l(product);
        product = l_shl(ctx, product, renorm);
        let mut exp = add(ctx, Word16(lagged_exp), Word16(energy_exp));
        exp = add(ctx, exp, Word16(renorm));
        exp = sub(ctx, Word16(62), exp);

        // `Isqrt_n` rewrites its exponent; the value used below is the one it
        // wrote, not the one passed in.
        let (inverse_root, exp) = isqrt_n(ctx, (product, exp.0));

        let cross_rounded = round(ctx, cross);
        let root_rounded = round(ctx, inverse_root);
        let scaled = l_mult(ctx, cross_rounded, root_rounded);
        let headroom = sub(ctx, Word16(31), Word16(cross_exp));
        let total = add(ctx, headroom, Word16(exp)).0;
        let shifted = l_shl(ctx, scaled, total);
        round(ctx, shifted)
    }

    /// Fold one half's lag into the smoothed history and update the weighting.
    fn absorb(&mut self, ctx: &mut DspContext, lag: i16) -> Option<i16> {
        let refreshed = if sub(ctx, self.gain, Word16(19661)).0 > 0 {
            // `Med_olag`: shift the five-lag history back, insert, take the
            // median. The shift only happens on this branch, so a weak half
            // leaves the history untouched.
            for i in (1..5).rev() {
                self.recent_lags[i] = self.recent_lags[i - 1];
            }
            self.recent_lags[0] = Word16(lag);
            self.smoothed_lag = median5(&self.recent_lags).0;
            self.confidence = Word16(32767);
            Some(self.smoothed_lag)
        } else {
            // `mult` floors, so the confidence decays to 0 and stays there
            // until a strong half resets it.
            self.confidence = mult(ctx, self.confidence, Word16(29491));
            None
        };
        self.weight_towards_history = sub(ctx, self.confidence, Word16(26214)).0 >= 0;
        refreshed
    }
}

/// Convolve `input` with the impulse response `response`, zero initial state.
///
/// Lower-triangular, so `output[n]` sees only `input[0..=n]`. `response` is
/// Q15; the output carries the input's Q.
///
/// # Panics
///
/// If `response` is shorter than `input`.
#[must_use]
pub fn convolve(ctx: &mut DspContext, input: &[Word16], response: &[Word16]) -> [Word16; L_SUBFR] {
    assert!(
        response.len() >= input.len(),
        "the impulse response must cover the whole subframe"
    );
    let mut output = [Word16(0); L_SUBFR];
    for (n, slot) in output.iter_mut().enumerate().take(input.len()) {
        let mut acc = Word32(0);
        for i in 0..=n {
            acc = l_mac(ctx, acc, input[i], response[n - i]);
        }
        *slot = round(ctx, acc);
    }
    output
}

/// Subtract a scaled filtered contribution from the target (`Updt_tar`).
///
/// `gain` is Q14. The result truncates rather than rounds — the reference uses
/// `extract_h`, and substituting `round` here shifts every codebook target by
/// half an LSB.
#[must_use]
pub fn update_target(
    ctx: &mut DspContext,
    target: &[Word16],
    filtered: &[Word16],
    gain: Word16,
) -> [Word16; L_SUBFR] {
    let mut out = [Word16(0); L_SUBFR];
    for (i, slot) in out.iter_mut().enumerate() {
        let mut acc = l_mult(ctx, target[i], Word16(16384));
        acc = l_msu(ctx, acc, filtered[i], gain);
        *slot = extract_h(l_shl(ctx, acc, 1));
    }
    out
}

/// The correlations `G_pitch` leaves behind for the gain quantiser.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct GainCorrelations {
    /// `<y1,y1>` normalised to Q15.
    pub energy: Word16,
    /// Exponent of [`Self::energy`].
    pub energy_exp: i16,
    /// `<xn,y1>` normalised to Q15.
    pub correlation: Word16,
    /// Exponent of [`Self::correlation`].
    pub correlation_exp: i16,
}

/// The least-squares pitch gain for one candidate, Q14, clamped to `[0, 1.2]`.
///
/// `target` is `xn` and `filtered` is the candidate through `h1`, both in the
/// frame's `shift` scaling. The correlations come back regardless of the
/// result — the reference fills them in *before* the negative-correlation early
/// return, and the gain quantiser reads them either way.
#[must_use]
pub fn pitch_gain(
    ctx: &mut DspContext,
    target: &[Word16],
    filtered: &[Word16],
) -> (Word16, GainCorrelations) {
    let (energy, energy_exp) = dot_product12(ctx, filtered, filtered);
    let (correlation, correlation_exp) = dot_product12(ctx, target, filtered);
    let energy = extract_h(energy);
    let correlation = extract_h(correlation);
    let coefficients = GainCorrelations {
        energy,
        energy_exp,
        correlation,
        correlation_exp,
    };

    if correlation.0 < 0 {
        return (Word16(0), coefficients);
    }

    // Halving guarantees `div_s`'s precondition; the lost bit is what puts the
    // quotient in Q14 rather than Q15.
    let halved = shr(ctx, correlation, 1);
    let mut gain = div_s(halved, energy);
    let exp = sub(ctx, Word16(correlation_exp), Word16(energy_exp)).0;
    // Saturating, and the saturation is load-bearing: an over-range ratio is
    // meant to land on the 1.2 clamp below rather than wrap.
    gain = shl(ctx, gain, exp);
    if sub(ctx, gain, Word16(19661)).0 > 0 {
        gain = Word16(19661);
    }
    (gain, coefficients)
}

/// The two long-term-prediction candidates and the choice between them.
#[derive(Clone, Copy, Debug)]
pub struct LtpDecision {
    /// `gain1`, Q14: gain of the unfiltered candidate. Zero below 12.65 kbit/s,
    /// where the candidate does not exist.
    pub sharp_gain: Word16,
    /// `gain2`, Q14: gain of the low-pass-filtered candidate.
    pub smooth_gain: Word16,
    /// `y1`: the unfiltered candidate through `h1`, as it stands *before* the
    /// choice is applied. Meaningless below 12.65 kbit/s.
    pub sharp_response: [Word16; L_SUBFR],
    /// `y2`: the filtered candidate through `h1`.
    pub smooth_response: [Word16; L_SUBFR],
    /// `select`: true when the unfiltered candidate wins.
    pub prefer_sharp: bool,
    /// `xn2`: the codebook-search target left by the winning candidate.
    pub codebook_target: [Word16; L_SUBFR],
    /// `g_coeff` of the winning candidate.
    pub correlations: GainCorrelations,
}

impl LtpDecision {
    /// `gain_pit`, Q14: the winning candidate's gain, before quantisation.
    #[must_use]
    pub const fn gain(&self) -> Word16 {
        if self.prefer_sharp {
            self.sharp_gain
        } else {
            self.smooth_gain
        }
    }

    /// `y1` as the gain quantiser sees it: the winning candidate's response.
    #[must_use]
    pub const fn response(&self) -> &[Word16; L_SUBFR] {
        if self.prefer_sharp {
            &self.sharp_response
        } else {
            &self.smooth_response
        }
    }
}

/// Build both long-term candidates, gain them, and keep the better one.
///
/// On entry `excitation[offset..offset + L_SUBFR + 1]` holds the adaptive
/// codebook vector from [`super::super::ltp::predict`] — 65 samples, because
/// the low-pass reads one past the subframe — and `excitation[offset - 1]` is
/// the previous subframe's final excitation. On return
/// `excitation[offset..offset + L_SUBFR]` holds whichever candidate won.
///
/// `target` is `xn` and `response` is `h1`, both already scaled by the frame's
/// `shift`. `clip` is the gain-clipping flag from [`GainClipping::clips`].
///
/// # Panics
///
/// If `excitation` does not reach one sample either side of the subframe.
pub fn choose_ltp(
    ctx: &mut DspContext,
    excitation: &mut [Word16],
    offset: usize,
    target: &[Word16],
    response: &[Word16],
    clip: bool,
    mode: PitchMode,
) -> LtpDecision {
    assert!(
        offset >= 1 && excitation.len() > offset + L_SUBFR,
        "the long-term low-pass reads one sample either side of the subframe"
    );

    let mut sharp_gain = Word16(0);
    let mut sharp_response = [Word16(0); L_SUBFR];
    let mut sharp_target = [Word16(0); L_SUBFR];
    let mut sharp_correlations = GainCorrelations::default();

    if mode.has_sharp_candidate() {
        sharp_response = convolve(ctx, &excitation[offset..offset + L_SUBFR], response);
        let (gain, coefficients) = pitch_gain(ctx, target, &sharp_response);
        sharp_gain = gain;
        sharp_correlations = coefficients;
        if clip && sub(ctx, sharp_gain, Word16(GP_CLIP)).0 > 0 {
            sharp_gain = Word16(GP_CLIP);
        }
        sharp_target = update_target(ctx, target, &sharp_response, sharp_gain);
    }

    let smoothed = low_pass(&excitation[offset - 1..=offset + L_SUBFR]);
    let smooth_response = convolve(ctx, &smoothed, response);
    let (mut smooth_gain, smooth_correlations) = pitch_gain(ctx, target, &smooth_response);
    if clip && sub(ctx, smooth_gain, Word16(GP_CLIP)).0 > 0 {
        smooth_gain = Word16(GP_CLIP);
    }
    let smooth_target = update_target(ctx, target, &smooth_response, smooth_gain);

    let prefer_sharp = mode.has_sharp_candidate() && {
        // One running saturating sum, not two energies subtracted: all 64
        // positive terms first, then all 64 negative ones. Splitting it
        // changes the answer wherever the accumulator saturates.
        let mut balance = Word32(0);
        for &v in &sharp_target {
            balance = l_mac(ctx, balance, v, v);
        }
        for &v in &smooth_target {
            balance = l_msu(ctx, balance, v, v);
        }
        // Plain C `<=` on the accumulator, so a tie goes to the unfiltered
        // candidate.
        balance.0 <= 0
    };

    let codebook_target = if prefer_sharp {
        sharp_target
    } else {
        excitation[offset..offset + L_SUBFR].copy_from_slice(&smoothed);
        smooth_target
    };

    LtpDecision {
        sharp_gain,
        smooth_gain,
        sharp_response,
        smooth_response,
        prefer_sharp,
        codebook_target,
        correlations: if prefer_sharp {
            sharp_correlations
        } else {
            smooth_correlations
        },
    }
}

/// What constrains one subframe's closed-loop search.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchLimits {
    /// The 16 integer lags to try.
    pub window: LagWindow,
    /// The reference's `pit_flag == 0`: this subframe carries an *absolute*
    /// pitch index rather than one relative to the last. Only those are allowed
    /// to drop below quarter-sample resolution, because only they have the
    /// index range to express a coarse long lag.
    pub absolute: bool,
    /// Where the fraction coarsens, which the index width decides.
    pub resolution: LagResolution,
}

/// The closed-loop pitch search, `Pitch_fr4`.
///
/// Returns `(T0, T0_frac)`: an integer lag in 12.8 kHz samples and a fraction
/// in quarters, always in `0..=3`.
///
/// `excitation[offset - t]` must be valid for every `t` up to
/// `limits.window.max + 4`; `excitation[offset..offset + L_SUBFR]` holds this
/// subframe's LP residual, which short lags genuinely read — a lag under 64
/// reaches into samples the adaptive codebook has not produced yet, and the
/// residual is what the reference puts there. `target` is `xn` and `response`
/// is `h1`.
///
/// # Panics
///
/// If the excitation does not reach back far enough for the window.
#[must_use]
pub fn closed_loop_lag(
    ctx: &mut DspContext,
    excitation: &[Word16],
    offset: usize,
    target: &[Word16],
    response: &[Word16],
    limits: SearchLimits,
) -> (i16, i16) {
    let SearchLimits {
        window,
        absolute,
        resolution,
    } = limits;
    let t_min = sub(ctx, Word16(window.min), Word16(L_INTERPOL1)).0;
    let t_max = add(ctx, Word16(window.max), Word16(L_INTERPOL1)).0;
    assert!(
        offset >= usize::try_from(t_max).expect("the lag window is positive"),
        "the excitation history is shorter than the search window"
    );

    let mut correlations = [Word16(0); 40];
    normalised_correlation(
        ctx,
        excitation,
        offset,
        target,
        response,
        (t_min, t_max),
        &mut correlations,
    );

    // Plain Rust here on purpose: `corr_v[corr_v_offset + t]` is C index
    // arithmetic, not signal arithmetic, so there is no basic operator to match.
    let at = |t: i16| usize::try_from(t - t_min).expect("the window is ordered");

    // Ascending from `t0_min` as incumbent, non-strict, so a tie goes to the
    // *larger* lag — the opposite handedness from the open-loop search.
    let mut best = correlations[at(window.min)];
    let mut lag = window.min;
    let first = add(ctx, Word16(window.min), Word16(1)).0;
    for t in first..=window.max {
        if sub(ctx, correlations[at(t)], best).0 >= 0 {
            best = correlations[at(t)];
            lag = t;
        }
    }

    if absolute && sub(ctx, Word16(lag), Word16(resolution.whole_from)).0 >= 0 {
        return (lag, 0);
    }

    // The second disjunct is an identity at 8.85 kbit/s and below, where
    // `half_from == PIT_MIN`: those modes have no quarter-sample lags at all.
    let coarse = (absolute && sub(ctx, Word16(lag), Word16(resolution.half_from)).0 >= 0)
        || sub(ctx, Word16(resolution.half_from), Word16(PIT_MIN)).0 == 0;
    let (step, mut fraction) = if coarse { (2i16, -2i16) } else { (1i16, -3i16) };
    // Load-bearing: without it the search could push the lag below the window
    // and the relative pitch index would underflow.
    if sub(ctx, Word16(lag), Word16(window.min)).0 == 0 {
        fraction = 0;
    }

    let centre = at(lag);
    let mut best = interpolate(ctx, &correlations, centre, fraction);
    let mut candidate = add(ctx, Word16(fraction), Word16(step)).0;
    while candidate <= 3 {
        let value = interpolate(ctx, &correlations, centre, candidate);
        // Strict, so a tie keeps the incumbent: the *smallest* fraction wins,
        // the opposite handedness from the integer stage above.
        if sub(ctx, value, best).0 > 0 {
            best = value;
            fraction = candidate;
        }
        // A plain C add, not the saturating operator.
        candidate += step;
    }

    if fraction < 0 {
        (
            sub(ctx, Word16(lag), Word16(1)).0,
            add(ctx, Word16(fraction), Word16(UP_SAMP)).0,
        )
    } else {
        (lag, fraction)
    }
}

/// `Norm_Corr`: the Q15 normalised correlation for every delay in the window.
///
/// `output[t - t_min]` receives the correlation for delay `t`.
fn normalised_correlation(
    ctx: &mut DspContext,
    excitation: &[Word16],
    offset: usize,
    target: &[Word16],
    response: &[Word16],
    bounds: (i16, i16),
    output: &mut [Word16],
) {
    let (t_min, t_max) = bounds;
    let back = |t: i16| offset - usize::try_from(t).expect("delays are positive");

    let mut filtered = convolve(
        ctx,
        &excitation[back(t_min)..back(t_min) + L_SUBFR],
        response,
    );

    // A per-subframe headroom shift from the target's energy. It is applied
    // identically to every candidate so it cannot reorder them on its own —
    // but it lands before the rounding to 16 bits, so it can still flip a
    // near-tie. Do not fold it away.
    let mut acc = Word32(1);
    for &v in target {
        acc = l_mac(ctx, acc, v, v);
    }
    let mut exp = sub(ctx, Word16(30), Word16(norm_l(acc)));
    exp = add(ctx, exp, Word16(2));
    let halved = shr(ctx, exp, 1);
    let scale = negate(ctx, halved).0;

    for t in t_min..=t_max {
        let mut acc = Word32(1);
        for i in 0..L_SUBFR {
            acc = l_mac(ctx, acc, target[i], filtered[i]);
        }
        let shift = norm_l(acc);
        let acc = l_shl(ctx, acc, shift);
        let corr_exp = sub(ctx, Word16(30), Word16(shift)).0;
        // Truncation here and below, but rounding on the result: the mix is
        // deliberate.
        let corr = extract_h(acc);

        let mut acc = Word32(1);
        for &v in &filtered {
            acc = l_mac(ctx, acc, v, v);
        }
        let shift = norm_l(acc);
        let acc = l_shl(ctx, acc, shift);
        let norm_exp = sub(ctx, Word16(30), Word16(shift)).0;
        let (acc, norm_exp) = isqrt_n(ctx, (acc, norm_exp));
        let norm = extract_h(acc);

        let mut product = l_mult(ctx, corr, norm);
        let exponents = add(ctx, Word16(corr_exp), Word16(norm_exp));
        let total = add(ctx, exponents, Word16(scale)).0;
        product = l_shl(ctx, product, total);
        output[usize::try_from(t - t_min).expect("the window is ordered")] = round(ctx, product);

        if t != t_max {
            // One cheaper step to the next delay, in place and descending so
            // each slot still sees the previous delay's value. This recursion
            // is *not* equal to re-convolving: `mult` floors where `Convolve`
            // accumulates in 32 bits and rounds once. Only the first delay may
            // use the convolution.
            let sample = excitation[back(t + 1)];
            for i in (1..L_SUBFR).rev() {
                let tap = mult(ctx, sample, response[i]);
                filtered[i] = add(ctx, tap, filtered[i - 1]);
            }
            filtered[0] = mult(ctx, sample, response[0]);
        }
    }
}

/// `Interpol_4`: quarter-sample interpolation of the correlation at `centre`.
///
/// `fraction` runs `-4..=3`; a negative one borrows a whole sample, which is
/// why the correlation is computed four delays either side of the window.
fn interpolate(
    ctx: &mut DspContext,
    correlations: &[Word16],
    centre: usize,
    fraction: i16,
) -> Word16 {
    let (phase, mut base) = if fraction < 0 {
        (add(ctx, Word16(fraction), Word16(UP_SAMP)).0, centre - 1)
    } else {
        (fraction, centre)
    };
    base -= usize::try_from(L_INTERPOL1 - 1).expect("the filter half-length is positive");

    let mut acc = Word32(0);
    let last_phase = sub(ctx, Word16(UP_SAMP), Word16(1));
    let start = sub(ctx, last_phase, Word16(phase));
    let mut k = usize::try_from(start.0).expect("the phase is 0..=3");
    for i in 0..2 * usize::try_from(L_INTERPOL1).expect("the filter half-length is positive") {
        acc = l_mac(ctx, acc, correlations[base + i], Word16(INTER4_1[k]));
        k += usize::try_from(UP_SAMP).expect("the upsampling factor is positive");
    }
    let doubled = l_shl(ctx, acc, 1);
    round(ctx, doubled)
}

/// Long-term tracker that decides when to clip the pitch gain (`gpclip.c`).
///
/// Accumulated LTP error makes a decoder that missed a frame diverge, so once
/// the pitch has been strongly predictive for about 250 ms the encoder caps the
/// gain at 0.95. Both halves of the test are smoothed: how resonant the LP
/// filter is, and how large the quantised pitch gain has been.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GainClipping {
    /// Smoothed minimum gap between adjacent ISFs.
    isf_gap: Word16,
    /// Smoothed quantised pitch gain, Q14.
    mean_gain: Word16,
}

impl Default for GainClipping {
    fn default() -> Self {
        Self::new()
    }
}

impl GainClipping {
    /// The reset state, `Init_gp_clip`.
    ///
    /// The gap starts at 307 for every mode, including the two that later
    /// clamp it to 384.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            isf_gap: Word16(307),
            mean_gain: Word16(9830),
        }
    }

    /// Fold this frame's *unquantised* ISF vector into the resonance measure.
    ///
    /// Called once per frame, before the subframe loop.
    ///
    /// # Panics
    ///
    /// If `isf` is shorter than the LP order.
    pub fn observe_isf(&mut self, ctx: &mut DspContext, mode: PitchMode, isf: &[Word16]) {
        assert!(isf.len() >= 16, "an AMR-WB ISF vector has 16 entries");
        let mut smallest = sub(ctx, isf[1], isf[0]);
        // The top gap, `isf[15] - isf[14]`, is deliberately excluded.
        for i in 2..15 {
            let gap = sub(ctx, isf[i], isf[i - 1]);
            if sub(ctx, gap, smallest).0 < 0 {
                smallest = gap;
            }
        }

        let carried = l_mult(ctx, Word16(26214), self.isf_gap);
        let mut smoothed = extract_h(l_mac(ctx, carried, Word16(6554), smallest));
        let ceiling = if mode.interoperable_clipping() {
            384
        } else {
            307
        };
        if sub(ctx, smoothed, Word16(ceiling)).0 > 0 {
            smoothed = Word16(ceiling);
        }
        self.isf_gap = smoothed;
    }

    /// Fold one subframe's *quantised* pitch gain into the long-term average.
    pub fn observe_gain(&mut self, ctx: &mut DspContext, mode: PitchMode, gain_pit: Word16) {
        let acc = if mode.interoperable_clipping() {
            let carried = l_mult(ctx, Word16(32113), self.mean_gain);
            l_mac(ctx, carried, Word16(655), gain_pit)
        } else {
            let carried = l_mult(ctx, Word16(29491), self.mean_gain);
            l_mac(ctx, carried, Word16(3277), gain_pit)
        };
        let mut gain = extract_h(acc);
        if sub(ctx, gain, Word16(9830)).0 < 0 {
            gain = Word16(9830);
        }
        self.mean_gain = gain;
    }

    /// Whether this subframe's pitch gains must be capped.
    #[must_use]
    pub fn clips(&self, ctx: &mut DspContext, mode: PitchMode) -> bool {
        if mode.interoperable_clipping() {
            // `16384 / DIST_ISF_MAX_IO` is C integer division: 42, not 42.67.
            let scaled = extract_l(l_mult(ctx, self.isf_gap, Word16(42)));
            let slope = mult(ctx, Word16(1638), scaled);
            let threshold = add(ctx, Word16(14746), slope);
            sub(ctx, self.mean_gain, threshold).0 > 0
        } else {
            sub(ctx, self.isf_gap, Word16(154)).0 < 0
                && sub(ctx, self.mean_gain, Word16(14746)).0 > 0
        }
    }
}

/// Build the adaptive codebook vector for one subframe, `Pred_lt4`.
///
/// A thin alias for [`super::super::ltp::predict`]: the encoder and decoder run
/// byte-for-byte the same interpolation, over the same 128-tap quarter-sample
/// filter, and the only difference is that the encoder asks for 65 samples
/// instead of 64 so the long-term low-pass can read one past the subframe.
///
/// # Panics
///
/// If `excitation` is too short for the lag's reach or the requested length.
pub fn predict_adaptive(
    excitation: &mut [Word16],
    offset: usize,
    lag: i16,
    fraction: i16,
    len: usize,
) {
    predict(
        excitation,
        offset,
        usize::try_from(lag).expect("the pitch lag is positive"),
        u8::try_from(fraction).expect("the pitch fraction is 0..=3"),
        len,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::OnceLock;

    /// 12.65 kbit/s: the rate the committed trace was produced at.
    const TRACE_BITS: u16 = 253;
    /// Frames committed to `wb_enc_trace.txt`.
    const TRACE_FRAMES: usize = 3;
    const SUBFRAMES: usize = 4;
    /// LP order.
    const M: usize = 16;
    /// `L_TOTAL`, the encoder's whole speech buffer.
    const L_TOTAL: usize = 384;
    /// `L_TOTAL - L_FRAME - L_FILT`: the part of the speech buffer that is
    /// carried between frames and rescaled rather than overwritten.
    const SPEECH_CARRY: usize = 116;
    /// Offset of `speech[0]` inside the whole buffer, `L_TOTAL - L_FRAME - L_NEXT`.
    const SPEECH_OFFSET: usize = 64;
    /// Offset of `new_speech[0]`, `L_TOTAL - L_FRAME - L_FILT`.
    const NEW_SPEECH_OFFSET: usize = SPEECH_CARRY;
    /// `PIT_MAX + L_INTERPOL`, the excitation history carried between frames.
    const EXC_HISTORY: usize = 248;
    /// `(L_FRAME + 1) + PIT_MAX + L_INTERPOL`.
    const EXC_TOTAL: usize = 505;
    /// `PREEMPH_FAC >> 1`, the pre-emphasis coefficient in Q14.
    const PREEMPH_MU: i16 = 11141;

    type TraceRows = HashMap<(i32, i32, String), Vec<i32>>;

    /// The committed encoder trace, indexed by (frame, subframe, name).
    fn trace() -> &'static TraceRows {
        static ROWS: OnceLock<TraceRows> = OnceLock::new();
        ROWS.get_or_init(|| {
            let text = include_str!("../../testdata/wb_enc_trace.txt");
            let mut rows = TraceRows::new();
            for line in text.lines() {
                let mut field = line.split_whitespace();
                if field.next() != Some("T") {
                    continue;
                }
                let frame: i32 = field.next().expect("frame").parse().expect("frame");
                let subframe: i32 = field.next().expect("subframe").parse().expect("subframe");
                let name = field.next().expect("name").to_owned();
                let values = field.map(|v| v.parse().expect("value")).collect();
                rows.insert((frame, subframe, name), values);
            }
            assert!(!rows.is_empty(), "the encoder trace parsed to nothing");
            rows
        })
    }

    fn maybe_row(frame: usize, subframe: i32, name: &str) -> Option<&'static [i32]> {
        let key = (
            i32::try_from(frame).expect("frame"),
            subframe,
            name.to_owned(),
        );
        trace().get(&key).map(Vec::as_slice)
    }

    fn row(frame: usize, subframe: i32, name: &str) -> Vec<Word16> {
        maybe_row(frame, subframe, name)
            .unwrap_or_else(|| {
                panic!("trace row {name} missing for frame {frame} subframe {subframe}")
            })
            .iter()
            .map(|&v| Word16(i16::try_from(v).expect("a trace row holds Word16 values")))
            .collect()
    }

    fn scalar(frame: usize, subframe: i32, name: &str) -> i16 {
        let values = row(frame, subframe, name);
        assert_eq!(values.len(), 1, "{name} is not a scalar row");
        values[0].0
    }

    /// One subframe's worth of what this module produced.
    struct Outcome {
        frame: usize,
        subframe: usize,
        lag: i16,
        fraction: i16,
        window: LagWindow,
        adaptive: [Word16; L_SUBFR],
        decision: LtpDecision,
        /// The second half of `cn[]` as the reference would have it: this
        /// subframe's reconstructed LP residual, minus the chosen adaptive
        /// contribution, scaled by the frame's `shift`. It exists to prove the
        /// harness fed the search the right residual.
        codebook_residual: [Word16; L_SUBFR / 2],
    }

    /// Everything one pass over the committed trace produced.
    struct Run {
        /// Per frame: the smoothed open-loop lag after the first half, or
        /// `None` where the reference did not refresh it.
        medians: Vec<Option<i16>>,
        outcomes: Vec<Outcome>,
    }

    /// Rebuild the encoder's `speech[]` buffer for one frame.
    ///
    /// `decimated` is the traced 12.8 kHz signal after the 50 Hz high-pass but
    /// *before* pre-emphasis. Pre-emphasising and scaling it by `Q_new`, and
    /// rescaling the carried history by `Q_new - Q_old`, is what the reference
    /// does at `cod_main.c` 393–414. Only the residual needs this, and only the
    /// residual is what no trace row carries.
    fn rebuild_speech(
        ctx: &mut DspContext,
        decimated: &[Word16],
        carry: &[Word16; SPEECH_CARRY],
        memory: Word16,
        scaling: (i16, i16),
    ) -> [Word16; L_TOTAL] {
        let (q_new, q_exp) = scaling;
        let mut speech = [Word16(0); L_TOTAL];
        speech[..SPEECH_CARRY].copy_from_slice(carry);
        scale_sig(ctx, &mut speech[..SPEECH_CARRY], q_exp);
        for (i, &sample) in decimated.iter().enumerate() {
            let previous = if i == 0 { memory } else { decimated[i - 1] };
            let mut acc = l_mult(ctx, sample, Word16(16384));
            acc = l_msu(ctx, acc, previous, Word16(PREEMPH_MU));
            acc = l_shl(ctx, acc, q_new);
            speech[NEW_SPEECH_OFFSET + i] = round(ctx, acc);
        }
        speech
    }

    /// `Residu(p_Aq, M, &speech[i_subfr], &exc[i_subfr], L_SUBFR)`.
    fn lp_residual(
        ctx: &mut DspContext,
        speech: &[Word16; L_TOTAL],
        subframe: usize,
        a: &[Word16],
    ) -> [Word16; L_SUBFR] {
        let mut residual = [Word16(0); L_SUBFR];
        for (i, slot) in residual.iter_mut().enumerate() {
            let at = SPEECH_OFFSET + subframe * L_SUBFR + i;
            let mut acc = l_mult(ctx, speech[at], a[0]);
            for (j, &coefficient) in a.iter().enumerate().skip(1) {
                acc = l_mac(ctx, acc, coefficient, speech[at - j]);
            }
            let acc = l_shl(ctx, acc, 4);
            *slot = round(ctx, acc);
        }
        residual
    }

    /// Drive the module over the three committed frames.
    ///
    /// Everything this module does not own comes from the trace: the target
    /// `xn`, the impulse response `h1`, the decimated weighted speech `wsp`,
    /// the quantised LP `Aq`, the unquantised ISFs, the quantised pitch gain
    /// and — crucially — the *final* excitation of every completed subframe.
    /// Handing the reference's own excitation back keeps one subframe's error
    /// from poisoning the next, so a failure names the subframe that caused it.
    ///
    /// The one thing that has to be rebuilt rather than read is this subframe's
    /// LP residual: the closed-loop search genuinely reads it, because a lag
    /// shorter than a subframe reaches into samples the adaptive codebook has
    /// not produced yet, and no trace row carries it. It is rebuilt from
    /// `decimated`, `Q_new` and `Aq` — pre-emphasis, scaling, then `Residu` —
    /// and checked against `cn` by its own test.
    fn run_trace() -> Run {
        let mode = PitchMode::from_frame_bits(TRACE_BITS);
        let resolution = mode.lag_resolution();
        let mut ctx = DspContext::default();

        let mut open = OpenLoopPitch::new();
        let mut clipping = GainClipping::new();

        // Encoder state the harness has to carry because the reference does,
        // at the values `Reset_encoder(st, 1)` leaves.
        let mut speech_carry = [Word16(0); SPEECH_CARRY];
        let mut preemph_memory = Word16(0);
        let mut q_old: i16 = 15;
        let mut exc_carry = [Word16(0); EXC_HISTORY];

        let mut medians = Vec::new();
        let mut outcomes = Vec::new();

        for frame in 0..TRACE_FRAMES {
            let q_new = scalar(frame, -1, "Q_new");
            let shift = scalar(frame, -1, "wsp_shift");
            let q_exp = q_new - q_old;
            q_old = q_new;

            // --- rebuild `speech[]`: pre-emphasise, scale, keep the history.
            let decimated = row(frame, -1, "decimated");
            let speech = rebuild_speech(
                &mut ctx,
                &decimated,
                &speech_carry,
                preemph_memory,
                (q_new, q_exp),
            );
            preemph_memory = decimated[decimated.len() - 1];
            speech_carry.copy_from_slice(&speech[256..256 + SPEECH_CARRY]);

            // --- the clipping tracker sees the unquantised ISFs first.
            clipping.observe_isf(&mut ctx, mode, &row(frame, -1, "isf_unq46"));

            // --- open loop.
            open.rescale(&mut ctx, q_exp, shift);
            let lags = open.analyse(&mut ctx, &row(frame, -1, "wsp"), mode);
            medians.push(lags.smoothed_after_first_half);

            // --- the excitation buffer the closed-loop search reads.
            scale_sig(&mut ctx, &mut exc_carry, q_exp);
            let mut exc = [Word16(0); EXC_TOTAL];
            exc[..EXC_HISTORY].copy_from_slice(&exc_carry);

            let quantised_lp = row(frame, -1, "Aq");
            let mut window = LagWindow::around(&mut ctx, lags.first_half);

            for subframe in 0..SUBFRAMES {
                let index = i32::try_from(subframe).expect("subframe");
                let base = EXC_HISTORY + subframe * L_SUBFR;
                let absolute =
                    subframe == 0 || (subframe == 2 && mode.third_subframe_is_absolute());
                if subframe == 2 && mode.third_subframe_is_absolute() {
                    window = LagWindow::around(&mut ctx, lags.second_half);
                }

                let residual = lp_residual(
                    &mut ctx,
                    &speech,
                    subframe,
                    &quantised_lp[subframe * (M + 1)..(subframe + 1) * (M + 1)],
                );
                exc[base..base + L_SUBFR].copy_from_slice(&residual);

                let target = row(frame, index, "xn");
                let response = row(frame, index, "h1");

                let (lag, fraction) = closed_loop_lag(
                    &mut ctx,
                    &exc,
                    base,
                    &target,
                    &response,
                    SearchLimits {
                        window,
                        absolute,
                        resolution,
                    },
                );
                if absolute {
                    window = LagWindow::around(&mut ctx, lag);
                }

                let clip = clipping.clips(&mut ctx, mode);
                predict_adaptive(&mut exc, base, lag, fraction, L_SUBFR + 1);
                let adaptive: [Word16; L_SUBFR] =
                    exc[base..base + L_SUBFR].try_into().expect("one subframe");

                let decision = choose_ltp(&mut ctx, &mut exc, base, &target, &response, clip, mode);

                // `Updt_tar(cn, cn, &exc[i_subfr], gain_pit, L_SUBFR)` then
                // `Scale_sig(cn, L_SUBFR, shift)`, over the half of `cn[]` that
                // is the raw residual.
                let winner: [Word16; L_SUBFR] =
                    exc[base..base + L_SUBFR].try_into().expect("one subframe");
                let mut updated = update_target(&mut ctx, &residual, &winner, decision.gain());
                scale_sig(&mut ctx, &mut updated, shift);
                let codebook_residual: [Word16; L_SUBFR / 2] =
                    updated[L_SUBFR / 2..].try_into().expect("half a subframe");

                outcomes.push(Outcome {
                    frame,
                    subframe,
                    lag,
                    fraction,
                    window,
                    adaptive,
                    decision,
                    codebook_residual,
                });

                // Hand the reference's own excitation back, so one subframe's
                // error cannot be inherited by the next.
                let final_excitation = row(frame, index, "exc_total");
                exc[base..base + L_SUBFR].copy_from_slice(&final_excitation);
                clipping.observe_gain(&mut ctx, mode, Word16(scalar(frame, index, "gain_pit")));
            }

            exc_carry.copy_from_slice(&exc[256..256 + EXC_HISTORY]);
        }

        Run { medians, outcomes }
    }

    fn compare(name: &str, outcome: &Outcome, got: &[Word16]) {
        let want = row(
            outcome.frame,
            i32::try_from(outcome.subframe).expect("subframe"),
            name,
        );
        assert_eq!(want.len(), got.len(), "{name}: length");
        for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                g.0, w.0,
                "frame {} subframe {}: {name}[{i}] = {} but TS 26.173 gives {}",
                outcome.frame, outcome.subframe, g.0, w.0
            );
        }
    }

    #[test]
    fn the_open_loop_median_is_bit_exact_against_ts26173() {
        // `T_op_med` is traced only where the reference took the strong-
        // correlation branch, so its *absence* is data too: a frame where this
        // module refreshed the median and the reference did not is a bug in the
        // 0.6 test, i.e. in `Hp_wsp` or the normalised correlation.
        let run = run_trace();
        assert_eq!(run.medians.len(), TRACE_FRAMES, "expected every frame");

        let mut refreshed = 0;
        for (frame, got) in run.medians.iter().enumerate() {
            let want = maybe_row(frame, -1, "T_op_med")
                .map(|v| i16::try_from(v[0]).expect("a decimated lag fits in 16 bits"));
            assert_eq!(
                *got, want,
                "frame {frame}: smoothed open-loop lag {got:?} against TS 26.173's {want:?}"
            );
            if want.is_some() {
                refreshed += 1;
            }
        }
        assert_eq!(
            refreshed, 2,
            "the committed trace refreshes the median on two of its three frames"
        );
    }

    #[test]
    fn the_closed_loop_lag_and_fraction_are_bit_exact_against_ts26173() {
        let run = run_trace();
        assert_eq!(
            run.outcomes.len(),
            TRACE_FRAMES * SUBFRAMES,
            "expected every subframe"
        );
        for outcome in &run.outcomes {
            let index = i32::try_from(outcome.subframe).expect("subframe");
            assert_eq!(
                outcome.lag,
                scalar(outcome.frame, index, "T0"),
                "frame {} subframe {}: integer lag",
                outcome.frame,
                outcome.subframe
            );
            assert_eq!(
                outcome.fraction,
                scalar(outcome.frame, index, "T0_frac"),
                "frame {} subframe {}: fraction",
                outcome.frame,
                outcome.subframe
            );
        }
    }

    #[test]
    fn the_lag_window_is_bit_exact_against_ts26173() {
        // The trace samples `T0_min`/`T0_max` after the absolute subframes have
        // already narrowed them, which is exactly the window subframes 2 and 4
        // then search.
        let run = run_trace();
        let mut compared = 0;
        for outcome in &run.outcomes {
            let index = i32::try_from(outcome.subframe).expect("subframe");
            assert_eq!(
                (outcome.window.min, outcome.window.max),
                (
                    scalar(outcome.frame, index, "T0_min"),
                    scalar(outcome.frame, index, "T0_max")
                ),
                "frame {} subframe {}: lag window",
                outcome.frame,
                outcome.subframe
            );
            assert_eq!(
                outcome.window.max - outcome.window.min,
                15,
                "the window is always 16 integer lags wide"
            );
            compared += 1;
        }
        assert_eq!(
            compared,
            TRACE_FRAMES * SUBFRAMES,
            "expected every subframe"
        );
    }

    #[test]
    fn the_adaptive_codebook_vector_is_bit_exact_against_ts26173() {
        let run = run_trace();
        assert_eq!(
            run.outcomes.len(),
            TRACE_FRAMES * SUBFRAMES,
            "expected every subframe"
        );
        for outcome in &run.outcomes {
            compare("adapt", outcome, &outcome.adaptive);
        }
    }

    #[test]
    fn the_filtered_adaptive_vector_is_bit_exact_against_ts26173() {
        // `y1` is traced before the choice is applied, so it is always the
        // unfiltered candidate's response.
        let run = run_trace();
        assert_eq!(
            run.outcomes.len(),
            TRACE_FRAMES * SUBFRAMES,
            "expected every subframe"
        );
        for outcome in &run.outcomes {
            compare("y1", outcome, &outcome.decision.sharp_response);
        }
    }

    #[test]
    fn the_two_candidate_gains_are_bit_exact_against_ts26173() {
        let run = run_trace();
        let mut compared = 0;
        for outcome in &run.outcomes {
            let index = i32::try_from(outcome.subframe).expect("subframe");
            assert_eq!(
                outcome.decision.sharp_gain.0,
                scalar(outcome.frame, index, "gain1"),
                "frame {} subframe {}: unfiltered pitch gain",
                outcome.frame,
                outcome.subframe
            );
            assert_eq!(
                outcome.decision.smooth_gain.0,
                scalar(outcome.frame, index, "gain2"),
                "frame {} subframe {}: filtered pitch gain",
                outcome.frame,
                outcome.subframe
            );
            compared += 1;
        }
        assert_eq!(
            compared,
            TRACE_FRAMES * SUBFRAMES,
            "expected every subframe"
        );

        // The committed frames exercise both edges of `G_pitch`: frame 0
        // subframe 1 takes the negative-correlation early return, and subframe
        // 2 lands on the 1.2 clamp. A gain path that got either wrong would
        // still look plausible.
        assert_eq!(
            scalar(0, 1, "gain1"),
            0,
            "the trace no longer covers the zero gain"
        );
        assert_eq!(
            scalar(0, 2, "gain1"),
            19661,
            "the trace no longer covers the 1.2 clamp"
        );
    }

    #[test]
    fn the_ltp_low_pass_choice_is_bit_exact_against_ts26173() {
        let run = run_trace();
        let mut compared = 0;
        let mut sharp = 0;
        for outcome in &run.outcomes {
            let index = i32::try_from(outcome.subframe).expect("subframe");
            let want = scalar(outcome.frame, index, "select") == 1;
            assert_eq!(
                outcome.decision.prefer_sharp, want,
                "frame {} subframe {}: LTP low-pass choice",
                outcome.frame, outcome.subframe
            );
            sharp += usize::from(want);
            compared += 1;
        }
        assert_eq!(
            compared,
            TRACE_FRAMES * SUBFRAMES,
            "expected every subframe"
        );
        // Both branches occur, so neither a hard-wired 0 nor a hard-wired 1
        // could pass this test.
        assert!(
            sharp > 0 && sharp < compared,
            "the committed trace should exercise both LTP candidates"
        );
    }

    #[test]
    fn the_reconstructed_residual_agrees_with_the_traced_codebook_target() {
        // The harness rebuilds this subframe's LP residual because the search
        // reads it and no trace row carries it. `cn[]`'s second half *is* that
        // residual, minus the chosen adaptive contribution and scaled — so if
        // this matches, a closed-loop failure is this module's and not the
        // harness's.
        let run = run_trace();
        let mut compared = 0;
        for outcome in &run.outcomes {
            let want = row(
                outcome.frame,
                i32::try_from(outcome.subframe).expect("subframe"),
                "cn",
            );
            for (i, &got) in outcome.codebook_residual.iter().enumerate() {
                assert_eq!(
                    got.0,
                    want[L_SUBFR / 2 + i].0,
                    "frame {} subframe {}: cn[{}]",
                    outcome.frame,
                    outcome.subframe,
                    L_SUBFR / 2 + i
                );
            }
            compared += 1;
        }
        assert_eq!(
            compared,
            TRACE_FRAMES * SUBFRAMES,
            "expected every subframe"
        );
    }

    #[test]
    fn the_open_loop_search_gives_a_tie_to_the_shorter_lag() {
        // Silence makes every candidate's weighted correlation exactly zero, so
        // all 98 tie. The reference visits descending and compares
        // non-strictly, so the *last* one visited wins: decimated lag 18, the
        // smallest the search can return. A `>` comparison would return 115.
        let mut ctx = DspContext::default();
        let mut open = OpenLoopPitch::new();
        let lags = open.analyse(
            &mut ctx,
            &[Word16(0); WSP_FRAME],
            PitchMode::from_frame_bits(TRACE_BITS),
        );
        assert_eq!(
            lags.first_half, 36,
            "an all-ties open-loop search must return the shortest reachable lag"
        );
    }

    #[test]
    fn the_open_loop_search_never_returns_its_lower_bound() {
        // The loop is `for (i = L_max; i > L_min; i--)`, so decimated lag 17 —
        // real lag 34, `PIT_MIN` — is unreachable. The all-ties case is the one
        // that would expose a `>=` bound.
        let mut ctx = DspContext::default();
        let mut open = OpenLoopPitch::new();
        let lags = open.analyse(
            &mut ctx,
            &[Word16(0); WSP_FRAME],
            PitchMode::from_frame_bits(TRACE_BITS),
        );
        assert_ne!(
            lags.first_half, PIT_MIN,
            "PIT_MIN is not an open-loop result"
        );
    }

    /// A closed-loop search over the given window in which every integer
    /// candidate correlates identically: silence gives each delay the same
    /// seeded accumulator and therefore the same normalised correlation.
    fn all_ties_closed_loop(window: LagWindow, absolute: bool) -> (i16, i16) {
        let mut ctx = DspContext::default();
        let excitation = [Word16(0); EXC_TOTAL];
        let target = [Word16(0); L_SUBFR];
        let response = [Word16(0); L_SUBFR];
        closed_loop_lag(
            &mut ctx,
            &excitation,
            EXC_HISTORY,
            &target,
            &response,
            SearchLimits {
                window,
                absolute,
                resolution: LagResolution::NINE_BIT,
            },
        )
    }

    #[test]
    fn the_closed_loop_integer_search_gives_a_tie_to_the_longer_lag() {
        // Ascending visit, non-strict comparison: the *last* candidate wins.
        // The window sits above `PIT_FR1_9b` on an absolute subframe, so the
        // whole-resolution early return fires and the answer is the integer
        // search's alone — nothing the fractional stage does can move it.
        // A `>` comparison would return 200.
        let (lag, fraction) = all_ties_closed_loop(LagWindow { min: 200, max: 215 }, true);
        assert_eq!(
            (lag, fraction),
            (215, 0),
            "an all-ties integer search must keep the longest lag in the window"
        );
    }

    #[test]
    fn the_closed_loop_fractional_search_gives_a_tie_to_the_smallest_fraction() {
        // A constant correlation is *not* a constant interpolation: the eight
        // taps a phase reads do not sum to the same value for every phase.
        // Fractions -2 and +2 read the same phase, so they tie exactly at the
        // maximum, and the strict comparison keeps the earlier — negative —
        // one. Normalising that borrows a whole sample, so the answer is
        // `(55 - 1, -2 + 4)`. A non-strict comparison would keep +2 instead and
        // return `(55, 2)`: same fraction, different lag.
        let (lag, fraction) = all_ties_closed_loop(LagWindow { min: 40, max: 55 }, false);
        assert_eq!(
            (lag, fraction),
            (54, 2),
            "the fractional search must keep the earlier of two equal phases"
        );
    }

    #[test]
    fn a_lag_on_the_window_floor_cannot_search_below_it() {
        // `if (t0 == t0_min) fraction = 0` — without it the fractional stage
        // could push the lag under the window and the relative pitch index
        // would underflow. The all-ties case lands on the window's *top*, so
        // this needs a correlation that peaks on the floor instead: a target
        // that is periodic at exactly the floor lag, with the same waveform
        // sitting one floor-lag back in the excitation.
        let mut ctx = DspContext::default();
        let mut excitation = [Word16(0); EXC_TOTAL];
        let window = LagWindow { min: 40, max: 55 };
        let mut target = [Word16(0); L_SUBFR];
        let mut response = [Word16(0); L_SUBFR];
        response[0] = Word16(16384);
        for (i, slot) in target.iter_mut().enumerate() {
            *slot = Word16(if i % 40 == 0 { 4000 } else { 0 });
            excitation[EXC_HISTORY - 40 + i] = *slot;
        }
        let (lag, fraction) = closed_loop_lag(
            &mut ctx,
            &excitation,
            EXC_HISTORY,
            &target,
            &response,
            SearchLimits {
                window,
                absolute: false,
                resolution: LagResolution::NINE_BIT,
            },
        );
        assert_eq!(
            lag, window.min,
            "the correlation should peak on the window floor"
        );
        assert!(
            (0..=3).contains(&fraction),
            "fraction {fraction} out of range"
        );
    }

    #[test]
    fn the_lag_window_clamps_at_both_ends() {
        let mut ctx = DspContext::default();
        assert_eq!(
            LagWindow::around(&mut ctx, PIT_MIN),
            LagWindow {
                min: PIT_MIN,
                max: PIT_MIN + 15
            },
            "a short lag clamps to PIT_MIN"
        );
        assert_eq!(
            LagWindow::around(&mut ctx, PIT_MAX),
            LagWindow {
                min: PIT_MAX - 15,
                max: PIT_MAX
            },
            "a long lag clamps to PIT_MAX and pulls the floor down with it"
        );
        assert_eq!(
            LagWindow::around(&mut ctx, 100),
            LagWindow { min: 92, max: 107 },
            "an interior lag sits 8 below and 7 above"
        );
    }

    #[test]
    fn the_lag_history_only_shifts_on_a_strong_correlation() {
        // `Med_olag` shifts `old_ol_lag[]` in place, and the reference only
        // calls it when the normalised correlation clears 0.6. A port that
        // shifted unconditionally would drift the median by one frame.
        let mut ctx = DspContext::default();
        let mut open = OpenLoopPitch::new();

        open.gain = Word16(19661); // exactly 0.6: the reference tests strict >
        assert_eq!(
            open.absorb(&mut ctx, 77),
            None,
            "0.6 exactly must not refresh"
        );
        assert_eq!(
            open.recent_lags,
            [Word16(40); 5],
            "the history moved anyway"
        );

        open.gain = Word16(19662);
        assert_eq!(
            open.absorb(&mut ctx, 77),
            Some(40),
            "one new lag among four 40s"
        );
        assert_eq!(
            open.recent_lags,
            [Word16(77), Word16(40), Word16(40), Word16(40), Word16(40)],
            "the history should have shifted exactly once"
        );
    }

    #[test]
    fn the_confidence_decays_to_zero_and_stays_there() {
        // `ada_w = mult(ada_w, 29491)` floors, so it never rounds back up.
        let mut ctx = DspContext::default();
        let mut open = OpenLoopPitch::new();
        open.gain = Word16(0);
        open.confidence = Word16(32767);
        for _ in 0..200 {
            open.absorb(&mut ctx, 50);
        }
        assert_eq!(
            open.confidence.0, 0,
            "the confidence should have floored to 0"
        );
        assert!(
            !open.weight_towards_history,
            "weighting should be off at zero"
        );
    }

    #[test]
    fn the_clipping_threshold_uses_integer_division_by_the_isf_maximum() {
        // `16384 / DIST_ISF_MAX_IO` is evaluated by the C compiler as 42, not
        // 42.67. At the widest gap that puts the threshold at exactly 16358;
        // rounding to 43 would put it at 16396 and the boundary would move.
        let mut ctx = DspContext::default();
        let mode = PitchMode::from_frame_bits(BITS_8K85);
        let mut clipping = GainClipping::new();
        clipping.isf_gap = Word16(384);

        clipping.mean_gain = Word16(16358);
        assert!(
            !clipping.clips(&mut ctx, mode),
            "the threshold is a strict >"
        );
        clipping.mean_gain = Word16(16359);
        assert!(
            clipping.clips(&mut ctx, mode),
            "one above the threshold must clip"
        );
    }

    #[test]
    fn the_wideband_clipping_test_needs_both_halves() {
        let mut ctx = DspContext::default();
        let mode = PitchMode::from_frame_bits(TRACE_BITS);
        let mut clipping = GainClipping::new();

        clipping.isf_gap = Word16(153);
        clipping.mean_gain = Word16(14747);
        assert!(
            clipping.clips(&mut ctx, mode),
            "resonant and predictive must clip"
        );

        clipping.isf_gap = Word16(154);
        assert!(
            !clipping.clips(&mut ctx, mode),
            "the gap test is a strict <"
        );

        clipping.isf_gap = Word16(153);
        clipping.mean_gain = Word16(14746);
        assert!(
            !clipping.clips(&mut ctx, mode),
            "the gain test is a strict >"
        );
    }

    #[test]
    fn the_clipping_tracker_resets_the_same_way_for_every_mode() {
        // `Init_gp_clip` writes 307 even for the modes that later clamp to 384.
        let fresh = GainClipping::new();
        assert_eq!((fresh.isf_gap.0, fresh.mean_gain.0), (307, 9830));
    }

    #[test]
    fn the_correlation_weighting_table_matches_the_reference_shape() {
        // Machine-extracted from `p_med_ol.tab`; these are the landmarks that
        // would move if a row were dropped or a value mistyped.
        assert_eq!(CORR_WEIGHT.len(), 199);
        assert_eq!(CORR_WEIGHT[0], 10772);
        assert_eq!(CORR_WEIGHT[198], 10728);
        assert!(
            CORR_WEIGHT[95..=101].iter().all(|&v| v == 16384),
            "the bump's plateau is indices 95..=101"
        );
        assert_eq!(CORR_WEIGHT[94], 16056);
        assert_eq!(CORR_WEIGHT[102], 16056);
        // The taper the search actually reads, `198 - 115 + lag`, must favour
        // short lags: index 101 at lag 18 down to index 198 at lag 115.
        assert!(CORR_WEIGHT[101] > CORR_WEIGHT[198]);
    }

    #[test]
    fn the_interpolation_filter_matches_the_reference_shape() {
        assert_eq!(INTER4_1.len(), 32);
        assert_eq!(INTER4_1[15], 14746, "the peak tap sits at phase 3 of tap 3");
        assert_eq!(INTER4_1[31], 0, "the last tap is the padding zero");
        assert_eq!(INTER4_1[0], -12);
    }

    #[test]
    fn the_mode_predicates_split_where_the_reference_does() {
        let narrow = PitchMode::from_frame_bits(BITS_6K60);
        let second = PitchMode::from_frame_bits(BITS_8K85);
        let wide = PitchMode::from_frame_bits(TRACE_BITS);
        let sid = PitchMode::from_frame_bits(35);

        assert!(narrow.open_loop_spans_frame());
        assert!(!second.open_loop_spans_frame());
        // A comfort-noise frame carries 35 bits, which is neither of the two
        // budgets tested for equality, so it takes the wideband branches.
        assert!(!sid.open_loop_spans_frame());
        assert!(!sid.interoperable_clipping());

        assert!(!narrow.has_sharp_candidate());
        assert!(!second.has_sharp_candidate());
        assert!(wide.has_sharp_candidate());
        assert!(!narrow.third_subframe_is_absolute());
        assert!(second.third_subframe_is_absolute());

        assert!(narrow.interoperable_clipping());
        assert!(second.interoperable_clipping());
        assert!(!wide.interoperable_clipping());

        assert_eq!(narrow.lag_resolution(), LagResolution::EIGHT_BIT);
        assert_eq!(wide.lag_resolution(), LagResolution::NINE_BIT);
    }
}
