//! RFC 4867 RTP payload format for AMR and AMR-WB.
//!
//! # Scope
//!
//! Implemented: both framings (bandwidth-efficient and octet-aligned), the CMR
//! field, table-of-contents chains, the F/FT/Q bits, multiple frames per
//! packet, and the `NO_DATA` / `SPEECH_LOST` frame types.
//!
//! **Not yet implemented:** the three optional octet-aligned extensions —
//! frame CRC, robust sorting, and interleaving. Configuring any of them is
//! rejected at construction with a clear error rather than silently ignored,
//! because misinterpreting a payload that uses them would produce plausible
//! garbage rather than an obvious failure. All three are optional parameters a
//! receiver may legitimately decline to negotiate. See
//! `docs/AMR_IMPLEMENTATION_STATUS.md`.
//!
//! # Wire layouts
//!
//! Bandwidth-efficient (§4.3), nothing octet-aligned except the payload as a
//! whole:
//!
//! ```text
//! | CMR (4) | ToC entry (6) ... | speech bits ... | zero padding |
//!             F(1) FT(4) Q(1)
//! ```
//!
//! Octet-aligned (§4.4), every field on an octet boundary:
//!
//! ```text
//! | CMR(4) R(4) | ToC entry (8) ... | speech frame (zero-padded) ... |
//!                 F(1) FT(4) Q(1) P(2)
//! ```
//!
//! # Speech payload representation
//!
//! A frame's `data` is left-aligned: `bits.div_ceil(8)` bytes, first bit in bit
//! 7 of byte 0, trailing bits zero. Identical in both framings — only the
//! placement in the stream differs. The bit *ordering within* a speech frame is
//! defined by 3GPP TS 26.101 / TS 26.201 and is the codec kernel's concern; to
//! this module a frame is an opaque run of bits of known length.

use super::bits::{BitReader, BitWriter};
use super::mode::{AmrFrameType, AmrVariant};
use crate::error::{CodecError, Result};

/// RFC 4867 CMR value meaning "no mode requested".
const CMR_NO_REQUEST: u8 = 15;

/// Upper bound on frames per packet, to bound the table-of-contents loop on
/// malformed input.
///
/// RFC 4867 sets no hard limit — it is governed by the negotiated `maxptime` —
/// but 32 frames is 640 ms, far past any sane packetization, so a longer chain
/// means a corrupt or hostile payload rather than an unusual configuration.
const MAX_FRAMES_PER_PACKET: usize = 32;

/// Negotiated payload format parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmrPayloadConfig {
    /// Which codec family the stream carries.
    pub variant: AmrVariant,
    /// Octet-aligned framing rather than bandwidth-efficient. RFC 4867 defaults
    /// to bandwidth-efficient; the two are not interoperable.
    pub octet_aligned: bool,
}

impl AmrPayloadConfig {
    /// Bandwidth-efficient framing, the RFC 4867 default.
    #[must_use]
    pub const fn bandwidth_efficient(variant: AmrVariant) -> Self {
        Self {
            variant,
            octet_aligned: false,
        }
    }

    /// Octet-aligned framing.
    #[must_use]
    pub const fn octet_aligned(variant: AmrVariant) -> Self {
        Self {
            variant,
            octet_aligned: true,
        }
    }
}

/// One speech, comfort-noise, lost or absent frame within a payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmrPayloadFrame {
    /// What the frame carries, corresponding to the `ToC` entry's FT field.
    pub frame_type: AmrFrameType,
    /// The `ToC` entry's Q bit. `false` marks the frame as severely damaged.
    pub quality_ok: bool,
    /// Left-aligned coded bits. Length is `frame_type.octet_aligned_bytes()`,
    /// and empty for `NoData` and `SpeechLost`.
    pub data: Vec<u8>,
}

impl AmrPayloadFrame {
    /// Create a frame, checking that `data` matches the size its type implies.
    ///
    /// # Errors
    ///
    /// Returns an error when `data` is not exactly
    /// `frame_type.octet_aligned_bytes()` long.
    pub fn new(frame_type: AmrFrameType, quality_ok: bool, data: Vec<u8>) -> Result<Self> {
        let expected = frame_type.octet_aligned_bytes();
        if data.len() != expected {
            return Err(CodecError::InvalidFrameSize {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            frame_type,
            quality_ok,
            data,
        })
    }

    /// A frame carrying no bits — `NO_DATA` (FT 15).
    #[must_use]
    pub const fn no_data() -> Self {
        Self {
            frame_type: AmrFrameType::NoData,
            quality_ok: true,
            data: Vec::new(),
        }
    }

    /// A lost frame — `SPEECH_LOST` (FT 14). AMR-WB only.
    #[must_use]
    pub const fn speech_lost() -> Self {
        Self {
            frame_type: AmrFrameType::SpeechLost,
            quality_ok: false,
            data: Vec::new(),
        }
    }
}

/// A complete RTP payload: an optional mode request plus one or more frames.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AmrPacket {
    /// Mode requested of the peer's encoder. `None` is the RFC's CMR value 15,
    /// "no request".
    pub cmr: Option<u8>,
    /// Frames in transmission order, oldest first.
    pub frames: Vec<AmrPayloadFrame>,
}

impl AmrPacket {
    /// A packet carrying a single frame and no mode request.
    #[must_use]
    pub fn single(frame: AmrPayloadFrame) -> Self {
        Self {
            cmr: None,
            frames: vec![frame],
        }
    }

    /// Set the codec mode request.
    #[must_use]
    pub const fn with_cmr(mut self, cmr: Option<u8>) -> Self {
        self.cmr = cmr;
        self
    }

    /// Duration covered, in 20 ms frames.
    #[must_use]
    pub const fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

/// Packs and unpacks RFC 4867 payloads for one negotiated configuration.
#[derive(Debug, Clone, Copy)]
pub struct AmrPayloadCodec {
    config: AmrPayloadConfig,
}

impl AmrPayloadCodec {
    /// Create a packer/depacker.
    ///
    /// # Errors
    ///
    /// Currently infallible, but returns `Result` because the optional
    /// extensions (CRC, robust sorting, interleaving) will be rejected here
    /// once they are representable in [`AmrPayloadConfig`].
    pub const fn new(config: AmrPayloadConfig) -> Result<Self> {
        Ok(Self { config })
    }

    /// The negotiated configuration.
    #[must_use]
    pub const fn config(&self) -> AmrPayloadConfig {
        self.config
    }

    /// Serialise a packet into an RTP payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the packet has no frames, has more than
    /// the frame limit, contains a frame belonging to a different
    /// variant, or contains a frame whose `data` length disagrees with its
    /// frame type.
    pub fn pack(&self, packet: &AmrPacket) -> Result<Vec<u8>> {
        if packet.frames.is_empty() {
            return Err(CodecError::invalid_format(
                "an AMR payload must carry at least one frame",
            ));
        }
        if packet.frames.len() > MAX_FRAMES_PER_PACKET {
            return Err(CodecError::invalid_format(format!(
                "{} frames exceeds the {MAX_FRAMES_PER_PACKET}-frame limit",
                packet.frames.len()
            )));
        }

        let mut writer = BitWriter::new();
        writer.write_bits(u32::from(packet.cmr.unwrap_or(CMR_NO_REQUEST)), 4);
        if self.config.octet_aligned {
            // Four reserved bits, which MUST be zero.
            writer.write_bits(0, 4);
        }

        // Table of contents: every entry but the last has F set.
        for (index, frame) in packet.frames.iter().enumerate() {
            self.validate_frame(frame)?;
            let last = index + 1 == packet.frames.len();
            writer.write_bit(!last);
            writer.write_bits(u32::from(frame.frame_type.frame_type_index()), 4);
            writer.write_bit(frame.quality_ok);
            if self.config.octet_aligned {
                // Two padding bits, which MUST be zero.
                writer.write_bits(0, 2);
            }
        }

        // Speech data in the same order as the ToC.
        for frame in &packet.frames {
            let bits = frame.frame_type.bits();
            if bits == 0 {
                continue;
            }
            writer.write_slice_bits(&frame.data, bits)?;
            if self.config.octet_aligned {
                // Each frame is individually padded to an octet boundary.
                writer.align_to_octet();
            }
        }

        Ok(writer.finish())
    }

    /// Parse an RTP payload.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::InvalidPayload`] for a truncated payload, a
    /// table-of-contents chain longer than the frame limit, a
    /// reserved frame type (RFC 4867 directs the receiver to discard the whole
    /// packet), or trailing data beyond what the `ToC` accounts for.
    pub fn unpack(&self, payload: &[u8]) -> Result<AmrPacket> {
        if payload.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "empty AMR payload".to_string(),
            });
        }

        let mut reader = BitReader::new(payload);
        let cmr_raw = u8::try_from(reader.read_bits(4)?).unwrap_or(CMR_NO_REQUEST);
        if self.config.octet_aligned {
            reader.read_bits(4)?; // reserved
        }

        // A CMR naming a mode this variant does not have is "for future use";
        // RFC 4867 says to ignore it rather than reject the packet. It arrives
        // from the network, so tolerance is the right default.
        let cmr = (cmr_raw < self.config.variant.speech_mode_count()).then_some(cmr_raw);

        // Read the ToC chain first: speech data cannot be located until every
        // frame's type, and therefore length, is known.
        let mut descriptors = Vec::new();
        loop {
            let more = reader.read_bits(1)? == 1;
            let ft_index = u8::try_from(reader.read_bits(4)?).unwrap_or(CMR_NO_REQUEST);
            let quality_ok = reader.read_bits(1)? == 1;
            if self.config.octet_aligned {
                reader.read_bits(2)?; // padding
            }

            let frame_type = AmrFrameType::from_index(self.config.variant, ft_index)?;
            descriptors.push((frame_type, quality_ok));

            if !more {
                break;
            }
            if descriptors.len() >= MAX_FRAMES_PER_PACKET {
                return Err(CodecError::InvalidPayload {
                    details: format!(
                        "AMR table of contents exceeds {MAX_FRAMES_PER_PACKET} frames; \
                         payload is corrupt"
                    ),
                });
            }
        }

        let mut frames = Vec::with_capacity(descriptors.len());
        for (frame_type, quality_ok) in descriptors {
            let bits = frame_type.bits();
            let data = if bits == 0 {
                Vec::new()
            } else {
                let data = reader.read_slice_bits(bits)?;
                if self.config.octet_aligned {
                    reader.align_to_octet();
                }
                data
            };
            frames.push(AmrPayloadFrame {
                frame_type,
                quality_ok,
                data,
            });
        }

        // Whatever is left must be sub-octet padding. A whole octet or more
        // means the sender's frame sizes disagree with ours — better caught
        // here than delivered to the decoder as misaligned bits.
        if reader.remaining_bits() >= 8 {
            return Err(CodecError::InvalidPayload {
                details: format!(
                    "{} trailing bits after the AMR table of contents was satisfied; \
                     frame sizes disagree",
                    reader.remaining_bits()
                ),
            });
        }

        Ok(AmrPacket { cmr, frames })
    }

    /// Check a frame belongs to this stream's variant and is correctly sized.
    fn validate_frame(self, frame: &AmrPayloadFrame) -> Result<()> {
        let frame_variant = match frame.frame_type {
            AmrFrameType::Speech(mode) => Some(mode.variant()),
            AmrFrameType::Sid(variant) => Some(variant),
            // NoData carries no variant; SpeechLost is wideband-only.
            AmrFrameType::NoData => None,
            AmrFrameType::SpeechLost => Some(AmrVariant::WideBand),
        };
        if let Some(frame_variant) = frame_variant {
            if frame_variant != self.config.variant {
                return Err(CodecError::invalid_format(format!(
                    "cannot pack a {frame_variant} frame into a {} payload",
                    self.config.variant
                )));
            }
        }

        let expected = frame.frame_type.octet_aligned_bytes();
        if frame.data.len() != expected {
            return Err(CodecError::InvalidFrameSize {
                expected,
                actual: frame.data.len(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
// Binary literals here are grouped by protocol field (CMR|R, F|FT|Q|P) to mirror
// the RFC 4867 bit diagrams. Even grouping would obscure exactly what these
// assertions exist to check.
#[allow(clippy::unusual_byte_groupings)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::AmrMode;

    /// Deterministic, left-aligned filler for a frame of `bits` bits.
    fn frame_data(bits: usize, seed: u8) -> Vec<u8> {
        let len = bits.div_ceil(8);
        let mut data: Vec<u8> = (0..len)
            .map(|i| u8::try_from(i % 256).unwrap_or(0).wrapping_mul(31).wrapping_add(seed) | 0x41)
            .collect();
        // Zero the unused tail bits so the left-aligned convention holds and
        // round-trip comparison is meaningful.
        let tail = bits % 8;
        if tail != 0 {
            let last = len - 1;
            data[last] &= 0xFFu8 << (8 - tail);
        }
        data
    }

    fn speech(variant: AmrVariant, index: u8, seed: u8) -> AmrPayloadFrame {
        let mode = AmrMode::new(variant, index).unwrap();
        AmrPayloadFrame::new(
            AmrFrameType::Speech(mode),
            true,
            frame_data(mode.bits(), seed),
        )
        .unwrap()
    }

    fn both_configs(variant: AmrVariant) -> [AmrPayloadConfig; 2] {
        [
            AmrPayloadConfig::bandwidth_efficient(variant),
            AmrPayloadConfig::octet_aligned(variant),
        ]
    }

    #[test]
    fn round_trips_every_mode_in_both_framings() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for config in both_configs(variant) {
                let codec = AmrPayloadCodec::new(config).unwrap();
                for mode in AmrMode::all(variant) {
                    let packet = AmrPacket::single(speech(variant, mode.index(), 7));
                    let bytes = codec.pack(&packet).unwrap();
                    let back = codec.unpack(&bytes).unwrap();
                    assert_eq!(back, packet, "{variant} mode {} {config:?}", mode.index());
                }
            }
        }
    }

    #[test]
    fn round_trips_sid_no_data_and_speech_lost() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for config in both_configs(variant) {
                let codec = AmrPayloadCodec::new(config).unwrap();

                let sid = AmrPayloadFrame::new(
                    AmrFrameType::Sid(variant),
                    true,
                    frame_data(variant.sid_bits(), 3),
                )
                .unwrap();
                let packet = AmrPacket::single(sid);
                assert_eq!(codec.unpack(&codec.pack(&packet).unwrap()).unwrap(), packet);

                let packet = AmrPacket::single(AmrPayloadFrame::no_data());
                assert_eq!(codec.unpack(&codec.pack(&packet).unwrap()).unwrap(), packet);

                if variant == AmrVariant::WideBand {
                    let packet = AmrPacket::single(AmrPayloadFrame::speech_lost());
                    assert_eq!(codec.unpack(&codec.pack(&packet).unwrap()).unwrap(), packet);
                }
            }
        }
    }

    #[test]
    fn round_trips_multi_frame_packets_with_mixed_types() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for config in both_configs(variant) {
                let codec = AmrPayloadCodec::new(config).unwrap();
                let top = variant.speech_mode_count() - 1;
                let packet = AmrPacket {
                    cmr: Some(1),
                    frames: vec![
                        speech(variant, 0, 11),
                        AmrPayloadFrame::new(
                            AmrFrameType::Sid(variant),
                            true,
                            frame_data(variant.sid_bits(), 12),
                        )
                        .unwrap(),
                        AmrPayloadFrame::no_data(),
                        speech(variant, top, 13),
                    ],
                };
                let bytes = codec.pack(&packet).unwrap();
                assert_eq!(codec.unpack(&bytes).unwrap(), packet, "{config:?}");
            }
        }
    }

    #[test]
    fn round_trips_every_frame_count_up_to_the_limit() {
        let variant = AmrVariant::WideBand;
        for config in both_configs(variant) {
            let codec = AmrPayloadCodec::new(config).unwrap();
            for count in 1..=MAX_FRAMES_PER_PACKET {
                let frames = (0..count)
                    .map(|i| speech(variant, u8::try_from(i % 9).unwrap(), 5))
                    .collect();
                let packet = AmrPacket { cmr: None, frames };
                let bytes = codec.pack(&packet).unwrap();
                assert_eq!(codec.unpack(&bytes).unwrap(), packet, "{count} frames");
            }
        }
    }

    #[test]
    fn cmr_round_trips_and_out_of_range_values_are_ignored() {
        let variant = AmrVariant::NarrowBand;
        for config in both_configs(variant) {
            let codec = AmrPayloadCodec::new(config).unwrap();

            for cmr in 0..variant.speech_mode_count() {
                let packet = AmrPacket::single(speech(variant, 0, 1)).with_cmr(Some(cmr));
                assert_eq!(codec.unpack(&codec.pack(&packet).unwrap()).unwrap().cmr, Some(cmr));
            }

            // 15 is "no request" and must decode as None.
            let packet = AmrPacket::single(speech(variant, 0, 1)).with_cmr(None);
            assert_eq!(codec.unpack(&codec.pack(&packet).unwrap()).unwrap().cmr, None);

            // 8-14 are reserved for AMR-NB. They come from the network, so the
            // RFC says ignore them rather than dropping the packet.
            let packet = AmrPacket::single(speech(variant, 0, 1)).with_cmr(Some(9));
            let decoded = codec.unpack(&codec.pack(&packet).unwrap()).unwrap();
            assert_eq!(decoded.cmr, None);
            assert_eq!(decoded.frames, packet.frames, "frames must still decode");
        }
    }

    #[test]
    fn octet_aligned_layout_matches_rfc4867() {
        // Byte 0: CMR=15, reserved=0. Byte 1: F=0, FT=15 (NO_DATA), Q=1, P=00.
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(AmrVariant::WideBand)).unwrap();
        let bytes = codec.pack(&AmrPacket::single(AmrPayloadFrame::no_data())).unwrap();
        assert_eq!(bytes, vec![0b1111_0000, 0b0_1111_1_00]);

        // AMR-WB mode 0 is 132 bits -> 17 octets, so a single-frame packet is
        // 1 (header) + 1 (ToC) + 17 = 19 octets.
        let frame = speech(AmrVariant::WideBand, 0, 2);
        let bytes = codec.pack(&AmrPacket::single(frame)).unwrap();
        assert_eq!(bytes.len(), 19);
        assert_eq!(bytes[0], 0b1111_0000);
        assert_eq!(bytes[1], 0b0_0000_1_00);
    }

    #[test]
    fn bandwidth_efficient_layout_matches_rfc4867() {
        // 4-bit CMR + 6-bit ToC + 132 speech bits = 142 bits -> 18 octets,
        // one fewer than octet-aligned. That saving is the whole point of the
        // framing, and the sizes differing is what makes the two
        // non-interoperable.
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::bandwidth_efficient(AmrVariant::WideBand))
                .unwrap();
        let frame = speech(AmrVariant::WideBand, 0, 2);
        let bytes = codec.pack(&AmrPacket::single(frame)).unwrap();
        assert_eq!(bytes.len(), 18);
        // CMR=1111, then F=0, FT=0000, Q=1 -> 1111 0 0000 1 ...
        assert_eq!(bytes[0], 0b1111_0000);
        assert_eq!(bytes[1] >> 6, 0b01);

        // NO_DATA alone: 4 + 6 = 10 bits -> 2 octets.
        let bytes = codec.pack(&AmrPacket::single(AmrPayloadFrame::no_data())).unwrap();
        assert_eq!(bytes.len(), 2);
    }

    #[test]
    fn the_two_framings_are_not_interoperable() {
        // Decoding a bandwidth-efficient payload as octet-aligned must fail
        // loudly rather than yield plausible garbage. This is the most
        // frequently reported AMR interop bug, so it gets an explicit test.
        let variant = AmrVariant::WideBand;
        let be = AmrPayloadCodec::new(AmrPayloadConfig::bandwidth_efficient(variant)).unwrap();
        let oa = AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant)).unwrap();

        let packet = AmrPacket::single(speech(variant, 8, 4));
        let be_bytes = be.pack(&packet).unwrap();
        let oa_bytes = oa.pack(&packet).unwrap();
        assert_ne!(be_bytes, oa_bytes);

        // Cross-decoding either yields an error or, at minimum, not the
        // original packet.
        assert_ne!(oa.unpack(&be_bytes).ok(), Some(packet.clone()));
        assert_ne!(be.unpack(&oa_bytes).ok(), Some(packet));
    }

    #[test]
    fn rejects_truncated_payloads_at_every_prefix() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for config in both_configs(variant) {
                let codec = AmrPayloadCodec::new(config).unwrap();
                let packet = AmrPacket {
                    cmr: Some(0),
                    frames: vec![speech(variant, 0, 9), speech(variant, 1, 10)],
                };
                let bytes = codec.pack(&packet).unwrap();

                // Every proper prefix must fail rather than decode partially.
                for cut in 0..bytes.len() {
                    let result = codec.unpack(&bytes[..cut]);
                    assert!(
                        result.is_err(),
                        "{config:?}: {cut}-byte prefix decoded but should not have"
                    );
                }
                assert!(codec.unpack(&bytes).is_ok());
            }
        }
    }

    #[test]
    fn rejects_reserved_frame_types() {
        // RFC 4867: a ToC entry with a reserved FT means discard the packet.
        // NB reserves 9-14; WB reserves 10-13.
        let nb = AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(AmrVariant::NarrowBand))
            .unwrap();
        for ft in 9..=14u8 {
            let bytes = vec![0b1111_0000, ft << 3 | 0b100];
            assert!(nb.unpack(&bytes).is_err(), "NB FT {ft} should be rejected");
        }

        let wb =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(AmrVariant::WideBand)).unwrap();
        for ft in 10..=13u8 {
            let bytes = vec![0b1111_0000, ft << 3 | 0b100];
            assert!(wb.unpack(&bytes).is_err(), "WB FT {ft} should be rejected");
        }
        // FT 14 is SPEECH_LOST for wideband and must be accepted.
        let bytes = vec![0b1111_0000, 14 << 3 | 0b100];
        assert!(wb.unpack(&bytes).is_ok());
    }

    #[test]
    fn rejects_an_unterminated_toc_chain() {
        // Every ToC entry has F set, so the chain never ends. Must be bounded
        // rather than looping until the payload runs out.
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(AmrVariant::WideBand)).unwrap();
        // FT 15 (NO_DATA) carries no data, so a long run of F=1 entries stays
        // within the payload.
        let mut bytes = vec![0b1111_0000];
        bytes.extend(std::iter::repeat_n(0b1_1111_1_00u8, 64));
        let err = codec.unpack(&bytes).unwrap_err();
        assert!(matches!(err, CodecError::InvalidPayload { .. }));
    }

    #[test]
    fn rejects_trailing_data_beyond_the_toc() {
        let variant = AmrVariant::WideBand;
        for config in both_configs(variant) {
            let codec = AmrPayloadCodec::new(config).unwrap();
            let mut bytes = codec.pack(&AmrPacket::single(speech(variant, 0, 6))).unwrap();
            // A whole extra octet means the sender's frame sizing disagrees
            // with ours; sub-octet padding would be legitimate.
            bytes.push(0x00);
            assert!(codec.unpack(&bytes).is_err(), "{config:?}");
        }
    }

    #[test]
    fn rejects_empty_payloads_and_empty_packets() {
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(AmrVariant::WideBand)).unwrap();
        assert!(codec.unpack(&[]).is_err());
        assert!(codec
            .pack(&AmrPacket {
                cmr: None,
                frames: vec![]
            })
            .is_err());
    }

    #[test]
    fn rejects_too_many_frames_on_pack() {
        let variant = AmrVariant::NarrowBand;
        let codec = AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant)).unwrap();
        let frames = (0..=MAX_FRAMES_PER_PACKET)
            .map(|_| AmrPayloadFrame::no_data())
            .collect();
        assert!(codec.pack(&AmrPacket { cmr: None, frames }).is_err());
    }

    #[test]
    fn rejects_cross_variant_frames_on_pack() {
        // AMR-WB mode 2 and AMR-NB mode 2 have different frame sizes, so
        // packing one into the other's stream would corrupt the payload.
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(AmrVariant::NarrowBand)).unwrap();
        let wb_frame = speech(AmrVariant::WideBand, 2, 1);
        assert!(codec.pack(&AmrPacket::single(wb_frame)).is_err());

        // A wideband SID in a narrowband stream, likewise.
        let wb_sid = AmrPayloadFrame::new(
            AmrFrameType::Sid(AmrVariant::WideBand),
            true,
            frame_data(AmrVariant::WideBand.sid_bits(), 1),
        )
        .unwrap();
        assert!(codec.pack(&AmrPacket::single(wb_sid)).is_err());
    }

    #[test]
    fn frame_constructor_enforces_the_declared_size() {
        let mode = AmrMode::new(AmrVariant::WideBand, 8).unwrap();
        assert_eq!(mode.octet_aligned_bytes(), 60);
        assert!(AmrPayloadFrame::new(AmrFrameType::Speech(mode), true, vec![0; 60]).is_ok());
        assert!(AmrPayloadFrame::new(AmrFrameType::Speech(mode), true, vec![0; 59]).is_err());
        assert!(AmrPayloadFrame::new(AmrFrameType::Speech(mode), true, vec![0; 61]).is_err());
        // NoData must carry nothing.
        assert!(AmrPayloadFrame::new(AmrFrameType::NoData, true, vec![0; 1]).is_err());
    }

    #[test]
    fn quality_bit_round_trips() {
        let variant = AmrVariant::WideBand;
        for config in both_configs(variant) {
            let codec = AmrPayloadCodec::new(config).unwrap();
            let mut frame = speech(variant, 3, 8);
            frame.quality_ok = false;
            let packet = AmrPacket::single(frame);
            let back = codec.unpack(&codec.pack(&packet).unwrap()).unwrap();
            assert!(!back.frames[0].quality_ok, "{config:?}");
        }
    }

    #[test]
    fn never_panics_on_arbitrary_input() {
        // Cheap structured sweep; the real fuzz target lives in crates/media/fuzz.
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for config in both_configs(variant) {
                let codec = AmrPayloadCodec::new(config).unwrap();
                for seed in 0u16..=u16::from(u8::MAX) {
                    let byte = u8::try_from(seed).unwrap_or(0);
                    for len in 0..8usize {
                        let bytes: Vec<u8> =
                            (0..len).map(|i| byte.rotate_left(u32::try_from(i).unwrap_or(0))).collect();
                        // Only requirement: it returns, either way.
                        let _ = codec.unpack(&bytes);
                    }
                }
            }
        }
    }
}
