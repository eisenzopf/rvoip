//! Homing frames for AMR-NB, TS 26.073 `e_homing.c` and `d_homing.c`.
//!
//! The narrowband counterpart of [`super::super::wb::homing`], and it is
//! considerably simpler than that one: the pattern is stored as *parameters*
//! and compared against the decoded parameters directly. No 15-bit repacking,
//! no left-aligned final word, and no masked fields — wideband's three
//! complications all come from its own table being stored in packed form.
//!
//! What homing is *for* is the same in both, and the wideband module's header
//! explains it: feed the encoder a frame of `0x0008` samples and it emits one
//! exact parameter pattern and resets; feed the decoder that pattern and it
//! emits `0x0008` and resets. Both ends then agree on their whole internal
//! state at a point the test sequence chooses. It is a protocol *around* the
//! codec, not a stage in it, and it lives outside `Decoder` here exactly as it
//! lives outside `Decoder_amr` in the reference.

use super::bitstream::parse;
use super::decoder_tables::{
    DHF_MR102, DHF_MR122, DHF_MR475, DHF_MR515, DHF_MR59, DHF_MR67, DHF_MR74, DHF_MR795,
};

/// The sample value every narrowband homing frame is made of — `EHF_MASK`.
pub const HOMING_SAMPLE: i16 = 0x0008;

/// Samples in a narrowband frame.
const L_FRAME: usize = 160;

/// How many parameters cover the LPC and the first subframe, per mode.
///
/// `prmnofsf`. A decoder that has already homed compares only this much,
/// because once homing has taken effect the rest of the frame carries the
/// *next* frame's content and comparing all of it would reject a legitimate
/// sequence.
const PRMNOFSF: [usize; 8] = [7, 7, 7, 7, 7, 8, 12, 18];

/// The decoder homing pattern for a mode, as parameters.
const fn pattern(mode_index: u8) -> &'static [i16] {
    match mode_index {
        0 => &DHF_MR475,
        1 => &DHF_MR515,
        2 => &DHF_MR59,
        3 => &DHF_MR67,
        4 => &DHF_MR74,
        5 => &DHF_MR795,
        6 => &DHF_MR102,
        _ => &DHF_MR122,
    }
}

/// Whether a frame of input is the encoder homing frame.
///
/// All 160 samples must be exactly [`HOMING_SAMPLE`]. Note the reference's own
/// implementation has a quirk worth not reproducing: it breaks out of the loop
/// on the first mismatch and returns `!j` where `j` is the last exclusive-or,
/// so an *empty* frame would read as a match through an uninitialised
/// variable. This returns false for a frame of the wrong length instead.
#[must_use]
pub fn is_encoder_homing_frame(samples: &[i16]) -> bool {
    samples.len() == L_FRAME && samples.iter().all(|&s| s == HOMING_SAMPLE)
}

/// Whether a payload is the decoder homing frame for `mode_index`.
///
/// `payload` is the RFC 4867 frame body, sorted by subjective importance as it
/// arrives on the wire; the pattern is defined over decoded parameters, so this
/// unsorts and unpacks first.
///
/// # Panics
/// If `mode_index` is not a speech mode, 0..=7.
#[must_use]
pub fn is_decoder_homing_frame(payload: &[u8], mode_index: u8) -> bool {
    matches(payload, mode_index, usize::MAX)
}

/// Whether a payload's LPC and *first subframe* match the homing pattern.
///
/// What a decoder that has already homed checks; see the private `PRMNOFSF` table.
///
/// # Panics
/// If `mode_index` is not a speech mode, 0..=7.
#[must_use]
pub fn is_decoder_homing_frame_first(payload: &[u8], mode_index: u8) -> bool {
    matches(payload, mode_index, PRMNOFSF[usize::from(mode_index)])
}

/// `dhf_test`: compare the first `count` decoded parameters against the
/// pattern.
fn matches(payload: &[u8], mode_index: u8, count: usize) -> bool {
    assert!(
        mode_index < 8,
        "homing is defined for the eight speech modes"
    );
    let Some(params) = parse(mode_index, payload) else {
        return false;
    };
    let want = pattern(mode_index);
    let count = count.min(want.len());
    params.len() >= count
        && (0..count).all(|i| i16::try_from(params[i]).is_ok_and(|p| p == want[i]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::nb::bitstream::parameter_widths;
    use crate::codecs::amr::nb::tables::{
        SORT_102, SORT_122, SORT_475, SORT_515, SORT_59, SORT_67, SORT_74, SORT_795,
    };

    const fn sort_table(mode_index: u8) -> &'static [u16] {
        match mode_index {
            0 => &SORT_475,
            1 => &SORT_515,
            2 => &SORT_59,
            3 => &SORT_67,
            4 => &SORT_74,
            5 => &SORT_795,
            6 => &SORT_102,
            _ => &SORT_122,
        }
    }

    /// Serialise a mode's pattern into the payload a receiver would see.
    ///
    /// Parameters to codec bits, then sorted into wire order — the same
    /// permutation the encoder applies. Building the payload straight from the
    /// table would test the comparison against its own inverse.
    fn homing_payload(mode_index: u8, flip: Option<usize>) -> Vec<u8> {
        let want = pattern(mode_index);
        let widths = parameter_widths(mode_index);
        let mut codec = Vec::new();
        for (&value, &width) in want.iter().zip(widths.iter()) {
            for k in (0..width).rev() {
                codec.push(u8::try_from((value >> k) & 1).expect("one bit"));
            }
        }
        if let Some(bit) = flip {
            codec[bit] ^= 1;
        }

        let sort = sort_table(mode_index);
        let mut out = vec![0u8; codec.len().div_ceil(8)];
        for (i, &source) in sort.iter().enumerate() {
            out[i / 8] |= codec[source as usize] << (7 - (i % 8));
        }
        out
    }

    #[test]
    fn the_encoder_homing_frame_is_exactly_that_and_nothing_near_it() {
        assert!(is_encoder_homing_frame(&[HOMING_SAMPLE; L_FRAME]));
        let mut nearly = [HOMING_SAMPLE; L_FRAME];
        nearly[159] = 9;
        assert!(!is_encoder_homing_frame(&nearly));
        assert!(!is_encoder_homing_frame(&[0i16; L_FRAME]));
        // Wideband's frame length, which must not be accepted here.
        assert!(!is_encoder_homing_frame(&[HOMING_SAMPLE; 320]));
    }

    /// Every mode recognises its own pattern and rejects the others'.
    ///
    /// The second half is what catches a table read at the wrong offset: eight
    /// patterns that all begin with the same LSF indices would pass a test that
    /// only ever checked its own row.
    #[test]
    fn each_mode_recognises_its_own_pattern_and_rejects_the_others() {
        let mut cross = 0;
        for mode in 0..8u8 {
            let payload = homing_payload(mode, None);
            assert!(
                is_decoder_homing_frame(&payload, mode),
                "mode {mode} rejected its own"
            );
            assert!(is_decoder_homing_frame_first(&payload, mode));

            for other in 0..8u8 {
                if other == mode {
                    continue;
                }
                assert!(
                    !is_decoder_homing_frame(&payload, other),
                    "mode {other} accepted mode {mode}'s homing frame"
                );
                cross += 1;
            }
        }
        assert_eq!(cross, 56, "eight modes cross-checked pairwise");
    }

    /// The encoder, fed the homing frame, emits the homing pattern.
    ///
    /// The claim that makes the table worth having. It also exercises the whole
    /// encoder from a cold start at every rate, which nothing else does — the
    /// bitstream fixtures all begin from ordinary speech.
    #[test]
    fn the_encoder_emits_each_modes_homing_pattern() {
        use crate::codecs::amr::nb::enc::encoder::{NbEncoder, Rate};

        for mode in 0..8u8 {
            let mut encoder = NbEncoder::new();
            let rate = Rate::from_index(mode).expect("a speech mode");
            let payload = encoder.encode_frame(&[HOMING_SAMPLE; L_FRAME], rate);
            assert!(
                is_decoder_homing_frame(&payload, mode),
                "mode {mode} did not emit its homing pattern"
            );
        }
    }

    /// The truncated comparison is genuinely shorter than the full one.
    ///
    /// A bit belonging to a *later* subframe must be ignored by the
    /// first-subframe test and caught by the full one. If `PRMNOFSF` were
    /// simply the whole parameter count the two would agree everywhere, and
    /// the distinction the reference draws would be decoration.
    #[test]
    fn the_first_subframe_test_ignores_what_the_full_test_catches() {
        let mut checked = 0;
        for mode in 0..8u8 {
            let widths = parameter_widths(mode);
            let first = PRMNOFSF[usize::from(mode)];
            assert!(
                first < widths.len(),
                "mode {mode}: PRMNOFSF covers the whole frame"
            );

            // The first bit of the parameter just past the first subframe.
            let offset: usize = widths[..first].iter().sum();
            let payload = homing_payload(mode, Some(offset));
            assert!(
                is_decoder_homing_frame_first(&payload, mode),
                "mode {mode}: a later subframe's bit reached the truncated test"
            );
            assert!(
                !is_decoder_homing_frame(&payload, mode),
                "mode {mode}: the full test missed a flipped bit"
            );

            // And a bit inside the first subframe fails both.
            let payload = homing_payload(mode, Some(0));
            assert!(!is_decoder_homing_frame_first(&payload, mode));
            assert!(!is_decoder_homing_frame(&payload, mode));
            checked += 1;
        }
        assert_eq!(checked, 8);
    }

    #[test]
    fn an_ordinary_frame_is_not_a_homing_frame() {
        for mode in 0..8u8 {
            assert!(!is_decoder_homing_frame(&[0x5Au8; 32], mode));
            assert!(!is_decoder_homing_frame(&[0u8; 32], mode));
            // Too short for any rate: refused rather than read past the end.
            assert!(!is_decoder_homing_frame(&[0xFFu8; 2], mode));
        }
    }
}
