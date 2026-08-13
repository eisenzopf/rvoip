//! The IF2 interface format: TS 26.101 Annex A (narrowband) and TS 26.201
//! Annex A (wideband).
//!
//! # What IF2 is
//!
//! The octet-aligned framing used across 3G interfaces and by file tools: a
//! header, the codec's Core Frame bits, then stuffing to the next octet
//! boundary. No CMR, no table of contents, no CRC — one frame per unit.
//!
//! # The two variants are genuinely different formats
//!
//! They come from separate specifications and agree on almost nothing:
//!
//! | | narrowband (26.101) | wideband (26.201) |
//! |---|---|---|
//! | Frame Type | four **LSBs** of octet 1 | four **MSBs** of octet 1 |
//! | Frame Quality Indicator | **absent** | 1 bit, after the frame type |
//! | Bit packing | **LSB-first** within each octet | **MSB-first** |
//! | SID Core Frame | 39 bits | 40 bits |
//!
//! Narrowband octet 1 is `d(3) d(2) d(1) d(0) | FT`, and octet 2 is
//! `d(11) … d(4)` — the bits walk upward from bit 1. Wideband octet 1 is
//! `FT | FQI | d(0) d(1) d(2)` and octet 2 is `d(3) … d(10)`, walking down
//! from bit 8. Assuming either layout covers both produces frames that a
//! dissector still labels with the right mode, because the mode is read from
//! the header nibble and nothing downstream checks the rest.
//!
//! # Provenance
//!
//! The layouts and every frame length here come from the two specifications'
//! own tables (26.101 Table A.1b, 26.201 Table A.1b, and the worked 6.70 and
//! 8.85 kbit/s examples). The specs are fetched for design and never
//! redistributed, exactly as the reference C is; nothing of theirs is in this
//! tree beyond the constants they define.
//!
//! # IF1
//!
//! The generic frame format of the two specs' §4: header (frame type, frame
//! quality indicator), auxiliary information (mode indication, mode request,
//! codec CRC), then the Core Frame, all MSB-first from bit 8 of each octet.
//! The two variants differ again, in their own way this time:
//!
//! | | narrowband (26.101 §4.3) | wideband (26.201 §4.3) |
//! |---|---|---|
//! | Octet 1 | FT(4) FQI(1) ModeInd(3) | FT(4) FQI(1) spare(3) |
//! | Octet 2 | ModeReq(3) spare(5) | ModeInd(4) ModeReq(4) |
//! | Octet 3 | Codec CRC | Codec CRC |
//! | FT 14/15 | frame type only, 4 bits | FT + FQI, 5 bits |
//!
//! The codec CRC is `G(x) = x^8 + x^6 + x^5 + x^4 + 1` over the Core Frame's
//! class A bits — the exact bit-reversal of RFC 4867's payload CRC
//! polynomial, so the two share nothing at runtime and each carries its own
//! hand-worked vectors.

use super::mode::{AmrFrameType, AmrVariant};
use crate::error::{CodecError, Result};

/// Whether a variant's IF2 header carries a Frame Quality Indicator bit.
///
/// Wideband does (TS 26.201 A.1b); narrowband does not (TS 26.101 A.1b).
const fn if2_has_fqi(variant: AmrVariant) -> bool {
    matches!(variant, AmrVariant::WideBand)
}

/// Header bits before the Core Frame: the frame type, plus wideband's FQI.
const fn if2_header_bits(variant: AmrVariant) -> usize {
    if if2_has_fqi(variant) {
        5
    } else {
        4
    }
}

/// Core Frame bits for a frame type, from the specs' Table A.1b.
const fn if2_core_bits(frame_type: AmrFrameType) -> usize {
    match frame_type {
        AmrFrameType::Speech(mode) => mode.bits(),
        // 26.101 A.1b: narrowband SID is 39 bits (35 comfort-noise bits, the
        // SID type indicator, and three of mode indication). 26.201 A.1b:
        // wideband SID is 40.
        AmrFrameType::Sid(AmrVariant::NarrowBand) => 39,
        AmrFrameType::Sid(AmrVariant::WideBand) => 40,
        AmrFrameType::NoData | AmrFrameType::SpeechLost => 0,
    }
}

/// Octets an IF2 frame occupies: header, Core Frame, then stuffing to the
/// octet boundary.
#[must_use]
pub const fn if2_frame_len(variant: AmrVariant, frame_type: AmrFrameType) -> usize {
    (if2_header_bits(variant) + if2_core_bits(frame_type)).div_ceil(8)
}

/// The frame-type index IF2 carries, using the same numbering as the RFC 4867
/// table of contents.
const fn if2_frame_type_index(frame_type: AmrFrameType) -> u8 {
    match frame_type {
        AmrFrameType::Speech(mode) => mode.index(),
        // 26.101 numbers narrowband SID 8; 26.201 numbers wideband SID 9.
        AmrFrameType::Sid(AmrVariant::NarrowBand) => 8,
        AmrFrameType::Sid(AmrVariant::WideBand) => 9,
        AmrFrameType::SpeechLost => 14,
        AmrFrameType::NoData => 15,
    }
}

/// Write one bit at logical position `index`, in the variant's own bit order.
///
/// Narrowband walks upward from bit 1 of each octet, wideband downward from
/// bit 8 — the single difference that makes the two formats incompatible
/// beyond their headers.
fn if2_set_bit(variant: AmrVariant, out: &mut [u8], index: usize, value: bool) {
    if !value {
        return;
    }
    let octet = index / 8;
    let within = index % 8;
    out[octet] |= if if2_has_fqi(variant) {
        0x80 >> within
    } else {
        1 << within
    };
}

/// Read one bit at logical position `index`, in the variant's own bit order.
fn if2_get_bit(variant: AmrVariant, data: &[u8], index: usize) -> bool {
    let octet = data[index / 8];
    let within = index % 8;
    let mask = if if2_has_fqi(variant) {
        0x80 >> within
    } else {
        1 << within
    };
    octet & mask != 0
}

/// Pack one frame into IF2.
///
/// `bits` are the Core Frame bits in the order the codec produces them, one
/// per element. `quality_ok` sets wideband's Frame Quality Indicator and is
/// ignored for narrowband, which has no such field.
///
/// # Errors
///
/// When `bits` does not match the frame type's Core Frame length.
pub fn if2_pack(
    variant: AmrVariant,
    frame_type: AmrFrameType,
    bits: &[u8],
    quality_ok: bool,
) -> Result<Vec<u8>> {
    let expected = if2_core_bits(frame_type);
    if bits.len() != expected {
        return Err(CodecError::invalid_format(format!(
            "IF2 {variant:?} frame needs {expected} core bits, got {}",
            bits.len()
        )));
    }

    let mut out = vec![0u8; if2_frame_len(variant, frame_type)];
    let index_value = if2_frame_type_index(frame_type);

    // The frame type occupies four bits, most significant first in both
    // formats -- what differs is where those four bits sit and which way the
    // rest of the frame then walks.
    for bit in 0..4 {
        let set = index_value & (0b1000 >> bit) != 0;
        if if2_has_fqi(variant) {
            if2_set_bit(variant, &mut out, bit, set);
        } else {
            // Narrowband: the four LSBs of octet 1, MSB of the field at bit 4.
            if set {
                out[0] |= 1 << (3 - bit);
            }
        }
    }

    let mut position = if2_header_bits(variant);
    if if2_has_fqi(variant) {
        if2_set_bit(variant, &mut out, 4, quality_ok);
    }
    for &bit in bits {
        if2_set_bit(variant, &mut out, position, bit != 0);
        position += 1;
    }
    Ok(out)
}

/// One unpacked IF2 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct If2Frame {
    /// What the frame carries.
    pub frame_type: AmrFrameType,
    /// Wideband's Frame Quality Indicator. Always `true` for narrowband,
    /// which has no such field and therefore cannot report a bad frame.
    pub quality_ok: bool,
    /// Core Frame bits, one per element.
    pub bits: Vec<u8>,
}

/// Unpack one IF2 frame.
///
/// # Errors
///
/// When the data is empty, its frame type is not valid for the variant, or it
/// is shorter than that frame type requires.
pub fn if2_unpack(variant: AmrVariant, data: &[u8]) -> Result<If2Frame> {
    let Some(&first) = data.first() else {
        return Err(CodecError::invalid_format("IF2 frame is empty"));
    };
    let index_value = if if2_has_fqi(variant) {
        first >> 4
    } else {
        first & 0x0F
    };
    let frame_type = AmrFrameType::from_index(variant, index_value)?;

    let expected_len = if2_frame_len(variant, frame_type);
    if data.len() < expected_len {
        return Err(CodecError::invalid_format(format!(
            "IF2 {variant:?} frame of type {frame_type:?} needs {expected_len} octets, got {}",
            data.len()
        )));
    }

    let quality_ok = !if2_has_fqi(variant) || if2_get_bit(variant, data, 4);
    let mut position = if2_header_bits(variant);
    let mut bits = Vec::with_capacity(if2_core_bits(frame_type));
    for _ in 0..if2_core_bits(frame_type) {
        bits.push(u8::from(if2_get_bit(variant, data, position)));
        position += 1;
    }
    Ok(If2Frame {
        frame_type,
        quality_ok,
        bits,
    })
}


// ---------------------------------------------------------------------------
// IF1: the generic frame format of TS 26.101 §4 / TS 26.201 §4.
// ---------------------------------------------------------------------------

/// The IF1 codec CRC: `G(x) = x^8 + x^6 + x^5 + x^4 + 1` over class A bits,
/// one input bit per element, plain long division from a zero remainder (the
/// specs state no preset or inversion).
///
/// This is the bit-reversal of RFC 4867's payload CRC polynomial, not the
/// same CRC — reusing one for the other produces values that are wrong for
/// every input longer than a bit.
#[must_use]
pub fn if1_crc(class_a_bits: &[u8]) -> u8 {
    let mut remainder = 0u8;
    for &bit in class_a_bits {
        let feedback = (remainder >> 7) ^ (bit & 1);
        remainder <<= 1;
        if feedback != 0 {
            remainder ^= 0x71;
        }
    }
    remainder
}

/// Class A bits of a frame type's Core Frame — the CRC's coverage.
///
/// For speech that is the mode's own class A prefix; for SID frames the
/// entire Core Frame is class A (26.101 / 26.201 Table 7).
const fn if1_class_a_bits(frame_type: AmrFrameType) -> usize {
    match frame_type {
        AmrFrameType::Speech(mode) => mode.class_a_bits(),
        AmrFrameType::Sid(_) => if2_core_bits(frame_type),
        AmrFrameType::NoData | AmrFrameType::SpeechLost => 0,
    }
}

/// Header plus auxiliary bits before the Core Frame.
///
/// Both variants spend three octets when auxiliary information is present;
/// the difference is how the fields divide them. FT 14/15 carry the frame
/// type alone (narrowband) or frame type plus FQI (wideband).
const fn if1_prefix_bits(variant: AmrVariant, frame_type: AmrFrameType) -> usize {
    match frame_type {
        AmrFrameType::NoData | AmrFrameType::SpeechLost => {
            if if2_has_fqi(variant) {
                5
            } else {
                4
            }
        }
        _ => 24,
    }
}

/// Octets an IF1 frame occupies.
#[must_use]
pub const fn if1_frame_len(variant: AmrVariant, frame_type: AmrFrameType) -> usize {
    (if1_prefix_bits(variant, frame_type) + if2_core_bits(frame_type)).div_ceil(8)
}

/// One IF1 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct If1Frame {
    /// What the frame carries.
    pub frame_type: AmrFrameType,
    /// The Frame Quality Indicator: `false` marks a bad or corrupted frame
    /// whose bits may still assist concealment.
    pub quality_ok: bool,
    /// Mode Indication: the mode this frame's sender is using, 0..=7
    /// (narrowband) or 0..=8 (wideband).
    pub mode_indication: u8,
    /// Mode Request: the mode this frame's sender asks its peer to use.
    pub mode_request: u8,
    /// Whether the codec CRC matched on unpack. A mismatch does not refuse
    /// the frame — 26.101 Table 1c maps it to `SPEECH_BAD`, whose bits may
    /// still assist concealment — but the caller must know. Always `true`
    /// for frames built locally.
    pub crc_ok: bool,
    /// Core Frame bits, one per element, in `d(0)..` order.
    pub bits: Vec<u8>,
}

/// MSB-first bit write, both variants: bit 0 is bit 8 of octet 1.
fn if1_set_bit(out: &mut [u8], index: usize, value: bool) {
    if value {
        out[index / 8] |= 0x80 >> (index % 8);
    }
}

fn if1_get_bit(data: &[u8], index: usize) -> bool {
    data[index / 8] & (0x80 >> (index % 8)) != 0
}

/// Pack one IF1 frame.
///
/// # Errors
///
/// When the Core Frame bits do not match the frame type's length, or the mode
/// fields are out of the variant's range.
pub fn if1_pack(variant: AmrVariant, frame: &If1Frame) -> Result<Vec<u8>> {
    let expected = if2_core_bits(frame.frame_type);
    if frame.bits.len() != expected {
        return Err(CodecError::invalid_format(format!(
            "IF1 {variant:?} frame needs {expected} core bits, got {}",
            frame.bits.len()
        )));
    }
    let top_mode = match variant {
        AmrVariant::NarrowBand => 7,
        AmrVariant::WideBand => 8,
    };
    if frame.mode_indication > top_mode || frame.mode_request > top_mode {
        return Err(CodecError::invalid_format(format!(
            "IF1 {variant:?} mode fields are 0..={top_mode}, got indication {} request {}",
            frame.mode_indication, frame.mode_request
        )));
    }

    let mut out = vec![0u8; if1_frame_len(variant, frame.frame_type)];
    let ft = if2_frame_type_index(frame.frame_type);

    // Frame type, MSB first, at bits 8..5 of octet 1 in both variants.
    for bit in 0..4 {
        if1_set_bit(&mut out, bit, ft & (0b1000 >> bit) != 0);
    }

    match frame.frame_type {
        AmrFrameType::NoData | AmrFrameType::SpeechLost => {
            // Narrowband stops after the frame type; wideband adds the FQI.
            if if2_has_fqi(variant) {
                if1_set_bit(&mut out, 4, frame.quality_ok);
            }
            return Ok(out);
        }
        _ => {}
    }

    if1_set_bit(&mut out, 4, frame.quality_ok);
    match variant {
        AmrVariant::NarrowBand => {
            // 26.101 Table 5: MI at bits 3..1 of octet 1, MR at bits 8..6 of
            // octet 2, five spare bits, then the CRC octet.
            for bit in 0..3 {
                if1_set_bit(&mut out, 5 + bit, frame.mode_indication & (0b100 >> bit) != 0);
            }
            for bit in 0..3 {
                if1_set_bit(&mut out, 8 + bit, frame.mode_request & (0b100 >> bit) != 0);
            }
        }
        AmrVariant::WideBand => {
            // 26.201 Table 5: three spare bits close octet 1, then MI at bits
            // 8..5 and MR at bits 4..1 of octet 2.
            for bit in 0..4 {
                if1_set_bit(&mut out, 8 + bit, frame.mode_indication & (0b1000 >> bit) != 0);
            }
            for bit in 0..4 {
                if1_set_bit(&mut out, 12 + bit, frame.mode_request & (0b1000 >> bit) != 0);
            }
        }
    }

    let class_a = if1_class_a_bits(frame.frame_type);
    out[2] = if1_crc(&frame.bits[..class_a]);

    for (offset, &bit) in frame.bits.iter().enumerate() {
        if1_set_bit(&mut out, 24 + offset, bit != 0);
    }
    Ok(out)
}

/// Unpack one IF1 frame, verifying its codec CRC.
///
/// # Errors
///
/// When the data is empty, its frame type is invalid for the variant, or it
/// is shorter than the frame type requires. A CRC mismatch is NOT an error —
/// the frame is returned with [`If1Frame::crc_ok`] false, because its bits
/// may still assist concealment (26.101 Table 1c).
pub fn if1_unpack(variant: AmrVariant, data: &[u8]) -> Result<If1Frame> {
    let Some(&first) = data.first() else {
        return Err(CodecError::invalid_format("IF1 frame is empty"));
    };
    let frame_type = AmrFrameType::from_index(variant, first >> 4)?;
    let expected_len = if1_frame_len(variant, frame_type);
    if data.len() < expected_len {
        return Err(CodecError::invalid_format(format!(
            "IF1 {variant:?} frame of type {frame_type:?} needs {expected_len} octets, got {}",
            data.len()
        )));
    }

    if matches!(
        frame_type,
        AmrFrameType::NoData | AmrFrameType::SpeechLost
    ) {
        return Ok(If1Frame {
            frame_type,
            quality_ok: !if2_has_fqi(variant) || if1_get_bit(data, 4),
            mode_indication: 0,
            mode_request: 0,
            crc_ok: true,
            bits: Vec::new(),
        });
    }

    let quality_ok = if1_get_bit(data, 4);
    let (mode_indication, mode_request) = match variant {
        AmrVariant::NarrowBand => (first & 0b0000_0111, data[1] >> 5),
        AmrVariant::WideBand => (data[1] >> 4, data[1] & 0x0F),
    };

    let core = if2_core_bits(frame_type);
    let mut bits = Vec::with_capacity(core);
    for offset in 0..core {
        bits.push(u8::from(if1_get_bit(data, 24 + offset)));
    }
    let class_a = if1_class_a_bits(frame_type);
    let crc_ok = if1_crc(&bits[..class_a]) == data[2];

    Ok(If1Frame {
        frame_type,
        quality_ok,
        mode_indication,
        mode_request,
        crc_ok,
        bits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::AmrMode;

    fn bits_of(len: usize) -> Vec<u8> {
        // Asymmetric on purpose: a reversed or shifted unpack must not
        // round-trip by luck.
        (0..len)
            .map(|index| u8::from(index % 3 == 0 || index % 7 == 1))
            .collect()
    }

    /// TS 26.101 Table A.1b and TS 26.201 Table A.1b, every row.
    ///
    /// These are the numbers the whole module is built on, and they are the
    /// ones an implementation gets wrong silently: a frame one octet short
    /// still carries a valid frame type, so a dissector still names the right
    /// mode.
    #[test]
    fn frame_lengths_match_the_specification_tables() {
        let nb = AmrVariant::NarrowBand;
        // 26.101 A.1b: 4 header bits, then the core frame.
        let nb_octets = [13usize, 14, 16, 18, 19, 21, 26, 31];
        for (index, want) in nb_octets.iter().enumerate() {
            let mode = AmrMode::new(nb, u8::try_from(index).expect("index")).expect("mode");
            assert_eq!(
                if2_frame_len(nb, AmrFrameType::Speech(mode)),
                *want,
                "narrowband mode {index}"
            );
        }
        assert_eq!(if2_frame_len(nb, AmrFrameType::Sid(nb)), 6, "narrowband SID");
        assert_eq!(if2_frame_len(nb, AmrFrameType::NoData), 1, "narrowband no-data");

        let wb = AmrVariant::WideBand;
        // 26.201 A.1b: 4 header bits plus the FQI, then the core frame.
        let wb_octets = [18usize, 23, 33, 37, 41, 47, 51, 59, 61];
        for (index, want) in wb_octets.iter().enumerate() {
            let mode = AmrMode::new(wb, u8::try_from(index).expect("index")).expect("mode");
            assert_eq!(
                if2_frame_len(wb, AmrFrameType::Speech(mode)),
                *want,
                "wideband mode {index}"
            );
        }
        assert_eq!(if2_frame_len(wb, AmrFrameType::Sid(wb)), 6, "wideband SID");
        assert_eq!(if2_frame_len(wb, AmrFrameType::NoData), 1, "wideband no-data");
    }

    /// TS 26.101 Table A.1a, the worked 6.70 kbit/s example.
    ///
    /// Octet 1 is `d(3) d(2) d(1) d(0) | FT(=3)` and octet 2 is
    /// `d(11) … d(4)`, so the frame type sits in the low nibble and the core
    /// bits walk upward from bit 1.
    #[test]
    fn narrowband_matches_the_worked_example_from_26_101() {
        let nb = AmrVariant::NarrowBand;
        let mode = AmrMode::new(nb, 3).expect("6.70 kbit/s");
        // d(0)=1, d(1)=0, d(2)=1, d(3)=1, then d(4)=1 and the rest zero.
        let mut bits = vec![0u8; 134];
        bits[0] = 1;
        bits[2] = 1;
        bits[3] = 1;
        bits[4] = 1;

        let packed = if2_pack(nb, AmrFrameType::Speech(mode), &bits, true).expect("packs");
        assert_eq!(packed.len(), 18, "26.101 A.1b gives 18 octets for 6.70");

        // Octet 1: low nibble is the frame type 3 = 0b0011; d(0) lands at bit
        // 5, d(2) at bit 7, d(3) at bit 8.
        assert_eq!(packed[0] & 0x0F, 3, "frame type in the low nibble");
        assert_eq!(packed[0] & 0b0001_0000, 0b0001_0000, "d(0) at bit 5");
        assert_eq!(packed[0] & 0b0010_0000, 0, "d(1) at bit 6 is zero");
        assert_eq!(packed[0] & 0b0100_0000, 0b0100_0000, "d(2) at bit 7");
        assert_eq!(packed[0] & 0b1000_0000, 0b1000_0000, "d(3) at bit 8");
        // Octet 2 starts at d(4), which is bit 1.
        assert_eq!(packed[1] & 0b0000_0001, 1, "d(4) at bit 1 of octet 2");
    }

    /// TS 26.201 Table A.1a, the worked 8.85 kbit/s example.
    ///
    /// Octet 1 is `FT(=1) | FQI | d(0) d(1) d(2)` and octet 2 is
    /// `d(3) … d(10)`, so the frame type sits in the high nibble and the core
    /// bits walk downward from bit 8.
    #[test]
    fn wideband_matches_the_worked_example_from_26_201() {
        let wb = AmrVariant::WideBand;
        let mode = AmrMode::new(wb, 1).expect("8.85 kbit/s");
        let mut bits = vec![0u8; 177];
        bits[0] = 1; // d(0)
        bits[2] = 1; // d(2)
        bits[3] = 1; // d(3)

        let packed = if2_pack(wb, AmrFrameType::Speech(mode), &bits, true).expect("packs");
        assert_eq!(packed.len(), 23, "26.201 A.1b gives 23 octets for 8.85");

        assert_eq!(packed[0] >> 4, 1, "frame type in the high nibble");
        assert_eq!(packed[0] & 0b0000_1000, 0b0000_1000, "FQI at bit 4");
        assert_eq!(packed[0] & 0b0000_0100, 0b0000_0100, "d(0) at bit 3");
        assert_eq!(packed[0] & 0b0000_0010, 0, "d(1) at bit 2 is zero");
        assert_eq!(packed[0] & 0b0000_0001, 1, "d(2) at bit 1");
        assert_eq!(packed[1] & 0b1000_0000, 0b1000_0000, "d(3) at bit 8 of octet 2");
    }

    /// Wideband's Frame Quality Indicator survives the round trip; narrowband
    /// has no such field and always reports a good frame.
    #[test]
    fn the_quality_indicator_is_wideband_only() {
        let wb = AmrVariant::WideBand;
        let mode = AmrMode::new(wb, 0).expect("6.60");
        let bits = bits_of(132);
        for quality in [true, false] {
            let packed = if2_pack(wb, AmrFrameType::Speech(mode), &bits, quality).expect("packs");
            let frame = if2_unpack(wb, &packed).expect("unpacks");
            assert_eq!(frame.quality_ok, quality, "wideband FQI must round-trip");
        }

        let nb = AmrVariant::NarrowBand;
        let mode = AmrMode::new(nb, 0).expect("4.75");
        let packed = if2_pack(nb, AmrFrameType::Speech(mode), &bits_of(95), false).expect("packs");
        let frame = if2_unpack(nb, &packed).expect("unpacks");
        assert!(
            frame.quality_ok,
            "narrowband has no FQI, so it cannot report a bad frame"
        );
    }

    #[test]
    fn every_mode_of_both_variants_round_trips() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for mode in AmrMode::all(variant) {
                let bits = bits_of(mode.bits());
                let packed =
                    if2_pack(variant, AmrFrameType::Speech(mode), &bits, true).expect("packs");
                let frame = if2_unpack(variant, &packed).expect("unpacks");
                assert_eq!(frame.frame_type, AmrFrameType::Speech(mode));
                assert_eq!(frame.bits, bits, "{variant:?} {mode:?} did not survive IF2");
            }
            // SID and no-data too: their lengths differ per variant.
            let sid = AmrFrameType::Sid(variant);
            let packed = if2_pack(variant, sid, &bits_of(if2_core_bits(sid)), true).expect("packs");
            assert_eq!(if2_unpack(variant, &packed).expect("unpacks").frame_type, sid);

            let packed = if2_pack(variant, AmrFrameType::NoData, &[], true).expect("packs");
            assert_eq!(packed.len(), 1, "a no-data frame is its header alone");
            assert_eq!(
                if2_unpack(variant, &packed).expect("unpacks").frame_type,
                AmrFrameType::NoData
            );
        }
    }

    #[test]
    fn a_wrong_bit_count_is_refused_rather_than_padded() {
        let wb = AmrVariant::WideBand;
        let mode = AmrMode::new(wb, 8).expect("23.85");
        assert!(if2_pack(wb, AmrFrameType::Speech(mode), &[1u8; 10], true).is_err());
        assert!(if2_pack(wb, AmrFrameType::Speech(mode), &[1u8; 500], true).is_err());
    }

    #[test]
    fn a_truncated_frame_is_refused_rather_than_read_past_its_end() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            let mode = AmrMode::new(variant, 0).expect("lowest mode");
            let packed = if2_pack(variant, AmrFrameType::Speech(mode), &bits_of(mode.bits()), true)
                .expect("packs");
            for length in 0..packed.len() {
                assert!(
                    if2_unpack(variant, &packed[..length]).is_err(),
                    "{variant:?}: a {length}-octet prefix must not parse as a whole frame"
                );
            }
        }
    }

    /// The IF1 codec CRC against independently hand-derived vectors.
    ///
    /// `G(x) = x^8 + x^6 + x^5 + x^4 + 1` (feedback 0x71), zero preset, no
    /// inversion — the specs state plain division and nothing else. Derived
    /// by working the LFSR by hand: a single 1 bit leaves exactly the
    /// feedback mask; eight shifts of `1 0000000` reduce x^8 once, giving
    /// 0x71 << 1 xor 0x71-fold-ins = 0xc1.
    #[test]
    fn the_codec_crc_matches_hand_worked_vectors() {
        assert_eq!(if1_crc(&[]), 0x00);
        assert_eq!(if1_crc(&[1]), 0x71);
        assert_eq!(if1_crc(&[1, 0, 0, 0, 0, 0, 0, 0]), 0xc1);
        assert_eq!(if1_crc(&[1; 8]), 0x7e);
        let alternating: Vec<u8> = (0..39).map(|i| u8::from(i % 2 == 1)).collect();
        assert_eq!(if1_crc(&alternating), 0x9b, "the 39-bit NB SID shape");
        assert_eq!(if1_crc(&[1; 54]), 0xe8, "the 54-bit WB 6.60 class A shape");
    }

    /// It is NOT the RFC 4867 payload CRC, whose polynomial is this one's
    /// bit-reversal. One shared vector proves they diverge.
    #[test]
    fn the_codec_crc_is_not_the_rfc_4867_payload_crc() {
        // payload.rs's hand-worked vector: frame_crc(&[0x80], 1) == 0xB8 —
        // a single 1 bit through the RFC's polynomial. Through IF1's it is
        // the IF1 feedback mask instead.
        assert_eq!(if1_crc(&[1]), 0x71);
        assert_ne!(if1_crc(&[1]), 0xB8);
    }

    /// TS 26.101 Table 5, bit for bit: AMR 6.7, good frame, Mode Request 1.
    ///
    /// Octet 1 is `FT(=3) FQI MI(=3)` = 0011 1 011, octet 2 is
    /// `MR(=1) spare(5)` = 001 00000.
    #[test]
    fn narrowband_if1_matches_the_worked_example_from_26_101() {
        let nb = AmrVariant::NarrowBand;
        let mode = AmrMode::new(nb, 3).expect("6.70 kbit/s");
        let frame = If1Frame {
            frame_type: AmrFrameType::Speech(mode),
            quality_ok: true,
            mode_indication: 3,
            mode_request: 1,
            crc_ok: true,
            bits: bits_of(134),
        };
        let packed = if1_pack(nb, &frame).expect("packs");
        assert_eq!(packed[0], 0b0011_1011, "octet 1: FT=3, FQI=1, MI=3");
        assert_eq!(packed[1], 0b0010_0000, "octet 2: MR=1, five spare zeros");
        assert_eq!(
            packed[2],
            if1_crc(&frame.bits[..mode.class_a_bits()]),
            "octet 3 is the CRC over the class A bits"
        );
        // d(0) at bit 8 of octet 4.
        assert_eq!(packed[3] & 0x80 != 0, frame.bits[0] != 0);
        // Total length from Table 7: 19 header/aux bits + 134 core = 153
        // bits -> 20 octets.
        assert_eq!(packed.len(), 20);
    }

    /// TS 26.201 Table 5, bit for bit: AMR-WB 12.65, good frame,
    /// Mode Indication 3, Mode Request 1.
    ///
    /// 12.65 kbit/s is frame type **2** — Table 1a, Table 7 and Annex A's
    /// Table A.1b all agree, and the example's own core runs d(0)..d(252),
    /// which is 12.65's 253 bits. (A flattened text extraction of the docx
    /// interleaves the example's "Mode Indication (=3)" cell into the frame
    /// type row, which is worth knowing before "checking" against it.)
    /// Octet 1 is `FT(=2) FQI spare(3)` = 0010 1 000, octet 2 is
    /// `MI(=3) MR(=1)` = 0011 0001.
    #[test]
    fn wideband_if1_matches_the_worked_example_from_26_201() {
        let wb = AmrVariant::WideBand;
        let mode = AmrMode::new(wb, 2).expect("12.65 kbit/s");
        let frame = If1Frame {
            frame_type: AmrFrameType::Speech(mode),
            quality_ok: true,
            mode_indication: 3,
            mode_request: 1,
            crc_ok: true,
            bits: bits_of(253),
        };
        let packed = if1_pack(wb, &frame).expect("packs");
        assert_eq!(packed[0], 0b0010_1000, "octet 1: FT=2, FQI=1, spare");
        assert_eq!(packed[1], 0b0011_0001, "octet 2: MI=3, MR=1");
        assert_eq!(packed[2], if1_crc(&frame.bits[..mode.class_a_bits()]));
        assert_eq!(packed[3] & 0x80 != 0, frame.bits[0] != 0);
        // Table 7: 21 header/aux bits + 253 core = 274 bits -> 35 octets.
        assert_eq!(packed.len(), 35);
    }

    /// Every mode of both variants round-trips with its mode fields and FQI.
    #[test]
    fn every_if1_mode_round_trips() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            let top = match variant {
                AmrVariant::NarrowBand => 7,
                AmrVariant::WideBand => 8,
            };
            for mode in AmrMode::all(variant) {
                let frame = If1Frame {
                    frame_type: AmrFrameType::Speech(mode),
                    quality_ok: mode.index() % 2 == 0,
                    mode_indication: mode.index(),
                    mode_request: top - mode.index(),
                    crc_ok: true,
                    bits: bits_of(mode.bits()),
                };
                let packed = if1_pack(variant, &frame).expect("packs");
                let out = if1_unpack(variant, &packed).expect("unpacks");
                assert_eq!(out, frame, "{variant:?} {mode:?} did not survive IF1");
                assert!(out.crc_ok, "a locally built frame must verify");
            }
        }
    }

    /// A flipped class A bit is detected; a flipped class B bit is not, which
    /// is the CRC doing what the spec sized it for rather than a gap.
    #[test]
    fn the_crc_covers_class_a_and_only_class_a() {
        let nb = AmrVariant::NarrowBand;
        let mode = AmrMode::new(nb, 7).expect("12.2");
        let frame = If1Frame {
            frame_type: AmrFrameType::Speech(mode),
            quality_ok: true,
            mode_indication: 7,
            mode_request: 7,
            crc_ok: true,
            bits: bits_of(mode.bits()),
        };
        let packed = if1_pack(nb, &frame).expect("packs");

        // Flip d(0): bit 8 of octet 4.
        let mut corrupted = packed.clone();
        corrupted[3] ^= 0x80;
        let out = if1_unpack(nb, &corrupted).expect("still parses");
        assert!(!out.crc_ok, "a class A flip must fail the CRC");

        // Flip the frame's last bit, deep in class B.
        let mut corrupted = packed;
        let last = 24 + mode.bits() - 1;
        corrupted[last / 8] ^= 0x80 >> (last % 8);
        let out = if1_unpack(nb, &corrupted).expect("parses");
        assert!(
            out.crc_ok,
            "class B is outside the CRC's coverage by specification"
        );
    }

    /// FT 14/15 are header-only, and the two variants disagree on whether
    /// that header includes the FQI.
    #[test]
    fn no_data_frames_are_header_only() {
        let nb_packed = if1_pack(
            AmrVariant::NarrowBand,
            &If1Frame {
                frame_type: AmrFrameType::NoData,
                quality_ok: false,
                mode_indication: 0,
                mode_request: 0,
                crc_ok: true,
                bits: Vec::new(),
            },
        )
        .expect("packs");
        assert_eq!(nb_packed.len(), 1);
        assert_eq!(nb_packed[0] >> 4, 15);
        let out = if1_unpack(AmrVariant::NarrowBand, &nb_packed).expect("unpacks");
        assert!(
            out.quality_ok,
            "narrowband FT15 has no FQI (26.101 Table 7), so quality cannot be reported"
        );

        let wb_packed = if1_pack(
            AmrVariant::WideBand,
            &If1Frame {
                frame_type: AmrFrameType::SpeechLost,
                quality_ok: false,
                mode_indication: 0,
                mode_request: 0,
                crc_ok: true,
                bits: Vec::new(),
            },
        )
        .expect("packs");
        assert_eq!(wb_packed.len(), 1);
        assert_eq!(wb_packed[0] >> 4, 14);
        let out = if1_unpack(AmrVariant::WideBand, &wb_packed).expect("unpacks");
        assert!(
            !out.quality_ok,
            "wideband FT14 carries the FQI and it must survive"
        );
    }

    /// Out-of-range mode fields and short data are refused.
    #[test]
    fn if1_refuses_bad_inputs() {
        let nb = AmrVariant::NarrowBand;
        let mode = AmrMode::new(nb, 0).expect("4.75");
        let mut frame = If1Frame {
            frame_type: AmrFrameType::Speech(mode),
            quality_ok: true,
            mode_indication: 8, // narrowband tops out at 7
            mode_request: 0,
            crc_ok: true,
            bits: bits_of(95),
        };
        assert!(if1_pack(nb, &frame).is_err());
        frame.mode_indication = 0;
        frame.bits = bits_of(94);
        assert!(if1_pack(nb, &frame).is_err());

        frame.bits = bits_of(95);
        let packed = if1_pack(nb, &frame).expect("packs");
        for length in 0..packed.len() {
            assert!(if1_unpack(nb, &packed[..length]).is_err());
        }
    }

    #[test]
    fn stuffing_bits_are_zero() {
        // The specs stuff to the octet boundary with unused bits; leaving them
        // uninitialised would make frames differ byte for byte between runs
        // while decoding identically.
        let nb = AmrVariant::NarrowBand;
        let mode = AmrMode::new(nb, 0).expect("4.75");
        // 4 + 95 = 99 bits, so the last five bits of octet 13 are stuffing,
        // and narrowband fills from bit 1 upward.
        let packed = if2_pack(nb, AmrFrameType::Speech(mode), &[1u8; 95], true).expect("packs");
        assert_eq!(packed.len(), 13);
        assert_eq!(packed[12] & 0b1111_1000, 0, "narrowband stuffing must be zero");

        let wb = AmrVariant::WideBand;
        let mode = AmrMode::new(wb, 1).expect("8.85");
        // 5 + 177 = 182 bits, so the last two bits of octet 23 are stuffing,
        // and wideband fills from bit 8 downward.
        let packed = if2_pack(wb, AmrFrameType::Speech(mode), &[1u8; 177], true).expect("packs");
        assert_eq!(packed.len(), 23);
        assert_eq!(packed[22] & 0b0000_0011, 0, "wideband stuffing must be zero");
    }
}
