//! AMR-WB encoder: the algebraic codebook search and its index packing.
//!
//! Implements, from 3GPP TS 26.173 (the TS 26.190 §6.8 fixed-point definition):
//! `cor_h_x()` (`cor_h_x.c`), `Pit_shrp()` (`pit_shrp.c`), `ACELP_2t64_fx()`
//! (`c2t64fx.c`), `ACELP_4t64_fx()` with its two file-static helpers
//! `cor_h_vec()` and `search_ixiy()` (`c4t64fx.c`), and all seven combinatorial
//! packers of `q_pulse.c`.
//!
//! # Why every comparison here is load-bearing
//!
//! This is a *search*, not a formula. An implementation that picks a
//! different-but-equally-good candidate produces perfectly plausible speech and
//! a different bitstream, so the parts that have to be reproduced exactly are
//! the objective at each nesting level, the comparison operator (`> 0`
//! throughout — strictly greater, so ties keep the *earlier* candidate), the
//! order positions are visited in (ascending sample index, always), and the
//! adaptive candidate set for the outer pulse of each stage. Two arithmetic
//! quirks are part of the decision rather than artefacts of it:
//!
//! * `mult(x, 32767) == x - 1` for `x > 0` (and `== x` for `x <= 0`). The
//!   reference folds pulse signs into the cross-correlation table with
//!   `mult(v, ±32767/−32768)`, so a "positive" sign is not the identity. Turning
//!   it into a sign flip changes which pair wins in near-ties.
//! * `mult(ps, ps)` is an arithmetic `>>15` that floors, so any `|ps| < 182`
//!   collapses the numerator to zero and the candidate can never beat a
//!   positive incumbent.
//!
//! # Validation
//!
//! The committed tests here run at 12.65 kbit/s, which is the rate of the
//! committed trace, and therefore cover the 36-bit budget only. They are
//! bit-exact against the TS 26.173 encoder's own per-subframe trace
//! (`testdata/wb_enc_trace.txt`) for the filtered codeword `y2` and — after the
//! caller's pre-emphasis and pitch sharpening — the codeword `code`, and
//! against the reference encoder's committed bitstream
//! (`testdata/amrwb_enc_mode2.amr`) for the four packed pulse indices of every
//! subframe.
//!
//! The other seven codebooks are covered here by round-tripping the real
//! search's own indices through this crate's already bit-exact decoder at every
//! budget, which pins each pulse's track, position and sign.
//!
//! Offline — with 50-frame traces at every rate from
//! `tools/trace-amrwb-encoder.sh`, which are not committed — the two-pulse
//! search and all seven four-track budgets were checked bit-exact over 200
//! subframes each for `y2`, `code` and the packed indices. Rerun that sweep
//! before trusting any change to the search.

// The reference's variable names are the specification's vocabulary — `k_cn`
// and `k_dn`, `g_pitch` and `g2_pitch` — and renaming them to satisfy the
// similar-names heuristic would make this module harder, not easier, to check
// against TS 26.173. Same exemption the `fixed_point` subtree takes, and for
// the same reason.
#![allow(clippy::similar_names)]

use crate::codecs::amr::wb::math::{dot_product12, isqrt_n};
use crate::fixed_point::arith::{add, extract_h, mult, mult_r, negate, round, sub};
use crate::fixed_point::arith32::{l_abs, l_add, l_deposit_h, l_mac, l_msu, l_mult, l_sub};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, shr, shr_r};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Samples in a subframe.
pub const L_SUBFR: usize = 64;

/// Pitch sharpening factor, 0.85 in Q15 (`PIT_SHARP` in `cnst.h`).
pub const PITCH_SHARP: Word16 = Word16(27853);

/// Positions per track in the four-track layout.
const NB_POS: usize = 16;

/// Tracks in the four-track layout.
const NB_TRACK: usize = 4;

/// Candidate positions pre-selected per track (`NB_MAX`).
const NB_MAX: i16 = 8;

/// Slots reserved per track in the pulse table (`NPMAXPT`), enough for the
/// 24-pulse mode's six pulses on one track.
const SLOTS_PER_TRACK: usize = 6;

/// Largest pulse count over all modes (`NB_PULSE_MAX`).
const MAX_PULSES: usize = 24;

/// Bit that carries a pulse's sign in a packed position (`NB_POS` used as a
/// mask in `q_pulse.c`). It is the literal 16 at every recursion depth — it
/// does *not* scale with the packer's `N`.
const SIGN_BIT: i32 = 16;

/// Starting track for each pulse of each iteration (`tipos` in `c4t64fx.c`).
///
/// Iteration `k` reads from offset `4k`, so it begins the cyclic track rotation
/// at track `k`. Every consecutive pair is `(t, (t+1) mod 4)`, which is what
/// makes [`Innovation`]'s cross-correlation table — indexed by the *first*
/// track of the pair — the right one to consult.
const TRACK_ROTATION: [usize; 36] = [
    0, 1, 2, 3, //
    1, 2, 3, 0, //
    2, 3, 0, 1, //
    3, 0, 1, 2, //
    0, 1, 2, 3, //
    1, 2, 3, 0, //
    2, 3, 0, 1, //
    3, 0, 1, 2, //
    0, 1, 2, 3,
];

/// How many bits one subframe spends on pulse positions and signs.
///
/// The 12-bit two-pulse codebook of 6.60 kbit/s is not in this enum: it has a
/// different structure and its own entry point, [`search_two_pulse`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PulseBudget {
    /// 20 bits, 4 pulses (8.85 kbit/s).
    B20,
    /// 36 bits, 8 pulses (12.65 kbit/s).
    B36,
    /// 44 bits, 10 pulses (14.25 kbit/s).
    B44,
    /// 52 bits, 12 pulses (15.85 kbit/s).
    B52,
    /// 64 bits, 16 pulses (18.25 kbit/s).
    B64,
    /// 72 bits, 18 pulses (19.85 kbit/s).
    B72,
    /// 88 bits, 24 pulses (23.05 and 23.85 kbit/s).
    B88,
}

impl PulseBudget {
    /// Bits spent on the algebraic codebook in one subframe.
    #[must_use]
    pub const fn bits(self) -> usize {
        match self {
            Self::B20 => 20,
            Self::B36 => 36,
            Self::B44 => 44,
            Self::B52 => 52,
            Self::B64 => 64,
            Self::B72 => 72,
            Self::B88 => 88,
        }
    }

    /// Pulses placed in one subframe.
    #[must_use]
    pub const fn pulses(self) -> usize {
        match self {
            Self::B20 => 4,
            Self::B36 => 8,
            Self::B44 => 10,
            Self::B52 => 12,
            Self::B64 => 16,
            Self::B72 => 18,
            Self::B88 => 24,
        }
    }

    /// Packed indices this budget produces, i.e. how much of
    /// [`Innovation::indices`] is meaningful.
    #[must_use]
    pub const fn index_count(self) -> usize {
        match self {
            Self::B20 | Self::B36 | Self::B44 | Self::B52 => 4,
            Self::B64 | Self::B72 | Self::B88 => 8,
        }
    }

    /// The budget a frame of `frame_bits` bits uses, or `None` at 6.60 kbit/s
    /// where the two-pulse codebook applies instead.
    ///
    /// The ladder is TS 26.173 `cod_main.c`'s: each rung is "at most this many
    /// bits per frame", so an unknown larger size falls through to 88.
    #[must_use]
    pub const fn from_frame_bits(frame_bits: usize) -> Option<Self> {
        match frame_bits {
            0..=132 => None,
            133..=177 => Some(Self::B20),
            178..=253 => Some(Self::B36),
            254..=285 => Some(Self::B44),
            286..=317 => Some(Self::B52),
            318..=365 => Some(Self::B64),
            366..=397 => Some(Self::B72),
            _ => Some(Self::B88),
        }
    }

    /// Iterations of the depth-first search.
    ///
    /// Only 23.85 kbit/s (477 bits) drops to a single iteration; 23.05 (461)
    /// still gets two. The reference tests `ser_size > 462`.
    const fn iterations(self, frame_bits: usize) -> usize {
        match self {
            Self::B20 | Self::B36 | Self::B44 | Self::B52 => 4,
            Self::B64 | Self::B72 => 3,
            Self::B88 => {
                if frame_bits > 462 {
                    1
                } else {
                    2
                }
            }
        }
    }

    /// Weight, Q12, given to the correlation term when it is mixed with the
    /// LTP residual to decide pulse signs. Falls as the pulse count rises.
    const fn mix_weight(self) -> Word16 {
        match self {
            Self::B20 => Word16(8192),
            Self::B36 | Self::B44 | Self::B52 => Word16(4096),
            Self::B64 => Word16(3277),
            Self::B72 => Word16(3072),
            Self::B88 => Word16(2048),
        }
    }

    /// Candidate positions tried for the *outer* pulse of each two-pulse stage.
    ///
    /// The inner pulse always tries all 16 positions of its track; the outer one
    /// is restricted to the `n` best-ranked positions of its own track, which is
    /// what keeps the search affordable as the pulse count grows.
    const fn stage_widths(self) -> &'static [i16] {
        match self {
            Self::B20 => &[4, 8],
            Self::B36 => &[4, 8, 8],
            Self::B44 | Self::B52 => &[4, 6, 8, 8],
            Self::B64 => &[4, 4, 6, 6, 8, 8],
            Self::B72 => &[2, 3, 4, 5, 6, 7, 8],
            Self::B88 => &[2, 2, 3, 4, 5, 6, 7, 8, 8, 8],
        }
    }

    /// Pulses fixed by the first stage, before the two-at-a-time stages begin.
    const fn first_stage_pulses(self) -> usize {
        match self {
            Self::B20 => 0,
            Self::B36 | Self::B44 => 2,
            Self::B52 | Self::B64 | Self::B72 | Self::B88 => 4,
        }
    }
}

/// One subframe's algebraic excitation, as chosen by the four-track search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Innovation {
    /// Algebraic excitation. Nominally Q9 with pulses of ±512, but scaled down
    /// by `2^-impulse_shift` — the gain quantiser absorbs the difference.
    pub code: [Word16; L_SUBFR],
    /// Packed per-track indices; only the first
    /// [`PulseBudget::index_count`] entries are meaningful.
    pub indices: [u16; 8],
    /// Chosen pulse positions as sample indices, in the order the search
    /// produced them. Two pulses may share a sample.
    pub pulses: [i16; MAX_PULSES],
    /// Per-track pulse slots, six per track: `position + 16` when the pulse is
    /// negative, plain `position` when positive, `-1` for an unused slot. This
    /// is what the packers consume.
    pub slots: [i16; MAX_PULSES],
    /// Right shift applied to the impulse response before the search, 0, 1 or 2.
    pub impulse_shift: i16,
}

/// One subframe's algebraic excitation from the 12-bit two-pulse codebook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TwoPulseInnovation {
    /// Algebraic excitation, Q9, with exactly two pulses of ±512.
    pub code: [Word16; L_SUBFR],
    /// Filtered excitation, Q9.
    pub filtered: [Word16; L_SUBFR],
    /// The 12-bit index: sign and position of the even-track pulse in bits
    /// 11..6, sign and position of the odd-track pulse in bits 5..0.
    pub index: u16,
    /// Sample positions of the two pulses: one even, one odd.
    pub positions: [usize; 2],
    /// Whether each pulse is positive.
    pub positive: [bool; 2],
}

/// Pitch sharpening: fold a delayed copy of `x` back into itself.
///
/// Applied to the impulse response before the search and to the codeword
/// after it, with `sharp` = [`PITCH_SHARP`]. `x` keeps whatever Q the caller
/// uses (Q12 for the impulse response, Q9 for the codeword).
///
/// This is an IIR, not an FIR: once `i >= 2 * pitch_lag` the tap reads a sample
/// this same loop has already rewritten. Filtering out of place — copying the
/// input first — gives a different, plausible-sounding result.
///
/// A `pitch_lag` at or beyond the subframe length leaves `x` unchanged.
pub fn pitch_sharpen(ctx: &mut DspContext, x: &mut [Word16], pitch_lag: usize, sharp: Word16) {
    for i in pitch_lag..x.len() {
        let acc = l_deposit_h(x[i]);
        let acc = l_mac(ctx, acc, x[i - pitch_lag], sharp);
        x[i] = round(ctx, acc);
    }
}

/// Correlate the codebook target with the impulse response (`cor_h_x`).
///
/// `impulse` is Q12, `target` carries the subframe's own scaling, and the
/// result is normalised to under 12 significant bits so that six times the sum
/// of the per-track maxima cannot saturate downstream.
///
/// Both accumulators are seeded with 1 rather than 0. That is not rounding: it
/// guarantees the output is never identically zero, so the normalising
/// `norm_l` never sees a zero argument.
#[must_use]
pub fn correlate_target(
    ctx: &mut DspContext,
    impulse: &[Word16; L_SUBFR],
    target: &[Word16; L_SUBFR],
) -> [Word16; L_SUBFR] {
    let mut raw = [Word32(0); L_SUBFR];
    let mut total = Word32(1);

    for track in 0..NB_TRACK {
        let mut peak = Word32(0);
        let mut i = track;
        while i < L_SUBFR {
            let mut acc = Word32(1);
            for j in i..L_SUBFR {
                acc = l_mac(ctx, acc, target[j], impulse[j - i]);
            }
            raw[i] = acc;
            let magnitude = l_abs(ctx, acc);
            if l_sub(ctx, magnitude, peak).0 > 0 {
                peak = magnitude;
            }
            i += NB_TRACK;
        }
        // total += 3 * peak / 8, as two floor shifts rather than one multiply.
        let quarter = l_shr(ctx, peak, 2);
        total = l_add(ctx, total, quarter);
        let eighth = l_shr(ctx, quarter, 1);
        total = l_add(ctx, total, eighth);
    }

    // Four bits of headroom, enough for sixteen times the running total.
    let scale = norm_l(total) - 4;
    let mut out = [Word16(0); L_SUBFR];
    for (o, &r) in out.iter_mut().zip(raw.iter()) {
        let shifted = l_shl(ctx, r, scale);
        *o = round(ctx, shifted);
    }
    out
}

/// Per-sample sign decisions, and the mixed correlation they came from.
struct SignDecision {
    /// `32767` where the pulse at this sample must be positive, `-32768` where
    /// negative. Used both as a flag and as a multiplicand.
    sign: [Word16; L_SUBFR],
    /// Exactly `-sign` in the saturating sense: `32767 ↔ -32768`.
    inverse: [Word16; L_SUBFR],
    /// The normalised mix `k_cn·cn + k_dn·dn` the decision was taken on.
    mixed: [Word16; L_SUBFR],
}

/// Fix one sign per sample from a normalised mix of the LTP residual and the
/// target correlation, and fold that sign into `dn` in place.
///
/// `rectify_mixed` is the one structural difference between the two codebooks:
/// the four-track search also negates `mixed` so that it is non-negative
/// afterwards, because it goes on to rank positions by it. The two-track
/// search never reads `mixed` again and leaves it signed.
fn decide_signs(
    ctx: &mut DspContext,
    dn: &mut [Word16; L_SUBFR],
    cn: &[Word16; L_SUBFR],
    mix_weight: Word16,
    rectify_mixed: bool,
) -> SignDecision {
    let residual = dot_product12(ctx, cn, cn);
    let (frac, exp) = isqrt_n(ctx, residual);
    // Saturation is reachable here and is part of the specified behaviour.
    let scaled = l_shl(ctx, frac, exp + 5);
    let k_cn = round(ctx, scaled);

    let correlation = dot_product12(ctx, dn, dn);
    let (frac, exp) = isqrt_n(ctx, correlation);
    let scaled = l_shl(ctx, frac, exp + 8);
    let k_dn = round(ctx, scaled);
    let k_dn = mult_r(ctx, mix_weight, k_dn);

    let mut mixed = [Word16(0); L_SUBFR];
    for i in 0..L_SUBFR {
        let residual = l_mult(ctx, k_cn, cn[i]);
        let acc = l_mac(ctx, residual, k_dn, dn[i]);
        let acc = l_shl(ctx, acc, 8);
        mixed[i] = extract_h(acc);
    }

    let mut sign = [Word16(0); L_SUBFR];
    let mut inverse = [Word16(0); L_SUBFR];
    for i in 0..L_SUBFR {
        if mixed[i].0 >= 0 {
            sign[i] = Word16(32767);
            inverse[i] = Word16(-32768);
        } else {
            sign[i] = Word16(-32768);
            inverse[i] = Word16(32767);
            dn[i] = negate(ctx, dn[i]);
            if rectify_mixed {
                mixed[i] = negate(ctx, mixed[i]);
            }
        }
    }

    SignDecision {
        sign,
        inverse,
        mixed,
    }
}

/// Running energy of the impulse response, as a prefix chain.
///
/// `prefix[n]` is `extract_h` of the accumulator after `n + 1` products, so the
/// energy of `h` shifted to sample position `p` is `prefix[63 - p]`. The
/// reference walks the same chain with a descending write pointer; addressing
/// it by truncation point instead is the same sequence of saturating `L_mac`s
/// and therefore the same numbers.
fn energy_prefix(ctx: &mut DspContext, h: &[Word16; L_SUBFR], seed: Word32) -> [Word16; L_SUBFR] {
    let mut acc = seed;
    let mut prefix = [Word16(0); L_SUBFR];
    for (slot, &v) in prefix.iter_mut().zip(h.iter()) {
        acc = l_mac(ctx, acc, v, v);
        *slot = extract_h(acc);
    }
    prefix
}

/// Running cross-correlation of the impulse response with itself at every odd
/// lag, as prefix chains indexed by `(lag - 1) / 2`.
///
/// Two pulses on adjacent tracks are always an odd number of samples apart,
/// which is why only odd lags exist. Within one lag the reference's `cor`
/// accumulator is never reset, so each entry is a prefix of the same chain;
/// walking each lag once costs exactly the reference's 1024 products.
fn cross_prefix(ctx: &mut DspContext, h: &[Word16; L_SUBFR]) -> [[Word16; L_SUBFR]; 32] {
    let mut table = [[Word16(0); L_SUBFR]; 32];
    for (slot, chain) in table.iter_mut().enumerate() {
        let lag = 2 * slot + 1;
        let mut acc = Word32(0x0000_8000);
        for i in 0..(L_SUBFR - lag) {
            acc = l_mac(ctx, acc, h[i], h[i + lag]);
            chain[i] = extract_h(acc);
        }
    }
    table
}

/// The impulse response shifted to `pos` and signed, or zero before `pos`.
///
/// The reference gets the leading zeros from a 64-sample zero prefix in front
/// of both the impulse response and its negation, which is what makes its
/// `h - pos` pointer legal.
#[inline]
const fn tap(
    h: &[Word16; L_SUBFR],
    h_inv: &[Word16; L_SUBFR],
    sign: Word16,
    pos: usize,
    i: usize,
) -> Word16 {
    if i < pos {
        Word16(0)
    } else if sign.0 < 0 {
        h_inv[i - pos]
    } else {
        h[i - pos]
    }
}

/// Search the 12-bit, two-pulse codebook used at 6.60 kbit/s.
///
/// `dn` is the target correlation from [`correlate_target`] and is **modified
/// in place**: the chosen sign is folded into it. `cn` is the LTP residual,
/// `impulse` the Q12 weighted synthesis impulse response. Both pulses are ±512
/// (1.0 in Q9); the filtered codeword is Q9 too.
///
/// All 32 × 32 combinations are tested, in ascending sample order, with the
/// incumbent carried across the whole grid rather than reset per even
/// position. The defaults `(0, 1)` survive if no candidate is ever accepted,
/// which is reachable when the first candidate's energy is negative enough.
///
/// # Panics
///
/// Never: both index fields are six bits by construction.
#[must_use]
pub fn search_two_pulse(
    ctx: &mut DspContext,
    dn: &mut [Word16; L_SUBFR],
    cn: &[Word16; L_SUBFR],
    impulse: &[Word16; L_SUBFR],
) -> TwoPulseInnovation {
    const POSITIONS: usize = 32;

    // 2.0 in Q12. This mode has no mode table; the weight is fixed.
    let signs = decide_signs(ctx, dn, cn, Word16(8192), false);

    // No `h_shift` here: the impulse response is used at its input Q12.
    let h = *impulse;
    let mut h_inv = [Word16(0); L_SUBFR];
    for (inv, &v) in h_inv.iter_mut().zip(h.iter()) {
        *inv = negate(ctx, v);
    }

    // Self energies, halved. The seed is 0x10000, which adds exactly +1 to
    // every result — a deliberate bias, not a rounding constant (the
    // four-track codebook uses 0x8000, which is one).
    let prefix = energy_prefix(ctx, &h, Word32(0x0001_0000));
    let mut self_energy = [[Word16(0); POSITIONS]; 2];
    for (track, row) in self_energy.iter_mut().enumerate() {
        for (n, slot) in row.iter_mut().enumerate() {
            // The halving is what puts the self terms one binade below the
            // cross terms, which is the 2·Phi weighting.
            *slot = shr(ctx, prefix[L_SUBFR - 1 - (2 * n + track)], 1);
        }
    }

    // Cross energies, *not* halved, so the cross term carries twice the weight
    // of the self terms. That is the 2·Phi factor the criterion needs.
    let chains = cross_prefix(ctx, &h);
    let mut cross = [Word16(0); POSITIONS * POSITIONS];
    for n0 in 0..POSITIONS {
        for n1 in 0..POSITIONS {
            let a = 2 * n0;
            let b = 2 * n1 + 1;
            let lag = a.abs_diff(b);
            let last = L_SUBFR - 1 - a.max(b);
            cross[n0 * POSITIONS + n1] = chains[(lag - 1) / 2][last];
        }
    }

    // Fold the sign pair into the cross term, keeping the `mult(x, 32767)`
    // off-by-one for positive `x`.
    for n0 in 0..POSITIONS {
        let i = 2 * n0;
        let table = if signs.sign[i].0 < 0 {
            &signs.inverse
        } else {
            &signs.sign
        };
        for n1 in 0..POSITIONS {
            let j = 2 * n1 + 1;
            let slot = n0 * POSITIONS + n1;
            cross[slot] = mult(ctx, cross[slot], table[j]);
        }
    }

    let mut best_sq = Word16(-1);
    let mut best_energy = Word16(1);
    let mut ix = 0usize;
    let mut iy = 1usize;

    for n0 in 0..POSITIONS {
        let i0 = 2 * n0;
        let corr0 = dn[i0];
        let energy0 = self_energy[0][n0];
        let mut chosen: Option<usize> = None;

        for n1 in 0..POSITIONS {
            let i1 = 2 * n1 + 1;
            let corr = add(ctx, corr0, dn[i1]);
            // Two saturating 16-bit adds, inner first: the reference does not
            // accumulate this in 32 bits.
            let inner = add(ctx, self_energy[1][n1], cross[n0 * POSITIONS + n1]);
            let energy = add(ctx, energy0, inner);
            let sq = mult(ctx, corr, corr);
            let gain = l_mult(ctx, best_energy, sq);
            let s = l_msu(ctx, gain, best_sq, energy);
            if s.0 > 0 {
                best_sq = sq;
                best_energy = energy;
                chosen = Some(i1);
            }
        }

        if let Some(i1) = chosen {
            ix = i0;
            iy = i1;
        }
    }

    let mut code = [Word16(0); L_SUBFR];
    let mut i0 = ix / 2;
    let mut i1 = iy / 2;
    let positive = [signs.sign[ix].0 > 0, signs.sign[iy].0 > 0];

    if positive[0] {
        code[ix] = Word16(512);
    } else {
        code[ix] = Word16(-512);
        i0 += POSITIONS;
    }
    if positive[1] {
        code[iy] = Word16(512);
    } else {
        code[iy] = Word16(-512);
        i1 += POSITIONS;
    }

    let mut filtered = [Word16(0); L_SUBFR];
    for (i, out) in filtered.iter_mut().enumerate() {
        let a = tap(&h, &h_inv, signs.sign[ix], ix, i);
        let b = tap(&h, &h_inv, signs.sign[iy], iy, i);
        // Q12 to Q9, rounding half up toward +infinity.
        let sum = add(ctx, a, b);
        *out = shr_r(ctx, sum, 3);
    }

    TwoPulseInnovation {
        code,
        filtered,
        index: u16::try_from(i0 * 64 + i1).expect("both fields are six bits"),
        positions: [ix, iy],
        positive,
    }
}

/// Correlate the impulse response with the excitation built so far, for every
/// position of one track, and add that track's self energies (`cor_h_vec`).
///
/// The cross term ends up scaled by four relative to a plain sum of products —
/// twice from `L_mac`, twice from the explicit shift — which is the 2·Phi the
/// energy criterion wants. The shift is allowed to saturate.
fn track_correlations(
    ctx: &mut DspContext,
    h: &[Word16; L_SUBFR],
    excitation: &[Word16; L_SUBFR],
    track: usize,
    sign: &[Word16; L_SUBFR],
    self_energy: &[Word16; NB_POS],
) -> [Word16; NB_POS] {
    let mut out = [Word16(0); NB_POS];
    for (n, slot) in out.iter_mut().enumerate() {
        let pos = track + NB_TRACK * n;
        let mut acc = Word32(0);
        for j in pos..L_SUBFR {
            acc = l_mac(ctx, acc, h[j - pos], excitation[j]);
        }
        let acc = l_shl(ctx, acc, 1);
        let corr = round(ctx, acc);
        // `mult` again carries the -1 bias when the sign is +32767.
        let signed = mult(ctx, corr, sign[pos]);
        *slot = add(ctx, signed, self_energy[n]);
    }
    out
}

/// Arguments to one two-pulse stage, grouped so the search reads as one call.
struct StageTables<'a> {
    dn: &'a [Word16; L_SUBFR],
    ranks: &'a [Word16; L_SUBFR],
    cor_x: &'a [Word16; NB_POS],
    cor_y: &'a [Word16; NB_POS],
    cross: &'a [Word16; NB_POS * NB_POS],
}

/// Find the best position for two pulses on adjacent tracks (`search_ixiy`).
///
/// Maximises `(sum of +/-dn)^2 / energy` over the whole pulse set built so far
/// plus the two new ones. The outer pulse is tried only at the `width`
/// best-ranked positions of its track; the inner one at all sixteen. Both are
/// visited in ascending sample order and the acceptance test is strict, so the
/// earliest pair wins a tie.
///
/// The incumbent is reset for every stage — unlike the iteration-level
/// incumbent in [`search_multi_pulse`], which persists. Confusing the two
/// scopes changes the result.
///
/// `correlation` and `energy` are updated in place. `energy` is written
/// unconditionally, so a stage that accepted nothing leaves it at 1.
fn best_pulse_pair(
    ctx: &mut DspContext,
    width: i16,
    track_x: usize,
    track_y: usize,
    correlation: &mut Word16,
    energy: &mut Word16,
    tables: &StageTables<'_>,
) -> (usize, usize) {
    // `ranks` holds `rank - 8` at selected positions and stays >= 0 elsewhere,
    // so this threshold admits exactly the `width` best-ranked positions.
    let threshold = sub(ctx, Word16(width), Word16(NB_MAX));
    let base = l_add(ctx, l_deposit_h(*energy), Word32(0x0000_8000));

    let mut best_sq = Word16(-1);
    let mut best_energy = Word16(1);
    let mut ix = track_x;
    let mut iy = track_y;

    for nx in 0..NB_POS {
        let x = track_x + NB_TRACK * nx;
        let corr_x = add(ctx, *correlation, tables.dn[x]);
        let energy_x = l_mac(ctx, base, tables.cor_x[nx], Word16(4096));

        if sub(ctx, tables.ranks[x], threshold).0 >= 0 {
            continue;
        }

        let mut chosen: Option<usize> = None;
        for ny in 0..NB_POS {
            let y = track_y + NB_TRACK * ny;
            let corr = add(ctx, corr_x, tables.dn[y]);
            let acc = l_mac(ctx, energy_x, tables.cor_y[ny], Word16(4096));
            // The cross term between the two new pulses gets twice the weight
            // of each pulse's own term, as 2·Phi requires.
            let acc = l_mac(ctx, acc, tables.cross[nx * NB_POS + ny], Word16(8192));
            let energy_16 = extract_h(acc);
            let sq = mult(ctx, corr, corr);
            let gain = l_mult(ctx, best_energy, sq);
            let s = l_msu(ctx, gain, best_sq, energy_16);
            if s.0 > 0 {
                best_sq = sq;
                best_energy = energy_16;
                chosen = Some(y);
            }
        }

        if let Some(y) = chosen {
            ix = x;
            iy = y;
        }
    }

    // Deliberately re-read from `dn` and associated to the right, which is not
    // the left-associated sum the loop built. With saturating 16-bit adds the
    // two are different numbers, and this is the one the next stage consumes.
    let pair = add(ctx, tables.dn[ix], tables.dn[iy]);
    *correlation = add(ctx, *correlation, pair);
    *energy = best_energy;
    (ix, iy)
}

/// Search the four-track algebraic codebook at any of its seven pulse budgets.
///
/// `dn` is the target correlation from [`correlate_target`] and is **modified
/// in place** by the sign decision. `cn` is the LTP residual, `impulse` the
/// Q12 weighted synthesis impulse response, and `frame_bits` the mode's frame
/// size — which only matters at 88 bits, where it chooses one iteration or two.
///
/// `filtered` is an in/out parameter, and deliberately so: it is only written
/// when some iteration is accepted. If none is — reachable when the first
/// candidate's squared correlation is zero and its energy non-positive — the
/// caller's incoming vector survives, is rescaled by the final `>> 3`, and is
/// what the gain quantiser sees. Zeroing it unconditionally would be a
/// different codec.
///
/// # Panics
///
/// Never: every position and pulse count is bounded by the mode table.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn search_multi_pulse(
    ctx: &mut DspContext,
    dn: &mut [Word16; L_SUBFR],
    cn: &[Word16; L_SUBFR],
    impulse: &[Word16; L_SUBFR],
    filtered: &mut [Word16; L_SUBFR],
    budget: PulseBudget,
    frame_bits: usize,
) -> Innovation {
    let pulse_count = budget.pulses();
    let widths = budget.stage_widths();
    let first_stage = budget.first_stage_pulses();

    // Survives if no iteration is ever accepted.
    let mut chosen = [0i16; MAX_PULSES];
    for (k, slot) in chosen.iter_mut().enumerate().take(pulse_count) {
        *slot = i16::try_from(k).expect("at most 24 pulses");
    }


    let signs = decide_signs(ctx, dn, cn, budget.mix_weight(), true);

    // Rank the eight best positions of each track by the mixed correlation,
    // overwriting it with `rank - 8`. Strictly greater, so among equal values
    // the lowest sample index wins — that decides `peak` and hence the whole
    // first stage of every iteration.
    let mut ranks = signs.mixed;
    let mut peak = [0usize; NB_TRACK];
    // Declared outside the track loop exactly as the reference does. Because
    // eight of a track's sixteen positions are still unmarked at every step,
    // an unmarked position always beats the -1 seed and always sets it.
    let mut pos = 0usize;
    for (track, top) in peak.iter_mut().enumerate() {
        for rank in 0..NB_MAX {
            let mut best = Word16(-1);
            let mut j = track;
            while j < L_SUBFR {
                if sub(ctx, ranks[j], best).0 > 0 {
                    best = ranks[j];
                    pos = j;
                }
                j += NB_TRACK;
            }
            ranks[pos] = sub(ctx, Word16(rank), Word16(NB_MAX));
            if rank == 0 {
                *top = pos;
            }
        }
    }

    // Scale the impulse response down when its energy would let many pulses
    // saturate the accumulated excitation.
    let mut acc = Word32(0);
    for &v in impulse {
        acc = l_mac(ctx, acc, v, v);
    }
    let energy_16 = extract_h(acc);
    // The reference writes these as two independent `if`s, not an if/else
    // chain: a four- or eight-pulse frame with a very energetic response takes
    // the two-bit shift even though it fails the first test. Written here as
    // the equivalent if/else with the strongest condition first.
    let impulse_shift = if energy_16.0 > 0x6000 {
        2
    } else {
        i16::from(pulse_count >= 12 && energy_16.0 > 1024)
    };

    let mut h = [Word16(0); L_SUBFR];
    let mut h_inv = [Word16(0); L_SUBFR];
    for i in 0..L_SUBFR {
        h[i] = shr(ctx, impulse[i], impulse_shift);
        h_inv[i] = negate(ctx, h[i]);
    }

    let prefix = energy_prefix(ctx, &h, Word32(0x0000_8000));
    let mut self_energy = [[Word16(0); NB_POS]; NB_TRACK];
    for (track, row) in self_energy.iter_mut().enumerate() {
        for (n, slot) in row.iter_mut().enumerate() {
            *slot = prefix[L_SUBFR - 1 - (NB_TRACK * n + track)];
        }
    }

    let chains = cross_prefix(ctx, &h);
    let mut cross = [[Word16(0); NB_POS * NB_POS]; NB_TRACK];
    for (track, row) in cross.iter_mut().enumerate() {
        let other = (track + 1) % NB_TRACK;
        for nx in 0..NB_POS {
            for ny in 0..NB_POS {
                let a = NB_TRACK * nx + track;
                let b = NB_TRACK * ny + other;
                let lag = a.abs_diff(b);
                let last = L_SUBFR - 1 - a.max(b);
                row[nx * NB_POS + ny] = chains[(lag - 1) / 2][last];
            }
        }
    }

    // Fold the sign pair into every cross term, with the `mult(x, 32767)`
    // off-by-one preserved.
    for (track, row) in cross.iter_mut().enumerate() {
        let other = (track + 1) % NB_TRACK;
        for nx in 0..NB_POS {
            let i = NB_TRACK * nx + track;
            let table = if signs.sign[i].0 < 0 {
                &signs.inverse
            } else {
                &signs.sign
            };
            for ny in 0..NB_POS {
                let j = NB_TRACK * ny + other;
                let slot = nx * NB_POS + ny;
                row[slot] = mult(ctx, row[slot], table[j]);
            }
        }
    }

    // The incumbent spans all iterations, so the earliest iteration wins ties.
    let mut best_sq = Word16(-1);
    let mut best_energy = Word16(1);

    for iteration in 0..budget.iterations(frame_bits) {
        let mut tracks = [0usize; MAX_PULSES];
        tracks[..pulse_count]
            .copy_from_slice(&TRACK_ROTATION[4 * iteration..4 * iteration + pulse_count]);

        let mut slots = [0i16; MAX_PULSES];
        let mut excitation = [Word16(0); L_SUBFR];
        let mut correlation = Word16(0);
        let mut energy = Word16(0);

        if first_stage == 2 {
            let ix = peak[tracks[0]];
            let iy = peak[tracks[1]];
            slots[0] = i16::try_from(ix).expect("a sample index");
            slots[1] = i16::try_from(iy).expect("a sample index");
            correlation = add(ctx, dn[ix], dn[iy]);
            let nx = ix / NB_TRACK;
            let ny = iy / NB_TRACK;
            let mut s = l_mult(ctx, self_energy[tracks[0]][nx], Word16(4096));
            s = l_mac(ctx, s, self_energy[tracks[1]][ny], Word16(4096));
            s = l_mac(ctx, s, cross[tracks[0]][nx * NB_POS + ny], Word16(8192));
            energy = round(ctx, s);
            for (i, slot) in excitation.iter_mut().enumerate() {
                let a = tap(&h, &h_inv, signs.sign[ix], ix, i);
                let b = tap(&h, &h_inv, signs.sign[iy], iy, i);
                *slot = add(ctx, a, b);
            }
            if budget == PulseBudget::B44 {
                // Forces the two odd pulses onto tracks 0 and 1 whatever the
                // iteration, which is what keeps the per-track counts at
                // 3/3/2/2 and the 13/13/9/9 bit split valid.
                tracks[8] = 0;
                tracks[9] = 1;
            }
        } else if first_stage == 4 {
            let fixed = [
                peak[tracks[0]],
                peak[tracks[1]],
                peak[tracks[2]],
                peak[tracks[3]],
            ];
            for (k, &p) in fixed.iter().enumerate() {
                slots[k] = i16::try_from(p).expect("a sample index");
            }
            correlation = dn[fixed[0]];
            for &p in &fixed[1..] {
                correlation = add(ctx, correlation, dn[p]);
            }
            for (i, slot) in excitation.iter_mut().enumerate() {
                let mut v = tap(&h, &h_inv, signs.sign[fixed[0]], fixed[0], i);
                for &p in &fixed[1..] {
                    v = add(ctx, v, tap(&h, &h_inv, signs.sign[p], p, i));
                }
                *slot = v;
            }
            let mut acc = Word32(0);
            for &v in &excitation {
                acc = l_mac(ctx, acc, v, v);
            }
            let scaled = l_shr(ctx, acc, 3);
            energy = round(ctx, scaled);
            if budget == PulseBudget::B72 {
                tracks[16] = 0;
                tracks[17] = 1;
            }
        }

        let mut j = first_stage;
        let mut stage = 0usize;
        while j < pulse_count {
            let track_x = tracks[j];
            let track_y = tracks[j + 1];
            debug_assert_eq!(
                track_y,
                (track_x + 1) % NB_TRACK,
                "every stage pair must be adjacent tracks, or the cross table is the wrong one"
            );
            let cor_x = track_correlations(
                ctx,
                &h,
                &excitation,
                track_x,
                &signs.sign,
                &self_energy[track_x],
            );
            let cor_y = track_correlations(
                ctx,
                &h,
                &excitation,
                track_y,
                &signs.sign,
                &self_energy[track_y],
            );
            let tables = StageTables {
                dn: &*dn,
                ranks: &ranks,
                cor_x: &cor_x,
                cor_y: &cor_y,
                cross: &cross[track_x],
            };
            let (ix, iy) = best_pulse_pair(
                ctx,
                widths[stage],
                track_x,
                track_y,
                &mut correlation,
                &mut energy,
                &tables,
            );
            slots[j] = i16::try_from(ix).expect("a sample index");
            slots[j + 1] = i16::try_from(iy).expect("a sample index");
            for (i, slot) in excitation.iter_mut().enumerate() {
                let a = tap(&h, &h_inv, signs.sign[ix], ix, i);
                let b = tap(&h, &h_inv, signs.sign[iy], iy, i);
                // Can saturate, and is meant to.
                let pair = add(ctx, a, b);
                *slot = add(ctx, *slot, pair);
            }
            j += 2;
            stage += 1;
        }

        let squared = mult(ctx, correlation, correlation);
        let gain = l_mult(ctx, best_energy, squared);
        let s = l_msu(ctx, gain, best_sq, energy);
        if s.0 > 0 {
            best_sq = squared;
            best_energy = energy;
            chosen[..pulse_count].copy_from_slice(&slots[..pulse_count]);
            *filtered = excitation;
        }
    }

    let mut code = [Word16(0); L_SUBFR];
    for v in filtered.iter_mut() {
        *v = shr_r(ctx, *v, 3);
    }
    // The amplitude shrinks with the impulse shift so that code and filtered
    // code stay in proportion; the gain quantiser makes up the difference.
    let amplitude = shr(ctx, Word16(512), impulse_shift);

    let mut slots = [-1i16; MAX_PULSES];
    for &position in chosen.iter().take(pulse_count) {
        let i = usize::try_from(position).expect("a sample index");
        let mut index = i16::try_from(i / NB_TRACK).expect("a position in 0..16");
        // Plain `&`, as in the reference: this is a track number, not
        // fixed-point arithmetic.
        let track = i & 0x03;
        if signs.sign[i].0 > 0 {
            code[i] = add(ctx, code[i], amplitude);
        } else {
            code[i] = sub(ctx, code[i], amplitude);
            index += 16;
        }
        // Pulses land in their track's six-slot block, in the order the search
        // produced them, at the first free slot.
        let mut slot = track * SLOTS_PER_TRACK;
        while slots[slot] >= 0 {
            slot += 1;
        }
        slots[slot] = index;
    }

    let indices = pack_indices(budget, &slots);

    Innovation {
        code,
        indices,
        pulses: chosen,
        slots,
        impulse_shift,
    }
}

/// Pack the per-track pulse slots into the mode's transmitted indices.
///
/// The counter into `slots` runs continuously across both sub-loops of the 44-
/// and 72-bit layouts: it is not reset when the track number restarts.
fn pack_indices(budget: PulseBudget, slots: &[i16; MAX_PULSES]) -> [u16; 8] {
    let mut indices = [0u16; 8];
    let mut wide = [0i32; MAX_PULSES];
    for (w, &s) in wide.iter_mut().zip(slots.iter()) {
        *w = i32::from(s);
    }
    let mut k = 0usize;

    match budget {
        PulseBudget::B20 => {
            for out in indices.iter_mut().take(NB_TRACK) {
                *out = narrow(pack_1p(wide[k], 4));
                k += SLOTS_PER_TRACK;
            }
        }
        PulseBudget::B36 => {
            for out in indices.iter_mut().take(NB_TRACK) {
                *out = narrow(pack_2p(wide[k], wide[k + 1], 4));
                k += SLOTS_PER_TRACK;
            }
        }
        PulseBudget::B44 => {
            for out in indices.iter_mut().take(2) {
                *out = narrow(pack_3p(wide[k], wide[k + 1], wide[k + 2], 4));
                k += SLOTS_PER_TRACK;
            }
            for out in &mut indices[2..NB_TRACK] {
                *out = narrow(pack_2p(wide[k], wide[k + 1], 4));
                k += SLOTS_PER_TRACK;
            }
        }
        PulseBudget::B52 => {
            for out in indices.iter_mut().take(NB_TRACK) {
                *out = narrow(pack_3p(wide[k], wide[k + 1], wide[k + 2], 4));
                k += SLOTS_PER_TRACK;
            }
        }
        PulseBudget::B64 => {
            for track in 0..NB_TRACK {
                let joined = pack_4p(&wide[k..k + 4], 4);
                indices[track] = narrow((joined >> 14) & 3);
                indices[track + NB_TRACK] = narrow(joined & 0x3FFF);
                k += SLOTS_PER_TRACK;
            }
        }
        PulseBudget::B72 => {
            for track in 0..2 {
                let joined = pack_5p(&wide[k..k + 5], 4);
                indices[track] = narrow((joined >> 10) & 0x03FF);
                indices[track + NB_TRACK] = narrow(joined & 0x03FF);
                k += SLOTS_PER_TRACK;
            }
            for track in 2..NB_TRACK {
                let joined = pack_4p(&wide[k..k + 4], 4);
                indices[track] = narrow((joined >> 14) & 3);
                indices[track + NB_TRACK] = narrow(joined & 0x3FFF);
                k += SLOTS_PER_TRACK;
            }
        }
        PulseBudget::B88 => {
            for track in 0..NB_TRACK {
                let joined = pack_6p(&wide[k..k + 6], 4);
                indices[track] = narrow((joined >> 11) & 0x07FF);
                indices[track + NB_TRACK] = narrow(joined & 0x07FF);
                k += SLOTS_PER_TRACK;
            }
        }
    }

    indices
}

/// Narrow a packed index to the width the bitstream carries it in.
#[inline]
fn narrow(index: i32) -> u16 {
    u16::try_from(index).expect("a packed index is at most 22 bits and non-negative")
}

// ---------------------------------------------------------------------------
// q_pulse.c — combinatorial index packing.
//
// Every packed position is `position (0..15) + 16` when the pulse is negative,
// and the sign bit is always bit 4 whatever the recursion's `N`. Deriving the
// sign mask from `N` instead is a classic way to break the 4-, 5- and 6-pulse
// packers while leaving the 1- and 2-pulse ones working.
//
// The reference writes these with basic operators, but every intermediate is a
// non-negative value under 2^22 (the widest result, `pack_6p` with N = 4, is 22
// bits) and every shift is by a compile-time constant under 20, so no `add`,
// `shl`, `L_add` or `L_shl` here can saturate. Plain integer arithmetic is
// therefore bit-identical, and the tests assert the width bound on every
// result. Sign handling is `&`, `^` and `%`, which the reference does with
// plain C operators too.
// ---------------------------------------------------------------------------

/// Pack one pulse into `n + 1` bits.
const fn pack_1p(pos: i32, n: u32) -> i32 {
    let mask = (1 << n) - 1;
    let mut index = pos & mask;
    if pos & SIGN_BIT != 0 {
        index += 1 << n;
    }
    index
}

/// Pack two pulses into `2n + 1` bits.
///
/// The pair is unordered, so the state freed by fixing an order carries a sign.
/// Note the asymmetry: the same-sign branch compares the *unmasked* positions
/// while the opposite-sign branch compares the masked ones. When `pack_3p`
/// calls this with `n - 1` the caller has already made the two agree above the
/// mask, so the branches coincide — but the code is written this way and the
/// decoder is its mirror image.
const fn pack_2p(pos1: i32, pos2: i32, n: u32) -> i32 {
    let mask = (1 << n) - 1;
    if (pos2 ^ pos1) & SIGN_BIT == 0 {
        let mut index = if pos1 <= pos2 {
            ((pos1 & mask) << n) + (pos2 & mask)
        } else {
            ((pos2 & mask) << n) + (pos1 & mask)
        };
        if pos1 & SIGN_BIT != 0 {
            index += 1 << (2 * n);
        }
        index
    } else if (pos1 & mask) <= (pos2 & mask) {
        let mut index = ((pos2 & mask) << n) + (pos1 & mask);
        if pos2 & SIGN_BIT != 0 {
            index += 1 << (2 * n);
        }
        index
    } else {
        let mut index = ((pos1 & mask) << n) + (pos2 & mask);
        if pos1 & SIGN_BIT != 0 {
            index += 1 << (2 * n);
        }
        index
    }
}

/// Pack three pulses into `3n + 1` bits.
///
/// Splits on which two of the three share a half-range. The branch order is
/// part of the code: `pos1`/`pos2` is tested first, then `pos1`/`pos3`.
const fn pack_3p(pos1: i32, pos2: i32, pos3: i32, n: u32) -> i32 {
    let half = 1 << (n - 1);
    if (pos1 ^ pos2) & half == 0 {
        pack_2p(pos1, pos2, n - 1) + ((pos1 & half) << n) + (pack_1p(pos3, n) << (2 * n))
    } else if (pos1 ^ pos3) & half == 0 {
        pack_2p(pos1, pos3, n - 1) + ((pos1 & half) << n) + (pack_1p(pos2, n) << (2 * n))
    } else {
        pack_2p(pos2, pos3, n - 1) + ((pos2 & half) << n) + (pack_1p(pos1, n) << (2 * n))
    }
}

/// Pack four pulses into `4n + 1` bits.
const fn pack_4p_odd(pos1: i32, pos2: i32, pos3: i32, pos4: i32, n: u32) -> i32 {
    let half = 1 << (n - 1);
    if (pos1 ^ pos2) & half == 0 {
        pack_2p(pos1, pos2, n - 1) + ((pos1 & half) << n) + (pack_2p(pos3, pos4, n) << (2 * n))
    } else if (pos1 ^ pos3) & half == 0 {
        pack_2p(pos1, pos3, n - 1) + ((pos1 & half) << n) + (pack_2p(pos2, pos4, n) << (2 * n))
    } else {
        pack_2p(pos2, pos3, n - 1) + ((pos2 & half) << n) + (pack_2p(pos1, pos4, n) << (2 * n))
    }
}

/// Split positions by which half-range they fall in, order preserved.
///
/// Returns the two halves in fixed-size buffers with their lengths, so the
/// packers stay allocation-free.
fn split_halves(pos: &[i32], half: i32) -> ([i32; 6], usize, [i32; 6], usize) {
    let mut low = [0i32; 6];
    let mut high = [0i32; 6];
    let (mut n_low, mut n_high) = (0usize, 0usize);
    for &p in pos {
        if p & half == 0 {
            low[n_low] = p;
            n_low += 1;
        } else {
            high[n_high] = p;
            n_high += 1;
        }
    }
    (low, n_low, high, n_high)
}

/// Pack four pulses into `4n` bits.
///
/// The two leading bits carry how many pulses fell in the low half-range. The
/// all-high and all-low cases both leave those two bits at zero and are
/// separated by bit `4n - 3`, which only the all-high case sets.
///
/// # Panics
///
/// Never: `split_halves` of four positions always yields one of five shapes.
fn pack_4p(pos: &[i32], n: u32) -> i32 {
    let n1 = n - 1;
    let half = 1 << n1;
    let (low, count, high, _) = split_halves(&pos[..4], half);

    let index = match count {
        0 => (1 << (4 * n - 3)) + pack_4p_odd(high[0], high[1], high[2], high[3], n1),
        1 => (pack_1p(low[0], n1) << (3 * n1 + 1)) + pack_3p(high[0], high[1], high[2], n1),
        2 => (pack_2p(low[0], low[1], n1) << (2 * n1 + 1)) + pack_2p(high[0], high[1], n1),
        3 => (pack_3p(low[0], low[1], low[2], n1) << n) + pack_1p(high[0], n1),
        4 => pack_4p_odd(low[0], low[1], low[2], low[3], n1),
        _ => unreachable!("four positions split into at most four"),
    };

    index + ((i32::try_from(count).expect("at most four") & 3) << (4 * n - 2))
}

/// Pack five pulses into `5n` bits.
///
/// Unlike the four- and six-pulse packers there is no trailing population
/// count: the top bit alone distinguishes the two shape families.
///
/// # Panics
///
/// Never: five positions split into one of six shapes.
fn pack_5p(pos: &[i32], n: u32) -> i32 {
    let n1 = n - 1;
    let half = 1 << n1;
    let (low, count, high, _) = split_halves(&pos[..5], half);
    let top = 1 << (5 * n - 1);
    let shift = 2 * n + 1;

    match count {
        0 => top + (pack_3p(high[0], high[1], high[2], n1) << shift) + pack_2p(high[3], high[4], n),
        1 => top + (pack_3p(high[0], high[1], high[2], n1) << shift) + pack_2p(high[3], low[0], n),
        2 => top + (pack_3p(high[0], high[1], high[2], n1) << shift) + pack_2p(low[0], low[1], n),
        3 => (pack_3p(low[0], low[1], low[2], n1) << shift) + pack_2p(high[0], high[1], n),
        4 => (pack_3p(low[0], low[1], low[2], n1) << shift) + pack_2p(low[3], high[0], n),
        5 => (pack_3p(low[0], low[1], low[2], n1) << shift) + pack_2p(low[3], low[4], n),
        _ => unreachable!("five positions split into at most five"),
    }
}

/// Pack six pulses into `6n - 2` bits.
///
/// The trailing two-bit field encodes the *shape class*, not the population
/// count: the 4-, 5- and 6-low cases reassign it to 2, 1 and 0 respectively
/// before it is stored, mirroring the 2-, 1- and 0-low cases with the halves
/// swapped. Emitting the raw count instead is a silent bit-error only reachable
/// on well-balanced pulse sets.
///
/// # Panics
///
/// Never: six positions split into one of seven shapes.
fn pack_6p(pos: &[i32], n: u32) -> i32 {
    let n1 = n - 1;
    let half = 1 << n1;
    let (low, count, high, _) = split_halves(&pos[..6], half);
    let top = 1 << (6 * n - 5);

    // The five- and four-pulse packers are handed the whole half and read only
    // its first five or four entries, exactly as the reference's array pointer
    // does.
    let (class, index) = match count {
        0 => (0, top + (pack_5p(&high, n1) << n) + pack_1p(high[5], n1)),
        1 => (1, top + (pack_5p(&high, n1) << n) + pack_1p(low[0], n1)),
        2 => (
            2,
            top + (pack_4p(&high, n1) << (2 * n1 + 1)) + pack_2p(low[0], low[1], n1),
        ),
        3 => (
            3,
            (pack_3p(low[0], low[1], low[2], n1) << (3 * n1 + 1))
                + pack_3p(high[0], high[1], high[2], n1),
        ),
        4 => (
            2,
            (pack_4p(&low, n1) << (2 * n1 + 1)) + pack_2p(high[0], high[1], n1),
        ),
        5 => (1, (pack_5p(&low, n1) << n) + pack_1p(high[0], n1)),
        6 => (0, (pack_5p(&low, n1) << n) + pack_1p(low[5], n1)),
        _ => unreachable!("six positions split into at most six"),
    };

    index + ((class & 3) << (6 * n - 4))
}

#[cfg(test)]
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::{AmrMode, AmrVariant};
    use crate::codecs::amr::storage;
    use crate::codecs::amr::wb::codebook as decoder;
    use crate::codecs::amr::wb::params::FrameParams;
    use crate::fixed_point::arith32::l_negate;

    /// The reference encoder's own per-subframe trace at 12.65 kbit/s.
    const TRACE: &str = include_str!("../../testdata/wb_enc_trace.txt");

    /// Frames of the trace committed alongside it.
    const TRACED_FRAMES: usize = 3;

    /// The bitstream the reference encoder produced from the same input.
    const BITSTREAM: &[u8] = include_bytes!("../../testdata/amrwb_enc_mode2.amr");

    /// 12.65 kbit/s, the rate the committed trace and bitstream were made at.
    const FRAME_BITS: usize = 253;

    fn row(frame: usize, subframe: i32, name: &str) -> Vec<i32> {
        let prefix = format!("T {frame} {subframe} {name} ");
        let line = TRACE
            .lines()
            .find(|l| l.starts_with(&prefix))
            .unwrap_or_else(|| panic!("trace row {frame} {subframe} {name} is missing"));
        line.split_whitespace()
            .skip(4)
            .map(|t| t.parse().expect("trace values are integers"))
            .collect()
    }

    fn vector(frame: usize, subframe: usize, name: &str) -> [Word16; L_SUBFR] {
        let values = row(frame, subframe as i32, name);
        assert_eq!(values.len(), L_SUBFR, "{name} is not a subframe vector");
        let mut out = [Word16(0); L_SUBFR];
        for (o, v) in out.iter_mut().zip(values) {
            *o = Word16(v as i16);
        }
        out
    }

    fn scalar(frame: usize, subframe: usize, name: &str) -> i32 {
        let values = row(frame, subframe as i32, name);
        assert_eq!(values.len(), 1, "{name} is not a scalar");
        values[0]
    }

    /// `Preemph` from `preemph.c`, run over the codeword after the search.
    ///
    /// Not part of this module — the subframe integration owns it — but the
    /// traced `code` row is captured after it, so comparing against that row
    /// means reproducing it. The saved-then-discarded memory is the
    /// reference's; the caller passes zero in.
    fn preemphasise(ctx: &mut DspContext, x: &mut [Word16; L_SUBFR], mu: Word16) {
        for i in (1..L_SUBFR).rev() {
            let acc = l_deposit_h(x[i]);
            let acc = l_msu(ctx, acc, x[i - 1], mu);
            x[i] = round(ctx, acc);
        }
        let acc = l_deposit_h(x[0]);
        let acc = l_msu(ctx, acc, Word16(0), mu);
        x[0] = round(ctx, acc);
    }

    /// Run the search on one traced subframe and hand back what it produced.
    fn search_traced(ctx: &mut DspContext, frame: usize, subframe: usize) -> Innovation {
        let mut dn = vector(frame, subframe, "dn");
        let cn = vector(frame, subframe, "cn");
        let h2 = vector(frame, subframe, "h2");
        let mut filtered = [Word16(0); L_SUBFR];
        search_multi_pulse(
            ctx,
            &mut dn,
            &cn,
            &h2,
            &mut filtered,
            PulseBudget::B36,
            FRAME_BITS,
        )
    }

    #[test]
    fn filtered_codeword_matches_the_reference_trace() {
        let mut ctx = DspContext::new();
        let mut compared = 0usize;

        for frame in 0..TRACED_FRAMES {
            for subframe in 0..4 {
                let mut dn = vector(frame, subframe, "dn");
                let cn = vector(frame, subframe, "cn");
                let h2 = vector(frame, subframe, "h2");
                let mut filtered = [Word16(0); L_SUBFR];
                let _ = search_multi_pulse(
                    &mut ctx,
                    &mut dn,
                    &cn,
                    &h2,
                    &mut filtered,
                    PulseBudget::B36,
                    FRAME_BITS,
                );
                assert_eq!(
                    filtered.to_vec(),
                    vector(frame, subframe, "y2").to_vec(),
                    "y2 differs at frame {frame} subframe {subframe}"
                );
                compared += 1;
            }
        }

        assert_eq!(compared, 12, "compared {compared} subframes, expected 12");
    }

    #[test]
    fn codeword_matches_the_reference_trace_after_sharpening() {
        let mut ctx = DspContext::new();
        // `tilt_code` is zero out of reset and thereafter derived from the
        // previous subframe's voice factor.
        let mut tilt = Word16(0);
        let mut compared = 0usize;

        for frame in 0..TRACED_FRAMES {
            for subframe in 0..4 {
                let result = search_traced(&mut ctx, frame, subframe);

                let mut code = result.code;
                preemphasise(&mut ctx, &mut code, tilt);
                // The lag increment happens before the impulse response is
                // sharpened and persists to this second call.
                let mut lag = scalar(frame, subframe, "T0");
                if scalar(frame, subframe, "T0_frac") > 2 {
                    lag += 1;
                }
                let lag = usize::try_from(lag).expect("a positive pitch lag");
                pitch_sharpen(&mut ctx, &mut code, lag, PITCH_SHARP);

                assert_eq!(
                    code.to_vec(),
                    vector(frame, subframe, "code").to_vec(),
                    "code differs at frame {frame} subframe {subframe}"
                );
                compared += 1;

                let voice_fac = Word16(scalar(frame, subframe, "voice_fac") as i16);
                let quarter = shr(&mut ctx, voice_fac, 2);
                tilt = add(&mut ctx, quarter, Word16(8192));
            }
        }

        assert_eq!(compared, 12, "compared {compared} subframes, expected 12");
    }

    #[test]
    fn pulse_indices_match_the_reference_bitstream() {
        let mut ctx = DspContext::new();
        let (variant, frames) = storage::read(BITSTREAM).expect("the fixture parses");
        assert_eq!(variant, AmrVariant::WideBand);
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("12.65 kbit/s");
        let mut compared = 0usize;

        for (frame, coded) in frames.iter().enumerate().take(TRACED_FRAMES) {
            let params = FrameParams::parse(mode, &coded.data).expect("the frame parses");
            for subframe in 0..4 {
                let result = search_traced(&mut ctx, frame, subframe);
                assert_eq!(
                    result.indices[..4].to_vec(),
                    params.subframes[subframe].pulses,
                    "pulse indices differ at frame {frame} subframe {subframe}"
                );
                compared += 1;
            }
        }

        assert_eq!(compared, 12, "compared {compared} subframes, expected 12");
    }

    /// The positions and signs themselves, not only the vector they produce.
    ///
    /// Decoding this module's own indices with the crate's bit-exact decoder
    /// and comparing against the traced codeword before sharpening pins each
    /// pulse's track, position and sign individually — a codeword that happens
    /// to match while a pulse sits on the wrong track cannot survive this.
    #[test]
    fn chosen_positions_and_signs_survive_a_decode() {
        let mut ctx = DspContext::new();
        let mut compared = 0usize;

        for frame in 0..TRACED_FRAMES {
            for subframe in 0..4 {
                let result = search_traced(&mut ctx, frame, subframe);
                let decoded = decoder::decode_4t64(&result.indices[..4], 36)
                    .expect("36 bits is a valid budget");
                let expected: Vec<i16> = result.code.iter().map(|v| v.0).collect();
                assert_eq!(
                    decoded.to_vec(),
                    expected,
                    "decoded pulses differ at frame {frame} subframe {subframe}"
                );

                // Eight pulses, two per track.
                for track in 0..NB_TRACK {
                    let used = result.slots[track * SLOTS_PER_TRACK..track * SLOTS_PER_TRACK + 6]
                        .iter()
                        .filter(|&&s| s >= 0)
                        .count();
                    assert_eq!(
                        used, 2,
                        "track {track} has {used} pulses at frame {frame} subframe {subframe}"
                    );
                }
                compared += 1;
            }
        }

        assert_eq!(compared, 12, "compared {compared} subframes, expected 12");
    }

    /// The stage search must keep the *earlier* of two equally good pairs.
    ///
    /// Two inner positions are given the same correlation and the same energy,
    /// so both score identically. The reference's `s > 0` keeps the first; a
    /// `>=` would keep the second, and both produce perfectly plausible speech.
    /// The later `x` positions then tie with the incumbent too, which also
    /// exercises the "no candidate accepted, leave `ix`/`iy` alone" path.
    #[test]
    fn a_tied_pulse_pair_resolves_to_the_earlier_one() {
        let mut ctx = DspContext::new();
        let mut dn = [Word16(0); L_SUBFR];
        // Track 1 samples 9 and 21 — inner positions 2 and 5 — score alike.
        dn[9] = Word16(1000);
        dn[21] = Word16(1000);
        // Every outer position admitted: the rank code is `rank - 8`.
        let ranks = [Word16(-8); L_SUBFR];
        let cor_x = [Word16(0); NB_POS];
        let cor_y = [Word16(0); NB_POS];
        let cross = [Word16(0); NB_POS * NB_POS];
        let tables = StageTables {
            dn: &dn,
            ranks: &ranks,
            cor_x: &cor_x,
            cor_y: &cor_y,
            cross: &cross,
        };

        let mut correlation = Word16(0);
        let mut energy = Word16(0);
        let (ix, iy) = best_pulse_pair(
            &mut ctx,
            8,
            0,
            1,
            &mut correlation,
            &mut energy,
            &tables,
        );

        assert_eq!(
            (ix, iy),
            (0, 9),
            "the first of two equally good inner positions must win"
        );
        assert_eq!(
            correlation.0, 1000,
            "the correlation is re-read from dn at the winning pair"
        );
    }

    /// A stage that accepts nothing still writes its energy, and leaves the
    /// positions at their track defaults.
    ///
    /// Reachable whenever no outer position is admitted. Guarding the write
    /// would leave the caller's previous energy in place and change every
    /// following stage.
    #[test]
    fn a_stage_that_accepts_nothing_still_writes_its_energy() {
        let mut ctx = DspContext::new();
        let dn = [Word16(1000); L_SUBFR];
        // Rank code >= 0 everywhere means no position was pre-selected, so the
        // threshold admits none of them.
        let ranks = [Word16(0); L_SUBFR];
        let cor_x = [Word16(0); NB_POS];
        let cor_y = [Word16(0); NB_POS];
        let cross = [Word16(0); NB_POS * NB_POS];
        let tables = StageTables {
            dn: &dn,
            ranks: &ranks,
            cor_x: &cor_x,
            cor_y: &cor_y,
            cross: &cross,
        };

        let mut correlation = Word16(50);
        let mut energy = Word16(1234);
        let (ix, iy) = best_pulse_pair(
            &mut ctx,
            8,
            2,
            3,
            &mut correlation,
            &mut energy,
            &tables,
        );

        assert_eq!((ix, iy), (2, 3), "the defaults are position 0 of each track");
        assert_eq!(energy.0, 1, "the untouched incumbent energy is written out");
        assert_eq!(correlation.0, 2050, "the defaults still update the sum");
    }

    /// The pre-selection ranks equal values by ascending sample index, which is
    /// what fixes the first stage of every iteration.
    #[test]
    fn equal_ranks_select_the_lowest_sample_index() {
        let mut ctx = DspContext::new();
        // A flat impulse response and a constant correlation make every
        // position of every track score the same.
        let impulse = [Word16(2048); L_SUBFR];
        let cn = [Word16(0); L_SUBFR];
        let mut dn = [Word16(1000); L_SUBFR];
        let mut filtered = [Word16(0); L_SUBFR];

        let result = search_multi_pulse(
            &mut ctx,
            &mut dn,
            &cn,
            &impulse,
            &mut filtered,
            PulseBudget::B36,
            FRAME_BITS,
        );

        // The first two pulses of a 36-bit search are the pre-selection's
        // top-ranked position on each of the iteration's first two tracks. With
        // every position of a track tied, the strict `>` in the ranking keeps
        // the one it saw first, which is the track's lowest sample.
        let (a, b) = (result.pulses[0], result.pulses[1]);
        assert!(
            a < 4 && b < 4,
            "tied tracks must rank their lowest sample first, got {a} and {b}"
        );
        assert_eq!(
            b,
            (a + 1) % 4,
            "the first stage always fixes two adjacent tracks"
        );
    }

    /// The prefix-chain formulation must agree with the reference's pointer
    /// walk over every entry of both correlation tables.
    ///
    /// The reference fills `rrixiy` with two interleaved descending pointer
    /// chains, one of which biases its fourth store by −16. Transcribing that
    /// literally here and checking all 1024 entries is what licenses computing
    /// them by truncation point instead.
    #[test]
    fn correlation_tables_agree_with_the_reference_pointer_walk() {
        // Track indices in each chain's visit order. Chain A biases its fourth
        // store by -16; chain B does not.
        const CHAIN_A: [usize; 4] = [2, 1, 0, 3];
        const CHAIN_B: [usize; 4] = [3, 2, 1, 0];

        let mut ctx = DspContext::new();
        // A deterministic, resonant-ish response: constant `h` would make the
        // two formulations agree for the wrong reason.
        let mut h = [Word16(0); L_SUBFR];
        let mut state = 12345u32;
        for v in &mut h {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            *v = Word16(((state >> 16) as i16) / 16);
        }

        // rrixix, four tracks of sixteen.
        let prefix = energy_prefix(&mut ctx, &h, Word32(0x0000_8000));
        let mut self_walked = [[Word16(0); NB_POS]; NB_TRACK];
        let mut acc = Word32(0x0000_8000);
        let mut p = 0usize;
        for i in 0..NB_POS {
            for track in (0..NB_TRACK).rev() {
                acc = l_mac(&mut ctx, acc, h[p], h[p]);
                p += 1;
                self_walked[track][NB_POS - 1 - i] = extract_h(acc);
            }
        }
        let mut checked = 0usize;
        for track in 0..NB_TRACK {
            for n in 0..NB_POS {
                assert_eq!(
                    prefix[L_SUBFR - 1 - (NB_TRACK * n + track)],
                    self_walked[track][n],
                    "rrixix[{track}][{n}]"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 64, "checked {checked} self energies, expected 64");

        // rrixiy, four tracks of 256, by the two pointer chains.
        let step = NB_POS as i32 + 1;
        let mut cross_walked = [[Word16(0); NB_POS * NB_POS]; NB_TRACK];

        for (chain, tail) in [(CHAIN_A, 3usize), (CHAIN_B, 1usize)] {
            let first = chain[0] == 2;
            let mut base: i32 = (NB_POS * NB_POS) as i32 - 1;
            let mut lag = if first { 1usize } else { 3usize };
            for k in 0..NB_POS {
                // Chain A starts all four pointers together; chain B starts its
                // first one a slot higher than the other three.
                let mut ptr = if first {
                    [base, base, base, base]
                } else {
                    [base, base - 1, base - 1, base - 1]
                };
                let mut acc = Word32(0x0000_8000);
                let (mut a, mut b) = (0usize, lag);
                for i in (k + 1)..=NB_POS {
                    let writes = if i == NB_POS { tail } else { 4 };
                    for (slot, &track) in chain.iter().enumerate().take(writes) {
                        acc = l_mac(&mut ctx, acc, h[a], h[b]);
                        a += 1;
                        b += 1;
                        // Only chain A biases its fourth store, and only there.
                        let bias = i32::from(first && slot == 3) * NB_POS as i32;
                        let at = usize::try_from(ptr[slot] - bias)
                            .expect("the chains stay inside the table");
                        cross_walked[track][at] = extract_h(acc);
                    }
                    if i < NB_POS {
                        for slot in &mut ptr {
                            *slot -= step;
                        }
                    }
                }
                base -= if first { NB_POS as i32 } else { 1 };
                lag += NB_TRACK;
            }
        }
        let chains = cross_prefix(&mut ctx, &h);
        let mut checked = 0usize;
        for (track, row) in cross_walked.iter().enumerate() {
            let other = (track + 1) % NB_TRACK;
            for nx in 0..NB_POS {
                for ny in 0..NB_POS {
                    let a = NB_TRACK * nx + track;
                    let b = NB_TRACK * ny + other;
                    let lag = a.abs_diff(b);
                    let last = L_SUBFR - 1 - a.max(b);
                    assert_eq!(
                        chains[(lag - 1) / 2][last],
                        row[nx * NB_POS + ny],
                        "rrixiy[{track}][{nx}][{ny}]"
                    );
                    checked += 1;
                }
            }
        }
        assert_eq!(
            checked,
            NB_TRACK * NB_POS * NB_POS,
            "checked {checked} cross energies, expected 1024"
        );
    }

    /// `cor_h_x` against the traced `dn`, which the search then consumes.
    #[test]
    fn target_correlation_matches_the_reference_trace() {
        let mut ctx = DspContext::new();
        let mut compared = 0usize;
        for frame in 0..TRACED_FRAMES {
            for subframe in 0..4 {
                let h2 = vector(frame, subframe, "h2");
                let xn2 = vector(frame, subframe, "xn2");
                let dn = correlate_target(&mut ctx, &h2, &xn2);
                assert_eq!(
                    dn.to_vec(),
                    vector(frame, subframe, "dn").to_vec(),
                    "dn differs at frame {frame} subframe {subframe}"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 12, "compared {compared} subframes, expected 12");
    }

    /// Pitch sharpening reads what it has already written.
    #[test]
    fn pitch_sharpening_is_recursive() {
        let mut ctx = DspContext::new();
        let mut x = [Word16(0); 8];
        x[0] = Word16(1000);
        pitch_sharpen(&mut ctx, &mut x, 2, PITCH_SHARP);
        // 1000 -> 850 at 2, then that 850 feeds 4, and so on. A non-recursive
        // filter would leave x[4] and x[6] at 0.
        assert_eq!(x[2].0, 850);
        assert_eq!(x[4].0, 723);
        assert_eq!(x[6].0, 615);
        assert_eq!(x[1].0, 0, "samples before the lag are untouched");
    }

    /// A lag at or past the subframe leaves the vector alone.
    #[test]
    fn pitch_sharpening_with_a_long_lag_is_a_no_op() {
        let mut ctx = DspContext::new();
        let original = [Word16(1234); L_SUBFR];
        let mut x = original;
        pitch_sharpen(&mut ctx, &mut x, 200, PITCH_SHARP);
        assert_eq!(x.to_vec(), original.to_vec());
    }

    /// The packers, at every budget, against the decoder that already agrees
    /// with the reference.
    ///
    /// The committed trace only exercises 36 bits. This closes the other six by
    /// running the real search at each budget on real traced inputs and
    /// requiring the decoder to recover exactly the pulse set that was packed.
    #[test]
    fn every_budget_round_trips_through_the_decoder() {
        let budgets = [
            PulseBudget::B20,
            PulseBudget::B36,
            PulseBudget::B44,
            PulseBudget::B52,
            PulseBudget::B64,
            PulseBudget::B72,
            PulseBudget::B88,
        ];
        let mut ctx = DspContext::new();
        let mut compared = 0usize;

        for budget in budgets {
            // 461 keeps the 88-bit mode at two iterations; the 477-bit case is
            // covered separately below.
            let frame_bits = if budget == PulseBudget::B88 {
                461
            } else {
                budget.bits() * 4
            };
            for frame in 0..TRACED_FRAMES {
                for subframe in 0..4 {
                    let mut dn = vector(frame, subframe, "dn");
                    let cn = vector(frame, subframe, "cn");
                    let h2 = vector(frame, subframe, "h2");
                    let mut filtered = [Word16(0); L_SUBFR];
                    let result = search_multi_pulse(
                        &mut ctx,
                        &mut dn,
                        &cn,
                        &h2,
                        &mut filtered,
                        budget,
                        frame_bits,
                    );

                    let count = budget.index_count();
                    let decoded = decoder::decode_4t64(&result.indices[..count], budget.bits())
                        .expect("a valid budget");
                    // The decoder always uses +/-512; the search scales its
                    // codeword down by the impulse shift, so compare in pulses.
                    let scale = 1i16 << result.impulse_shift;
                    let expected: Vec<i16> = result.code.iter().map(|v| v.0 * scale).collect();
                    assert_eq!(
                        decoded.to_vec(),
                        expected,
                        "{budget:?} round trip differs at frame {frame} subframe {subframe}"
                    );
                    compared += 1;
                }
            }
        }

        assert_eq!(
            compared,
            budgets.len() * 12,
            "compared {compared} searches, expected {}",
            budgets.len() * 12
        );
    }

    /// The two-pulse codebook's index, codeword and filtered codeword must
    /// agree with each other and with the decoder.
    ///
    /// The committed trace is 12.65 kbit/s, so it cannot pin this search's
    /// output; what it can do is supply realistic inputs. The decoder — which
    /// is bit-exact against the reference — must recover exactly the two pulses
    /// that were placed, and the filtered codeword must be the sum of the two
    /// shifted impulse responses. That covers the index layout and the codeword
    /// build; the search itself is covered by the offline 6.60 kbit/s sweep
    /// described in the module documentation.
    #[test]
    fn the_two_pulse_codebook_is_self_consistent() {
        let mut ctx = DspContext::new();
        let mut compared = 0usize;

        for frame in 0..TRACED_FRAMES {
            for subframe in 0..4 {
                let mut dn = vector(frame, subframe, "dn");
                let cn = vector(frame, subframe, "cn");
                let h2 = vector(frame, subframe, "h2");
                let result = search_two_pulse(&mut ctx, &mut dn, &cn, &h2);

                let decoded = decoder::decode_2t64(result.index);
                let expected: Vec<i16> = result.code.iter().map(|v| v.0).collect();
                assert_eq!(
                    decoded.to_vec(),
                    expected,
                    "the index does not decode to its own codeword at frame {frame} subframe {subframe}"
                );

                let [ix, iy] = result.positions;
                assert_eq!(ix % 2, 0, "the first pulse is on the even track");
                assert_eq!(iy % 2, 1, "the second pulse is on the odd track");

                // y[i] = shr_r(+/-h[i-ix] +/- h[i-iy], 3), with the taps before
                // each pulse reading zero.
                for i in 0..L_SUBFR {
                    let mut sum = Word16(0);
                    for (pos, positive) in result.positions.iter().zip(result.positive) {
                        if i >= *pos {
                            let tap = h2[i - *pos];
                            let signed = if positive {
                                tap
                            } else {
                                negate(&mut ctx, tap)
                            };
                            sum = add(&mut ctx, sum, signed);
                        }
                    }
                    let expected = shr_r(&mut ctx, sum, 3);
                    assert_eq!(
                        result.filtered[i], expected,
                        "filtered sample {i} at frame {frame} subframe {subframe}"
                    );
                }
                compared += 1;
            }
        }

        assert_eq!(compared, 12, "compared {compared} subframes, expected 12");
    }

    /// 23.85 kbit/s drops the 88-bit search to a single iteration; 23.05 keeps
    /// two. A wrong threshold here is invisible at every other rate.
    #[test]
    fn the_widest_budget_changes_iteration_count_at_23_85() {
        assert_eq!(PulseBudget::B88.iterations(461), 2, "23.05 kbit/s");
        assert_eq!(PulseBudget::B88.iterations(477), 1, "23.85 kbit/s");
        assert_eq!(PulseBudget::B88.iterations(462), 2, "the test is > 462");
        assert_eq!(PulseBudget::B88.iterations(463), 1);
    }

    /// The frame-size ladder, including the 6.60 kbit/s hole.
    #[test]
    fn budgets_follow_the_frame_size_ladder() {
        let expected = [
            (132, None),
            (177, Some(PulseBudget::B20)),
            (253, Some(PulseBudget::B36)),
            (285, Some(PulseBudget::B44)),
            (317, Some(PulseBudget::B52)),
            (365, Some(PulseBudget::B64)),
            (397, Some(PulseBudget::B72)),
            (461, Some(PulseBudget::B88)),
            (477, Some(PulseBudget::B88)),
        ];
        for (bits, budget) in expected {
            assert_eq!(PulseBudget::from_frame_bits(bits), budget, "{bits} bits");
        }
    }

    /// Two pulses on the same sample accumulate rather than clamp, and every
    /// sample's amplitude is exactly its pulse count times the unit.
    #[test]
    fn codeword_amplitude_counts_the_pulses_on_each_sample() {
        let mut ctx = DspContext::new();
        let mut compared = 0usize;
        for frame in 0..TRACED_FRAMES {
            for subframe in 0..4 {
                let result = search_traced(&mut ctx, frame, subframe);
                let unit = i32::from(512i16 >> result.impulse_shift);
                let mut seen = [0i32; L_SUBFR];
                for &p in result.pulses.iter().take(8) {
                    seen[usize::try_from(p).expect("a sample index")] += 1;
                }
                for (i, &n) in seen.iter().enumerate() {
                    assert_eq!(
                        i32::from(result.code[i].0).abs(),
                        n * unit,
                        "sample {i} carries {n} pulses at frame {frame} subframe {subframe}"
                    );
                }
                compared += 1;
            }
        }
        assert_eq!(compared, 12, "compared {compared} subframes, expected 12");
    }

    /// `norm_l` complements negative inputs rather than negating them, which
    /// moves every scale factor this module derives.
    #[test]
    fn negative_normalisation_uses_the_complement_rule() {
        assert_eq!(norm_l(Word32(0xC000_0000u32 as i32)), 1);
        let mut ctx = DspContext::new();
        assert_eq!(l_negate(&mut ctx, Word32(0x8000_0000u32 as i32)).0, i32::MAX);
    }

    /// The packers never exceed their advertised width, which is what licenses
    /// plain integer arithmetic in place of the reference's basic operators.
    #[test]
    fn packed_indices_stay_within_their_advertised_width() {
        let mut state = 987u32;
        let mut next = move || {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            i32::try_from((state >> 20) % 32).expect("under 32")
        };
        let mut checked = 0usize;
        for _ in 0..2000 {
            let pos: [i32; 6] = std::array::from_fn(|_| next());
            for (width, value) in [
                (5, pack_1p(pos[0], 4)),
                (9, pack_2p(pos[0], pos[1], 4)),
                (13, pack_3p(pos[0], pos[1], pos[2], 4)),
                (16, pack_4p(&pos, 4)),
                (20, pack_5p(&pos, 4)),
                (22, pack_6p(&pos, 4)),
            ] {
                assert!(
                    (0..1 << width).contains(&value),
                    "a {width}-bit packing produced {value}"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 12000, "checked {checked} packings");
    }
}
