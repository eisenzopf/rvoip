//! AMR-NB's six algebraic (fixed) codebooks, one family per rate.
//!
//! Implements TS 26.073 `d2_9pf.c`, `d2_11pf.c`, `d3_14pf.c`, `d4_17pf.c`,
//! `d8_31pf.c` and `d1035pf.c`, plus the pitch-sharpening pass `dec_amr.c`
//! runs over the decoded codevector before the gain stage.
//!
//! Validated bit-exactly against the `cb2i40_9`, `cb2i40_11`, `cb3i40_14`,
//! `cb4i40_17`, `cb8i40_31` and `cb10i40_35` sections of
//! `testdata/nb_stages.txt`, which `tools/amrnb_stage_oracle.c` produced by
//! calling TS 26.073's own six decoders.
//!
//! # Six codebooks, not one parameterised codebook
//!
//! Each rate gets its own pulse count and its own way of packing positions, and
//! the six differ in ways that a shared implementation would have to smuggle
//! back in as flags:
//!
//! - **Amplitude.** 4.75 through 7.95 write `+8191` for a positive pulse and
//!   `-8192` for a negative one — deliberately asymmetric. 10.2 writes `±8191`.
//!   12.2 writes `±4096`, because its codevector is Q12 where every other rate's
//!   is Q13.
//! - **Sign polarity.** For 4.75 through 7.95 a sign bit of 1 means *positive*.
//!   For 10.2 and 12.2 a sign bit of 0 means positive. Inverting this yields
//!   audio that is loud and wrong rather than obviously broken.
//! - **Assignment versus accumulation.** The four narrow codebooks *store* each
//!   pulse, so two pulses landing on one sample leave one pulse. 10.2 and 12.2
//!   store the first pulse of a track and *add* the second, so a collision
//!   doubles the amplitude — the only way their codevectors reach 2.0.
//! - **Gray coding.** Only 7.4/7.95 and 12.2 gray-decode their position fields.
//!
//! # Q-formats
//!
//! Codevectors are Q13 for every rate except 12.2, which is Q12. The
//! sharpening factor is Q15, so `mult(code, factor)` stays in the codevector's
//! own Q and the accumulation is homogeneous.
//!
//! # Untrusted input
//!
//! These parameters arrive from the network. Several fields have a legal range
//! narrower than their bit width — 10.2 kbit/s packs 125x8 combinations into
//! ten bits and 25x4 into seven — and the reference C computes a position index
//! past the end of a 40-sample codevector when fed a value wider still. Handed
//! all sixteen bits, its seven-bit index decodes pulse 7 to slot 79, which
//! writes 279 samples past the codevector. That is a stack overwrite in C and
//! would be a panic here, so there are two guards:
//!
//! 1. **Every parameter is masked to its field width on entry** (`field`). The
//!    bit unpacker cannot emit a value wider than the field it read, so this is
//!    a no-op on any real frame; it is what bounds 10.2's third compressed
//!    index for a hand-built or corrupted parameter vector.
//! 2. **A decoded position is clamped into the subframe** (`sample`) instead of
//!    indexing out of range. On a legal frame this is unreachable — 10.2's
//!    reference clamp on `MSBs` and the masking above between them keep every
//!    slot inside its track.
//!
//! The tests carry the argument rather than the comments: the 10.2 position
//! decode is checked exhaustively over its entire 10-/10-/7-bit domain, masking
//! is checked to be a no-op on every fixture case, and every decoder is called
//! with all-ones parameters.

use super::decoder_tables::{DGRAY, START_POS_2I40_9};
use super::{L_SUBFR, SHARPMAX};
use crate::fixed_point::arith::{add, extract_l, mult, negate, sub};
use crate::fixed_point::arith32::l_mult;
use crate::fixed_point::shift::{l_shr, shl, shr};
use crate::fixed_point::types::{DspContext, Word16};

/// One subframe's fixed-codebook contribution.
///
/// Q13 for every rate except 12.2 kbit/s, which is Q12.
pub type Codevector = [Word16; L_SUBFR];

/// Pulse amplitude for the 9-, 11-, 14- and 17-bit codebooks.
///
/// The asymmetry is in the reference and is not a typo: `+1.0` is one LSB short
/// of full scale while `-1.0` is exact. "Fixing" it changes the codevector.
const POSITIVE_PULSE: Word16 = Word16(8191);
/// Negative pulse amplitude for the 9-, 11-, 14- and 17-bit codebooks.
const NEGATIVE_PULSE: Word16 = Word16(-8192);

/// Pulse amplitude at 10.2 kbit/s, symmetric where the narrower codebooks are
/// not.
const PULSE_MR102: Word16 = Word16(8191);
/// Negative pulse amplitude at 10.2 kbit/s.
const PULSE_MR102_NEG: Word16 = Word16(-8191);

/// Pulse amplitude at 12.2 kbit/s. Half the others because the codevector is
/// Q12, not Q13 — the one place where getting the Q wrong scales the whole
/// fixed-codebook contribution by two.
const PULSE_MR122: Word16 = Word16(4096);
/// Negative pulse amplitude at 12.2 kbit/s.
const PULSE_MR122_NEG: Word16 = Word16(-4096);

/// Track stride at 10.2 kbit/s: four interleaved tracks of ten slots.
const STEP_MR102: i16 = 4;
/// Number of tracks at 10.2 kbit/s.
const TRACKS_MR102: usize = 4;
/// Track stride everywhere else: five interleaved tracks of eight slots.
const STEP: i16 = 5;
/// Number of tracks at 12.2 kbit/s.
const TRACKS_MR122: usize = 5;

/// `1/25` in Q15, as the reference spells the division.
const RECIP_25: Word16 = Word16(1311);
/// `1/5` in Q15.
const RECIP_5: Word16 = Word16(6554);

/// The fixed-codebook parameters one subframe carries, by rate family.
///
/// Six variants rather than one struct with optional fields, because the six
/// codebooks genuinely take different parameters: only 4.75/5.15 need the
/// subframe number, and the two widest rates receive a vector of sub-indices
/// instead of a packed position word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixedCodebook {
    /// 4.75 and 5.15 kbit/s: 7 position bits, 2 sign bits, and the subframe
    /// number — the track pairs rotate across the four subframes, which is why
    /// this is the only family that needs to know where it is in the frame.
    TwoPulses9Bit {
        /// Subframe number within the frame, 0..=3.
        subframe: u8,
        /// Two sign bits, LSB first.
        signs: u16,
        /// Seven position bits.
        positions: u16,
    },
    /// 5.90 kbit/s: 9 position bits, 2 sign bits.
    TwoPulses11Bit {
        /// Two sign bits, LSB first.
        signs: u16,
        /// Nine position bits.
        positions: u16,
    },
    /// 6.70 kbit/s: 11 position bits, 3 sign bits.
    ThreePulses14Bit {
        /// Three sign bits, LSB first.
        signs: u16,
        /// Eleven position bits.
        positions: u16,
    },
    /// 7.40 and 7.95 kbit/s: 13 position bits, 4 sign bits, gray-coded.
    FourPulses17Bit {
        /// Four sign bits, LSB first.
        signs: u16,
        /// Thirteen position bits.
        positions: u16,
    },
    /// 10.2 kbit/s: four 1-bit signs then three compressed position words of
    /// 10, 10 and 7 bits.
    EightPulses31Bit([u16; 7]),
    /// 12.2 kbit/s: five 4-bit fields (three position bits plus a sign) then
    /// five 3-bit position-only fields.
    TenPulses35Bit([u16; 10]),
}

impl FixedCodebook {
    /// Decode this subframe's codevector.
    ///
    /// Q13, except [`FixedCodebook::TenPulses35Bit`] which is Q12.
    #[must_use]
    pub fn decode(self, ctx: &mut DspContext) -> Codevector {
        match self {
            Self::TwoPulses9Bit {
                subframe,
                signs,
                positions,
            } => decode_two_pulses_9bit(ctx, subframe, signs, positions),
            Self::TwoPulses11Bit { signs, positions } => {
                decode_two_pulses_11bit(ctx, signs, positions)
            }
            Self::ThreePulses14Bit { signs, positions } => {
                decode_three_pulses_14bit(ctx, signs, positions)
            }
            Self::FourPulses17Bit { signs, positions } => {
                decode_four_pulses_17bit(ctx, signs, positions)
            }
            Self::EightPulses31Bit(params) => decode_eight_pulses_31bit(ctx, &params),
            Self::TenPulses35Bit(params) => decode_ten_pulses_35bit(ctx, &params),
        }
    }
}

/// Take the low `bits` of a received parameter.
///
/// The first of the two untrusted-input guards. The bit unpacker cannot emit a
/// value wider than the field it read, so on a real frame this changes nothing.
/// Where it earns its place is 10.2 kbit/s' seven-bit position word: that one
/// *does* keep consuming whatever bits it is handed, and unmasked sixteen bits
/// decode pulse 7 to slot 79 instead of at most 9.
#[inline]
fn field(value: u16, bits: u32) -> Word16 {
    debug_assert!(
        bits <= 13,
        "the widest AMR-NB parameter field is thirteen bits"
    );
    // Plain arithmetic: a bit field is a bit field, not a fixed-point quantity.
    let masked = value & ((1u16 << bits) - 1);
    // Thirteen bits cannot reach the sign bit, so the conversion never fails;
    // the fallback keeps the guard total rather than being reachable.
    Word16(i16::try_from(masked).unwrap_or(0))
}

/// A decoded pulse position as a subframe sample index.
///
/// The second untrusted-input guard. Every position the six decoders can
/// produce from a masked parameter is already inside the subframe — the tests
/// check that exhaustively for 10.2 and by construction for the rest — so this
/// clamp is unreachable. It exists because the alternative, on a position that
/// somehow escaped, is a panic in the middle of a media path — and one pulse
/// pinned to the end of a corrupt subframe is a better failure than a dropped
/// call.
#[inline]
fn sample(position: Word16) -> usize {
    usize::from(position.0.max(0).unsigned_abs()).min(L_SUBFR - 1)
}

/// `i * 5`, written as `add(i, shl(i, 2))` the way the four narrow codebooks
/// write it.
#[inline]
fn times_five(ctx: &mut DspContext, i: Word16) -> Word16 {
    let quadruple = shl(ctx, i, 2);
    add(ctx, i, quadruple)
}

/// `i * k` via the reference's `extract_l(L_shr(L_mult(i, k), 1))` idiom.
///
/// `L_mult` doubles as it multiplies and `L_shr` by one undoes exactly that, so
/// this is an exact integer product with no rounding anywhere in it. The two
/// wide codebooks spell their position scaling this way where the narrow ones
/// use shift-and-add; both are exact, and each is kept as its own file writes it.
#[inline]
fn exact_product(ctx: &mut DspContext, i: Word16, k: i16) -> Word16 {
    let doubled = l_mult(ctx, i, Word16(k));
    extract_l(l_shr(ctx, doubled, 1))
}

/// Zero a codevector and store one signed pulse per position, consuming the
/// sign word LSB first.
///
/// Shared by the 9-, 11-, 14- and 17-bit codebooks, which differ only in how
/// they derive the positions. The store is an *assignment*: at 5.90 the two
/// pulses can share a track and therefore a sample, and the second pulse then
/// replaces the first rather than doubling it.
fn place_pulses(ctx: &mut DspContext, signs: Word16, positions: &[Word16]) -> Codevector {
    let mut code = [Word16(0); L_SUBFR];
    let mut signs = signs;

    for &position in positions {
        // Plain C `&` on the sign word, then a basic-operator shift.
        let positive = (signs.0 & 1) != 0;
        signs = shr(ctx, signs, 1);
        code[sample(position)] = if positive {
            POSITIVE_PULSE
        } else {
            NEGATIVE_PULSE
        };
    }

    code
}

/// Two pulses in nine bits — 4.75 and 5.15 kbit/s, TS 26.073
/// `decode_2i40_9bits`.
///
/// `subframe` is the subframe's index in the frame, 0..=3; the two tracks a
/// pulse pair may use rotate with it. `signs` carries two bits, LSB first, with
/// 1 meaning a positive pulse. `positions` carries seven bits: two 3-bit slot
/// indices and, in bit 6, which of the two track pairs this subframe uses.
///
/// Returns a Q13 codevector.
#[must_use]
pub fn decode_two_pulses_9bit(
    ctx: &mut DspContext,
    subframe: u8,
    signs: u16,
    positions: u16,
) -> Codevector {
    let index = field(positions, 7);
    // The pair selector is the field's top bit; plain C `&` with 64, then a
    // basic-operator shift, as the reference has it.
    let pair = shr(ctx, Word16(index.0 & 64), 6);
    // A subframe number outside 0..=3 cannot come from the decoder's own loop
    // counter; masking keeps the table subscript in range regardless.
    let subframe = Word16(i16::from(subframe & 3));
    let pair_base = shl(ctx, pair, 3);
    let subframe_base = shl(ctx, subframe, 1);
    let base = add(ctx, pair_base, subframe_base);

    let slot = Word16(index.0 & 7);
    let scaled = times_five(ctx, slot);
    let first = add(ctx, scaled, start_position(base));

    let index = shr(ctx, index, 3);
    let slot = Word16(index.0 & 7);
    // The reference builds this subscript as a separate `add` on the previous
    // one rather than as `j*8 + subNr*2 + 1` in one go.
    let next = add(ctx, base, Word16(1));
    let scaled = times_five(ctx, slot);
    let second = add(ctx, scaled, start_position(next));

    place_pulses(ctx, field(signs, 2), &[first, second])
}

/// Track start offset for the 9-bit codebook, by `pair * 8 + subframe * 2 + p`.
///
/// The subscript is bounded by construction — the pair is one bit and the
/// subframe is masked to two — so the `min` here is dead, and kept only so that
/// a table regenerated at the wrong size cannot turn into a panic.
#[inline]
fn start_position(subscript: Word16) -> Word16 {
    let at = usize::from(subscript.0.max(0).unsigned_abs());
    Word16(START_POS_2I40_9[at.min(START_POS_2I40_9.len() - 1)])
}

/// Two pulses in eleven bits — 5.90 kbit/s, TS 26.073 `decode_2i40_11bits`.
///
/// `signs` carries two bits, LSB first, 1 meaning positive. `positions` carries
/// nine: a 1-bit track selector and a 3-bit slot for the first pulse, then a
/// 2-bit track selector and a 3-bit slot for the second.
///
/// The two pulses can land on the same sample — the first pulse's track offset
/// is 1 or 3 and the second's is one of 0, 1, 2, 4, so both can be 1. The store
/// overwrites, so the amplitude does not double.
///
/// Returns a Q13 codevector.
#[must_use]
pub fn decode_two_pulses_11bit(ctx: &mut DspContext, signs: u16, positions: u16) -> Codevector {
    let index = field(positions, 9);

    let track = Word16(index.0 & 1);
    let index = shr(ctx, index, 1);
    let slot = Word16(index.0 & 7);
    // Offset 1 or 3: the selector contributes two, not one.
    let scaled = times_five(ctx, slot);
    let offset = add(ctx, scaled, Word16(1));
    let doubled = shl(ctx, track, 1);
    let first = add(ctx, offset, doubled);

    let index = shr(ctx, index, 3);
    let track = Word16(index.0 & 3);
    let index = shr(ctx, index, 2);
    let slot = Word16(index.0 & 7);
    // Selector 3 means track offset 4, not 3 — the four offsets are 0, 1, 2, 4
    // because offset 3 belongs to no pair here.
    let selects_last = sub(ctx, track, Word16(3)).0 == 0;
    let scaled = times_five(ctx, slot);
    let second = if selects_last {
        add(ctx, scaled, Word16(4))
    } else {
        add(ctx, scaled, track)
    };

    place_pulses(ctx, field(signs, 2), &[first, second])
}

/// Three pulses in fourteen bits — 6.70 kbit/s, TS 26.073
/// `decode_3i40_14bits`.
///
/// `signs` carries three bits, LSB first, 1 meaning positive. `positions`
/// carries eleven: a 3-bit slot for a pulse fixed to track 0, then a selector
/// and slot for a pulse on track 1 or 3, then the same for track 2 or 4.
///
/// The three offset sets are disjoint, so the three positions always differ.
///
/// Returns a Q13 codevector.
#[must_use]
pub fn decode_three_pulses_14bit(ctx: &mut DspContext, signs: u16, positions: u16) -> Codevector {
    let index = field(positions, 11);

    let slot = Word16(index.0 & 7);
    let first = times_five(ctx, slot);

    let index = shr(ctx, index, 3);
    let track = Word16(index.0 & 1);
    let index = shr(ctx, index, 1);
    let slot = Word16(index.0 & 7);
    let scaled = times_five(ctx, slot);
    let offset = add(ctx, scaled, Word16(1));
    let doubled = shl(ctx, track, 1);
    let second = add(ctx, offset, doubled);

    let index = shr(ctx, index, 3);
    let track = Word16(index.0 & 1);
    let index = shr(ctx, index, 1);
    let slot = Word16(index.0 & 7);
    let scaled = times_five(ctx, slot);
    let offset = add(ctx, scaled, Word16(2));
    let doubled = shl(ctx, track, 1);
    let third = add(ctx, offset, doubled);

    place_pulses(ctx, field(signs, 3), &[first, second, third])
}

/// Four pulses in seventeen bits — 7.40 and 7.95 kbit/s, TS 26.073
/// `decode_4i40_17bits`.
///
/// `signs` carries four bits, LSB first, 1 meaning positive. `positions`
/// carries thirteen: four gray-coded 3-bit slots plus, at bit 9, a raw bit that
/// moves the fourth pulse between tracks 3 and 4. That bit is *not* gray-coded;
/// all four slot fields are.
///
/// Returns a Q13 codevector.
#[must_use]
pub fn decode_four_pulses_17bit(ctx: &mut DspContext, signs: u16, positions: u16) -> Codevector {
    let index = field(positions, 13);

    let slot = ungray(index);
    let first = times_five(ctx, slot);

    let index = shr(ctx, index, 3);
    let slot = ungray(index);
    // Track 1. The reference's comment here says `pos1 = i*5+1` for this pulse
    // and `pos2 = i*5+1` for the next; the second comment is wrong and the code
    // adds two. The code is what conformance is measured against.
    let scaled = times_five(ctx, slot);
    let second = add(ctx, scaled, Word16(1));

    let index = shr(ctx, index, 3);
    let slot = ungray(index);
    let scaled = times_five(ctx, slot);
    let third = add(ctx, scaled, Word16(2));

    let index = shr(ctx, index, 3);
    let track = Word16(index.0 & 1);
    let index = shr(ctx, index, 1);
    let slot = ungray(index);
    let scaled = times_five(ctx, slot);
    let offset = add(ctx, scaled, Word16(3));
    let fourth = add(ctx, offset, track);

    place_pulses(ctx, field(signs, 4), &[first, second, third, fourth])
}

/// Gray-decode the low three bits of a position field.
///
/// Only 7.4/7.95 and 12.2 send gray-coded slots, and within 7.4/7.95 only the
/// four 3-bit slot fields — the extra 1-bit track selector is raw. The map is
/// its own inverse in neither direction, so decoding with the *forward* table
/// (which differs from this one in two entries) yields a legal codevector at the
/// wrong slots; `the_gray_map_and_its_inverse_are_inverses` is what catches that.
#[inline]
fn ungray(index: Word16) -> Word16 {
    // Plain C `&`, and a plain lookup whose subscript the mask already bounds.
    Word16(DGRAY[usize::from((index.0 & 7).unsigned_abs())])
}

/// Eight pulses in thirty-one bits — 10.2 kbit/s, TS 26.073
/// `dec_8i40_31bits`.
///
/// `params` is the seven received fields: four 1-bit signs, one per track, then
/// three compressed position words of 10, 10 and 7 bits. A sign bit of **0**
/// means a positive pulse here — the opposite of the narrower codebooks.
///
/// Only four signs are sent for eight pulses. The second pulse of a track takes
/// the first's sign, negated when the encoder placed it *before* the first;
/// that ordering is the channel over which the missing four bits travel.
///
/// Returns a Q13 codevector whose samples are `0`, `±8191`, or `±16382` where
/// both pulses of a track landed on one sample.
#[must_use]
pub fn decode_eight_pulses_31bit(ctx: &mut DspContext, params: &[u16; 7]) -> Codevector {
    let mut code = [Word16(0); L_SUBFR];
    let (signs, slots) = decompress_code(ctx, params);

    for track in 0..TRACKS_MR102 {
        let offset = Word16(i16::try_from(track).unwrap_or(0));

        let scaled = exact_product(ctx, slots[track], STEP_MR102);
        let first = add(ctx, scaled, offset);
        let mut sign = if signs[track] {
            PULSE_MR102
        } else {
            PULSE_MR102_NEG
        };
        code[sample(first)] = sign;

        let scaled = exact_product(ctx, slots[track + 4], STEP_MR102);
        let second = add(ctx, scaled, offset);
        if sub(ctx, second, first).0 < 0 {
            sign = negate(ctx, sign);
        }
        // Accumulate, unlike the narrow codebooks. The negation above fires only
        // on a *strictly* earlier second pulse, so two pulses on one sample keep
        // the same sign: this doubles and never cancels.
        let at = sample(second);
        code[at] = add(ctx, code[at], sign);
    }

    code
}

/// Unpack 10.2 kbit/s' seven received fields into four track signs and eight
/// slot indices.
///
/// The four `bool`s are "this track's first pulse is positive". The eight slots
/// are 0..=9 each, indexed by pulse: pulse `p` sits on track `p % 4`.
fn decompress_code(ctx: &mut DspContext, params: &[u16; 7]) -> ([bool; 4], [Word16; 8]) {
    // 0 is positive, 1 is negative — inverted relative to every narrower rate.
    let signs = [
        field(params[0], 1).0 == 0,
        field(params[1], 1).0 == 0,
        field(params[2], 1).0 == 0,
        field(params[3], 1).0 == 0,
    ];

    let mut slots = [Word16(0); 8];

    // The compression groups *pulses*, not tracks — pulses 0/4/1, then 2/6/5,
    // then 3/7, where pulse p sits on track p % 4. Three pulses share a ten-bit
    // word so that each gets one bit down in the word's three protected LSBs,
    // and a flip of one of those moves its pulse by a single slot rather than
    // across the subframe.
    let word = field(params[4], 10);
    let msbs = shr(ctx, word, 3);
    let (a, b, c) = decompress_triple(ctx, msbs, Word16(word.0 & 7));
    slots[0] = a;
    slots[4] = b;
    slots[1] = c;

    let word = field(params[5], 10);
    let msbs = shr(ctx, word, 3);
    let (a, b, c) = decompress_triple(ctx, msbs, Word16(word.0 & 7));
    slots[2] = a;
    slots[6] = b;
    slots[5] = c;

    let word = field(params[6], 7);
    let msbs = shr(ctx, word, 2);
    let (a, b) = decompress_pair(ctx, msbs, Word16(word.0 & 3));
    slots[3] = a;
    slots[7] = b;

    (signs, slots)
}

/// Split a ten-bit compressed word into three slot indices, TS 26.073
/// `decompress10`.
///
/// The word is `125 * 8`: `msbs` walks a 5x5x5 grid that places all three
/// pulses at even slot *pairs*, and `lsbs` then nudges each of the three to the
/// odd slot of its pair, one bit per pulse.
fn decompress_triple(ctx: &mut DspContext, msbs: Word16, lsbs: Word16) -> (Word16, Word16, Word16) {
    // Load-bearing: `msbs` is a ten-bit field shifted right by three, so it
    // reaches 127, while only 0..=124 is legal. Without this the last three
    // values decode to slot 10 and index past the codevector. The reference has
    // this clamp for the same reason.
    let msbs = if sub(ctx, msbs, Word16(124)).0 > 0 {
        Word16(124)
    } else {
        msbs
    };

    // `msbs % 25`, via a Q15 reciprocal rather than a divide.
    let plane = mult(ctx, msbs, RECIP_25);
    let scaled = exact_product(ctx, plane, 25);
    let remainder = sub(ctx, msbs, scaled);

    // `(remainder % 5) * 2`.
    let row = mult(ctx, remainder, RECIP_5);
    let scaled = exact_product(ctx, row, 5);
    let column = sub(ctx, remainder, scaled);
    let low = shl(ctx, column, 1);

    // `lsbs % 4`, which is exact because `lsbs` is non-negative.
    let quarters = shr(ctx, lsbs, 2);
    let scaled = shl(ctx, quarters, 2);
    let corner = sub(ctx, lsbs, scaled);

    // Plain C `&` picks this pulse's own corner bit.
    let a = add(ctx, low, Word16(corner.0 & 1));
    let middle = shl(ctx, row, 1);
    let carry = shr(ctx, corner, 1);
    let b = add(ctx, middle, carry);
    // The reference recomputes `msbs / 25` here rather than keeping it from the
    // remainder step above, because it overwrote that variable in place. Same
    // value; recomputed the same way so the two spellings cannot drift apart.
    let plane = mult(ctx, msbs, RECIP_25);
    let high = shl(ctx, plane, 1);
    let c = add(ctx, high, quarters);

    (a, b, c)
}

/// Split the seven-bit compressed word into the two slot indices of pulses 3
/// and 7.
///
/// Hand-inlined in the reference rather than reusing `decompress10`, and not
/// merely a narrower version of it: the 5x5 grid is walked **boustrophedon**,
/// with odd rows traversed backwards, so that adjacent codes stay adjacent in
/// position. Dropping the row reversal gives pulses that are wrong but
/// plausible.
fn decompress_pair(ctx: &mut DspContext, msbs: Word16, lsbs: Word16) -> (Word16, Word16) {
    // `(msbs * 25 + 12) / 32`, rounding into 0..=24. The `+12` is an explicit
    // offset before a truncating shift, not a rounding shift — `shr_r` would
    // add 16 and give a different grid.
    let scaled = exact_product(ctx, msbs, 25);
    let rounded = add(ctx, scaled, Word16(12));
    let cell = shr(ctx, rounded, 5);

    let row = mult(ctx, cell, RECIP_5);
    // Plain C `&`: which direction this row runs.
    let reversed = (row.0 & 1) == 1;
    let scaled = exact_product(ctx, row, 5);
    let mut column = sub(ctx, cell, scaled);
    if reversed {
        column = sub(ctx, Word16(4), column);
    }

    let doubled = shl(ctx, column, 1);
    let a = add(ctx, doubled, Word16(lsbs.0 & 1));
    let doubled = shl(ctx, row, 1);
    let carry = shr(ctx, lsbs, 1);
    let b = add(ctx, doubled, carry);
    (a, b)
}

/// Ten pulses in thirty-five bits — 12.2 kbit/s, TS 26.073
/// `dec_10i40_35bits`.
///
/// `params` is the ten received fields: five of four bits (a gray-coded 3-bit
/// slot plus a sign in bit 3) and five of three bits (slot only). Pulse `j` and
/// pulse `j + 5` share track `j` and the single sign bit, with the second pulse
/// negated when it precedes the first — the same trick 10.2 uses.
///
/// A sign bit of **0** means positive.
///
/// Returns a **Q12** codevector — half the scale of every other rate — whose
/// samples are `0`, `±4096` or `±8192`.
#[must_use]
pub fn decode_ten_pulses_35bit(ctx: &mut DspContext, params: &[u16; 10]) -> Codevector {
    let mut code = [Word16(0); L_SUBFR];

    for track in 0..TRACKS_MR122 {
        let offset = Word16(i16::try_from(track).unwrap_or(0));
        let packed = field(params[track], 4);

        let slot = ungray(packed);
        let scaled = exact_product(ctx, slot, STEP);
        let first = add(ctx, scaled, offset);

        // Basic-operator shift, then a plain C mask for the sign bit.
        let negative = (shr(ctx, packed, 3).0 & 1) != 0;
        let mut sign = if negative {
            PULSE_MR122_NEG
        } else {
            PULSE_MR122
        };
        code[sample(first)] = sign;

        // The paired field carries no sign of its own — three bits, all slot.
        let slot = ungray(field(params[track + 5], 3));
        let scaled = exact_product(ctx, slot, STEP);
        let second = add(ctx, scaled, offset);
        if sub(ctx, second, first).0 < 0 {
            sign = negate(ctx, sign);
        }
        let at = sample(second);
        code[at] = add(ctx, code[at], sign);
    }

    code
}

/// Sharpen the codevector with its own recent past, the loop `dec_amr.c` runs
/// immediately after the codebook decoder returns.
///
/// `lag` is the **integer part** of the current subframe's pitch lag, 17..=143
/// at 12.2 kbit/s and 19..=143 elsewhere — not the `PIT_MIN` of 20/18, which
/// bounds the *fractional* lag. A lag of a subframe or more leaves the
/// codevector untouched. `factor` is Q15, from [`sharpening_factor`]; `code` is
/// Q13, or Q12 at 12.2, and `mult` preserves it.
///
/// Three details are load-bearing:
///
/// - It is **in place and self-referencing**. For `i >= 2 * lag` the sample it
///   reads has already been sharpened once in this same pass, which turns the
///   loop into a short IIR comb. That is reachable at every rate, since the lag
///   can be as low as 17, and never more than two levels deep, since three times
///   the smallest lag exceeds the subframe. Sharpening from a snapshot copy
///   would be a different filter.
/// - `mult` **floors toward negative infinity**, and codevector samples are
///   negative half the time. `mult(-8192, 26034)` is `-6509`, not the `-6508` a
///   truncating multiply gives.
/// - `add` **saturates asymmetrically**, to `-32768` or `32767`. At 10.2 a
///   doubled pulse can drive the recursion past 39000 in magnitude, so this is
///   reachable rather than theoretical.
pub fn sharpen(ctx: &mut DspContext, code: &mut Codevector, lag: i16, factor: Word16) {
    // A negative lag is not producible by the lag decoder; treating it as zero
    // keeps the slice arithmetic total instead of reading behind the buffer.
    let lag = usize::from(lag.max(0).unsigned_abs());

    for i in lag..L_SUBFR {
        let echo = mult(ctx, code[i - lag], factor);
        code[i] = add(ctx, code[i], echo);
    }
}

/// The Q15 factor [`sharpen`] takes, from its Q14 source.
///
/// The source differs by rate and the difference is easy to get wrong: at 12.2
/// it is the **current** subframe's decoded pitch gain, taken after error
/// concealment has had its say; at every other rate it is the persistent
/// sharpening state, which holds the pitch gain of an *earlier* subframe.
///
/// The saturation here is deliberate, and the reference marks it with a
/// commented-out `if (pit_sharp > 1.0) pit_sharp = 1.0`. The persistent state is
/// clipped to [`SHARPMAX`] and so never reaches it, but 12.2's top five
/// quantised pitch gains do: `19660` doubles to `39320` and comes back `32767`.
#[must_use]
pub fn sharpening_factor(ctx: &mut DspContext, source: Word16) -> Word16 {
    shl(ctx, source, 1)
}

/// Fold a decoded pitch gain into the persistent sharpening state, Q14.
///
/// The clip is what keeps [`sharpening_factor`] from saturating at the seven
/// rates that feed it from here. The *cadence* of the update is not this
/// module's: every rate but 4.75 stores after each subframe, while 4.75 stores
/// only after the odd ones, because it quantises gains for subframe pairs.
#[must_use]
pub fn sharpening_state(gain_pitch: Word16) -> Word16 {
    // A plain comparison: the reference's `sub` here can only saturate on the
    // negative side, where the branch is the same either way.
    Word16(gain_pitch.0.min(SHARPMAX))
}

#[cfg(test)]
mod tests {
    use super::super::vectors::rows;
    use super::*;

    fn ctx() -> DspContext {
        DspContext::default()
    }

    /// A fixture row pair: the `case` line's integers and the codevector the
    /// reference produced for them.
    fn cases(section: &str) -> Vec<(Vec<i32>, Vec<Word16>)> {
        let rows = rows(section);
        let mut out = Vec::new();
        let mut pending: Option<Vec<i32>> = None;

        for row in rows {
            match row.label {
                "case" => {
                    assert!(pending.is_none(), "{section}: two case lines in a row");
                    pending = Some(row.ints());
                }
                "nz" => {
                    let case = pending.take().expect("a codevector without a case line");
                    out.push((case, row.pulses(L_SUBFR)));
                }
                other => panic!("{section}: unexpected row {other:?}"),
            }
        }

        assert!(
            pending.is_none(),
            "{section}: trailing case without a vector"
        );
        out
    }

    fn as_u16(v: i32) -> u16 {
        u16::try_from(v).expect("fixture parameters are non-negative and fit a field")
    }

    #[test]
    fn nine_bit_codebook_is_bit_exact() {
        let mut c = ctx();
        let cases = cases("cb2i40_9");
        assert_eq!(cases.len(), 656, "cb2i40_9 case count");

        for (case, want) in &cases {
            let [sub_nr, signs, positions] = case[..] else {
                panic!("cb2i40_9 case is `subNr sign index`, got {case:?}")
            };
            let got = decode_two_pulses_9bit(
                &mut c,
                u8::try_from(sub_nr).expect("subframe 0..=3"),
                as_u16(signs),
                as_u16(positions),
            );
            assert_eq!(
                &got[..],
                &want[..],
                "subframe {sub_nr} sign {signs} index {positions}"
            );
        }
    }

    #[test]
    fn eleven_bit_codebook_is_bit_exact() {
        let mut c = ctx();
        let cases = cases("cb2i40_11");
        assert_eq!(cases.len(), 602, "cb2i40_11 case count");

        for (case, want) in &cases {
            let [signs, positions] = case[..] else {
                panic!("cb2i40_11 case is `sign index`, got {case:?}")
            };
            let got = decode_two_pulses_11bit(&mut c, as_u16(signs), as_u16(positions));
            assert_eq!(&got[..], &want[..], "sign {signs} index {positions}");
        }
    }

    #[test]
    fn fourteen_bit_codebook_is_bit_exact() {
        let mut c = ctx();
        let cases = cases("cb3i40_14");
        assert_eq!(cases.len(), 1297, "cb3i40_14 case count");

        for (case, want) in &cases {
            let [signs, positions] = case[..] else {
                panic!("cb3i40_14 case is `sign index`, got {case:?}")
            };
            let got = decode_three_pulses_14bit(&mut c, as_u16(signs), as_u16(positions));
            assert_eq!(&got[..], &want[..], "sign {signs} index {positions}");
        }
    }

    #[test]
    fn seventeen_bit_codebook_is_bit_exact() {
        let mut c = ctx();
        let cases = cases("cb4i40_17");
        assert_eq!(cases.len(), 1756, "cb4i40_17 case count");

        for (case, want) in &cases {
            let [signs, positions] = case[..] else {
                panic!("cb4i40_17 case is `sign index`, got {case:?}")
            };
            let got = decode_four_pulses_17bit(&mut c, as_u16(signs), as_u16(positions));
            assert_eq!(&got[..], &want[..], "sign {signs} index {positions}");
        }
    }

    #[test]
    fn thirty_one_bit_codebook_is_bit_exact() {
        let mut c = ctx();
        let cases = cases("cb8i40_31");
        assert_eq!(cases.len(), 250, "cb8i40_31 case count");

        for (case, want) in &cases {
            assert_eq!(case.len(), 7, "cb8i40_31 case is seven parameters");
            let mut params = [0u16; 7];
            for (slot, value) in params.iter_mut().zip(case) {
                *slot = as_u16(*value);
            }
            let got = decode_eight_pulses_31bit(&mut c, &params);
            assert_eq!(&got[..], &want[..], "params {params:?}");
        }
    }

    #[test]
    fn thirty_five_bit_codebook_is_bit_exact() {
        let mut c = ctx();
        let cases = cases("cb10i40_35");
        assert_eq!(cases.len(), 250, "cb10i40_35 case count");

        for (case, want) in &cases {
            assert_eq!(case.len(), 10, "cb10i40_35 case is ten parameters");
            let mut params = [0u16; 10];
            for (slot, value) in params.iter_mut().zip(case) {
                *slot = as_u16(*value);
            }
            let got = decode_ten_pulses_35bit(&mut c, &params);
            assert_eq!(&got[..], &want[..], "params {params:?}");
        }
    }

    #[test]
    fn the_dispatch_enum_agrees_with_the_six_functions() {
        // The enum is what the frame decoder will call, so a variant wired to
        // the wrong function would make every fixture test above vacuous.
        let mut c = ctx();
        let mut compared = 0;

        for (case, want) in cases("cb2i40_9") {
            let variant = FixedCodebook::TwoPulses9Bit {
                subframe: u8::try_from(case[0]).expect("subframe 0..=3"),
                signs: as_u16(case[1]),
                positions: as_u16(case[2]),
            };
            assert_eq!(&variant.decode(&mut c)[..], &want[..]);
            compared += 1;
        }
        for (case, want) in cases("cb2i40_11") {
            let variant = FixedCodebook::TwoPulses11Bit {
                signs: as_u16(case[0]),
                positions: as_u16(case[1]),
            };
            assert_eq!(&variant.decode(&mut c)[..], &want[..]);
            compared += 1;
        }
        for (case, want) in cases("cb3i40_14") {
            let variant = FixedCodebook::ThreePulses14Bit {
                signs: as_u16(case[0]),
                positions: as_u16(case[1]),
            };
            assert_eq!(&variant.decode(&mut c)[..], &want[..]);
            compared += 1;
        }
        for (case, want) in cases("cb4i40_17") {
            let variant = FixedCodebook::FourPulses17Bit {
                signs: as_u16(case[0]),
                positions: as_u16(case[1]),
            };
            assert_eq!(&variant.decode(&mut c)[..], &want[..]);
            compared += 1;
        }
        for (case, want) in cases("cb8i40_31") {
            let mut params = [0u16; 7];
            for (slot, value) in params.iter_mut().zip(&case) {
                *slot = as_u16(*value);
            }
            assert_eq!(
                &FixedCodebook::EightPulses31Bit(params).decode(&mut c)[..],
                &want[..]
            );
            compared += 1;
        }
        for (case, want) in cases("cb10i40_35") {
            let mut params = [0u16; 10];
            for (slot, value) in params.iter_mut().zip(&case) {
                *slot = as_u16(*value);
            }
            assert_eq!(
                &FixedCodebook::TenPulses35Bit(params).decode(&mut c)[..],
                &want[..]
            );
            compared += 1;
        }

        assert_eq!(compared, 656 + 602 + 1297 + 1756 + 250 + 250);
    }

    // ---------------------------------------------------------------- tables

    #[test]
    fn the_gray_map_and_its_inverse_are_inverses() {
        // A shared-assumption oracle cannot catch a table generated from the
        // wrong symbol: `gray` and `dgray` differ in only two entries, and a
        // codevector built with the wrong one is still a legal codevector.
        use super::super::decoder_tables::GRAY;

        for (code, &decoded) in DGRAY.iter().enumerate() {
            let back = GRAY[usize::try_from(decoded).expect("slot 0..=7")];
            assert_eq!(
                usize::try_from(back).expect("slot 0..=7"),
                code,
                "gray[dgray[{code}]] must be {code}"
            );
        }
        let mut seen = [false; 8];
        for &slot in &DGRAY {
            let slot = usize::try_from(slot).expect("slot 0..=7");
            assert!(
                slot < 8 && !seen[slot],
                "dgray must permute the eight slots"
            );
            seen[slot] = true;
        }
    }

    #[test]
    fn the_nine_bit_track_pairs_never_collide() {
        // The 9-bit codebook *assigns* rather than accumulates, so if the two
        // tracks of a pair ever shared a start offset one pulse would silently
        // vanish. They do not, and this is the invariant that says so.
        assert_eq!(START_POS_2I40_9.len(), 16);
        for entry in START_POS_2I40_9 {
            assert!(
                (0..5).contains(&entry),
                "track offsets are 0..=4, got {entry}"
            );
        }
        for pair in 0..2 {
            for subframe in 0..4 {
                let base = pair * 8 + subframe * 2;
                assert_ne!(
                    START_POS_2I40_9[base],
                    START_POS_2I40_9[base + 1],
                    "pair {pair} subframe {subframe} puts both pulses on one track"
                );
            }
        }
    }

    // ------------------------------------------------------------ properties

    /// Total pulse magnitude, which accumulation must conserve.
    fn total_magnitude(code: &Codevector) -> i32 {
        code.iter().map(|s| i32::from(s.0).abs()).sum()
    }

    #[test]
    fn the_wide_codebooks_conserve_pulse_magnitude() {
        // Eight (ten) pulses go in; a collision doubles one sample rather than
        // adding a ninth. Cancellation would break this sum, and cancellation is
        // exactly what a sign-negation applied on `pos2 <= pos1` instead of
        // `pos2 < pos1` would produce — a plausible off-by-one that the fixture
        // sweep might miss but this cannot.
        let mut c = ctx();
        let mut compared = 0;

        for (case, want) in cases("cb8i40_31") {
            assert_eq!(total_magnitude(&want.clone().try_into().unwrap()), 8 * 8191);
            let mut params = [0u16; 7];
            for (slot, value) in params.iter_mut().zip(&case) {
                *slot = as_u16(*value);
            }
            assert_eq!(
                total_magnitude(&decode_eight_pulses_31bit(&mut c, &params)),
                8 * 8191
            );
            compared += 1;
        }
        for (case, want) in cases("cb10i40_35") {
            assert_eq!(
                total_magnitude(&want.clone().try_into().unwrap()),
                10 * 4096
            );
            let mut params = [0u16; 10];
            for (slot, value) in params.iter_mut().zip(&case) {
                *slot = as_u16(*value);
            }
            assert_eq!(
                total_magnitude(&decode_ten_pulses_35bit(&mut c, &params)),
                10 * 4096
            );
            compared += 1;
        }

        assert_eq!(compared, 500);
    }

    #[test]
    fn the_narrow_codebooks_place_one_pulse_per_track() {
        // Exhaustive over every position field, which the fixture is not for
        // the 14- and 17-bit books. What it pins is that each pulse lands on
        // the set of tracks (position modulo five) that its own field is
        // allowed to reach — the invariant a mis-shifted position field breaks,
        // and one that survives a wrong *slot* so it cannot merely restate the
        // bit-exact test.
        //
        // Tracks are matched as a multiset, not in position order: a pulse on
        // track 0 with a high slot index sits *after* a pulse on track 3 with a
        // low one.
        let mut c = ctx();

        let residues = |code: &Codevector| -> Vec<usize> {
            let mut r: Vec<usize> = (0..L_SUBFR)
                .filter(|&i| code[i].0 != 0)
                .map(|i| i % 5)
                .collect();
            r.sort_unstable();
            r
        };

        for positions in 0u16..2048 {
            let tracks = residues(&decode_three_pulses_14bit(&mut c, 0, positions));
            assert_eq!(tracks.len(), 3, "index {positions} lost a pulse");
            assert!(
                tracks.contains(&0),
                "index {positions}: no pulse on track 0"
            );
            assert_eq!(
                tracks.iter().filter(|t| [1, 3].contains(t)).count(),
                1,
                "index {positions}: the second pulse left tracks 1 and 3"
            );
            assert_eq!(
                tracks.iter().filter(|t| [2, 4].contains(t)).count(),
                1,
                "index {positions}: the third pulse left tracks 2 and 4"
            );
        }

        for positions in 0u16..8192 {
            let tracks = residues(&decode_four_pulses_17bit(&mut c, 0, positions));
            assert_eq!(tracks.len(), 4, "index {positions} lost a pulse");
            for track in [0, 1, 2] {
                assert!(
                    tracks.contains(&track),
                    "index {positions}: no pulse on track {track}"
                );
            }
            assert_eq!(
                tracks.iter().filter(|t| [3, 4].contains(t)).count(),
                1,
                "index {positions}: the fourth pulse left tracks 3 and 4"
            );
        }
    }

    #[test]
    fn the_eleven_bit_codebook_overwrites_a_collision_instead_of_doubling_it() {
        // Both pulses can land on offset 1 of the same track. The reference
        // stores rather than accumulates, so the survivor keeps unit amplitude.
        // A decoder that accumulated would sound almost right.
        let mut c = ctx();
        let mut collisions = 0;

        for positions in 0u16..512 {
            let code = decode_two_pulses_11bit(&mut c, 0b11, positions);
            let nonzero: Vec<Word16> = code.into_iter().filter(|s| s.0 != 0).collect();
            assert!(!nonzero.is_empty() && nonzero.len() <= 2);
            for sample in &nonzero {
                assert_eq!(
                    sample.0, POSITIVE_PULSE.0,
                    "index {positions} doubled a pulse"
                );
            }
            if nonzero.len() == 1 {
                collisions += 1;
            }
        }

        assert!(
            collisions > 0,
            "the collision case must be reachable at all"
        );
    }

    // --------------------------------------------------------- bounds safety

    #[test]
    fn the_ten_two_position_decode_stays_within_its_tracks() {
        // Exhaustive over the entire position domain of 10.2 kbit/s: the two
        // ten-bit words and the seven-bit word decide all eight slots between
        // them, and the signs cannot move a pulse. Every slot must be 0..=9, or
        // `sample()` would have to clamp and the codevector would be wrong
        // rather than merely out of range.
        let mut c = ctx();
        let mut checked = 0;

        for word in 0u16..1024 {
            for (params, indices) in [
                ([0, 0, 0, 0, word, 0, 0], [0usize, 4, 1]),
                ([0, 0, 0, 0, 0, word, 0], [2, 6, 5]),
            ] {
                let (_, slots) = decompress_code(&mut c, &params);
                for i in indices {
                    assert!(
                        (0..10).contains(&slots[i].0),
                        "word {word} gave slot {} for pulse {i}",
                        slots[i].0
                    );
                }
                checked += 1;
            }
        }

        for word in 0u16..128 {
            let (_, slots) = decompress_code(&mut c, &[0, 0, 0, 0, 0, 0, word]);
            for i in [3usize, 7] {
                assert!(
                    (0..10).contains(&slots[i].0),
                    "word {word} gave slot {} for pulse {i}",
                    slots[i].0
                );
            }
            checked += 1;
        }

        assert_eq!(checked, 1024 * 2 + 128);
    }

    #[test]
    fn parameters_wider_than_their_fields_decode_without_panicking() {
        // The hazard is concrete: fed the full sixteen bits, 10.2's seven-bit
        // word decodes pulse 7 to slot 79, which in C writes 279 samples past
        // the end of the codevector. Masking to the field width is the guard,
        // and masking is a no-op for anything the unpacker can emit — so an
        // over-wide value must decode as its low bits do.
        let mut c = ctx();

        let wide = [u16::MAX; 7];
        let masked = [1u16, 1, 1, 1, 0x3FF, 0x3FF, 0x7F];
        assert_eq!(
            decode_eight_pulses_31bit(&mut c, &wide),
            decode_eight_pulses_31bit(&mut c, &masked)
        );
        assert_eq!(
            total_magnitude(&decode_eight_pulses_31bit(&mut c, &wide)),
            8 * 8191
        );

        let wide = [u16::MAX; 10];
        let masked = [0xFu16, 0xF, 0xF, 0xF, 0xF, 7, 7, 7, 7, 7];
        assert_eq!(
            decode_ten_pulses_35bit(&mut c, &wide),
            decode_ten_pulses_35bit(&mut c, &masked)
        );
        assert_eq!(
            total_magnitude(&decode_ten_pulses_35bit(&mut c, &wide)),
            10 * 4096
        );

        // The four narrow books consume only their own bits, so an over-wide
        // parameter is inert there too; and an impossible subframe number must
        // not index off the end of the start-position table.
        assert_eq!(
            decode_two_pulses_9bit(&mut c, 3, u16::MAX, u16::MAX),
            decode_two_pulses_9bit(&mut c, 3, 0b11, 0x7F)
        );
        for subframe in 0u8..=255 {
            let code = decode_two_pulses_9bit(&mut c, subframe, 0b11, 0x7F);
            assert_eq!(code.iter().filter(|s| s.0 != 0).count(), 2);
        }
        assert_eq!(
            decode_two_pulses_11bit(&mut c, u16::MAX, u16::MAX),
            decode_two_pulses_11bit(&mut c, 0b11, 0x1FF)
        );
        assert_eq!(
            decode_three_pulses_14bit(&mut c, u16::MAX, u16::MAX),
            decode_three_pulses_14bit(&mut c, 0b111, 0x7FF)
        );
        assert_eq!(
            decode_four_pulses_17bit(&mut c, u16::MAX, u16::MAX),
            decode_four_pulses_17bit(&mut c, 0b1111, 0x1FFF)
        );
    }

    #[test]
    fn masking_never_changes_a_fixture_case() {
        // The other half of the argument above: if masking altered any value the
        // oracle used, the bit-exact tests would be measuring a different
        // function from the reference's. It cannot, because the oracle's own
        // parameters are already inside their fields — assert that rather than
        // assume it.
        for (case, _) in cases("cb8i40_31") {
            for (value, bits) in case.iter().zip([1u32, 1, 1, 1, 10, 10, 7]) {
                assert!(*value < (1 << bits), "{value} does not fit {bits} bits");
            }
        }
        for (case, _) in cases("cb10i40_35") {
            for (value, bits) in case.iter().zip([4u32, 4, 4, 4, 4, 3, 3, 3, 3, 3]) {
                assert!(*value < (1 << bits), "{value} does not fit {bits} bits");
            }
        }
    }

    // ------------------------------------------------------------ sharpening

    #[test]
    fn sharpening_floors_toward_negative_infinity() {
        // The single most likely way to get this loop wrong. `mult` clears the
        // low fifteen bits of the product *before* shifting, so it floors rather
        // than truncating and a negative product rounds *away* from zero. The
        // ±8191 pair is the sharpest witness: the same magnitude in gives 6507
        // one way and 6508 the other. A truncating multiply would give 6507
        // both ways, and a round-to-nearest one 6508 both ways.
        let mut c = ctx();
        let factor = sharpening_factor(&mut c, Word16(SHARPMAX));
        assert_eq!(factor.0, 26034);

        let mut code = [Word16(0); L_SUBFR];
        code[0] = NEGATIVE_PULSE;
        code[1] = Word16(-8191);
        code[2] = POSITIVE_PULSE;
        sharpen(&mut c, &mut code, 19, factor);

        assert_eq!(code[19].0, -6509, "mult(-8192, 26034) floors to -6509");
        assert_eq!(code[20].0, -6508, "mult(-8191, 26034) floors to -6508");
        assert_eq!(code[21].0, 6507, "mult(8191, 26034) floors the other way");
    }

    #[test]
    fn sharpening_feeds_back_on_itself_within_one_pass() {
        // At the shortest lag the loop reads samples it wrote earlier in the
        // same pass. Two levels are reachable, three are not, because three
        // times the smallest lag exceeds the subframe.
        let mut c = ctx();
        let factor = sharpening_factor(&mut c, Word16(SHARPMAX));

        let mut code = [Word16(0); L_SUBFR];
        code[0] = NEGATIVE_PULSE;
        sharpen(&mut c, &mut code, 19, factor);

        assert_eq!(code[19].0, -6509);
        // -6509 sharpened again. A snapshot-based loop would leave this zero.
        assert_eq!(code[38].0, -5172);
        assert_eq!(code[39].0, 0, "sample 39 has no ancestor two lags back");
    }

    #[test]
    fn sharpening_saturates_asymmetrically_rather_than_wrapping() {
        // Reachable, not theoretical: 10.2 kbit/s can hand this loop a doubled
        // pulse of -16382, and at the shortest lag the recursion drives sample
        // 38 to -39739 before saturation.
        let mut c = ctx();
        let factor = sharpening_factor(&mut c, Word16(SHARPMAX));

        let mut code = [Word16(0); L_SUBFR];
        code[0] = Word16(-16382);
        code[19] = Word16(-16382);
        code[38] = Word16(-16382);
        sharpen(&mut c, &mut code, 19, factor);

        assert_eq!(code[19].0, -29398);
        assert_eq!(
            code[38].0,
            i16::MIN,
            "-39739 saturates to -32768, not -32767"
        );
    }

    #[test]
    fn a_lag_of_a_whole_subframe_leaves_the_codevector_alone() {
        let mut c = ctx();
        let factor = sharpening_factor(&mut c, Word16(SHARPMAX));
        let original = decode_four_pulses_17bit(&mut c, 0b1010, 1234);

        for lag in [40i16, 41, 143] {
            let mut code = original;
            sharpen(&mut c, &mut code, lag, factor);
            assert_eq!(code, original, "lag {lag} must not reach into the subframe");
        }
    }

    #[test]
    fn the_sharpening_factor_saturates_only_where_the_reference_says() {
        // 12.2 kbit/s feeds this the raw quantised pitch gain, whose top five
        // entries double past full scale. The seven other rates feed it the
        // clipped state, which cannot.
        let mut c = ctx();

        assert_eq!(sharpening_factor(&mut c, Word16(SHARPMAX)).0, 26034);
        assert_eq!(sharpening_factor(&mut c, Word16(16383)).0, 32766);
        assert_eq!(sharpening_factor(&mut c, Word16(16384)).0, i16::MAX);
        assert_eq!(sharpening_factor(&mut c, Word16(19660)).0, i16::MAX);

        assert_eq!(sharpening_state(Word16(19660)).0, SHARPMAX);
        assert_eq!(sharpening_state(Word16(1000)).0, 1000);
        assert_eq!(sharpening_state(Word16(SHARPMAX)).0, SHARPMAX);
    }
}
