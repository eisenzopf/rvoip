//! The IF2 interface format (TS 26.101 Annex B, TS 26.201 for wideband).
//!
//! # What IF2 is, and what it is not
//!
//! IF2 is the compact framing used across 3G interfaces and by some file
//! tools: a four-bit frame type, then the codec bits, then zero padding to the
//! next octet. No CMR, no table of contents, no CRC — one frame per unit, and
//! the frame type is the only metadata.
//!
//! The codec bits are the same importance-sorted order the RFC 4867 payload
//! already uses, which is the part worth stating plainly: `sort_tables` are
//! not an RTP thing, they are how AMR orders bits everywhere, so IF2 differs
//! from an octet-aligned RTP payload only in its header and padding.
//!
//! # Wideband only, and why
//!
//! Wireshark 4.6 reads our wideband IF2 frames correctly for all nine modes,
//! which confirms both the frame-type placement (high nibble) and that the
//! coded bits follow it. For narrowband the same probe says "Illegal
//! Frametype" for every frame: narrowband carries the type in the *low*
//! nibble instead, which varying that nibble confirms across all eight modes.
//!
//! That much is measured. What is *not* measured is where narrowband then
//! puts its coded bits — the oracle reports a frame's mode but never decodes
//! its speech bits, so it cannot distinguish "payload starts at octet 1" from
//! "payload fills the high nibble first". Both round-trip perfectly against
//! themselves and only one is right on real equipment.
//!
//! So narrowband IF2 is **refused** rather than approximated. TS 26.101
//! Annex B settles it in a paragraph; until this repository has that
//! paragraph, an error naming the gap is worth more than a plausible guess.
//!
//! # IF1 is deliberately absent
//!
//! IF1 carries a richer header — frame quality indicator, mode indication,
//! mode request, and a SID type indicator — whose exact bit positions come
//! from TS 26.101 §4 and TS 26.201, tables this repository does not have. The
//! reference implementations we build (TS 26.073/26.173) do not implement IF1
//! either; they use the MIME storage format. Wireshark's dissector has an IF1
//! mode, but reading its source is out (copyleft) and probing it as a black
//! box establishes only where *it* believes the fields sit, which is not the
//! same as the spec — and it cannot check the codec bits at all.
//!
//! Guessing those offsets would produce something that looks implemented,
//! passes its own round-trip tests, and is wrong on the wire against real
//! equipment. That is the failure mode this codebase has spent the most effort
//! avoiding, so IF1 waits for the tables rather than being approximated.

use super::mode::{AmrFrameType, AmrVariant};
use crate::error::{CodecError, Result};

/// Octets an IF2 frame occupies for a given frame type.
///
/// Four header bits plus the coded bits, rounded up to an octet.
#[must_use]
pub const fn if2_frame_len(frame_type: AmrFrameType) -> usize {
    (4 + if2_payload_bits(frame_type)).div_ceil(8)
}

/// Coded bits carried for a frame type, excluding the header nibble.
const fn if2_payload_bits(frame_type: AmrFrameType) -> usize {
    match frame_type {
        AmrFrameType::Speech(mode) => mode.bits(),
        // SID is 35 bits in both variants (TS 26.101 §4.2.1, TS 26.201).
        AmrFrameType::Sid(_) => 35,
        AmrFrameType::NoData | AmrFrameType::SpeechLost => 0,
    }
}

/// Whether this variant's IF2 frame type sits in the low nibble of the first
/// octet rather than the high one.
///
/// **The two variants differ**, which is the single most surprising thing in
/// this module and was found by the oracle rather than reasoned out: feeding
/// Wireshark 4.6 one frame per mode, AMR-WB reads correctly with the type in
/// the *high* nibble, while AMR-NB reads every such frame as "Illegal
/// Frametype" and reads the type from the *low* nibble instead — varying that
/// nibble walks all eight narrowband modes exactly, and varying the high one
/// changes nothing. TS 26.101 (narrowband) and TS 26.201 (wideband) are
/// separate documents, so a differing bit convention is theirs to have.
const fn if2_type_in_low_nibble(variant: AmrVariant) -> bool {
    matches!(variant, AmrVariant::NarrowBand)
}

/// The variant an IF2 frame type belongs to.
const fn if2_variant(frame_type: AmrFrameType) -> Option<AmrVariant> {
    match frame_type {
        AmrFrameType::Speech(mode) => Some(mode.variant()),
        AmrFrameType::Sid(variant) => Some(variant),
        // NO_DATA and SPEECH_LOST carry no variant of their own; the caller's
        // session knows it.
        AmrFrameType::NoData | AmrFrameType::SpeechLost => None,
    }
}

/// The error every narrowband IF2 call returns, naming what is missing rather
/// than what failed.
fn unverified_narrowband_layout() -> CodecError {
    CodecError::invalid_format(
        "AMR-NB IF2 is not implemented: the frame type is known to sit in the low nibble \
         (confirmed against Wireshark 4.6) but where the coded bits begin is not \
         externally verifiable here, and TS 26.101 Annex B is not available to this \
         repository. AMR-WB IF2 is supported.",
    )
}

/// The frame-type index IF2 carries, using the same numbering as the RFC 4867
/// table of contents.
const fn if2_frame_type_index(frame_type: AmrFrameType) -> u8 {
    match frame_type {
        AmrFrameType::Speech(mode) => mode.index(),
        AmrFrameType::Sid(_) => 8,
        AmrFrameType::SpeechLost => 14,
        AmrFrameType::NoData => 15,
    }
}

/// Pack one frame into IF2.
///
/// `bits` are the coded bits in the sorted order the payload format uses, one
/// per element, most significant first.
///
/// # Errors
///
/// When `bits` does not match the frame type's length.
pub fn if2_pack(frame_type: AmrFrameType, bits: &[u8]) -> Result<Vec<u8>> {
    if if2_variant(frame_type).is_some_and(if2_type_in_low_nibble) {
        return Err(unverified_narrowband_layout());
    }
    let expected = if2_payload_bits(frame_type);
    if bits.len() != expected {
        return Err(CodecError::invalid_format(format!(
            "IF2 frame needs {expected} coded bits, got {}",
            bits.len()
        )));
    }

    let mut out = vec![0u8; if2_frame_len(frame_type)];
    out[0] = if2_frame_type_index(frame_type) << 4;
    // Coded bits follow the header nibble.
    for (index, &bit) in bits.iter().enumerate() {
        if bit != 0 {
            let position = 4 + index;
            out[position / 8] |= 0x80 >> (position % 8);
        }
    }
    Ok(out)
}

/// Unpack one IF2 frame, returning its type and coded bits.
///
/// # Errors
///
/// When the data is empty, its frame-type nibble is not a valid type for the
/// variant, or it is shorter than that type requires.
pub fn if2_unpack(variant: AmrVariant, data: &[u8]) -> Result<(AmrFrameType, Vec<u8>)> {
    let Some(&first) = data.first() else {
        return Err(CodecError::invalid_format("IF2 frame is empty"));
    };
    if if2_type_in_low_nibble(variant) {
        return Err(unverified_narrowband_layout());
    }
    let frame_type = AmrFrameType::from_index(variant, first >> 4)?;
    let expected_len = if2_frame_len(frame_type);
    if data.len() < expected_len {
        return Err(CodecError::invalid_format(format!(
            "IF2 frame of type {:?} needs {expected_len} octets, got {}",
            frame_type,
            data.len()
        )));
    }

    let bit_count = if2_payload_bits(frame_type);
    let mut bits = Vec::with_capacity(bit_count);
    for index in 0..bit_count {
        let position = 4 + index;
        let octet = data[position / 8];
        bits.push(u8::from(octet & (0x80 >> (position % 8)) != 0));
    }
    Ok((frame_type, bits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::AmrMode;

    fn bits_of(mode: AmrMode) -> Vec<u8> {
        // A deterministic pattern that is not symmetric, so a reversed or
        // shifted unpack cannot round-trip by luck.
        (0..mode.bits())
            .map(|index| u8::from(index % 3 == 0 || index % 7 == 1))
            .collect()
    }

    #[test]
    fn every_wideband_mode_round_trips() {
        let variant = AmrVariant::WideBand;
        for mode in AmrMode::all(variant) {
            let bits = bits_of(mode);
            let packed = if2_pack(AmrFrameType::Speech(mode), &bits).expect("packs");
            assert_eq!(
                packed.len(),
                (4 + mode.bits()).div_ceil(8),
                "IF2 length is the header nibble plus the coded bits"
            );
            let (frame_type, unpacked) = if2_unpack(variant, &packed).expect("unpacks");
            assert_eq!(frame_type, AmrFrameType::Speech(mode));
            assert_eq!(unpacked, bits, "{mode:?} did not survive IF2");
        }
    }

    /// Narrowband is refused, and the error says why rather than pretending
    /// the format is unsupported in general.
    #[test]
    fn narrowband_is_refused_with_the_reason_named() {
        let mode = AmrMode::new(AmrVariant::NarrowBand, 7).expect("12.2");
        let error = if2_pack(AmrFrameType::Speech(mode), &bits_of(mode))
            .expect_err("narrowband IF2 must be refused, not guessed");
        let text = error.to_string();
        assert!(text.contains("26.101"), "the error must name what is missing: {text}");

        let error = if2_unpack(AmrVariant::NarrowBand, &[0x07, 0x00])
            .expect_err("narrowband IF2 must be refused on the read side too");
        assert!(error.to_string().contains("AMR-WB IF2 is supported"));
    }

    /// The two variants put the frame type in different nibbles, and this is
    /// the assertion that keeps it that way.
    ///
    /// Pinned against Wireshark 4.6: wideband frames with the type in the high
    /// nibble read as their mode, and narrowband frames only read correctly
    /// with it in the low nibble — with the high nibble it calls every one
    /// "Illegal Frametype". Regenerate with
    /// `examples/if2_vectors.rs` and check with
    /// `amr.encoding.version:AMR IF2`.
    #[test]
    fn the_wideband_frame_type_is_the_high_nibble() {
        for mode in AmrMode::all(AmrVariant::WideBand) {
            let packed = if2_pack(AmrFrameType::Speech(mode), &bits_of(mode)).expect("packs");
            assert_eq!(
                packed[0] >> 4,
                mode.index(),
                "AMR-WB carries the IF2 frame type in the high nibble"
            );
        }
    }

    #[test]
    fn sid_and_no_data_carry_their_own_lengths() {
        let sid = AmrFrameType::Sid(AmrVariant::WideBand);
        let packed = if2_pack(sid, &[1u8; 35]).expect("packs a SID");
        // 4 + 35 = 39 bits -> 5 octets.
        assert_eq!(packed.len(), 5);
        assert_eq!(packed[0] >> 4, 8, "SID is frame type 8");

        let no_data = if2_pack(AmrFrameType::NoData, &[]).expect("packs NO_DATA");
        assert_eq!(no_data.len(), 1, "a NO_DATA frame is its header alone");
        assert_eq!(no_data[0] >> 4, 15);
    }

    #[test]
    fn a_wrong_bit_count_is_refused_rather_than_padded() {
        let mode = AmrMode::new(AmrVariant::WideBand, 8).expect("23.85");
        assert!(if2_pack(AmrFrameType::Speech(mode), &[1u8; 10]).is_err());
        assert!(if2_pack(AmrFrameType::Speech(mode), &[1u8; 500]).is_err());
    }

    #[test]
    fn a_truncated_frame_is_refused_rather_than_read_past_its_end() {
        let mode = AmrMode::new(AmrVariant::WideBand, 8).expect("23.85");
        let packed = if2_pack(AmrFrameType::Speech(mode), &bits_of(mode)).expect("packs");
        for length in 0..packed.len() {
            assert!(
                if2_unpack(AmrVariant::WideBand, &packed[..length]).is_err(),
                "a {length}-octet prefix must not parse as a whole frame"
            );
        }
    }

    #[test]
    fn padding_bits_are_zero() {
        // TS 26.101 pads to the octet boundary with zeros; a packer that left
        // them uninitialised would produce frames that differ byte for byte
        // between runs while decoding identically, which is a nightmare to
        // diff against a reference.
        let mode = AmrMode::new(AmrVariant::WideBand, 0).expect("6.60");
        // 4 + 132 = 136 bits, an exact 17 octets, so use 8.85 for a partial
        // octet: 4 + 177 = 181 bits -> 23 octets with 3 padding bits.
        let packed = if2_pack(AmrFrameType::Speech(mode), &[1u8; 132]).expect("packs");
        assert_eq!(packed.len(), 17);

        let mode = AmrMode::new(AmrVariant::WideBand, 1).expect("8.85");
        let packed = if2_pack(AmrFrameType::Speech(mode), &[1u8; 177]).expect("packs");
        assert_eq!(packed.len(), 23);
        assert_eq!(packed[22] & 0b0000_0111, 0, "padding must be zero");
    }
}
