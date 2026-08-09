//! AMR-NB and AMR-WB codec modes and frame types.
//!
//! Spec provenance: frame sizes and class A bit counts are taken from
//! RFC 4867 §3.6 (AMR) and §3.7 (AMR-WB); bit rates from 3GPP TS 26.071
//! (AMR-NB) and TS 26.171 (AMR-WB).
//!
//! Note that several secondary sources swap the AMR-NB 6.70 and 7.40 frame
//! sizes. RFC 4867 is authoritative: 6.70 carries 134 bits, 7.40 carries 148.

use crate::error::{CodecError, Result};
use std::fmt;

/// Which AMR codec family a mode belongs to.
///
/// The two families share a payload format and a frame duration but are always
/// distinct RTP payload types, and their mode indices mean different things: NB
/// mode 0 is 4.75 kbit/s while WB mode 0 is 6.60 kbit/s. [`AmrMode`] carries the
/// variant so the two cannot be confused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AmrVariant {
    /// AMR narrowband: 8 kHz, 160 samples per 20 ms frame, 8 speech modes.
    NarrowBand,
    /// AMR wideband (also ITU-T G.722.2): 16 kHz, 320 samples per 20 ms frame,
    /// 9 speech modes.
    WideBand,
}

impl AmrVariant {
    /// Number of speech modes: 8 for narrowband, 9 for wideband.
    #[must_use]
    pub const fn speech_mode_count(self) -> u8 {
        match self {
            Self::NarrowBand => 8,
            Self::WideBand => 9,
        }
    }

    /// Sample rate in Hz.
    #[must_use]
    pub const fn sample_rate(self) -> u32 {
        match self {
            Self::NarrowBand => 8_000,
            Self::WideBand => 16_000,
        }
    }

    /// PCM samples in one 20 ms frame.
    #[must_use]
    pub const fn frame_samples(self) -> usize {
        match self {
            Self::NarrowBand => 160,
            Self::WideBand => 320,
        }
    }

    /// RTP clock rate in Hz. Equal to the sample rate for both variants.
    #[must_use]
    pub const fn clock_rate(self) -> u32 {
        self.sample_rate()
    }

    /// Frame type index carrying comfort noise (SID) for this variant.
    ///
    /// RFC 4867 assigns FT 8 for AMR-NB and FT 9 for AMR-WB.
    #[must_use]
    pub const fn sid_frame_type(self) -> u8 {
        match self {
            Self::NarrowBand => 8,
            Self::WideBand => 9,
        }
    }

    /// SID payload size in bits: 39 for narrowband, 40 for wideband.
    ///
    /// Every SID bit is class A. RFC 4867 lists 39 of 39 for narrowband, and
    /// TS 26.201 states the wideband comfort-noise bits "are all mapped to
    /// Class A".
    #[must_use]
    pub const fn sid_bits(self) -> usize {
        match self {
            Self::NarrowBand => 39,
            Self::WideBand => 40,
        }
    }

    /// SDP encoding name (`a=rtpmap`) for this variant.
    #[must_use]
    pub const fn sdp_name(self) -> &'static str {
        match self {
            Self::NarrowBand => "AMR",
            Self::WideBand => "AMR-WB",
        }
    }

    /// Magic number prefixing a single-channel AMR storage file (RFC 4867 §5.1).
    #[must_use]
    pub const fn storage_magic(self) -> &'static [u8] {
        match self {
            Self::NarrowBand => b"#!AMR\n",
            Self::WideBand => b"#!AMR-WB\n",
        }
    }
}

impl fmt::Display for AmrVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.sdp_name())
    }
}

/// AMR-NB speech frame sizes in bits, indexed by mode. RFC 4867 §3.6.
const NB_BITS: [usize; 8] = [95, 103, 118, 134, 148, 159, 204, 244];

/// AMR-NB class A bit counts, indexed by mode. RFC 4867 §3.6.
///
/// Class A bits are the perceptually critical ones; they are what the optional
/// payload CRC covers and what unequal error protection prioritises. They are
/// the frame's leading bits — see [`AmrMode::class_a_bits`].
const NB_CLASS_A: [usize; 8] = [42, 49, 55, 58, 61, 75, 65, 81];

/// AMR-NB bit rates in bit/s, indexed by mode. 3GPP TS 26.071.
const NB_BITRATE: [u32; 8] = [4_750, 5_150, 5_900, 6_700, 7_400, 7_950, 10_200, 12_200];

/// AMR-WB speech frame sizes in bits, indexed by mode. RFC 4867 §3.7.
const WB_BITS: [usize; 9] = [132, 177, 253, 285, 317, 365, 397, 461, 477];

/// AMR-WB class A bit counts, indexed by mode. 3GPP TS 26.201 Table 2.
///
/// RFC 4867 tabulates the narrowband counts but defers the wideband ones to
/// TS 26.201, so these come from that table directly. Cross-check: class
/// A + B + C equals the frame total in [`WB_BITS`] for every mode.
const WB_CLASS_A: [usize; 9] = [54, 64, 72, 72, 72, 72, 72, 72, 72];

/// AMR-WB bit rates in bit/s, indexed by mode. 3GPP TS 26.171.
const WB_BITRATE: [u32; 9] = [
    6_600, 8_850, 12_650, 14_250, 15_850, 18_250, 19_850, 23_050, 23_850,
];

/// A validated AMR speech coding mode.
///
/// Construction is checked against the variant's mode count, so an out-of-range
/// index or an NB index applied to a WB stream cannot be represented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AmrMode {
    variant: AmrVariant,
    index: u8,
}

impl AmrMode {
    /// Create a mode from a variant and mode index.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::InvalidConfig`] when `index` is not a speech mode
    /// for `variant` — that is, not 0–7 for narrowband or 0–8 for wideband.
    /// Comfort-noise and no-data frames are represented by [`AmrFrameType`], not
    /// by this type.
    pub fn new(variant: AmrVariant, index: u8) -> Result<Self> {
        if index >= variant.speech_mode_count() {
            return Err(CodecError::invalid_config(format!(
                "{variant} mode {index} out of range (valid: 0-{})",
                variant.speech_mode_count() - 1
            )));
        }
        Ok(Self { variant, index })
    }

    /// The variant this mode belongs to.
    #[must_use]
    pub const fn variant(self) -> AmrVariant {
        self.variant
    }

    /// The mode index, as used in the RFC 4867 FT and CMR fields.
    #[must_use]
    pub const fn index(self) -> u8 {
        self.index
    }

    /// Encoded speech frame size in bits, excluding any payload headers.
    #[must_use]
    pub const fn bits(self) -> usize {
        match self.variant {
            AmrVariant::NarrowBand => NB_BITS[self.index as usize],
            AmrVariant::WideBand => WB_BITS[self.index as usize],
        }
    }

    /// Encoded speech frame size in bytes when octet-aligned, i.e. `bits`
    /// rounded up to a whole number of octets.
    #[must_use]
    pub const fn octet_aligned_bytes(self) -> usize {
        self.bits().div_ceil(8)
    }

    /// Number of class A bits in this mode.
    ///
    /// Class A bits are the error-sensitive ones the optional payload CRC
    /// covers. Crucially they are the frame's *leading* bits: TS 26.201 states
    /// that for AMR-WB mode 6.60 "the Class A bits are d(0)..d(53)", and the
    /// narrowband frame structure is ordered the same way. So a CRC over the
    /// first `class_a_bits()` bits of the payload is the CRC the spec means.
    #[must_use]
    pub const fn class_a_bits(self) -> usize {
        match self.variant {
            AmrVariant::NarrowBand => NB_CLASS_A[self.index as usize],
            AmrVariant::WideBand => WB_CLASS_A[self.index as usize],
        }
    }

    /// Nominal bit rate in bit/s.
    #[must_use]
    pub const fn bitrate(self) -> u32 {
        match self.variant {
            AmrVariant::NarrowBand => NB_BITRATE[self.index as usize],
            AmrVariant::WideBand => WB_BITRATE[self.index as usize],
        }
    }

    /// Every speech mode of a variant, in ascending bit-rate order.
    #[must_use]
    pub fn all(variant: AmrVariant) -> Vec<Self> {
        (0..variant.speech_mode_count())
            .map(|index| Self { variant, index })
            .collect()
    }
}

impl fmt::Display for AmrMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // e.g. "AMR-WB 12.65 kbit/s"
        let kbits = f64::from(self.bitrate()) / 1000.0;
        write!(f, "{} {kbits:.2} kbit/s", self.variant)
    }
}

/// The kind of frame carried in one 20 ms slot, mirroring the RFC 4867 FT field.
///
/// This is what [`crate::types::CodedFrame`] carries, and it is why AMR cannot
/// use output length alone to signal frame type the way G.729 does: an empty
/// payload is ambiguous between "not transmitted" and "lost".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AmrFrameType {
    /// A speech frame coded in the given mode.
    Speech(AmrMode),
    /// A comfort-noise (SID) frame. FT 8 for narrowband, FT 9 for wideband.
    Sid(AmrVariant),
    /// No data transmitted for this frame — FT 15. Distinct from a lost frame:
    /// the sender deliberately sent nothing, typically during DTX.
    NoData,
    /// The frame was lost in transit — FT 14. AMR-WB only; RFC 4867 does not
    /// define `SPEECH_LOST` for narrowband.
    SpeechLost,
}

impl AmrFrameType {
    /// The RFC 4867 frame type index for this frame.
    #[must_use]
    pub const fn frame_type_index(self) -> u8 {
        match self {
            Self::Speech(mode) => mode.index(),
            Self::Sid(variant) => variant.sid_frame_type(),
            Self::SpeechLost => 14,
            Self::NoData => 15,
        }
    }

    /// Payload size in bits, excluding payload headers. Zero for `NoData` and
    /// `SpeechLost`, which carry a table-of-contents entry but no speech bits.
    #[must_use]
    pub const fn bits(self) -> usize {
        match self {
            Self::Speech(mode) => mode.bits(),
            Self::Sid(variant) => variant.sid_bits(),
            Self::NoData | Self::SpeechLost => 0,
        }
    }

    /// Payload size in bytes when octet-aligned.
    #[must_use]
    pub const fn octet_aligned_bytes(self) -> usize {
        self.bits().div_ceil(8)
    }

    /// Decode a frame type index for a given variant.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::InvalidPayload`] for indices RFC 4867 reserves —
    /// 9–14 for narrowband and 10–13 for wideband. The RFC directs receivers to
    /// discard the whole packet in that case, so this is a hard error rather
    /// than a frame-level one.
    pub fn from_index(variant: AmrVariant, index: u8) -> Result<Self> {
        if index < variant.speech_mode_count() {
            return Ok(Self::Speech(AmrMode { variant, index }));
        }
        if index == variant.sid_frame_type() {
            return Ok(Self::Sid(variant));
        }
        if index == 15 {
            return Ok(Self::NoData);
        }
        if index == 14 && variant == AmrVariant::WideBand {
            return Ok(Self::SpeechLost);
        }
        Err(CodecError::InvalidPayload {
            details: format!("frame type {index} is reserved for {variant}; discard the packet"),
        })
    }
}

/// The set of modes a stream is permitted to use, per the SDP `mode-set`
/// parameter (RFC 4867 §8.1).
///
/// `mode-set` is a hard restriction, not a preference list: a sender must not
/// use a mode outside the negotiated set, and an empty intersection means the
/// payload type has to be rejected rather than defaulted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmrModeSet {
    variant: AmrVariant,
    /// Bit `n` set means mode `n` is permitted.
    mask: u16,
}

impl AmrModeSet {
    /// The set containing every mode of a variant, which is the RFC 4867 default
    /// when `mode-set` is absent from the SDP.
    #[must_use]
    pub const fn all(variant: AmrVariant) -> Self {
        // speech_mode_count() is 8 or 9, so the shift is always in range.
        let mask = (1u16 << variant.speech_mode_count()) - 1;
        Self { variant, mask }
    }

    /// Build a set from explicit mode indices.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::InvalidConfig`] when any index is out of range for
    /// the variant, or when `modes` is empty — an empty mode set is not
    /// expressible in SDP and would leave the encoder with nothing to send.
    pub fn from_indices(variant: AmrVariant, modes: &[u8]) -> Result<Self> {
        if modes.is_empty() {
            return Err(CodecError::invalid_config(
                "AMR mode-set must contain at least one mode",
            ));
        }
        let mut mask = 0u16;
        for &index in modes {
            // Validate through AmrMode so the range rule lives in one place.
            let mode = AmrMode::new(variant, index)?;
            mask |= 1u16 << mode.index();
        }
        Ok(Self { variant, mask })
    }

    /// The variant this set applies to.
    #[must_use]
    pub const fn variant(&self) -> AmrVariant {
        self.variant
    }

    /// Whether a mode is permitted. Modes of the other variant never are.
    #[must_use]
    pub const fn contains(&self, mode: AmrMode) -> bool {
        // `matches!` keeps this const-compatible; AmrVariant has no PartialEq in
        // const context.
        match (self.variant, mode.variant()) {
            (AmrVariant::NarrowBand, AmrVariant::NarrowBand)
            | (AmrVariant::WideBand, AmrVariant::WideBand) => {
                self.mask & (1u16 << mode.index()) != 0
            }
            _ => false,
        }
    }

    /// The permitted modes, in ascending bit-rate order.
    #[must_use]
    pub fn modes(&self) -> Vec<AmrMode> {
        AmrMode::all(self.variant)
            .into_iter()
            .filter(|&mode| self.contains(mode))
            .collect()
    }

    /// The highest permitted mode. Never `None`, since the set is non-empty by
    /// construction, but returned as an `Option` to avoid a panic path.
    #[must_use]
    pub fn highest(&self) -> Option<AmrMode> {
        self.modes().last().copied()
    }

    /// The lowest permitted mode.
    #[must_use]
    pub fn lowest(&self) -> Option<AmrMode> {
        self.modes().first().copied()
    }

    /// Intersect two mode sets, as offer/answer negotiation requires.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::InvalidConfig`] when the variants differ, or when
    /// the intersection is empty — the latter is the case RFC 4867 says must
    /// lead to rejecting the payload type, so it is surfaced as an error rather
    /// than an empty set.
    pub fn intersect(&self, other: &Self) -> Result<Self> {
        if self.variant != other.variant {
            return Err(CodecError::invalid_config(
                "cannot intersect AMR mode sets of different variants",
            ));
        }
        let mask = self.mask & other.mask;
        if mask == 0 {
            return Err(CodecError::invalid_config(
                "AMR mode-set intersection is empty; the payload type must be rejected",
            ));
        }
        Ok(Self {
            variant: self.variant,
            mask,
        })
    }

    /// Render as the value of an SDP `mode-set` parameter, e.g. `0,2,4`.
    #[must_use]
    pub fn to_sdp_value(&self) -> String {
        self.modes()
            .iter()
            .map(|mode| mode.index().to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nb_frame_sizes_match_rfc4867() {
        // RFC 4867 §3.6 table, including the 6.70/7.40 pair that several
        // secondary sources transpose.
        let expected = [
            (0, 4_750, 95, 42, 12),
            (1, 5_150, 103, 49, 13),
            (2, 5_900, 118, 55, 15),
            (3, 6_700, 134, 58, 17),
            (4, 7_400, 148, 61, 19),
            (5, 7_950, 159, 75, 20),
            (6, 10_200, 204, 65, 26),
            (7, 12_200, 244, 81, 31),
        ];
        for (index, bitrate, bits, class_a, bytes) in expected {
            let mode = AmrMode::new(AmrVariant::NarrowBand, index).unwrap();
            assert_eq!(mode.bitrate(), bitrate, "mode {index} bitrate");
            assert_eq!(mode.bits(), bits, "mode {index} bits");
            assert_eq!(mode.class_a_bits(), class_a, "mode {index} class A");
            assert_eq!(mode.octet_aligned_bytes(), bytes, "mode {index} bytes");
        }
    }

    #[test]
    fn wb_frame_sizes_match_rfc4867() {
        // Frame sizes from RFC 4867 §3.7; class A counts from TS 26.201 Table 2.
        let expected = [
            (0, 6_600, 132, 54, 17),
            (1, 8_850, 177, 64, 23),
            (2, 12_650, 253, 72, 32),
            (3, 14_250, 285, 72, 36),
            (4, 15_850, 317, 72, 40),
            (5, 18_250, 365, 72, 46),
            (6, 19_850, 397, 72, 50),
            (7, 23_050, 461, 72, 58),
            (8, 23_850, 477, 72, 60),
        ];
        for (index, bitrate, bits, class_a, bytes) in expected {
            let mode = AmrMode::new(AmrVariant::WideBand, index).unwrap();
            assert_eq!(mode.bitrate(), bitrate, "mode {index} bitrate");
            assert_eq!(mode.bits(), bits, "mode {index} bits");
            assert_eq!(mode.class_a_bits(), class_a, "mode {index} class A");
            assert_eq!(mode.octet_aligned_bytes(), bytes, "mode {index} bytes");
        }
    }

    #[test]
    fn class_a_bits_never_exceed_the_frame() {
        // TS 26.201 Table 2 gives class A + B + C = total. We only carry the
        // class A column, so the invariant we can assert is that it fits.
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for mode in AmrMode::all(variant) {
                assert!(
                    mode.class_a_bits() <= mode.bits(),
                    "{mode}: {} class A bits in a {}-bit frame",
                    mode.class_a_bits(),
                    mode.bits()
                );
            }
        }
    }

    #[test]
    fn wb_mode_8_adds_exactly_one_high_band_gain_per_subframe() {
        // 23.85 kbit/s is 23.05 plus 4 bits of high-band gain in each of the
        // four subframes. If this ever fails, one of the two entries is wrong.
        let m7 = AmrMode::new(AmrVariant::WideBand, 7).unwrap();
        let m8 = AmrMode::new(AmrVariant::WideBand, 8).unwrap();
        assert_eq!(m8.bits() - m7.bits(), 16);
    }

    #[test]
    fn frame_bits_are_consistent_with_bitrate() {
        // Every mode's bit count must equal bitrate * 20 ms.
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for mode in AmrMode::all(variant) {
                let expected = (mode.bitrate() as usize) / 50;
                assert_eq!(
                    mode.bits(),
                    expected,
                    "{mode} carries {} bits but its rate implies {expected}",
                    mode.bits()
                );
            }
        }
    }

    #[test]
    fn mode_index_is_range_checked_per_variant() {
        assert!(AmrMode::new(AmrVariant::NarrowBand, 7).is_ok());
        assert!(AmrMode::new(AmrVariant::NarrowBand, 8).is_err());
        assert!(AmrMode::new(AmrVariant::WideBand, 8).is_ok());
        assert!(AmrMode::new(AmrVariant::WideBand, 9).is_err());
    }

    #[test]
    fn frame_type_indices_follow_rfc4867() {
        let nb = AmrVariant::NarrowBand;
        let wb = AmrVariant::WideBand;

        assert_eq!(AmrFrameType::Sid(nb).frame_type_index(), 8);
        assert_eq!(AmrFrameType::Sid(wb).frame_type_index(), 9);
        assert_eq!(AmrFrameType::NoData.frame_type_index(), 15);
        assert_eq!(AmrFrameType::SpeechLost.frame_type_index(), 14);

        assert_eq!(AmrFrameType::Sid(nb).bits(), 39);
        assert_eq!(AmrFrameType::Sid(wb).bits(), 40);
        assert_eq!(AmrFrameType::NoData.bits(), 0);
    }

    #[test]
    fn reserved_frame_types_are_rejected_per_variant() {
        // NB reserves 9-14; WB reserves 10-13 and uses 14 for SPEECH_LOST.
        for index in 9..=14u8 {
            assert!(
                AmrFrameType::from_index(AmrVariant::NarrowBand, index).is_err(),
                "NB FT {index} should be reserved"
            );
        }
        for index in 10..=13u8 {
            assert!(
                AmrFrameType::from_index(AmrVariant::WideBand, index).is_err(),
                "WB FT {index} should be reserved"
            );
        }
        assert_eq!(
            AmrFrameType::from_index(AmrVariant::WideBand, 14).unwrap(),
            AmrFrameType::SpeechLost
        );
        // SPEECH_LOST is wideband-only.
        assert!(AmrFrameType::from_index(AmrVariant::NarrowBand, 14).is_err());
    }

    #[test]
    fn frame_type_round_trips_through_its_index() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            let mut types = vec![AmrFrameType::Sid(variant), AmrFrameType::NoData];
            types.extend(AmrMode::all(variant).into_iter().map(AmrFrameType::Speech));
            if variant == AmrVariant::WideBand {
                types.push(AmrFrameType::SpeechLost);
            }
            for frame_type in types {
                let index = frame_type.frame_type_index();
                assert_eq!(
                    AmrFrameType::from_index(variant, index).unwrap(),
                    frame_type,
                    "{variant} FT {index} did not round-trip"
                );
            }
        }
    }

    #[test]
    fn default_mode_set_contains_every_mode() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            let set = AmrModeSet::all(variant);
            assert_eq!(set.modes().len(), variant.speech_mode_count() as usize);
            for mode in AmrMode::all(variant) {
                assert!(set.contains(mode));
            }
        }
    }

    #[test]
    fn mode_set_never_matches_the_other_variant() {
        let nb_set = AmrModeSet::all(AmrVariant::NarrowBand);
        let wb_mode_0 = AmrMode::new(AmrVariant::WideBand, 0).unwrap();
        assert!(!nb_set.contains(wb_mode_0));
    }

    #[test]
    fn mode_set_intersection_follows_offer_answer_rules() {
        let variant = AmrVariant::WideBand;
        let offer = AmrModeSet::from_indices(variant, &[0, 1, 2, 3]).unwrap();
        let answer = AmrModeSet::from_indices(variant, &[2, 3, 4, 5]).unwrap();

        let negotiated = offer.intersect(&answer).unwrap();
        assert_eq!(negotiated.to_sdp_value(), "2,3");
        assert_eq!(negotiated.highest().unwrap().index(), 3);
        assert_eq!(negotiated.lowest().unwrap().index(), 2);

        // Disjoint sets must fail rather than silently defaulting to all modes.
        let disjoint = AmrModeSet::from_indices(variant, &[6, 7, 8]).unwrap();
        assert!(offer.intersect(&disjoint).is_err());

        // Cross-variant intersection is a programming error, not a negotiation
        // outcome.
        let nb_set = AmrModeSet::all(AmrVariant::NarrowBand);
        assert!(offer.intersect(&nb_set).is_err());
    }

    #[test]
    fn empty_mode_set_is_rejected() {
        assert!(AmrModeSet::from_indices(AmrVariant::NarrowBand, &[]).is_err());
        assert!(AmrModeSet::from_indices(AmrVariant::NarrowBand, &[0, 9]).is_err());
    }

    #[test]
    fn variant_constants_match_rfc4867() {
        let nb = AmrVariant::NarrowBand;
        assert_eq!(nb.sample_rate(), 8_000);
        assert_eq!(nb.clock_rate(), 8_000);
        assert_eq!(nb.frame_samples(), 160);
        assert_eq!(nb.sdp_name(), "AMR");
        assert_eq!(nb.storage_magic(), b"#!AMR\n");

        let wb = AmrVariant::WideBand;
        assert_eq!(wb.sample_rate(), 16_000);
        assert_eq!(wb.clock_rate(), 16_000);
        assert_eq!(wb.frame_samples(), 320);
        assert_eq!(wb.sdp_name(), "AMR-WB");
        assert_eq!(wb.storage_magic(), b"#!AMR-WB\n");

        // A 20 ms frame at the variant's rate.
        for variant in [nb, wb] {
            assert_eq!(variant.frame_samples() * 50, variant.sample_rate() as usize);
        }
    }
}
