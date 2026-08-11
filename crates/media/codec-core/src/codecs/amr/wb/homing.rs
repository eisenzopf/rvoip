//! Homing frames for AMR-WB, TS 26.173 `homing.c`.
//!
//! # What a homing frame is for
//!
//! The specification's own way of driving a codec to a known state in the
//! middle of a stream. Feed the encoder a frame whose every sample is
//! `0x0008` and it emits one exact pattern of parameters and then resets;
//! feed the decoder that pattern and it emits a frame of `0x0008` and resets.
//! Both ends therefore agree on their entire internal state at a point the
//! test sequence chooses, which is what lets a conformance vector be compared
//! from its first sample rather than after an unspecified warm-up.
//!
//! It is not part of encoding or decoding. In the reference these tests live
//! in the *driver*, outside `coder()` and `decoder()`, and they stay outside
//! here too: homing is a protocol around the codec, not a stage in it.
//!
//! # The 23.85 kbit/s exception
//!
//! Every other rate compares its whole parameter frame. 23.85 alone masks the
//! four high-band energy fields out before comparing, because those depend on
//! the encoder's own high-band state rather than only on the input — so a
//! homing frame arriving mid-stream would not reproduce them.

use super::homing_tables::{DHF, PRMS_FIRST_SUBFRAME};

/// The sample value every homing frame is made of.
pub const HOMING_SAMPLE: i16 = 0x0008;

/// Bits per parameter word in the homing comparison.
const PRML: usize = 15;

/// Speech bits per frame, by mode index.
const NB_OF_BITS: [usize; 9] = [132, 177, 253, 285, 317, 365, 397, 461, 477];

/// Whether a frame of input is the encoder homing frame.
///
/// Every one of the 320 samples must be exactly [`HOMING_SAMPLE`]. A frame
/// that is merely very quiet is not a homing frame, which is why the committed
/// DTX fixture asserts its background is never digital silence.
#[must_use]
pub fn is_encoder_homing_frame(samples: &[i16]) -> bool {
    samples.len() == 320 && samples.iter().all(|&s| s == HOMING_SAMPLE)
}

/// Whether a payload is the decoder homing frame for `mode`.
///
/// `payload` is the RFC 4867 payload, sorted by subjective importance as it
/// arrives on the wire. The pattern is defined over *codec* bits, so this
/// unsorts first — comparing the sorted bytes directly matches nothing, which
/// is the mistake this signature exists to prevent.
///
/// # Panics
/// If `mode` is not a speech mode, 0..=8.
#[must_use]
pub fn is_decoder_homing_frame(payload: &[u8], mode: usize) -> bool {
    codec_bits(payload, mode).is_some_and(|bits| dhf_test(&bits, mode, NB_OF_BITS[mode]))
}

/// Unsort a payload into codec bits, one per entry.
fn codec_bits(payload: &[u8], mode: usize) -> Option<Vec<u8>> {
    let mode = crate::codecs::amr::mode::AmrMode::new(
        crate::codecs::amr::mode::AmrVariant::WideBand,
        u8::try_from(mode).ok()?,
    )
    .ok()?;
    super::bitstream::CodecBits::unpack(mode, payload).map(|b| b.bits().to_vec())
}

/// Whether a payload's *first subframe* is the decoder homing frame's.
///
/// A decoder already homed checks only this much: once homing has taken
/// effect the rest of the frame carries the next frame's content, so
/// comparing all of it would fail on a legitimate sequence.
///
/// # Panics
/// If `mode` is not a speech mode, 0..=8.
#[must_use]
pub fn is_decoder_homing_frame_first(payload: &[u8], mode: usize) -> bool {
    codec_bits(payload, mode).is_some_and(|bits| dhf_test(&bits, mode, PRMS_FIRST_SUBFRAME[mode]))
}

/// `dhf_test`: unpack `bits` worth of 15-bit words and compare against the
/// mode's pattern.
fn dhf_test(codec: &[u8], mode: usize, bits: usize) -> bool {
    assert!(mode < 9, "homing is defined for the nine speech modes");
    if codec.len() < bits {
        return false;
    }
    let want = &DHF[mode];

    let mut read = SerialBits::new(codec);
    let mut param = [0i16; 32];
    let (count, shift) = if mode == 8 {
        // 23.85 kbit/s: the four high-band energy fields are masked out. The
        // masks are the reference's own and are transcribed rather than
        // derived, because which bits of which word each gain occupies is a
        // fact about the bit layout and not something to recompute.
        for slot in param.iter_mut().take(10) {
            *slot = read.take(PRML);
        }
        param[10] = read.take(PRML) & 0x61FF;
        for slot in param.iter_mut().take(17).skip(11) {
            *slot = read.take(PRML);
        }
        param[17] = read.take(PRML) & -0x1F00; // 0xE0FF as i16
        for slot in param.iter_mut().take(24).skip(18) {
            *slot = read.take(PRML);
        }
        param[24] = read.take(PRML) & 0x7F0F;
        for slot in param.iter_mut().take(31).skip(25) {
            *slot = read.take(PRML);
        }
        param[31] = read.take(8) << 7;
        // The reference's index stops at 31: `param[31]` is the last word it
        // writes, and the final comparison is against that one.
        (31, 0)
    } else {
        // Whole 15-bit words while a whole one remains, then the remainder
        // left-aligned so the comparison sees it in the same position the
        // table holds it.
        let mut i = 0usize;
        let mut consumed = 0usize;
        while bits.saturating_sub(PRML) > consumed {
            param[i] = read.take(PRML);
            consumed += PRML;
            i += 1;
        }
        let tail = bits - consumed;
        param[i] = read.take(tail) << (PRML - tail);
        (i, PRML - tail)
    };

    // Everything before the final word must match exactly; the final word is
    // compared only down to the bits that were actually present.
    for i in 0..count {
        if param[i] != want[i] {
            return false;
        }
    }
    let mask = (0x7fffi16 >> shift) << shift;
    param[count] == (want[count] & mask)
}

/// A most-significant-bit-first reader over codec bits, one per entry.
struct SerialBits<'a> {
    codec: &'a [u8],
    at: usize,
}

impl<'a> SerialBits<'a> {
    const fn new(codec: &'a [u8]) -> Self {
        Self { codec, at: 0 }
    }

    fn take(&mut self, width: usize) -> i16 {
        let mut value = 0i16;
        for _ in 0..width {
            value = (value << 1) | i16::from(self.codec[self.at]);
            self.at += 1;
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoder_homing_frame_is_exactly_that_and_nothing_near_it() {
        assert!(is_encoder_homing_frame(&[HOMING_SAMPLE; 320]));
        // One sample off is not a homing frame, and nor is digital silence.
        let mut nearly = [HOMING_SAMPLE; 320];
        nearly[319] = 9;
        assert!(!is_encoder_homing_frame(&nearly));
        assert!(!is_encoder_homing_frame(&[0i16; 320]));
        // Nor is a frame of the wrong length made of the right samples.
        assert!(!is_encoder_homing_frame(&[HOMING_SAMPLE; 319]));
    }

    /// Every mode's own pattern is recognised, and no other mode's is.
    ///
    /// The second half is what catches a table read at the wrong offset: nine
    /// patterns that all begin `3168` would pass a test that only checked its
    /// own row.
    #[test]
    #[allow(clippy::needless_range_loop)]
    fn each_mode_recognises_its_own_pattern_and_rejects_the_others() {
        let mut checked = 0;
        for mode in 0..9usize {
            let payload = homing_payload(mode);
            assert!(
                is_decoder_homing_frame(&payload, mode),
                "mode {mode} did not recognise its own homing frame"
            );
            assert!(is_decoder_homing_frame_first(&payload, mode));
            for other in 0..9usize {
                if other == mode || NB_OF_BITS[other] > NB_OF_BITS[mode] {
                    continue;
                }
                assert!(
                    !is_decoder_homing_frame(&payload, other),
                    "mode {other} accepted mode {mode}'s homing frame"
                );
                checked += 1;
            }
        }
        assert!(checked >= 30, "only {checked} cross-mode pairs compared");
    }

    /// The encoder, fed the homing frame, emits the homing pattern.
    ///
    /// This is the claim that matters: the tables above are only worth having
    /// if the codec actually produces them. It also exercises the whole
    /// encoder from a cold start at every rate, which nothing else does — the
    /// bitstream fixtures all begin from ordinary speech.
    #[test]
    fn the_encoder_emits_each_modes_homing_pattern() {
        use crate::codecs::amr::wb::enc::encoder::{Rate, WbEncoder};

        for mode in 0..9u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let mut encoder = WbEncoder::new();
            let payload = encoder.encode_frame(&[HOMING_SAMPLE; 320], rate);
            assert!(
                is_decoder_homing_frame(&payload, mode as usize),
                "mode {mode} did not emit its homing pattern"
            );
        }
    }

    #[test]
    fn an_ordinary_frame_is_not_a_homing_frame() {
        for mode in 0..9usize {
            let payload = vec![0x5Au8; 64];
            assert!(!is_decoder_homing_frame(&payload, mode));
            let zeros = vec![0u8; 64];
            assert!(!is_decoder_homing_frame(&zeros, mode));
        }
    }

    /// 23.85 kbit/s ignores the high-band energy bits, and only it does.
    #[test]
    fn the_high_band_gains_are_masked_at_23_85_and_nowhere_else() {
        // A bit the `param[10] & 0x61FF` mask discards. That mask keeps bits
        // 14, 13 and 8..0 of the fifteen-bit word, so 12..9 are the masked
        // ones; bit 12 is codec bit 10*15 + (14 - 12). A *codec* position, not
        // a payload one -- the payload is a permutation of these.
        let masked = 10 * PRML + 2;
        assert!(
            is_decoder_homing_frame(&homing_payload_with_flip(8, Some(masked)), 8),
            "23.85 must ignore the masked high-band bits"
        );
        // And a kept bit of the same word must still be rejected, or the test
        // above would pass for a comparison that ignored everything.
        let kept = 10 * PRML + 1;
        assert!(!is_decoder_homing_frame(&homing_payload_with_flip(8, Some(kept)), 8));

        // The same position in a rate that masks nothing must be rejected.
        assert!(!is_decoder_homing_frame(&homing_payload_with_flip(7, Some(masked)), 7));
    }

    /// Serialise a mode's pattern into the payload a receiver would see.
    ///
    /// Codec bits first, then sorted into wire order — the same permutation
    /// the encoder applies. Building the payload directly from the table would
    /// test the comparison against its own inverse and prove nothing about the
    /// sorting.
    fn homing_payload(mode: usize) -> Vec<u8> {
        homing_payload_with_flip(mode, None)
    }

    /// The same, with one codec bit inverted.
    fn homing_payload_with_flip(mode: usize, flip: Option<usize>) -> Vec<u8> {
        let bits = NB_OF_BITS[mode];
        let want = &DHF[mode];
        let mut codec = vec![0u8; bits];
        let (mut at, mut word) = (0usize, 0usize);
        while at < bits {
            let width = PRML.min(bits - at);
            // The reference stores the final partial word left-aligned.
            let value = if width == PRML {
                want[word]
            } else {
                want[word] >> (PRML - width)
            };
            for k in 0..width {
                codec[at] = u8::try_from((value >> (width - 1 - k)) & 1).expect("one bit");
                at += 1;
            }
            word += 1;
        }
        if let Some(bit) = flip {
            codec[bit] ^= 1;
        }

        let amr = crate::codecs::amr::mode::AmrMode::new(
            crate::codecs::amr::mode::AmrVariant::WideBand,
            u8::try_from(mode).expect("a speech mode"),
        )
        .expect("a speech mode");
        let sort = super::super::bitstream::sort_table_for(amr);
        let mut out = vec![0u8; bits.div_ceil(8)];
        for (i, &source) in sort.iter().enumerate() {
            out[i / 8] |= codec[source as usize] << (7 - (i % 8));
        }
        out
    }
}
