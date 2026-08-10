//! RFC 4867 RTP payload format for AMR and AMR-WB.
//!
//! # Scope
//!
//! Both framings (bandwidth-efficient and octet-aligned), the CMR field,
//! table-of-contents chains, the F/FT/Q bits, multiple frames per packet, the
//! `NO_DATA` / `SPEECH_LOST` frame types, and the three optional
//! octet-aligned extensions: frame CRC, robust sorting, and interleaving.
//!
//! Interleaving is carried but not *performed*: the ILL/ILP fields are emitted
//! and parsed, and exposed on [`AmrPacket`], but reordering frame-blocks across
//! packets requires buffering that belongs with the jitter buffer rather than
//! the payload format. See [`AmrInterleaving`].
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
//! | CMR(4) R(4) | [ILL(4) ILP(4)] | ToC entry (8) ... | [CRC(8) ...] | speech ... |
//!                  if interleaving   F(1) FT(4) Q(1) P(2)  if crc
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
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmrPayloadConfig {
    /// Which codec family the stream carries.
    pub variant: AmrVariant,
    /// Octet-aligned framing rather than bandwidth-efficient. RFC 4867 defaults
    /// to bandwidth-efficient; the two are not interoperable.
    pub octet_aligned: bool,
    /// Emit and check a per-frame CRC over the class A bits. Implies
    /// `octet_aligned`.
    pub crc: bool,
    /// Interleave speech octets across frames. Implies `octet_aligned`.
    pub robust_sorting: bool,
    /// Carry the ILL/ILP interleaving fields in the header. Implies
    /// `octet_aligned`.
    pub interleaving: bool,
}

impl AmrPayloadConfig {
    /// Bandwidth-efficient framing, the RFC 4867 default.
    #[must_use]
    pub const fn bandwidth_efficient(variant: AmrVariant) -> Self {
        Self {
            variant,
            octet_aligned: false,
            crc: false,
            robust_sorting: false,
            interleaving: false,
        }
    }

    /// Octet-aligned framing.
    #[must_use]
    pub const fn octet_aligned(variant: AmrVariant) -> Self {
        Self {
            octet_aligned: true,
            ..Self::bandwidth_efficient(variant)
        }
    }

    /// Enable the per-frame CRC. Forces octet alignment, which the RFC requires.
    #[must_use]
    pub const fn with_crc(mut self) -> Self {
        self.crc = true;
        self.octet_aligned = true;
        self
    }

    /// Enable robust sorting. Forces octet alignment.
    #[must_use]
    pub const fn with_robust_sorting(mut self) -> Self {
        self.robust_sorting = true;
        self.octet_aligned = true;
        self
    }

    /// Carry the ILL/ILP interleaving fields. Forces octet alignment.
    #[must_use]
    pub const fn with_interleaving(mut self) -> Self {
        self.interleaving = true;
        self.octet_aligned = true;
        self
    }

    /// Whether any option requires octet alignment.
    const fn needs_octet_align(self) -> bool {
        self.crc || self.robust_sorting || self.interleaving
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

/// The RFC 4867 §4.4.1 interleaving fields.
///
/// **Carried, not applied.** This type transports ILL/ILP faithfully, but
/// reassembling the original frame-block order means holding frames from up to
/// `ill + 1` packets and emitting them out of arrival order — that is jitter
/// buffer work, and doing it here would duplicate reordering logic that layer
/// already owns. A receiver that negotiates interleaving must act on these
/// values itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmrInterleaving {
    /// Interleaving length minus one, in frame-blocks. The group spans
    /// `ill + 1` packets.
    pub ill: u8,
    /// This packet's index within the group, `0..=ill`.
    pub ilp: u8,
}

impl AmrInterleaving {
    /// Create interleaving fields.
    ///
    /// # Errors
    ///
    /// Returns an error when `ilp > ill` — RFC 4867 requires the index to fall
    /// inside the group — or when either exceeds the 4 bits available.
    pub fn new(ill: u8, ilp: u8) -> Result<Self> {
        if ill > 0x0F || ilp > 0x0F {
            return Err(CodecError::invalid_format(
                "AMR ILL and ILP are 4-bit fields",
            ));
        }
        if ilp > ill {
            return Err(CodecError::invalid_format(format!(
                "AMR interleaving index {ilp} is outside its group (ILL={ill})"
            )));
        }
        Ok(Self { ill, ilp })
    }

    /// Number of packets in the interleaving group.
    #[must_use]
    pub const fn group_len(self) -> u8 {
        self.ill.saturating_add(1)
    }
}

/// A complete RTP payload: an optional mode request plus one or more frames.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AmrPacket {
    /// Mode requested of the peer's encoder. `None` is the RFC's CMR value 15,
    /// "no request".
    pub cmr: Option<u8>,
    /// Interleaving position, present only when interleaving is negotiated.
    pub interleaving: Option<AmrInterleaving>,
    /// Frames in transmission order, oldest first.
    pub frames: Vec<AmrPayloadFrame>,
}

impl AmrPacket {
    /// A packet carrying a single frame and no mode request.
    #[must_use]
    pub fn single(frame: AmrPayloadFrame) -> Self {
        Self {
            cmr: None,
            interleaving: None,
            frames: vec![frame],
        }
    }

    /// Set the interleaving position.
    #[must_use]
    pub const fn with_interleaving(mut self, interleaving: Option<AmrInterleaving>) -> Self {
        self.interleaving = interleaving;
        self
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

/// Feedback mask for the RFC 4867 §4.4.2.1 frame CRC.
///
/// The generator is `C(x) = 1 + x^2 + x^3 + x^4 + x^8`, which the RFC writes
/// low-order-first as `101110001`. Dropping the implicit `x^8` term leaves
/// `10111000` — the value the RFC says to XOR in — which is `0xB8`.
const CRC_FEEDBACK: u8 = 0b1011_1000;

/// Compute the RFC 4867 frame CRC over the leading `class_a_bits` of `data`.
///
/// The register starts at zero. For each bit: XOR the register's least
/// significant bit with the input bit, shift the register right (zero in from
/// the left), and if the XOR was 1, XOR in [`CRC_FEEDBACK`].
///
/// Covering the *leading* bits is correct because AMR frames are ordered by
/// subjective importance with class A first — TS 26.201 puts AMR-WB 6.60's
/// class A bits at `d(0)..d(53)`, and narrowband is ordered the same way.
fn frame_crc(data: &[u8], class_a_bits: usize) -> u8 {
    let mut crc = 0u8;
    for index in 0..class_a_bits {
        let bit = data[index / 8] >> (7 - index % 8) & 1;
        let feedback = (crc & 1) ^ bit;
        crc >>= 1;
        if feedback == 1 {
            crc ^= CRC_FEEDBACK;
        }
    }
    crc
}

/// Class A bit count for a frame type, or `None` when no CRC applies.
///
/// TS 26.201: "When Frame Type Index of table 1a is 14 or 15, the CRC field is
/// not included in the Generic AMR-WB frame." So `NO_DATA` and `SPEECH_LOST`
/// contribute no CRC. Comfort-noise bits "are all mapped to Class A", and
/// RFC 4867 lists AMR-NB SID as 39 class A bits of 39, so a SID frame's CRC
/// covers all of it.
const fn crc_class_a_bits(frame_type: AmrFrameType) -> Option<usize> {
    match frame_type {
        AmrFrameType::Speech(mode) => Some(mode.class_a_bits()),
        AmrFrameType::Sid(variant) => Some(variant.sid_bits()),
        AmrFrameType::NoData | AmrFrameType::SpeechLost => None,
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
    /// Returns an error when the configuration requests CRC, robust sorting or
    /// interleaving without octet alignment. RFC 4867 makes all three
    /// octet-aligned-only, and the constructors on [`AmrPayloadConfig`] set
    /// alignment automatically — reaching this error means the struct was built
    /// field-by-field with an inconsistent combination.
    pub fn new(config: AmrPayloadConfig) -> Result<Self> {
        if config.needs_octet_align() && !config.octet_aligned {
            return Err(CodecError::invalid_config(
                "AMR crc, robust-sorting and interleaving all require octet-align=1",
            ));
        }
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

        if self.config.interleaving {
            let interleaving = packet.interleaving.ok_or_else(|| {
                CodecError::invalid_format(
                    "interleaving is negotiated but the packet carries no ILL/ILP",
                )
            })?;
            writer.write_bits(u32::from(interleaving.ill), 4);
            writer.write_bits(u32::from(interleaving.ilp), 4);
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

        // The CRC list follows the whole ToC, before any speech data.
        if self.config.crc {
            for frame in &packet.frames {
                if let Some(class_a) = crc_class_a_bits(frame.frame_type) {
                    writer.write_bits(u32::from(frame_crc(&frame.data, class_a)), 8);
                }
            }
        }

        if self.config.robust_sorting {
            Self::write_robust_sorted(&mut writer, &packet.frames);
        } else {
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

        let interleaving = if self.config.interleaving {
            let ill = u8::try_from(reader.read_bits(4)?).unwrap_or(0);
            let ilp = u8::try_from(reader.read_bits(4)?).unwrap_or(0);
            // ILP > ILL is malformed, but it arrives from the network. Clamp
            // rather than drop the packet: the speech is still decodable, and
            // only the reordering position is unusable.
            Some(AmrInterleaving {
                ill,
                ilp: ilp.min(ill),
            })
        } else {
            None
        };

        // A CMR naming a mode this variant does not have is "for future use";
        // RFC 4867 says to ignore it rather than reject the packet. It arrives
        // from the network, so tolerance is the right default.
        let cmr = (cmr_raw < self.config.variant.speech_mode_count()).then_some(cmr_raw);

        let descriptors = self.read_table_of_contents(&mut reader)?;

        // The CRC list sits between the ToC and the speech data, so it must be
        // consumed before frames can be located.
        let mut crcs = Vec::new();
        if self.config.crc {
            for (frame_type, _) in &descriptors {
                if crc_class_a_bits(*frame_type).is_some() {
                    crcs.push(u8::try_from(reader.read_bits(8)?).unwrap_or(0));
                }
            }
        }

        let mut frames = if self.config.robust_sorting {
            Self::read_robust_sorted(&mut reader, &descriptors)?
        } else {
            let mut frames = Vec::with_capacity(descriptors.len());
            for (frame_type, quality_ok) in &descriptors {
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
                    frame_type: *frame_type,
                    quality_ok: *quality_ok,
                    data,
                });
            }
            frames
        };

        // A failed CRC means the class A bits are damaged. RFC 4867 has the
        // receiver mark the frame bad rather than discard the packet, which is
        // exactly what the Q bit is for: the decoder then conceals instead of
        // trusting corrupt parameters.
        if self.config.crc {
            let mut crc_index = 0;
            for frame in &mut frames {
                let Some(class_a) = crc_class_a_bits(frame.frame_type) else {
                    continue;
                };
                let expected = crcs.get(crc_index).copied().unwrap_or(0);
                crc_index += 1;
                if frame_crc(&frame.data, class_a) != expected {
                    frame.quality_ok = false;
                }
            }
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

        Ok(AmrPacket {
            cmr,
            interleaving,
            frames,
        })
    }

    /// Read the table-of-contents chain.
    ///
    /// This comes before any speech data because a frame cannot be located
    /// until every preceding frame's type — and therefore length — is known.
    ///
    /// # Errors
    ///
    /// Returns an error for a truncated chain, a reserved frame type, or a
    /// chain longer than the frame limit.
    fn read_table_of_contents(
        self,
        reader: &mut BitReader<'_>,
    ) -> Result<Vec<(AmrFrameType, bool)>> {
        let mut descriptors = Vec::new();
        loop {
            let more = reader.read_bits(1)? == 1;
            let ft_index = u8::try_from(reader.read_bits(4)?).unwrap_or(CMR_NO_REQUEST);
            let quality_ok = reader.read_bits(1)? == 1;
            if self.config.octet_aligned {
                reader.read_bits(2)?; // padding
            }

            descriptors.push((
                AmrFrameType::from_index(self.config.variant, ft_index)?,
                quality_ok,
            ));

            if !more {
                return Ok(descriptors);
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
    }

    /// Write speech octets robust-sorted (RFC 4867 §4.4.3).
    ///
    /// The payload carries the first octet of every frame, then the second
    /// octet of every frame, and so on for as many rounds as the longest frame
    /// has octets. Frames shorter than the current round simply contribute
    /// nothing, so a single lost packet damages one octet of each frame rather
    /// than destroying one frame outright.
    fn write_robust_sorted(writer: &mut BitWriter, frames: &[AmrPayloadFrame]) {
        let longest = frames.iter().map(|f| f.data.len()).max().unwrap_or(0);
        for round in 0..longest {
            for frame in frames {
                if let Some(&octet) = frame.data.get(round) {
                    writer.write_bits(u32::from(octet), 8);
                }
            }
        }
    }

    /// Read robust-sorted speech octets back into per-frame buffers.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is too short for the octet count the
    /// table of contents implies.
    fn read_robust_sorted(
        reader: &mut BitReader<'_>,
        descriptors: &[(AmrFrameType, bool)],
    ) -> Result<Vec<AmrPayloadFrame>> {
        let lengths: Vec<usize> = descriptors
            .iter()
            .map(|(frame_type, _)| frame_type.octet_aligned_bytes())
            .collect();
        let mut buffers: Vec<Vec<u8>> = lengths.iter().map(|&len| vec![0u8; len]).collect();

        let longest = lengths.iter().copied().max().unwrap_or(0);
        for round in 0..longest {
            for buffer in &mut buffers {
                if let Some(slot) = buffer.get_mut(round) {
                    *slot = u8::try_from(reader.read_bits(8)?).unwrap_or(0);
                }
            }
        }

        Ok(descriptors
            .iter()
            .zip(buffers)
            .map(|(&(frame_type, quality_ok), data)| AmrPayloadFrame {
                frame_type,
                quality_ok,
                data,
            })
            .collect())
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
                    interleaving: None,
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
                let packet = AmrPacket {
                    cmr: None,
                    interleaving: None,
                    frames,
                };
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
                    interleaving: None,
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
                interleaving: None,
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
        assert!(codec
            .pack(&AmrPacket {
                cmr: None,
                interleaving: None,
                frames
            })
            .is_err());
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

    // ---- optional octet-aligned extensions ----

    #[test]
    fn crc_matches_the_rfc4867_reference_algorithm() {
        // Independent implementation of RFC 4867 §4.4.2.1 as prose describes
        // it, to check the shift-and-feedback version used in the module.
        fn reference_crc(data: &[u8], class_a_bits: usize) -> u8 {
            // Register as 8 separate bits, LSB at index 0.
            let mut reg = [0u8; 8];
            for index in 0..class_a_bits {
                let bit = data[index / 8] >> (7 - index % 8) & 1;
                let feedback = reg[0] ^ bit;
                reg.rotate_left(1);
                reg[7] = 0;
                if feedback == 1 {
                    // "10111000" XOR-ed in, MSB first.
                    for (i, mask) in [1, 0, 1, 1, 1, 0, 0, 0].into_iter().enumerate() {
                        reg[7 - i] ^= mask;
                    }
                }
            }
            reg.iter().enumerate().fold(0u8, |acc, (i, &b)| acc | b << i)
        }

        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for mode in AmrMode::all(variant) {
                for seed in [0u8, 1, 37, 200, 255] {
                    let data = frame_data(mode.bits(), seed);
                    assert_eq!(
                        frame_crc(&data, mode.class_a_bits()),
                        reference_crc(&data, mode.class_a_bits()),
                        "{mode} seed {seed}"
                    );
                }
            }
        }
    }

    #[test]
    fn crc_detects_damage_to_class_a_bits() {
        let variant = AmrVariant::WideBand;
        let codec = AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant).with_crc())
            .unwrap();
        let mode = AmrMode::new(variant, 2).unwrap();
        let packet = AmrPacket::single(speech(variant, 2, 5));
        let mut bytes = codec.pack(&packet).unwrap();

        // Clean payload: CRC passes, quality stays good.
        assert!(codec.unpack(&bytes).unwrap().frames[0].quality_ok);

        // Flip a class A bit in the speech data. Header is 1 octet, ToC 1, CRC
        // 1, so speech starts at offset 3.
        let speech_start = 3;
        bytes[speech_start] ^= 0x80;
        let damaged = codec.unpack(&bytes).unwrap();
        assert!(
            !damaged.frames[0].quality_ok,
            "a corrupt class A bit must clear the Q bit"
        );
        // The packet is still delivered — RFC 4867 marks the frame bad rather
        // than discarding, so the decoder can conceal.
        assert_eq!(damaged.frames.len(), 1);

        // Flipping a class B bit is invisible to the CRC by design: class B/C
        // degrade gracefully and are deliberately left unprotected.
        let class_b_byte = speech_start + mode.class_a_bits() / 8 + 1;
        let mut bytes = codec.pack(&packet).unwrap();
        bytes[class_b_byte] ^= 0x01;
        assert!(codec.unpack(&bytes).unwrap().frames[0].quality_ok);
    }

    #[test]
    fn crc_round_trips_for_every_mode_and_frame_type() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            let codec =
                AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant).with_crc()).unwrap();
            for mode in AmrMode::all(variant) {
                let packet = AmrPacket::single(speech(variant, mode.index(), 17));
                let back = codec.unpack(&codec.pack(&packet).unwrap()).unwrap();
                assert_eq!(back, packet, "{mode}");
            }

            // SID carries a CRC (all its bits are class A); NO_DATA does not.
            let sid = AmrPayloadFrame::new(
                AmrFrameType::Sid(variant),
                true,
                frame_data(variant.sid_bits(), 4),
            )
            .unwrap();
            let packet = AmrPacket {
                cmr: None,
                interleaving: None,
                frames: vec![sid, AmrPayloadFrame::no_data()],
            };
            assert_eq!(codec.unpack(&codec.pack(&packet).unwrap()).unwrap(), packet);
        }
    }

    #[test]
    fn crc_adds_one_octet_per_frame_that_carries_one() {
        let variant = AmrVariant::WideBand;
        let plain = AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant)).unwrap();
        let with_crc =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant).with_crc()).unwrap();

        // Two speech frames plus a NO_DATA: two CRCs, not three.
        let packet = AmrPacket {
            cmr: None,
            interleaving: None,
            frames: vec![
                speech(variant, 0, 1),
                AmrPayloadFrame::no_data(),
                speech(variant, 1, 2),
            ],
        };
        let a = plain.pack(&packet).unwrap().len();
        let b = with_crc.pack(&packet).unwrap().len();
        assert_eq!(b - a, 2);
    }

    #[test]
    fn robust_sorting_interleaves_octets_across_frames() {
        let variant = AmrVariant::WideBand;
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant).with_robust_sorting())
                .unwrap();

        // Two frames with recognisable first octets.
        let mut f0 = speech(variant, 0, 1);
        let mut f1 = speech(variant, 0, 2);
        f0.data[0] = 0xA0;
        f0.data[1] = 0xA1;
        f1.data[0] = 0xB0;
        f1.data[1] = 0xB1;
        let packet = AmrPacket {
            cmr: None,
            interleaving: None,
            frames: vec![f0, f1],
        };
        let bytes = codec.pack(&packet).unwrap();

        // Header 1 + ToC 2 = speech starts at 3, then A0 B0 A1 B1 ...
        assert_eq!(&bytes[3..7], &[0xA0, 0xB0, 0xA1, 0xB1]);
        assert_eq!(codec.unpack(&bytes).unwrap(), packet);
    }

    #[test]
    fn robust_sorting_round_trips_with_mixed_frame_lengths() {
        // Frames of different lengths drop out of later rounds, which is the
        // part of §4.4.3 easiest to get wrong.
        let variant = AmrVariant::WideBand;
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant).with_robust_sorting())
                .unwrap();
        let packet = AmrPacket {
            cmr: Some(3),
            interleaving: None,
            frames: vec![
                speech(variant, 8, 1), // 60 octets
                speech(variant, 0, 2), // 17 octets
                AmrPayloadFrame::new(
                    AmrFrameType::Sid(variant),
                    true,
                    frame_data(variant.sid_bits(), 3),
                )
                .unwrap(), // 5 octets
                AmrPayloadFrame::no_data(), // 0 octets
                speech(variant, 4, 4), // 40 octets
            ],
        };
        let bytes = codec.pack(&packet).unwrap();
        assert_eq!(codec.unpack(&bytes).unwrap(), packet);

        // Same total payload size as unsorted — sorting reorders, not resizes.
        let plain = AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant)).unwrap();
        assert_eq!(bytes.len(), plain.pack(&packet).unwrap().len());
    }

    #[test]
    fn robust_sorting_round_trips_every_mode() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            let codec = AmrPayloadCodec::new(
                AmrPayloadConfig::octet_aligned(variant).with_robust_sorting(),
            )
            .unwrap();
            for mode in AmrMode::all(variant) {
                let packet = AmrPacket {
                    cmr: None,
                    interleaving: None,
                    frames: vec![speech(variant, mode.index(), 6), speech(variant, 0, 7)],
                };
                assert_eq!(codec.unpack(&codec.pack(&packet).unwrap()).unwrap(), packet);
            }
        }
    }

    #[test]
    fn interleaving_fields_round_trip() {
        let variant = AmrVariant::WideBand;
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant).with_interleaving())
                .unwrap();
        for ill in 0..=15u8 {
            for ilp in 0..=ill {
                let interleaving = AmrInterleaving::new(ill, ilp).unwrap();
                let packet = AmrPacket::single(speech(variant, 0, 3))
                    .with_interleaving(Some(interleaving));
                let back = codec.unpack(&codec.pack(&packet).unwrap()).unwrap();
                assert_eq!(back.interleaving, Some(interleaving));
                assert_eq!(back.frames, packet.frames);
            }
        }
    }

    #[test]
    fn interleaving_adds_exactly_one_octet() {
        let variant = AmrVariant::WideBand;
        let plain = AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant)).unwrap();
        let interleaved =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant).with_interleaving())
                .unwrap();
        let frame = speech(variant, 0, 3);
        let a = plain.pack(&AmrPacket::single(frame.clone())).unwrap().len();
        let b = interleaved
            .pack(
                &AmrPacket::single(frame)
                    .with_interleaving(Some(AmrInterleaving::new(3, 1).unwrap())),
            )
            .unwrap()
            .len();
        assert_eq!(b - a, 1);
    }

    #[test]
    fn interleaving_index_outside_its_group_is_rejected_locally_but_clamped_on_the_wire() {
        // Constructing an invalid position is a local bug.
        assert!(AmrInterleaving::new(2, 3).is_err());
        assert!(AmrInterleaving::new(16, 0).is_err());
        assert_eq!(AmrInterleaving::new(3, 3).unwrap().group_len(), 4);

        // Receiving one is a peer bug: clamp rather than drop, since the speech
        // is still decodable and only the reordering position is unusable.
        let variant = AmrVariant::WideBand;
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant).with_interleaving())
                .unwrap();
        // ILL=2, ILP=5 in the second octet.
        let mut bytes = codec
            .pack(
                &AmrPacket::single(speech(variant, 0, 3))
                    .with_interleaving(Some(AmrInterleaving::new(2, 1).unwrap())),
            )
            .unwrap();
        bytes[1] = 2 << 4 | 5;
        let back = codec.unpack(&bytes).unwrap();
        assert_eq!(back.interleaving.unwrap().ilp, 2, "ILP clamped to ILL");
    }

    #[test]
    fn packing_without_required_interleaving_fields_is_an_error() {
        let variant = AmrVariant::WideBand;
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(variant).with_interleaving())
                .unwrap();
        // Negotiated but absent: silently emitting zeros would put every packet
        // in group 0, which a receiver would reassemble wrongly.
        assert!(codec.pack(&AmrPacket::single(speech(variant, 0, 3))).is_err());
    }

    #[test]
    fn extensions_force_octet_alignment() {
        let variant = AmrVariant::WideBand;
        for config in [
            AmrPayloadConfig::bandwidth_efficient(variant).with_crc(),
            AmrPayloadConfig::bandwidth_efficient(variant).with_robust_sorting(),
            AmrPayloadConfig::bandwidth_efficient(variant).with_interleaving(),
        ] {
            assert!(config.octet_aligned, "{config:?} must force octet alignment");
        }

        // A hand-built inconsistent config is rejected rather than silently
        // producing a payload no peer can parse.
        let inconsistent = AmrPayloadConfig {
            variant,
            octet_aligned: false,
            crc: true,
            robust_sorting: false,
            interleaving: false,
        };
        assert!(AmrPayloadCodec::new(inconsistent).is_err());
    }

    #[test]
    fn all_extensions_together_round_trip() {
        let variant = AmrVariant::WideBand;
        let config = AmrPayloadConfig::octet_aligned(variant)
            .with_crc()
            .with_robust_sorting()
            .with_interleaving();
        let codec = AmrPayloadCodec::new(config).unwrap();
        let packet = AmrPacket {
            cmr: Some(5),
            interleaving: Some(AmrInterleaving::new(4, 2).unwrap()),
            frames: vec![
                speech(variant, 8, 1),
                AmrPayloadFrame::no_data(),
                speech(variant, 2, 2),
            ],
        };
        let bytes = codec.pack(&packet).unwrap();
        assert_eq!(codec.unpack(&bytes).unwrap(), packet);
    }

    #[test]
    fn extension_payloads_reject_truncation_at_every_prefix() {
        let variant = AmrVariant::WideBand;
        let config = AmrPayloadConfig::octet_aligned(variant)
            .with_crc()
            .with_robust_sorting()
            .with_interleaving();
        let codec = AmrPayloadCodec::new(config).unwrap();
        let packet = AmrPacket {
            cmr: None,
            interleaving: Some(AmrInterleaving::new(1, 0).unwrap()),
            frames: vec![speech(variant, 3, 1), speech(variant, 0, 2)],
        };
        let bytes = codec.pack(&packet).unwrap();
        for cut in 0..bytes.len() {
            assert!(codec.unpack(&bytes[..cut]).is_err(), "{cut}-byte prefix");
        }
        assert!(codec.unpack(&bytes).is_ok());
    }

    // ---- captured from a real implementation ----

    /// AMR-WB RTP payloads captured from `FreeSWITCH` 1.10.12 (`mod_amrwb`,
    /// `vo-amrwbenc`) during a live call, length-prefixed with a 16-bit
    /// big-endian length so they can be split without guessing.
    ///
    /// Genuine encoder output, not our own bytes echoed back: the call was
    /// bridged into a conference so `FreeSWITCH` had to mix in linear PCM and
    /// re-encode, and all 50 payloads differ from one another. An earlier
    /// capture using `&echo` produced 50 byte-identical payloads because
    /// `FreeSWITCH` passed them through untouched — which would have made this a
    /// test of our own packetizer wearing a disguise.
    const FREESWITCH_AMRWB_RTP: &[u8] =
        include_bytes!("testdata/freeswitch_amrwb_be.rtp");

    /// Split the length-prefixed capture into individual RTP payloads.
    fn freeswitch_payloads() -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut rest = FREESWITCH_AMRWB_RTP;
        while rest.len() >= 2 {
            let len = usize::from(u16::from_be_bytes([rest[0], rest[1]]));
            assert!(rest.len() >= 2 + len, "truncated capture fixture");
            out.push(rest[2..2 + len].to_vec());
            rest = &rest[2 + len..];
        }
        out
    }

    #[test]
    fn unpacks_real_freeswitch_amr_wb_rtp() {
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::bandwidth_efficient(AmrVariant::WideBand))
                .unwrap();
        let payloads = freeswitch_payloads();
        assert_eq!(payloads.len(), 50, "fixture should carry 50 payloads");

        for (index, payload) in payloads.iter().enumerate() {
            // 4-bit CMR + 6-bit ToC + 477 speech bits = 487 bits = 61 octets.
            // FreeSWITCH independently produces exactly the size our mode
            // table predicts.
            assert_eq!(payload.len(), 61, "payload {index}");

            let packet = codec
                .unpack(payload)
                .unwrap_or_else(|e| panic!("payload {index} failed to parse: {e}"));

            assert_eq!(packet.frames.len(), 1, "payload {index}");
            let frame = &packet.frames[0];
            let AmrFrameType::Speech(mode) = frame.frame_type else {
                panic!("payload {index} was not a speech frame: {:?}", frame.frame_type);
            };
            assert_eq!(mode.index(), 8, "payload {index}: expected mode 8 (23.85)");
            assert_eq!(mode.bits(), 477);
            assert!(frame.quality_ok, "payload {index}: Q bit clear");
            assert_eq!(frame.data.len(), 60, "payload {index}: speech octets");

            // No mode request: CMR 15 means the peer wants nothing.
            assert_eq!(packet.cmr, None, "payload {index}");
        }
    }

    #[test]
    fn real_payloads_repack_to_the_same_bytes() {
        // Stronger than a parse: our packetizer must reproduce the peer's
        // exact octets from the parsed form. Any disagreement about bit
        // placement or padding shows up here.
        let codec =
            AmrPayloadCodec::new(AmrPayloadConfig::bandwidth_efficient(AmrVariant::WideBand))
                .unwrap();
        for (index, payload) in freeswitch_payloads().iter().enumerate() {
            let packet = codec.unpack(payload).unwrap();
            let repacked = codec.pack(&packet).unwrap();
            assert_eq!(&repacked, payload, "payload {index} did not round-trip");
        }
    }

    #[test]
    fn real_payloads_carry_genuine_encoder_output() {
        // Guards the fixture itself. If a future re-capture accidentally
        // records pass-through instead of transcoded audio, every payload
        // becomes identical and the two tests above would still pass while
        // proving much less.
        let payloads = freeswitch_payloads();
        let distinct: std::collections::HashSet<&Vec<u8>> = payloads.iter().collect();
        assert!(
            distinct.len() > payloads.len() / 2,
            "fixture looks like pass-through, not encoder output: only {} distinct of {}",
            distinct.len(),
            payloads.len()
        );
        // And the speech bits are not all zero.
        assert!(payloads.iter().all(|p| p[2..].iter().any(|&b| b != 0)));
    }

    #[test]
    fn real_payloads_are_rejected_as_octet_aligned() {
        // The capture is bandwidth-efficient (the peer offered
        // octet-align=0). Parsing it as octet-aligned must not quietly
        // succeed — that is the interop failure mode this format has.
        let oa = AmrPayloadCodec::new(AmrPayloadConfig::octet_aligned(AmrVariant::WideBand))
            .unwrap();
        let mismatched = freeswitch_payloads()
            .iter()
            .filter(|p| oa.unpack(p).is_ok())
            .count();
        assert_eq!(mismatched, 0, "octet-aligned parse accepted BE payloads");
    }

    #[test]
    fn never_panics_on_arbitrary_input() {
        // Cheap structured sweep; the real fuzz target lives in crates/media/fuzz.
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            let configs = [
                AmrPayloadConfig::bandwidth_efficient(variant),
                AmrPayloadConfig::octet_aligned(variant),
                AmrPayloadConfig::octet_aligned(variant)
                    .with_crc()
                    .with_robust_sorting()
                    .with_interleaving(),
            ];
            for config in configs {
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
