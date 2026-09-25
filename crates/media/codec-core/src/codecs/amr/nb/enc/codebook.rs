//! The AMR-NB encoder's eight algebraic-codebook searches, TS 26.090 §5.8.
//!
//! Implements, from the TS 26.073 fixed-point reference: `cor_h_x`/`cor_h_x2`
//! and `cor_h` (`cor_h.c`), `set_sign` and `set_sign12k2` (`set_sign.c`), the
//! per-rate entry points `code_2i40_9bits` (`c2_9pf.c`), `code_2i40_11bits`
//! (`c2_11pf.c`), `code_3i40_14bits` (`c3_14pf.c`), `code_4i40_17bits`
//! (`c4_17pf.c`), `code_8i40_31bits` (`c8_31pf.c`) and `code_10i40_35bits`
//! (`c1035pf.c`) with their searches, `build_code`s, `compress_code` and `q_p`,
//! the shared depth-first `search_10and8i40` (`s10_8pf.c`), and the dispatcher
//! `cbsearch` (`cbsearch.c`) including the pitch sharpening it applies itself
//! at 10.2 and 12.2 kbit/s.
//!
//! # What validated it
//!
//! `testdata/nb_enc_trace.txt`, produced by driving TS 26.073's own encoder at
//! **7.40 kbit/s** over `testdata/amrnb_enc_input.pcm`. The committed tests
//! drive [`search`] from the traced `xn2`, `h1` and `res` rows plus `T0` and a
//! sharpening factor reconstructed from the traced `gain_pit`, and compare the
//! resulting `code` and `y2` rows over three frames and twelve subframes. The
//! *chosen index* is checked separately, against the position and sign fields
//! of `testdata/amrnb_enc_mode4.amr` — the reference encoder's own bitstream —
//! because two different indices can build codevectors that score identically
//! and only the index says which one the reference picked.
//!
//! During development the same harness ran over all fifty frames (200
//! subframes) of `tools/trace-amrnb-encoder.sh`'s output at **every one of the
//! eight rates**, so every codebook here has been checked against the
//! reference. Only 7.40 kbit/s is covered by a test that ships, because only
//! that rate's trace is committed.
//!
//! # Why so much of this file is about tie-breaking
//!
//! Every stage maximises `ps²/alp` through the cross-multiplied difference
//! `alp_best·sq_cand − sq_best·alp_cand`, and accepts on **strictly greater
//! than zero** — never `>=`. An implementation that used `>=` would pick a
//! different-but-equally-good pulse on every exact tie, produce speech that
//! sounds identical, and fail conformance. The visit order therefore matters as
//! much as the objective, and each search states both.
//!
//! Three traps in particular survive a plausible-sounding implementation:
//!
//! - **The `sq = -1, alp = 1` sentinel does not force the first candidate in.**
//!   The test reduces to `sq1 + alp_16 > 0`, and `alp_16` can be negative
//!   because the private `correlate_impulse` helper folds the pulse signs into `rr`'s
//!   off-diagonal. A stage can genuinely finish with its index still at the
//!   reset value, and a whole sweep can finish with `codvec` still at its
//!   `codvec[i] = i` default.
//! - **`ps` resets to zero, not to the incoming `ps0`.** A stage that accepts
//!   nothing hands the next stage a zero correlation rather than the one it
//!   inherited.
//! - **The sign product in `rr` is `mult(sign_i, sign_j)`, which is `+32766`
//!   or `−32767`** — not `±1`. Positive and negative correlations are scaled
//!   by different factors, and collapsing them to a sign flip changes which
//!   candidate wins.
//!
//! # Q-formats
//!
//! `h` is Q12 and the target `xn2` is Q0. The correlation `dn` is normalised so
//! that the sum of the per-track maxima cannot saturate, and `rr` is normalised
//! against `h`'s own energy — neither carries a fixed Q. Codevectors come out
//! Q13 at every rate except 12.2 kbit/s, which is Q12; the filtered codevector
//! `y` is Q12.

use super::super::codebook::FixedCodebook;
use super::super::decoder_tables::{
    GRAY, START_POS1_2I40_11, START_POS2_2I40_11, START_POS_2I40_9, TRACK_TABLE_2I40_9,
};
use super::super::math::inv_sqrt;
use super::super::L_SUBFR;
use crate::fixed_point::arith::{add, extract_h, extract_l, mult, negate, round, sub};
use crate::fixed_point::arith32::{l_abs, l_add, l_deposit_h, l_mac, l_msu, l_mult, l_sub};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Codevector length, `L_CODE` in the reference — the same 40 samples as a
/// subframe.
pub const L_CODE: usize = L_SUBFR;

/// Track stride at every rate except 10.2 kbit/s.
const STEP: i16 = 5;
/// Number of interleaved position tracks at every rate except 10.2 kbit/s.
const NB_TRACK: usize = 5;
/// Track stride at 10.2 kbit/s: four tracks of ten slots rather than five of
/// eight.
const STEP_MR102: i16 = 4;
/// Number of interleaved position tracks at 10.2 kbit/s.
const NB_TRACK_MR102: usize = 4;

/// Most pulses any of the eight codebooks places.
const MAX_PULSES: usize = 10;

/// `1/5` in Q15, as the reference spells `position / 5`.
///
/// `mult` is an arithmetic `>>15` that floors, so this is exact for `0..=39`
/// and is *not* interchangeable with integer division over a wider input.
const RECIP_5: Word16 = Word16(6554);
/// `1/25` in Q15 for 10.2 kbit/s' third compressed index. Nominally
/// `1/24.994`, which is not the same as dividing by 25 for every input it sees.
const RECIP_25: Word16 = Word16(1311);

/// Search weight `1/2`, Q15.
const W_1_2: Word16 = Word16(16384);
/// Search weight `1/4`.
const W_1_4: Word16 = Word16(8192);
/// Search weight `1/8`.
const W_1_8: Word16 = Word16(4096);
/// Search weight `1/16`.
const W_1_16: Word16 = Word16(2048);
/// Search weight `1/32`.
const W_1_32: Word16 = Word16(1024);
/// Search weight `1/64`.
const W_1_64: Word16 = Word16(512);
/// Search weight `1/128`.
const W_1_128: Word16 = Word16(256);

/// Positive pulse amplitude, Q13, for the four narrow codebooks.
const POSITIVE_PULSE: Word16 = Word16(8191);
/// Negative pulse amplitude, Q13. Deliberately asymmetric with
/// [`POSITIVE_PULSE`].
const NEGATIVE_PULSE: Word16 = Word16(-8192);
/// Pulse amplitude at 10.2 kbit/s: symmetric, and accumulated rather than
/// stored, so two pulses on one sample double it.
const PULSE_MR102: Word16 = Word16(8191);
/// Pulse amplitude at 12.2 kbit/s. Half the rest, because its codevector is
/// Q12.
const PULSE_MR122: Word16 = Word16(4096);

/// Filter sign for a positive pulse in the `y = h * code` convolution.
const SIGN_POSITIVE: Word16 = Word16(32767);
/// Filter sign for a negative pulse: `−32768`, not `−32767`. The convolution
/// is asymmetric in exactly the way the pulse amplitudes are.
const SIGN_NEGATIVE: Word16 = Word16(-32768);
/// Filter sign magnitude at 12.2 kbit/s, matching its Q12 codevector.
const SIGN_MR122: Word16 = Word16(8192);

/// The 40×40 correlation matrix `rr` the searches evaluate `alp` from.
pub type ImpulseCorrelations = [[Word16; L_CODE]; L_CODE];

/// One subframe's fixed-codebook decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Innovation {
    /// The codevector, **with** pitch sharpening folded in. Q13, or Q12 at
    /// 12.2 kbit/s.
    pub code: [Word16; L_CODE],
    /// The codevector filtered by the (already sharpened) impulse response,
    /// Q12. The reference's `y2`.
    pub filtered: [Word16; L_CODE],
    /// The parameters that encode it — the same type the decoder consumes, so
    /// a round trip through [`FixedCodebook::decode`] checks the packing
    /// directly.
    pub params: FixedCodebook,
}

/// Everything one subframe's codebook search reads.
///
/// Two of the six fields are lag-derived and easy to transpose — `pitch_sharp`
/// and `gain_pit` are both Q14 pitch gains that mean different things — so they
/// travel named.
#[derive(Clone, Copy, Debug)]
pub struct CodebookInputs<'a> {
    /// The codebook target `xn2`, Q0.
    pub target: &'a [Word16; L_CODE],
    /// The weighted synthesis filter's impulse response, Q12, **unsharpened**.
    ///
    /// Read, not modified: the reference sharpens `st->h1` in place and leaves
    /// it that way, but `cod_amr` recomputes `h1` from scratch every subframe
    /// and never reads the sharpened copy back, so the mutation is not
    /// observable and is not worth exporting.
    pub impulse: &'a [Word16; L_CODE],
    /// `res2`, the LP residual minus the adaptive contribution, Q0.
    ///
    /// Only 10.2 and 12.2 kbit/s read it, and they use it to *sign* the pulses
    /// rather than to place them.
    pub ltp_residual: &'a [Word16; L_CODE],
    /// The integer pitch lag `T0`.
    pub lag: i16,
    /// The persistent sharpening state, Q14. Ignored at 12.2 kbit/s.
    pub pitch_sharp: Word16,
    /// The closed-loop pitch gain, Q14, already quantised at 12.2 kbit/s where
    /// it — and not `pitch_sharp` — drives the sharpening.
    pub gain_pit: Word16,
}

/// Search this subframe's fixed codebook, TS 26.073 `cbsearch`.
///
/// `mode_index` is the rate in the reference's numeric order, `0` = 4.75 kbit/s
/// through `7` = 12.2; anything above 7 is treated as 12.2, matching the
/// reference's `else`. `subframe` is the position in the frame, `0..=3`, and
/// only 4.75 and 5.15 kbit/s use it — their track pairs rotate across the
/// frame.
#[must_use]
pub fn search(
    ctx: &mut DspContext,
    mode_index: u8,
    subframe: u8,
    inputs: &CodebookInputs<'_>,
) -> Innovation {
    let &CodebookInputs {
        target,
        impulse,
        ltp_residual,
        lag,
        pitch_sharp,
        gain_pit,
    } = inputs;
    match mode_index {
        0 | 1 => two_pulses_9bit(ctx, subframe, target, impulse, lag, pitch_sharp),
        2 => two_pulses_11bit(ctx, target, impulse, lag, pitch_sharp),
        3 => three_pulses_14bit(ctx, target, impulse, lag, pitch_sharp),
        4 | 5 => four_pulses_17bit(ctx, target, impulse, lag, pitch_sharp),
        6 => eight_pulses_31bit(ctx, target, impulse, lag, pitch_sharp, ltp_residual),
        // The sharpening factor at 12.2 kbit/s is the *quantised pitch gain*,
        // not the persistent sharpening state. Order matters: `cl_ltp`
        // quantises the pitch gain before this runs, and feeding the
        // unquantised value here diverges the whole rate.
        _ => ten_pulses_35bit(ctx, target, impulse, lag, gain_pit, ltp_residual),
    }
}

// ---------------------------------------------------------------------------
// Shared front end
// ---------------------------------------------------------------------------

/// Correlate the target with the impulse response, TS 26.073 `cor_h_x2`.
///
/// `dn[n] = sum_{i>=n} x[i]·h[i−n]`, normalised so that the sum of the per-track
/// absolute maxima cannot saturate. `scale` is the reference's `sf`: 1 for the
/// six narrow rates, 2 for 10.2 and 12.2.
///
/// The running total starts at **5, not 0** — a deliberate bias that keeps
/// `norm_l` off an all-zero input.
fn correlate_target(
    ctx: &mut DspContext,
    h: &[Word16; L_CODE],
    x: &[Word16; L_CODE],
    scale: i16,
    tracks: usize,
    step: i16,
) -> [Word16; L_CODE] {
    let stride = usize::try_from(step).expect("the track stride is 4 or 5");
    let mut wide = [Word32(0); L_CODE];
    let mut total = Word32(5);

    for track in 0..tracks {
        let mut max = Word32(0);
        let mut i = track;
        while i < L_CODE {
            let mut s = Word32(0);
            for j in i..L_CODE {
                s = l_mac(ctx, s, x[j], h[j - i]);
            }
            wide[i] = s;
            let magnitude = l_abs(ctx, s);
            // Strict `>`: on a tie the earlier position keeps the maximum.
            // Only the value is used, so the direction is immaterial here —
            // recorded because "immaterial" is a claim, not an assumption.
            if l_sub(ctx, magnitude, max).0 > 0 {
                max = magnitude;
            }
            i += stride;
        }
        let half = l_shr(ctx, max, 1);
        total = l_add(ctx, total, half);
    }

    let shift = sub(ctx, Word16(norm_l(total)), Word16(scale));
    let mut dn = [Word16(0); L_CODE];
    for (out, &value) in dn.iter_mut().zip(wide.iter()) {
        // `shift` may be negative, in which case `l_shl` right-shifts.
        let scaled = l_shl(ctx, value, shift.0);
        *out = round(ctx, scaled);
    }
    dn
}

/// Pulse signs and per-track pruning, TS 26.073 `set_sign`.
///
/// Rewrites `dn` as its own magnitude — every later stage assumes it is
/// non-negative — and returns the signs alongside `dn2`, a copy in which all
/// but the `keep` largest entries of each track have been struck out with `−1`.
/// `keep` is 8 (no pruning) at 4.75/5.15/5.90, 6 at 6.70 and 4 at 7.40/7.95.
fn set_sign(
    ctx: &mut DspContext,
    dn: &mut [Word16; L_CODE],
    keep: usize,
) -> ([Word16; L_CODE], [Word16; L_CODE]) {
    let mut signs = [Word16(0); L_CODE];
    let mut pruned = [Word16(0); L_CODE];

    for i in 0..L_CODE {
        let mut value = dn[i];
        if value.0 >= 0 {
            signs[i] = SIGN_POSITIVE;
        } else {
            // ±32767, never ±32768 — and `negate` saturates, so a `dn` of
            // −32768 comes back 32767 rather than wrapping.
            signs[i] = Word16(-32767);
            value = negate(ctx, value);
        }
        dn[i] = value;
        pruned[i] = value;
    }

    let stride = usize::try_from(STEP).expect("the track stride is 5");
    // The reference declares `pos` once, outside both loops, and never resets
    // it. With `8 - keep <= 4` there is always a non-negative candidate left,
    // so the stale value cannot be read — the scope is reproduced rather than
    // tightened, because "cannot be read" is the claim being made.
    let mut pos = 0usize;
    for track in 0..NB_TRACK {
        for _ in 0..(8 - keep) {
            let mut min = Word16(0x7fff);
            let mut j = track;
            while j < L_CODE {
                // Strict `<`: among equal magnitudes the lowest position in the
                // track is the one struck out.
                if pruned[j].0 >= 0 && sub(ctx, pruned[j], min).0 < 0 {
                    min = pruned[j];
                    pos = j;
                }
                j += stride;
            }
            pruned[pos] = Word16(-1);
        }
    }

    (signs, pruned)
}

/// What [`set_sign_joint`] hands the wide-rate search.
struct JointSigns {
    /// Per-position pulse signs, ±32767.
    signs: [Word16; L_CODE],
    /// Per-track position of the strongest joint correlation.
    pos_max: [i16; NB_TRACK],
    /// Starting track for each of the `2·tracks` pulses: a cyclic enumeration
    /// beginning at the winning track. `ipos[i] == ipos[i + tracks]`, which is
    /// what guarantees exactly two pulses land in every track.
    ipos: [i16; MAX_PULSES],
}

/// Pulse signs for 10.2 and 12.2 kbit/s, TS 26.073 `set_sign12k2`.
///
/// The sign comes from the **joint** correlation of the LTP residual `cn` and
/// the target correlation `dn`, not from `dn` alone. The consequence is easy to
/// miss: unlike [`set_sign`], this leaves `dn` with entries that are still
/// negative, and a search written on the assumption that `dn >= 0` is plausible
/// and wrong.
///
/// Both energy sums are seeded with **256**, which changes the `inv_sqrt`
/// result and so the normalisation of both signals.
fn set_sign_joint(
    ctx: &mut DspContext,
    dn: &mut [Word16; L_CODE],
    cn: &[Word16; L_CODE],
    tracks: usize,
    step: i16,
) -> JointSigns {
    fn normalise(ctx: &mut DspContext, v: &[Word16; L_CODE]) -> Word16 {
        let mut s = Word32(256);
        for &x in v {
            s = l_mac(ctx, s, x, x);
        }
        let s = inv_sqrt(ctx, s);
        let s = l_shl(ctx, s, 5);
        extract_h(s)
    }
    let residual_scale = normalise(ctx, cn);
    let target_scale = normalise(ctx, dn);

    let mut signs = [Word16(0); L_CODE];
    let mut energy = [Word16(0); L_CODE];
    for i in 0..L_CODE {
        let mut value = dn[i];
        let residual_part = l_mult(ctx, residual_scale, cn[i]);
        let joint = l_mac(ctx, residual_part, target_scale, value);
        let scaled = l_shl(ctx, joint, 10);
        let mut correlation = round(ctx, scaled);
        if correlation.0 >= 0 {
            signs[i] = SIGN_POSITIVE;
        } else {
            signs[i] = Word16(-32767);
            correlation = negate(ctx, correlation);
            value = negate(ctx, value);
        }
        dn[i] = value;
        energy[i] = correlation;
    }

    let stride = usize::try_from(step).expect("the track stride is 4 or 5");
    let mut pos_max = [0i16; NB_TRACK];
    let mut best_track = 0i16;
    let mut max_of_all = Word16(-1);
    let mut pos = 0usize;
    for (track, best) in pos_max.iter_mut().enumerate().take(tracks) {
        let mut max = Word16(-1);
        let mut j = track;
        while j < L_CODE {
            // Strict `>`: the earliest position in the track wins a tie.
            if sub(ctx, energy[j], max).0 > 0 {
                max = energy[j];
                pos = j;
            }
            j += stride;
        }
        *best = i16::try_from(pos).expect("a position is 0..=39");
        // Strict `>` again: the lowest track index wins a tie for the start.
        if sub(ctx, max, max_of_all).0 > 0 {
            max_of_all = max;
            best_track = i16::try_from(track).expect("a track index is small");
        }
    }

    let mut ipos = [0i16; MAX_PULSES];
    let width = i16::try_from(tracks).expect("a track count is small");
    let mut cursor = best_track;
    ipos[0] = cursor;
    ipos[tracks] = cursor;
    for i in 1..tracks {
        cursor += 1;
        if cursor >= width {
            cursor = 0;
        }
        ipos[i] = cursor;
        ipos[i + tracks] = cursor;
    }

    JointSigns {
        signs,
        pos_max,
        ipos,
    }
}

/// Autocorrelation of the impulse response with the pulse signs folded in,
/// TS 26.073 `cor_h`.
///
/// `rr[i][j] = <h[n−i], h[n−j]>·sign[i]·sign[j]` off the diagonal. Three details
/// are load-bearing:
///
/// - the diagonal is filled **backwards**, so `rr[39][39]` is `h2[0]²` and
///   `rr[0][0]` is the whole energy;
/// - the diagonal carries **no** sign factor;
/// - the sign product is `mult(sign_i, sign_j)`, i.e. `+32766` for a matching
///   pair and `−32767` for a mismatched one. That asymmetry is a real scaling
///   difference of one part in 32768, and it decides ties.
fn correlate_impulse(
    ctx: &mut DspContext,
    h: &[Word16; L_CODE],
    signs: &[Word16; L_CODE],
) -> ImpulseCorrelations {
    // Seeded with 2, not 0.
    let mut energy = Word32(2);
    for &tap in h {
        energy = l_mac(ctx, energy, tap, tap);
    }

    let mut h2 = [Word16(0); L_CODE];
    let saturated = sub(ctx, extract_h(energy), Word16(32767)).0 == 0;
    if saturated {
        // The energy saturated: there is no headroom to normalise into, so
        // halve instead.
        for (out, &tap) in h2.iter_mut().zip(h.iter()) {
            *out = shr(ctx, tap, 1);
        }
    } else {
        let halved = l_shr(ctx, energy, 1);
        let reciprocal = inv_sqrt(ctx, halved);
        let mut k = extract_h(l_shl(ctx, reciprocal, 7));
        // 32440 Q15 = 0.99: a margin so the normalised response cannot round up
        // into saturation.
        k = mult(ctx, k, Word16(32440));
        for (out, &tap) in h2.iter_mut().zip(h.iter()) {
            let product = l_mult(ctx, tap, k);
            let scaled = l_shl(ctx, product, 9);
            *out = round(ctx, scaled);
        }
    }

    let mut rr = [[Word16(0); L_CODE]; L_CODE];

    let mut s = Word32(0);
    for (k, &tap) in h2.iter().enumerate() {
        s = l_mac(ctx, s, tap, tap);
        let i = L_CODE - 1 - k;
        rr[i][i] = round(ctx, s);
    }

    for dec in 1..L_CODE {
        let mut s = Word32(0);
        for k in 0..(L_CODE - dec) {
            s = l_mac(ctx, s, h2[k], h2[k + dec]);
            let j = L_CODE - 1 - k;
            let i = j - dec;
            let product = mult(ctx, signs[i], signs[j]);
            let magnitude = round(ctx, s);
            let value = mult(ctx, magnitude, product);
            rr[j][i] = value;
            rr[i][j] = value;
        }
    }

    rr
}

/// Convolve the chosen pulses with the impulse response — the tail every
/// `build_code` shares.
///
/// The reference indexes `h` negatively (`p = h - codvec[k]`) and relies on
/// `h[-40..-1]` being zero; the guard here is that zero, spelled out.
fn filter_pulses(
    ctx: &mut DspContext,
    h: &[Word16; L_CODE],
    positions: &[i16],
    signs: &[Word16],
) -> [Word16; L_CODE] {
    let mut y = [Word16(0); L_CODE];
    for (i, out) in y.iter_mut().enumerate() {
        let mut s = Word32(0);
        for (&position, &sign) in positions.iter().zip(signs.iter()) {
            let offset = i16::try_from(i).expect("a sample index is 0..=39") - position;
            let tap = if offset < 0 {
                Word16(0)
            } else {
                h[usize::try_from(offset).expect("checked non-negative")]
            };
            s = l_mac(ctx, s, tap, sign);
        }
        *out = round(ctx, s);
    }
    y
}

/// Split a position into `(position / 5, position % 5)` the way the reference
/// does, with a Q15 reciprocal rather than a division.
fn split_position(ctx: &mut DspContext, position: i16) -> (Word16, usize) {
    let index = mult(ctx, Word16(position), RECIP_5);
    let fivefold = l_mult(ctx, index, Word16(5));
    let base = extract_l(l_shr(ctx, fivefold, 1));
    let track = sub(ctx, Word16(position), base);
    (
        index,
        usize::try_from(track.0).expect("a track index is 0..=4"),
    )
}

/// `add(shl(index, bits), offset)`, the shape every position-packing branch
/// takes: shift the sub-index into its field, then set the sub-track bits.
fn shift_into_field(ctx: &mut DspContext, index: Word16, bits: i16, offset: i16) -> Word16 {
    let shifted = shl(ctx, index, bits);
    add(ctx, shifted, Word16(offset))
}

/// The accept test every stage of every search shares.
///
/// Maximising `ps²/alp` without dividing: accept when
/// `alp_incumbent·sq_candidate − sq_incumbent·alp_candidate > 0`. **Strictly**
/// greater, so an exact tie keeps the incumbent — and because every sweep
/// visits its track in ascending order, that means the earlier candidate.
///
/// The `(sq, alp) = (−1, 1)` sentinel does not make this unconditionally true
/// for a first candidate: it reduces to `sq_candidate + alp_candidate > 0`, and
/// `alp_candidate` can be negative because `rr` carries the pulse signs.
fn improves(
    ctx: &mut DspContext,
    incumbent: (Word16, Word16),
    candidate: (Word16, Word16),
) -> bool {
    let (best_sq, best_alp) = incumbent;
    let (sq, alp) = candidate;
    let scaled = l_mult(ctx, best_alp, sq);
    l_msu(ctx, scaled, best_sq, alp).0 > 0
}

/// Add the pitch contribution to a signal, in place and self-referencing.
///
/// `v[i] += mult(v[i − lag], factor)` running forward, so once `i − lag >= lag`
/// the loop reads entries it has already modified. That recursion is the point
/// — it is what turns one pulse into a train — and a read-before-write pass
/// gives a different, quieter codevector.
///
/// The reference guards the narrow rates' copy of this loop with
/// `if (T0 < L_CODE)` and leaves the wide rates' copy unguarded; the guard is
/// redundant, because a lag of 40 or more produces no iterations either way.
fn add_pitch_contribution(
    ctx: &mut DspContext,
    v: &mut [Word16; L_CODE],
    lag: i16,
    factor: Word16,
) {
    let Ok(lag) = usize::try_from(lag) else {
        return;
    };
    for i in lag..L_CODE {
        let echo = mult(ctx, v[i - lag], factor);
        v[i] = add(ctx, v[i], echo);
    }
}

// ---------------------------------------------------------------------------
// The two-pulse searches: 4.75 / 5.15 (9 bit) and 5.90 (11 bit)
// ---------------------------------------------------------------------------

/// Running best across a two-pulse search: the reference's `psk`, `alpk` and
/// `codvec`.
struct BestPair {
    /// `psk`, the squared correlation of the best pair so far.
    numerator: Word16,
    /// `alpk`, its energy.
    denominator: Word16,
    /// `codvec`, defaulting to `[0, 1]` — the output if nothing is accepted.
    positions: [i16; 2],
}

/// One `(i0, i1)` sweep from a fixed pair of track starts, 8×8 evaluations.
///
/// Shared verbatim by the 9-bit and 11-bit codebooks, which differ only in how
/// many track pairs they sweep. Note that no `ps` is carried out of the inner
/// loop: with two pulses there is no third stage to carry it to.
fn two_pulse_pass(
    ctx: &mut DspContext,
    dn: &[Word16; L_CODE],
    rr: &ImpulseCorrelations,
    ipos: [i16; 2],
    best: &mut BestPair,
) {
    let mut i0 = ipos[0];
    while i0 < 40 {
        let first = usize::try_from(i0).expect("a position is 0..=39");
        let ps0 = dn[first];
        let alp0 = l_mult(ctx, rr[first][first], W_1_4);

        let mut sq = Word16(-1);
        let mut alp = Word16(1);
        let mut chosen = ipos[1];

        let mut i1 = ipos[1];
        while i1 < 40 {
            let second = usize::try_from(i1).expect("a position is 0..=39");
            let ps1 = add(ctx, ps0, dn[second]);
            let mut alp1 = l_mac(ctx, alp0, rr[second][second], W_1_4);
            alp1 = l_mac(ctx, alp1, rr[first][second], W_1_2);
            let sq1 = mult(ctx, ps1, ps1);
            let alp16 = round(ctx, alp1);
            if improves(ctx, (sq, alp), (sq1, alp16)) {
                sq = sq1;
                alp = alp16;
                chosen = i1;
            }
            i1 += STEP;
        }

        if improves(ctx, (best.numerator, best.denominator), (sq, alp)) {
            best.numerator = sq;
            best.denominator = alp;
            best.positions = [i0, chosen];
        }
        i0 += STEP;
    }
}

/// 4.75 and 5.15 kbit/s, TS 26.073 `code_2i40_9bits`.
///
/// Two pulses, 7 position bits and 2 sign bits. The two track pairs come from
/// `startPos` and rotate across the four subframes, which is why this is the
/// only codebook that needs to know where it is in the frame.
///
/// Visit order: track pair (2) → `i0` (8 positions) → `i1` (8), 128
/// evaluations, no pruning.
fn two_pulses_9bit(
    ctx: &mut DspContext,
    subframe: u8,
    target: &[Word16; L_CODE],
    h: &[Word16; L_CODE],
    lag: i16,
    pitch_sharp: Word16,
) -> Innovation {
    let sharp = shl(ctx, pitch_sharp, 1);
    let mut h = *h;
    add_pitch_contribution(ctx, &mut h, lag, sharp);

    let mut dn = correlate_target(ctx, &h, target, 1, NB_TRACK, STEP);
    // `keep = 8` means no pruning; this search ignores `dn2` entirely.
    let (signs, _) = set_sign(ctx, &mut dn, 8);
    let rr = correlate_impulse(ctx, &h, &signs);

    let mut best = BestPair {
        numerator: Word16(-1),
        denominator: Word16(1),
        positions: [0, 1],
    };
    for pair in 0..2usize {
        let base = usize::from(subframe) * 2 + 8 * pair;
        two_pulse_pass(
            ctx,
            &dn,
            &rr,
            [START_POS_2I40_9[base], START_POS_2I40_9[base + 1]],
            &mut best,
        );
    }

    let (mut code, positions, sign_word, filter_signs) =
        place_narrow_pulses(ctx, &best.positions, &signs, |ctx, k, position| {
            let (mut index, track) = split_position(ctx, position);
            // A `-1` entry marks a track this subframe's search never visits,
            // so it is unreachable; the reference's `else` treats it exactly
            // like `1`, and so does this.
            let first = TRACK_TABLE_2I40_9[5 * usize::from(subframe) + track];
            if k == 0 {
                if first != 0 {
                    index = add(ctx, index, Word16(64));
                }
                (index, 0)
            } else {
                (shl(ctx, index, 3), 1)
            }
        });

    let filtered = filter_pulses(ctx, &h, &best.positions, &filter_signs);
    add_pitch_contribution(ctx, &mut code, lag, sharp);

    Innovation {
        code,
        filtered,
        params: FixedCodebook::TwoPulses9Bit {
            subframe,
            signs: sign_word,
            positions,
        },
    }
}

/// 5.90 kbit/s, TS 26.073 `code_2i40_11bits`.
///
/// Two pulses, 9 position bits and 2 sign bits. Same inner arithmetic as the
/// 9-bit codebook, but eight track pairs rather than two with `track1`
/// outermost — so on a tie the earliest `(track1, track2)` pair wins. 512
/// evaluations.
fn two_pulses_11bit(
    ctx: &mut DspContext,
    target: &[Word16; L_CODE],
    h: &[Word16; L_CODE],
    lag: i16,
    pitch_sharp: Word16,
) -> Innovation {
    let sharp = shl(ctx, pitch_sharp, 1);
    let mut h = *h;
    add_pitch_contribution(ctx, &mut h, lag, sharp);

    let mut dn = correlate_target(ctx, &h, target, 1, NB_TRACK, STEP);
    let (signs, _) = set_sign(ctx, &mut dn, 8);
    let rr = correlate_impulse(ctx, &h, &signs);

    let mut best = BestPair {
        numerator: Word16(-1),
        denominator: Word16(1),
        positions: [0, 1],
    };
    for &start1 in &START_POS1_2I40_11 {
        for &start2 in &START_POS2_2I40_11 {
            two_pulse_pass(ctx, &dn, &rr, [start1, start2], &mut best);
        }
    }

    let (mut code, positions, sign_word, filter_signs) =
        place_narrow_pulses(ctx, &best.positions, &signs, |ctx, k, position| {
            let (index, track) = split_position(ctx, position);
            match track {
                0 => (shift_into_field(ctx, index, 6, 0), 1),
                1 if k == 0 => (shift_into_field(ctx, index, 1, 0), 0),
                1 => (shift_into_field(ctx, index, 6, 16), 1),
                2 => (shift_into_field(ctx, index, 6, 32), 1),
                3 => (shift_into_field(ctx, index, 1, 1), 0),
                _ => (shift_into_field(ctx, index, 6, 48), 1),
            }
        });

    let filtered = filter_pulses(ctx, &h, &best.positions, &filter_signs);
    add_pitch_contribution(ctx, &mut code, lag, sharp);

    Innovation {
        code,
        filtered,
        params: FixedCodebook::TwoPulses11Bit {
            signs: sign_word,
            positions,
        },
    }
}

// ---------------------------------------------------------------------------
// The three- and four-pulse searches: 6.70 (14 bit), 7.40 / 7.95 (17 bit)
// ---------------------------------------------------------------------------

/// Running best across a three- or four-pulse search.
struct BestPulses {
    /// `psk`.
    numerator: Word16,
    /// `alpk`.
    denominator: Word16,
    /// `codvec`, defaulting to `[0, 1, 2, …]`.
    positions: [i16; 4],
}

/// One sweep from a fixed track assignment, for the 3- and 4-pulse codebooks.
///
/// Stage 0 sweeps `i0` over the positions its track kept after pruning; every
/// later stage sweeps its own track unpruned and keeps only that stage's
/// winner, so this is depth-first rather than exhaustive.
///
/// Two details a cleaner-looking rewrite loses:
///
/// - `ps` resets to **0** at the top of each stage, so a stage that accepts
///   nothing hands the next one a zero rather than the correlation it
///   inherited;
/// - the final stage of a four-pulse search seeds `alp0` with
///   `l_deposit_h(alp)` — a shift of 16 — where stage 2 uses
///   `l_mult(alp, 1/4)`, a shift of 14. The two differ by a factor of four in
///   how heavily the accumulated energy is weighted against the last pulse's
///   own, and swapping them is invisible in the output audio.
fn pulse_pass(
    ctx: &mut DspContext,
    dn: &[Word16; L_CODE],
    pruned: &[Word16; L_CODE],
    rr: &ImpulseCorrelations,
    ipos: &[i16],
    best: &mut BestPulses,
) {
    let pulses = ipos.len();
    let last = pulses - 1;

    let mut i0 = ipos[0];
    while i0 < 40 {
        let first = usize::try_from(i0).expect("a position is 0..=39");
        if pruned[first].0 < 0 {
            i0 += STEP;
            continue;
        }

        let mut chosen = [0i16; 4];
        chosen[0] = i0;
        let mut ps = dn[first];
        let mut alp0 = l_mult(ctx, rr[first][first], W_1_4);
        let mut alp = Word16(1);
        let mut sq = Word16(-1);

        for stage in 1..pulses {
            if stage >= 2 {
                alp0 = if stage == last && pulses == 4 {
                    l_deposit_h(alp)
                } else {
                    l_mult(ctx, alp, W_1_4)
                };
            }
            let (diagonal, cross) = if stage == 1 {
                (W_1_4, W_1_2)
            } else {
                (W_1_16, W_1_8)
            };
            let ps0 = ps;

            sq = Word16(-1);
            alp = Word16(1);
            ps = Word16(0);
            let mut ix = ipos[stage];

            let mut candidate = ipos[stage];
            while candidate < 40 {
                let slot = usize::try_from(candidate).expect("a position is 0..=39");
                let ps1 = add(ctx, ps0, dn[slot]);
                let mut alp1 = l_mac(ctx, alp0, rr[slot][slot], diagonal);
                // Descending over the pulses already fixed: the reference
                // accumulates `rr[i2][i3]`, then `rr[i1][i3]`, then
                // `rr[i0][i3]`, and saturation makes the order observable.
                for &earlier in chosen[..stage].iter().rev() {
                    let fixed = usize::try_from(earlier).expect("a position is 0..=39");
                    alp1 = l_mac(ctx, alp1, rr[fixed][slot], cross);
                }
                let sq1 = mult(ctx, ps1, ps1);
                let alp16 = round(ctx, alp1);
                if improves(ctx, (sq, alp), (sq1, alp16)) {
                    sq = sq1;
                    ps = ps1;
                    alp = alp16;
                    ix = candidate;
                }
                candidate += STEP;
            }

            chosen[stage] = ix;
        }

        if improves(ctx, (best.numerator, best.denominator), (sq, alp)) {
            best.numerator = sq;
            best.denominator = alp;
            best.positions = chosen;
        }
        i0 += STEP;
    }
}

/// 6.70 kbit/s, TS 26.073 `code_3i40_14bits`.
///
/// Three pulses, 11 position bits and 3 sign bits. `set_sign` keeps 6 of the 8
/// positions per track. Visit order: `track1 ∈ {1,3}` → `track2 ∈ {2,4}` →
/// three cyclic rotations of the track assignment → `i0` (6 surviving) → `i1`
/// (8) → `i2` (8).
///
/// The rotation is what makes the pruning matter: each rotation prunes a
/// *different* track, because pruning applies to whichever track has landed in
/// slot 0.
fn three_pulses_14bit(
    ctx: &mut DspContext,
    target: &[Word16; L_CODE],
    h: &[Word16; L_CODE],
    lag: i16,
    pitch_sharp: Word16,
) -> Innovation {
    let sharp = shl(ctx, pitch_sharp, 1);
    let mut h = *h;
    add_pitch_contribution(ctx, &mut h, lag, sharp);

    let mut dn = correlate_target(ctx, &h, target, 1, NB_TRACK, STEP);
    let (signs, pruned) = set_sign(ctx, &mut dn, 6);
    let rr = correlate_impulse(ctx, &h, &signs);

    let mut best = BestPulses {
        numerator: Word16(-1),
        denominator: Word16(1),
        positions: [0, 1, 2, 0],
    };
    for track1 in [1i16, 3] {
        for track2 in [2i16, 4] {
            let mut ipos = [0i16, track1, track2];
            for _ in 0..3 {
                pulse_pass(ctx, &dn, &pruned, &rr, &ipos, &mut best);
                ipos.rotate_right(1);
            }
        }
    }

    let chosen = &best.positions[..3];
    let (mut code, positions, sign_word, filter_signs) =
        place_narrow_pulses(ctx, chosen, &signs, |ctx, _k, position| {
            let (index, track) = split_position(ctx, position);
            match track {
                0 => (index, 0),
                1 => (shift_into_field(ctx, index, 4, 0), 1),
                2 => (shift_into_field(ctx, index, 8, 0), 2),
                3 => (shift_into_field(ctx, index, 4, 8), 1),
                _ => (shift_into_field(ctx, index, 8, 128), 2),
            }
        });

    let filtered = filter_pulses(ctx, &h, chosen, &filter_signs);
    add_pitch_contribution(ctx, &mut code, lag, sharp);

    Innovation {
        code,
        filtered,
        params: FixedCodebook::ThreePulses14Bit {
            signs: sign_word,
            positions,
        },
    }
}

/// 7.40 and 7.95 kbit/s, TS 26.073 `code_4i40_17bits`.
///
/// Four pulses, 13 position bits and 4 sign bits. `set_sign` keeps 4 of the 8
/// positions per track. Visit order: `track ∈ {3,4}` → four cyclic rotations →
/// `i0` (4 surviving) → `i1`, `i2`, `i3` (8 each).
///
/// Positions are **Gray-coded before** they are shifted into place, which is
/// the opposite of what 12.2 kbit/s does.
fn four_pulses_17bit(
    ctx: &mut DspContext,
    target: &[Word16; L_CODE],
    h: &[Word16; L_CODE],
    lag: i16,
    pitch_sharp: Word16,
) -> Innovation {
    let sharp = shl(ctx, pitch_sharp, 1);
    let mut h = *h;
    add_pitch_contribution(ctx, &mut h, lag, sharp);

    let mut dn = correlate_target(ctx, &h, target, 1, NB_TRACK, STEP);
    let (signs, pruned) = set_sign(ctx, &mut dn, 4);
    let rr = correlate_impulse(ctx, &h, &signs);

    let mut best = BestPulses {
        numerator: Word16(-1),
        denominator: Word16(1),
        positions: [0, 1, 2, 3],
    };
    for track in [3i16, 4] {
        let mut ipos = [0i16, 1, 2, track];
        for _ in 0..4 {
            pulse_pass(ctx, &dn, &pruned, &rr, &ipos, &mut best);
            ipos.rotate_right(1);
        }
    }

    let (mut code, positions, sign_word, filter_signs) =
        place_narrow_pulses(ctx, &best.positions, &signs, |ctx, _k, position| {
            let (index, track) = split_position(ctx, position);
            let index = Word16(GRAY[usize::try_from(index.0).expect("a sub-index is 0..=7")]);
            match track {
                0 => (index, 0),
                1 => (shift_into_field(ctx, index, 3, 0), 1),
                2 => (shift_into_field(ctx, index, 6, 0), 2),
                3 => (shift_into_field(ctx, index, 10, 0), 3),
                _ => (shift_into_field(ctx, index, 10, 512), 3),
            }
        });

    let filtered = filter_pulses(ctx, &h, &best.positions, &filter_signs);
    add_pitch_contribution(ctx, &mut code, lag, sharp);

    Innovation {
        code,
        filtered,
        params: FixedCodebook::FourPulses17Bit {
            signs: sign_word,
            positions,
        },
    }
}

/// The `build_code` body the four narrow codebooks share.
///
/// `encode` maps a pulse to `(index contribution, sign-bit position)`;
/// everything else — the asymmetric `+8191`/`−8192` amplitudes, the
/// `±32767`/`−32768` filter signs, the accumulation of the position word — is
/// common. Returns the codevector **before** pitch sharpening, the position
/// word, the sign word, and the filter signs.
fn place_narrow_pulses<F>(
    ctx: &mut DspContext,
    positions: &[i16],
    signs: &[Word16; L_CODE],
    mut encode: F,
) -> ([Word16; L_CODE], u16, u16, [Word16; 4])
where
    F: FnMut(&mut DspContext, usize, i16) -> (Word16, i16),
{
    let mut code = [Word16(0); L_CODE];
    let mut filter_signs = [Word16(0); 4];
    let mut index_word = Word16(0);
    let mut sign_word = Word16(0);

    for (k, &position) in positions.iter().enumerate() {
        let slot = usize::try_from(position).expect("a position is 0..=39");
        let (contribution, sign_bit) = encode(ctx, k, position);

        if signs[slot].0 > 0 {
            code[slot] = POSITIVE_PULSE;
            filter_signs[k] = SIGN_POSITIVE;
            let bit = shl(ctx, Word16(1), sign_bit);
            sign_word = add(ctx, sign_word, bit);
        } else {
            code[slot] = NEGATIVE_PULSE;
            filter_signs[k] = SIGN_NEGATIVE;
        }
        index_word = add(ctx, index_word, contribution);
    }

    (
        code,
        u16::try_from(index_word.0).expect("a position word is non-negative"),
        u16::try_from(sign_word.0).expect("a sign word is non-negative"),
        filter_signs,
    )
}

// ---------------------------------------------------------------------------
// The wide searches: 10.2 (31 bit, 8 pulses) and 12.2 (35 bit, 10 pulses)
// ---------------------------------------------------------------------------

/// How many pulses, tracks and slots a wide rate's search covers.
///
/// The reference passes the three as separate arguments to one function; they
/// are grouped here because only two combinations exist and getting one of the
/// three wrong silently changes the number of stages rather than failing.
#[derive(Clone, Copy, Debug)]
struct WideLayout {
    /// Pulses to place.
    pulses: usize,
    /// Distance between consecutive slots of a track.
    step: i16,
    /// Number of interleaved tracks.
    tracks: usize,
}

/// 10.2 kbit/s: eight pulses over four tracks of ten slots.
const MR102_LAYOUT: WideLayout = WideLayout {
    pulses: 8,
    step: STEP_MR102,
    tracks: NB_TRACK_MR102,
};

/// 12.2 kbit/s: ten pulses over five tracks of eight slots.
const MR122_LAYOUT: WideLayout = WideLayout {
    pulses: 10,
    step: STEP,
    tracks: NB_TRACK,
};

/// Weights for one `(a, b)` pair stage of [`search_pairs`].
struct PairStage {
    /// Weight on `rr[b][b]` in the pre-pass.
    pre_diagonal: Word16,
    /// Weight on each `rr[fixed][b]` in the pre-pass.
    pre_cross: Word16,
    /// Weight on `rr[a][a]` in the inner sweep.
    inner_diagonal: Word16,
    /// Weight on each `rr[fixed][a]` in the inner sweep.
    inner_cross: Word16,
    /// Weight on the pre-pass result `rrv[b]`.
    rrv: Word16,
    /// Weight on `rr[a][b]`, the term that couples the pair.
    pair: Word16,
}

/// The four pair stages, in the order they run.
///
/// The weights halve stage by stage because each stage adds two more pulses to
/// a sum that already carries the ones before it. 10.2 kbit/s runs the first
/// three; 12.2 runs all four.
const PAIR_STAGES: [PairStage; 4] = [
    PairStage {
        pre_diagonal: W_1_8,
        pre_cross: W_1_4,
        inner_diagonal: W_1_16,
        inner_cross: W_1_8,
        rrv: W_1_2,
        pair: W_1_8,
    },
    PairStage {
        pre_diagonal: W_1_8,
        pre_cross: W_1_4,
        inner_diagonal: W_1_32,
        inner_cross: W_1_16,
        rrv: W_1_4,
        pair: W_1_16,
    },
    PairStage {
        pre_diagonal: W_1_16,
        pre_cross: W_1_8,
        inner_diagonal: W_1_64,
        inner_cross: W_1_32,
        rrv: W_1_4,
        pair: W_1_32,
    },
    PairStage {
        pre_diagonal: W_1_16,
        pre_cross: W_1_8,
        inner_diagonal: W_1_128,
        inner_cross: W_1_64,
        rrv: W_1_8,
        pair: W_1_64,
    },
];

/// The paired depth-first search 10.2 and 12.2 kbit/s share,
/// TS 26.073 `search_10and8i40`.
///
/// This is **not** a nested search over all pulses. Its shape:
///
/// - `i0` is fixed for the whole call at `pos_max[ipos[0]]` and is never
///   searched; `ipos[0]` is never rotated;
/// - the outer loop runs `tracks − 1` times, not `tracks`;
/// - `i1` is likewise read from `pos_max` rather than searched;
/// - the remaining pulses are chosen two at a time, each pair jointly over
///   `positions × positions` candidates and conditioned on every pulse already
///   fixed;
/// - between outer iterations only `ipos[1..pulses]` rotates left, and the
///   bound is the *pulse* count, so the two slots naming the same track stay
///   paired.
///
/// `ipos` is modified in place, exactly as in the reference.
fn search_pairs(
    ctx: &mut DspContext,
    layout: WideLayout,
    dn: &[Word16; L_CODE],
    rr: &ImpulseCorrelations,
    ipos: &mut [i16; MAX_PULSES],
    pos_max: &[i16; NB_TRACK],
) -> [i16; MAX_PULSES] {
    let WideLayout {
        pulses,
        step,
        tracks,
    } = layout;
    let stages = (pulses - 2) / 2;

    let mut codvec = [0i16; MAX_PULSES];
    for (i, slot) in codvec.iter_mut().enumerate() {
        *slot = i16::try_from(i).expect("a pulse index is small");
    }
    let mut best_numerator = Word16(-1);
    let mut best_denominator = Word16(1);

    let i0 = pos_max[usize::try_from(ipos[0]).expect("a track index is small")];

    // Shared by every pre-pass and never cleared. Safe only because each
    // pre-pass writes every position of its own track before the sweep that
    // reads it — which is why the write loop has to stay complete.
    let mut rrv = [Word16(0); L_CODE];

    for _ in 1..tracks {
        let mut chosen = [0i16; MAX_PULSES];
        chosen[0] = i0;
        chosen[1] = pos_max[usize::try_from(ipos[1]).expect("a track index is small")];

        let first = usize::try_from(chosen[0]).expect("a position is 0..=39");
        let second = usize::try_from(chosen[1]).expect("a position is 0..=39");
        let mut ps = add(ctx, dn[first], dn[second]);
        let mut alp0 = l_mult(ctx, rr[first][first], W_1_16);
        alp0 = l_mac(ctx, alp0, rr[second][second], W_1_16);
        alp0 = l_mac(ctx, alp0, rr[first][second], W_1_8);

        let mut sq = Word16(-1);
        let mut alp = Word16(1);

        for (stage, weights) in PAIR_STAGES.iter().take(stages).enumerate() {
            let fixed = 2 + 2 * stage;
            let a_track = ipos[fixed];
            let b_track = ipos[fixed + 1];

            let mut b = b_track;
            while b < 40 {
                let slot = usize::try_from(b).expect("a position is 0..=39");
                let mut s = l_mult(ctx, rr[slot][slot], weights.pre_diagonal);
                // Ascending over the pulses already fixed — unlike the 3- and
                // 4-pulse searches, which accumulate descending.
                for &earlier in &chosen[..fixed] {
                    let p = usize::try_from(earlier).expect("a position is 0..=39");
                    s = l_mac(ctx, s, rr[p][slot], weights.pre_cross);
                }
                rrv[slot] = round(ctx, s);
                b += step;
            }

            let ps0 = ps;
            sq = Word16(-1);
            alp = Word16(1);
            ps = Word16(0);
            let mut ia = a_track;
            let mut ib = b_track;

            let mut a = a_track;
            while a < 40 {
                let a_slot = usize::try_from(a).expect("a position is 0..=39");
                let ps1 = add(ctx, ps0, dn[a_slot]);
                let mut alp1 = l_mac(ctx, alp0, rr[a_slot][a_slot], weights.inner_diagonal);
                for &earlier in &chosen[..fixed] {
                    let p = usize::try_from(earlier).expect("a position is 0..=39");
                    alp1 = l_mac(ctx, alp1, rr[p][a_slot], weights.inner_cross);
                }

                let mut b = b_track;
                while b < 40 {
                    let b_slot = usize::try_from(b).expect("a position is 0..=39");
                    let ps2 = add(ctx, ps1, dn[b_slot]);
                    let mut alp2 = l_mac(ctx, alp1, rrv[b_slot], weights.rrv);
                    alp2 = l_mac(ctx, alp2, rr[a_slot][b_slot], weights.pair);
                    let sq2 = mult(ctx, ps2, ps2);
                    let alp16 = round(ctx, alp2);
                    if improves(ctx, (sq, alp), (sq2, alp16)) {
                        sq = sq2;
                        ps = ps2;
                        alp = alp16;
                        ia = a;
                        ib = b;
                    }
                    b += step;
                }
                a += step;
            }

            chosen[fixed] = ia;
            chosen[fixed + 1] = ib;
            alp0 = l_mult(ctx, alp, W_1_2);
        }

        if improves(ctx, (best_numerator, best_denominator), (sq, alp)) {
            best_numerator = sq;
            best_denominator = alp;
            codvec[..pulses].copy_from_slice(&chosen[..pulses]);
        }

        ipos[1..pulses].rotate_left(1);
    }

    codvec
}

/// 10.2 kbit/s, TS 26.073 `code_8i40_31bits`.
///
/// Eight pulses over four tracks of ten slots, two per track: four sign bits
/// plus 10 + 10 + 7 position bits. The pitch sharpening for this rate lives in
/// `cbsearch` rather than in the codebook, and is applied to `h` before the
/// search and to `code` after it.
fn eight_pulses_31bit(
    ctx: &mut DspContext,
    target: &[Word16; L_CODE],
    h: &[Word16; L_CODE],
    lag: i16,
    pitch_sharp: Word16,
    ltp_residual: &[Word16; L_CODE],
) -> Innovation {
    let sharp = shl(ctx, pitch_sharp, 1);
    let mut h = *h;
    add_pitch_contribution(ctx, &mut h, lag, sharp);

    // `scale = 2` selects the GSM-EFR normalisation; the four-track form
    // matches this rate's own track layout.
    let mut dn = correlate_target(ctx, &h, target, 2, NB_TRACK_MR102, STEP_MR102);
    let mut joint = set_sign_joint(ctx, &mut dn, ltp_residual, NB_TRACK_MR102, STEP_MR102);
    let rr = correlate_impulse(ctx, &h, &joint.signs);
    let codvec = search_pairs(ctx, MR102_LAYOUT, &dn, &rr, &mut joint.ipos, &joint.pos_max);

    let (mut code, filter_signs, sign_index, pos_index) =
        build_code_mr102(ctx, &codvec[..8], &joint.signs);
    let filtered = filter_pulses(ctx, &h, &codvec[..8], &filter_signs);
    add_pitch_contribution(ctx, &mut code, lag, sharp);

    Innovation {
        code,
        filtered,
        params: FixedCodebook::EightPulses31Bit(compress_mr102(ctx, sign_index, &pos_index)),
    }
}

/// 12.2 kbit/s, TS 26.073 `code_10i40_35bits`.
///
/// Ten pulses over five tracks of eight slots, two per track: five 4-bit fields
/// (sign plus Gray-coded position) and five 3-bit Gray-coded positions.
fn ten_pulses_35bit(
    ctx: &mut DspContext,
    target: &[Word16; L_CODE],
    h: &[Word16; L_CODE],
    lag: i16,
    gain_pit: Word16,
    ltp_residual: &[Word16; L_CODE],
) -> Innovation {
    // `shl` saturates, which is the implicit "clip the factor at 1.0": the top
    // quantised pitch gains double past 32767 and come back 32767.
    let sharp = shl(ctx, gain_pit, 1);
    let mut h = *h;
    add_pitch_contribution(ctx, &mut h, lag, sharp);

    let mut dn = correlate_target(ctx, &h, target, 2, NB_TRACK, STEP);
    let mut joint = set_sign_joint(ctx, &mut dn, ltp_residual, NB_TRACK, STEP);
    let rr = correlate_impulse(ctx, &h, &joint.signs);
    let codvec = search_pairs(ctx, MR122_LAYOUT, &dn, &rr, &mut joint.ipos, &joint.pos_max);

    let (mut code, filter_signs, indices) = build_code_mr122(ctx, &codvec, &joint.signs);
    let filtered = filter_pulses(ctx, &h, &codvec, &filter_signs);
    add_pitch_contribution(ctx, &mut code, lag, sharp);

    Innovation {
        code,
        filtered,
        params: FixedCodebook::TenPulses35Bit(indices),
    }
}

/// 10.2 kbit/s `build_code`.
///
/// Returns the codevector (before sharpening), the filter signs, the four
/// per-track sign bits, and the eight per-pulse position indices.
///
/// The second pulse of a track transmits its sign **implicitly, through the
/// order of the two positions**. The swap condition is inverted between the
/// same-sign and different-sign cases, so the tie `first == second` resolves
/// opposite ways in the two — get it backwards and the stream decodes to a
/// valid-looking, wrong excitation.
///
/// Only `pos_index[0..4]` and `sign_index[0..4]` start at `−1`; the upper half
/// is written only when the second pulse of each track lands, which is
/// guaranteed because [`set_sign_joint`] pairs the tracks.
fn build_code_mr102(
    ctx: &mut DspContext,
    codvec: &[i16],
    signs: &[Word16; L_CODE],
) -> (
    [Word16; L_CODE],
    [Word16; 8],
    [i16; NB_TRACK_MR102],
    [i16; 8],
) {
    let mut code = [Word16(0); L_CODE];
    let mut filter_signs = [Word16(0); 8];
    let mut sign_index = [-1i16; NB_TRACK_MR102];
    let mut pos_index = [-1i16; 8];

    for (k, &position) in codvec.iter().enumerate() {
        let slot = usize::try_from(position).expect("a position is 0..=39");
        let place = position >> 2;
        let track = usize::try_from(position & 3).expect("a track index is 0..=3");

        let sign_bit = if signs[slot].0 > 0 {
            // Both pulses of a track *accumulate*, so a collision doubles the
            // amplitude rather than losing a pulse.
            code[slot] = add(ctx, code[slot], PULSE_MR102);
            filter_signs[k] = SIGN_POSITIVE;
            0
        } else {
            code[slot] = sub(ctx, code[slot], PULSE_MR102);
            filter_signs[k] = SIGN_NEGATIVE;
            1
        };

        if pos_index[track] < 0 {
            pos_index[track] = place;
            sign_index[track] = sign_bit;
        } else {
            let same_sign = (sign_bit ^ sign_index[track]) & 1 == 0;
            let ordered = pos_index[track] <= place;
            if same_sign == ordered {
                // Same sign and already ordered, or different sign and out of
                // order: the first pulse stays where it is.
                pos_index[track + NB_TRACK_MR102] = place;
            } else {
                pos_index[track + NB_TRACK_MR102] = pos_index[track];
                pos_index[track] = place;
                sign_index[track] = sign_bit;
            }
        }
    }

    (code, filter_signs, sign_index, pos_index)
}

/// Pack three position indices into one 10-bit word, TS 26.073 `compress10`.
///
/// The three least-significant bits are kept out of the base-5 digits so that a
/// single bit error moves a pulse by one slot rather than across the subframe.
fn compress10(ctx: &mut DspContext, a: i16, b: i16, c: i16) -> i16 {
    let fives = l_mult(ctx, Word16(b >> 1), Word16(5));
    let fives = extract_l(l_shr(ctx, fives, 1));
    let twenty_fives = l_mult(ctx, Word16(c >> 1), Word16(25));
    let twenty_fives = extract_l(l_shr(ctx, twenty_fives, 1));

    let digits = add(ctx, fives, twenty_fives);
    let digits = add(ctx, Word16(a >> 1), digits);
    let base = shl(ctx, digits, 3);

    let low = add(ctx, Word16((b & 1) << 1), Word16((c & 1) << 2));
    let low = add(ctx, Word16(a & 1), low);
    add(ctx, base, low).0
}

/// 10.2 kbit/s `compress_code`: four sign bits and three position words.
///
/// The last word's `mult(x, 1311)` is a flooring `>>15` of `x/24.994`, not a
/// division by 25; the two disagree for some inputs and the reference means the
/// former.
fn compress_mr102(
    ctx: &mut DspContext,
    sign_index: [i16; NB_TRACK_MR102],
    pos_index: &[i16; 8],
) -> [u16; 7] {
    let mut out = [0u16; 7];
    for (slot, &sign) in out.iter_mut().zip(sign_index.iter()) {
        *slot = u16::try_from(sign).expect("a sign bit is 0 or 1");
    }
    out[4] = u16::try_from(compress10(ctx, pos_index[0], pos_index[4], pos_index[1]))
        .expect("a compressed index is non-negative");
    out[5] = u16::try_from(compress10(ctx, pos_index[2], pos_index[6], pos_index[5]))
        .expect("a compressed index is non-negative");

    // The third word folds i3 and i7 into 7 bits. When i7's second-lowest bit
    // is set, i3's base-5 digit is *reflected* — that is what stops one bit
    // error in the packed word from moving both pulses the same way.
    let reflected = (pos_index[7] >> 1) & 1 == 1;
    let ia = if reflected {
        sub(ctx, Word16(4), Word16(pos_index[3] >> 1))
    } else {
        Word16(pos_index[3] >> 1)
    };
    let fives = l_mult(ctx, Word16(pos_index[7] >> 1), Word16(5));
    let ib = extract_l(l_shr(ctx, fives, 1));
    let ib = add(ctx, ia, ib);
    let ib = shift_into_field(ctx, ib, 5, 12);
    let scaled = mult(ctx, ib, RECIP_25);
    let ic = shl(ctx, scaled, 2);
    let low = add(ctx, Word16((pos_index[7] & 1) << 1), ic);
    let word = add(ctx, Word16(pos_index[3] & 1), low);
    out[6] = u16::try_from(word.0).expect("a compressed index is non-negative");
    out
}

/// 12.2 kbit/s `build_code`, including the `q_p` Gray coding.
///
/// Bit 3 of each index carries the pulse sign, so the "same sign" test is a
/// comparison of that bit. The two ordering branches differ in a way that looks
/// like a typo and is not: the same-sign branch compares the **whole** index
/// while the different-sign branch compares only the low three bits, and the
/// `<=` tie goes opposite ways in the two.
///
/// `q_p` Gray-codes **after** packing and only the low three bits, keeping bit
/// 3 for the first five indices — the opposite of the 17-bit codebook, which
/// Gray-codes before shifting.
fn build_code_mr122(
    ctx: &mut DspContext,
    codvec: &[i16; MAX_PULSES],
    signs: &[Word16; L_CODE],
) -> ([Word16; L_CODE], [Word16; MAX_PULSES], [u16; MAX_PULSES]) {
    let mut code = [Word16(0); L_CODE];
    let mut filter_signs = [Word16(0); MAX_PULSES];
    let mut indices = [-1i16; MAX_PULSES];

    for (k, &position) in codvec.iter().enumerate() {
        let slot = usize::try_from(position).expect("a position is 0..=39");
        let (index, track) = split_position(ctx, position);
        let index = if signs[slot].0 > 0 {
            code[slot] = add(ctx, code[slot], PULSE_MR122);
            filter_signs[k] = SIGN_MR122;
            index
        } else {
            code[slot] = sub(ctx, code[slot], PULSE_MR122);
            filter_signs[k] = negate(ctx, SIGN_MR122);
            add(ctx, index, Word16(8))
        };

        if indices[track] < 0 {
            indices[track] = index.0;
        } else if (index.0 ^ indices[track]) & 8 == 0 {
            if indices[track] <= index.0 {
                indices[track + NB_TRACK] = index.0;
            } else {
                indices[track + NB_TRACK] = indices[track];
                indices[track] = index.0;
            }
        } else if (indices[track] & 7) <= (index.0 & 7) {
            indices[track + NB_TRACK] = indices[track];
            indices[track] = index.0;
        } else {
            indices[track + NB_TRACK] = index.0;
        }
    }

    let mut packed = [0u16; MAX_PULSES];
    for (i, (out, &raw)) in packed.iter_mut().zip(indices.iter()).enumerate() {
        let gray = u16::try_from(GRAY[usize::try_from(raw & 7).expect("masked to 0..=7")])
            .expect("a Gray code is 0..=7");
        *out = if i < NB_TRACK {
            u16::try_from(raw & 0x8).expect("masked to 0 or 8") | gray
        } else {
            gray
        };
    }

    (code, filter_signs, packed)
}

#[cfg(test)]
mod tests {
    use super::super::super::bitstream::parse;
    use super::super::super::codebook::sharpening_state;
    use super::*;

    use std::collections::HashMap;
    use std::sync::OnceLock;

    /// Frames committed to `nb_enc_trace.txt`.
    const TRACE_FRAMES: usize = 3;
    /// Subframes per frame.
    const SUBFRAMES: usize = 4;
    /// The rate the committed trace was produced at: 7.40 kbit/s.
    const TRACE_MODE: u8 = 4;
    /// Subframes the committed trace covers.
    const TRACE_SUBFRAMES: usize = TRACE_FRAMES * SUBFRAMES;

    type TraceRows = HashMap<(i32, i32, String), Vec<i32>>;

    fn trace() -> &'static TraceRows {
        static ROWS: OnceLock<TraceRows> = OnceLock::new();
        ROWS.get_or_init(|| {
            let text = include_str!("../../testdata/nb_enc_trace.txt");
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

    fn row(frame: usize, subframe: usize, name: &str) -> Vec<Word16> {
        let key = (
            i32::try_from(frame).expect("frame"),
            i32::try_from(subframe).expect("subframe"),
            name.to_owned(),
        );
        trace()
            .get(&key)
            .unwrap_or_else(|| panic!("trace row {name} missing at frame {frame} sf {subframe}"))
            .iter()
            .map(|&v| Word16(i16::try_from(v).expect("a trace row holds Word16 values")))
            .collect()
    }

    fn vector(frame: usize, subframe: usize, name: &str) -> [Word16; L_CODE] {
        let values = row(frame, subframe, name);
        assert_eq!(values.len(), L_CODE, "{name} is not a subframe vector");
        let mut out = [Word16(0); L_CODE];
        out.copy_from_slice(&values);
        out
    }

    fn scalar(frame: usize, subframe: usize, name: &str) -> Word16 {
        let values = row(frame, subframe, name);
        assert_eq!(values.len(), 1, "{name} is not a scalar row");
        values[0]
    }

    fn ctx() -> DspContext {
        DspContext::default()
    }

    /// Rebuild `res2`, the LTP residual `cbsearch` receives, from rows the
    /// trace does carry.
    ///
    /// `cl_ltp` computes `res2[i] = res[i] − extract_h(2·exc[i]·gain_pit)`.
    /// Note `extract_h`, which truncates: `calc_unfilt_energies` forms the same
    /// difference with `round` instead, and the two differ by an LSB.
    fn ltp_residual(ctx: &mut DspContext, frame: usize, subframe: usize) -> [Word16; L_CODE] {
        let res = vector(frame, subframe, "res");
        let exc = vector(frame, subframe, "adapt");
        let gain = scalar(frame, subframe, "gain_pit_ol");
        let mut out = [Word16(0); L_CODE];
        for i in 0..L_CODE {
            let product = l_mult(ctx, exc[i], gain);
            let scaled = l_shl(ctx, product, 1);
            out[i] = sub(ctx, res[i], extract_h(scaled));
        }
        out
    }

    /// One pass over the committed trace, feeding [`search`] the traced inputs.
    ///
    /// The sharpening state is the one input no row carries. It is
    /// `min(previous subframe's quantised gain_pit, SHARPMAX)`, carried across
    /// frames and starting at zero — `subframePostProc` reduced to the one line
    /// that touches it. That reconstruction was checked against a development
    /// trace which dumps `st->sharp` directly, over 200 subframes at each of
    /// the eight rates, and agreed everywhere.
    fn run(ctx: &mut DspContext) -> Vec<(usize, usize, Word16, Innovation)> {
        let mut out = Vec::new();
        let mut sharp = Word16(0);
        for frame in 0..TRACE_FRAMES {
            for subframe in 0..SUBFRAMES {
                let target = vector(frame, subframe, "xn2");
                let h = vector(frame, subframe, "h1");
                let residual = ltp_residual(ctx, frame, subframe);
                let lag = scalar(frame, subframe, "T0").0;
                let gain_pit = scalar(frame, subframe, "gain_pit_ol");

                let innovation = search(
                    ctx,
                    TRACE_MODE,
                    u8::try_from(subframe).expect("a subframe is 0..=3"),
                    &CodebookInputs {
                        target: &target,
                        impulse: &h,
                        ltp_residual: &residual,
                        lag,
                        pitch_sharp: sharp,
                        gain_pit,
                    },
                );
                out.push((frame, subframe, sharp, innovation));

                sharp = sharpening_state(scalar(frame, subframe, "gain_pit"));
            }
        }
        assert_eq!(
            out.len(),
            TRACE_SUBFRAMES,
            "the harness produced nothing to compare"
        );
        out
    }

    #[test]
    fn codevector_is_bit_exact_against_the_74_trace() {
        let mut c = ctx();
        let run = run(&mut c);
        let mut compared = 0usize;
        for (frame, subframe, _, innovation) in &run {
            assert_eq!(
                innovation.code,
                vector(*frame, *subframe, "code"),
                "code differs at frame {frame} subframe {subframe}"
            );
            compared += 1;
        }
        assert_eq!(
            compared, TRACE_SUBFRAMES,
            "compared {compared} subframes, expected {TRACE_SUBFRAMES}"
        );
    }

    #[test]
    fn filtered_codevector_is_bit_exact_against_the_74_trace() {
        let mut c = ctx();
        let run = run(&mut c);
        let mut compared = 0usize;
        for (frame, subframe, _, innovation) in &run {
            assert_eq!(
                innovation.filtered,
                vector(*frame, *subframe, "y2"),
                "y2 differs at frame {frame} subframe {subframe}"
            );
            compared += 1;
        }
        assert_eq!(
            compared, TRACE_SUBFRAMES,
            "compared {compared} subframes, expected {TRACE_SUBFRAMES}"
        );
    }

    /// The index test the codevector test cannot replace.
    ///
    /// Compares the chosen pulse positions and signs against the corresponding
    /// fields of the reference encoder's own bitstream. A search that maximised
    /// the same objective but resolved a tie the other way would still build a
    /// codevector, and would fail here.
    ///
    /// 7.40 kbit/s lays its 19 parameters out as three LSF words then, per
    /// subframe, `(lag, positions, signs, gain)` — so the codebook fields are
    /// at `4 + 4·subframe` and `5 + 4·subframe`.
    #[test]
    fn chosen_positions_and_signs_match_the_reference_bitstream() {
        /// `#!AMR\n`.
        const HEADER: usize = 6;
        /// One table-of-contents byte plus 19 payload bytes for 148 bits.
        const FRAME_BYTES: usize = 20;

        let stream = include_bytes!("../../testdata/amrnb_enc_mode4.amr");
        let mut c = ctx();
        let run = run(&mut c);

        let mut compared = 0usize;
        for (frame, subframe, _, innovation) in &run {
            let start = HEADER + frame * FRAME_BYTES + 1;
            let payload = &stream[start..start + FRAME_BYTES - 1];
            let params = parse(TRACE_MODE, payload).expect("the committed frame parses");
            let want_positions = params[4 + 4 * subframe];
            let want_signs = params[5 + 4 * subframe];

            let FixedCodebook::FourPulses17Bit { signs, positions } = innovation.params else {
                panic!("7.40 kbit/s must produce a 17-bit four-pulse index");
            };
            assert_eq!(
                positions, want_positions,
                "position index differs at frame {frame} subframe {subframe}"
            );
            assert_eq!(
                signs, want_signs,
                "sign index differs at frame {frame} subframe {subframe}"
            );
            compared += 1;
        }
        assert_eq!(
            compared, TRACE_SUBFRAMES,
            "compared {compared} subframes, expected {TRACE_SUBFRAMES}"
        );
    }

    /// The packed index decodes back to the codevector the search built.
    ///
    /// Together with the bitstream test above this closes the loop: the index
    /// is the reference's, and it means what this module thinks it means.
    #[test]
    fn packed_index_round_trips_through_the_decoder() {
        let mut c = ctx();
        let run = run(&mut c);
        let mut compared = 0usize;
        for (frame, subframe, sharp, innovation) in &run {
            let mut rebuilt = innovation.params.decode(&mut c);
            let factor = shl(&mut c, *sharp, 1);
            add_pitch_contribution(
                &mut c,
                &mut rebuilt,
                scalar(*frame, *subframe, "T0").0,
                factor,
            );
            assert_eq!(
                rebuilt, innovation.code,
                "the packed index does not decode to the chosen codevector \
                 at frame {frame} subframe {subframe}"
            );
            compared += 1;
        }
        assert_eq!(
            compared, TRACE_SUBFRAMES,
            "compared {compared} subframes, expected {TRACE_SUBFRAMES}"
        );
    }

    /// Exactly four pulses, at the positions the reference's index names.
    ///
    /// Reads the pulses back off the decoded parameters rather than off the
    /// codevector, because sharpening turns four non-zero samples into many.
    #[test]
    fn every_subframe_places_four_unit_pulses() {
        let mut c = ctx();
        let run = run(&mut c);
        let mut compared = 0usize;
        for (frame, subframe, _, innovation) in &run {
            let decoded = innovation.params.decode(&mut c);
            let pulses: Vec<(usize, i16)> = decoded
                .iter()
                .enumerate()
                .filter(|(_, v)| v.0 != 0)
                .map(|(i, v)| (i, v.0))
                .collect();
            assert_eq!(
                pulses.len(),
                4,
                "frame {frame} subframe {subframe}: expected four pulses, got {pulses:?}"
            );
            // One pulse per track, which is what the 17-bit layout encodes.
            let mut tracks: Vec<usize> = pulses.iter().map(|(i, _)| i % 5).collect();
            tracks.sort_unstable();
            tracks.dedup();
            assert_eq!(
                tracks.len(),
                4,
                "frame {frame} subframe {subframe}: two pulses share a track"
            );
            for &(position, amplitude) in &pulses {
                assert!(
                    amplitude == POSITIVE_PULSE.0 || amplitude == NEGATIVE_PULSE.0,
                    "pulse at {position} has amplitude {amplitude}, not ±1"
                );
            }
            compared += 1;
        }
        assert_eq!(
            compared, TRACE_SUBFRAMES,
            "compared {compared} subframes, expected {TRACE_SUBFRAMES}"
        );
    }

    /// A flat landscape: every candidate scores identically, so the winner is
    /// whichever was visited first.
    ///
    /// This is the check that separates "maximises the right thing" from
    /// "matches the reference": a `>=` accept rule would keep the *last* tied
    /// candidate and pass every energy-based test.
    #[test]
    fn a_tie_keeps_the_earliest_candidate() {
        let mut c = ctx();
        let dn = [Word16(1000); L_CODE];
        let mut rr = [[Word16(0); L_CODE]; L_CODE];
        for (i, column) in rr.iter_mut().enumerate() {
            column[i] = Word16(1000);
        }

        let mut best = BestPair {
            numerator: Word16(-1),
            denominator: Word16(1),
            positions: [0, 1],
        };
        two_pulse_pass(&mut c, &dn, &rr, [0, 1], &mut best);
        assert_eq!(
            best.positions,
            [0, 1],
            "with every candidate tied, the first visited pair must win"
        );

        let mut best = BestPair {
            numerator: Word16(-1),
            denominator: Word16(1),
            positions: [0, 1],
        };
        two_pulse_pass(&mut c, &dn, &rr, [2, 3], &mut best);
        assert_eq!(
            best.positions,
            [2, 3],
            "the first visited pair of *this* track pair must win"
        );
    }

    /// The `sq = −1, alp = 1` sentinel does not force the first candidate in.
    ///
    /// With a zero correlation and a negative cross term the accept test
    /// `sq1 + alp_16 > 0` fails for every candidate, so each stage finishes at
    /// its reset index and the sweep finishes with `codvec` still at its
    /// `codvec[i] = i` default. An implementation that special-cased "the first
    /// candidate always wins" would return three real positions here.
    #[test]
    fn a_stage_that_accepts_nothing_keeps_its_reset_index() {
        let mut c = ctx();
        let dn = [Word16(0); L_CODE];
        let pruned = [Word16(0); L_CODE];
        let mut rr = [[Word16(-1000); L_CODE]; L_CODE];
        for (i, column) in rr.iter_mut().enumerate() {
            column[i] = Word16(0);
        }

        let mut best = BestPulses {
            numerator: Word16(-1),
            denominator: Word16(1),
            positions: [0, 1, 2, 0],
        };
        pulse_pass(&mut c, &dn, &pruned, &rr, &[0, 1, 2], &mut best);
        assert_eq!(
            best.positions,
            [0, 1, 2, 0],
            "nothing was accepted, so codvec must still hold its default"
        );
        assert_eq!(best.numerator.0, -1, "psk must be untouched");
        assert_eq!(best.denominator.0, 1, "alpk must be untouched");
    }

    /// `cor_h`'s sign product is `mult(s_i, s_j)`, which is asymmetric.
    ///
    /// Using `±1` instead would change `rr[i][j]` by one part in 32768 for
    /// every matched pair — enough to flip a tie.
    #[test]
    fn impulse_correlation_sign_product_is_asymmetric() {
        let mut c = ctx();
        assert_eq!(mult(&mut c, Word16(32767), Word16(32767)).0, 32766);
        assert_eq!(mult(&mut c, Word16(32767), Word16(-32767)).0, -32767);

        let mut h = [Word16(0); L_CODE];
        h[0] = Word16(4096);
        h[1] = Word16(2048);
        let mut signs = [SIGN_POSITIVE; L_CODE];
        signs[1] = Word16(-32767);
        let rr = correlate_impulse(&mut c, &h, &signs);
        assert!(
            rr[0][1].0 < 0,
            "a mismatched sign pair must give a negative off-diagonal"
        );
        assert!(
            rr[1][1].0 >= 0,
            "the diagonal carries no sign factor and must stay non-negative"
        );
        assert_eq!(rr[0][1], rr[1][0], "rr must be symmetric");
    }

    /// The diagonal runs backwards: `rr[39][39]` is the first tap's energy and
    /// `rr[0][0]` is the whole response's.
    #[test]
    fn impulse_correlation_diagonal_runs_backwards() {
        let mut c = ctx();
        let mut h = [Word16(0); L_CODE];
        for (i, tap) in h.iter_mut().enumerate() {
            *tap = Word16(1000 - i16::try_from(i).expect("small") * 20);
        }
        let signs = [SIGN_POSITIVE; L_CODE];
        let rr = correlate_impulse(&mut c, &h, &signs);
        assert!(
            rr[0][0].0 > rr[39][39].0,
            "rr[0][0] accumulates every tap and rr[39][39] only the first"
        );
    }

    /// `set_sign` keeps exactly `n` positions per track and strikes the
    /// smallest, breaking ties toward the lowest position index.
    #[test]
    fn set_sign_prunes_the_smallest_and_breaks_ties_low() {
        let mut c = ctx();
        let mut dn = [Word16(0); L_CODE];
        for (i, slot) in dn.iter_mut().enumerate() {
            *slot = Word16(i16::try_from(i).expect("small") * 10);
        }
        // A tie between positions 0 and 5, both in track 0.
        dn[0] = Word16(100);
        dn[5] = Word16(100);
        let (_, pruned) = set_sign(&mut c, &mut dn, 4);
        for track in 0..NB_TRACK {
            let kept = (track..L_CODE)
                .step_by(usize::try_from(STEP).expect("small"))
                .filter(|&j| pruned[j].0 >= 0)
                .count();
            assert_eq!(kept, 4, "track {track} kept {kept} positions, expected 4");
        }
        assert_eq!(
            pruned[0].0, -1,
            "on a tie the lowest position is struck first"
        );
    }

    /// `set_sign` makes `dn` non-negative; `set_sign12k2` does not.
    ///
    /// This is the difference a shared implementation would erase, and erasing
    /// it gives the two wide rates a plausible but wrong search.
    #[test]
    fn joint_sign_setting_leaves_dn_signed() {
        let mut c = ctx();
        let dn = [Word16(1000); L_CODE];
        let cn = [Word16(-20000); L_CODE];

        let mut rectified = dn;
        set_sign(&mut c, &mut rectified, 8);
        assert!(
            rectified.iter().all(|v| v.0 >= 0),
            "set_sign must rectify dn"
        );

        let mut signed = dn;
        let joint = set_sign_joint(&mut c, &mut signed, &cn, NB_TRACK, STEP);
        assert!(
            joint.signs.iter().all(|s| s.0 < 0),
            "the joint correlation is negative here, so every sign must be"
        );
        assert!(
            signed.iter().any(|v| v.0 < 0),
            "set_sign12k2 negates dn where the joint sign is negative, so dn \
             must not come back rectified"
        );
    }

    /// `ipos` pairs every track with itself, which is what guarantees exactly
    /// two pulses per track at the wide rates.
    #[test]
    fn joint_sign_setting_pairs_every_track() {
        let mut c = ctx();
        let mut dn = [Word16(0); L_CODE];
        let mut cn = [Word16(0); L_CODE];
        for i in 0..L_CODE {
            dn[i] = Word16(i16::try_from(i).expect("small") * 100);
            cn[i] = Word16(i16::try_from(i).expect("small") * 50);
        }
        let joint = set_sign_joint(&mut c, &mut dn, &cn, NB_TRACK, STEP);
        for i in 0..NB_TRACK {
            assert_eq!(
                joint.ipos[i],
                joint.ipos[i + NB_TRACK],
                "track slot {i} is not paired with its twin"
            );
        }
        let mut seen: Vec<i16> = joint.ipos[..NB_TRACK].to_vec();
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3, 4], "ipos must enumerate every track");
    }

    /// The pitch-sharpening recursion reads entries it has already written.
    #[test]
    fn pitch_contribution_is_self_referencing() {
        let mut c = ctx();
        let mut v = [Word16(0); L_CODE];
        v[0] = Word16(8192);
        add_pitch_contribution(&mut c, &mut v, 10, Word16(16384));
        // The terms at 20 and 30 exist only because indices 10 and 20 were read
        // after being written.
        assert_eq!(v[10].0, 4096);
        assert_eq!(v[20].0, 2048);
        assert_eq!(v[30].0, 1024);
    }

    /// `mult(i, 6554)` is `floor(i/5)` over the whole position range, and the
    /// remainder it implies is `i % 5`.
    #[test]
    fn position_split_is_exact_over_the_subframe() {
        let mut c = ctx();
        for i in 0..40i16 {
            let (index, track) = split_position(&mut c, i);
            assert_eq!(index.0, i / 5, "index of {i}");
            assert_eq!(
                track,
                usize::try_from(i % 5).expect("small"),
                "track of {i}"
            );
        }
    }
}
