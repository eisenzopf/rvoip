//! AMR / AMR-WB payload format handler (RFC 4867).
//!
//! # This is a compatibility shim, not the real API
//!
//! [`PayloadFormat`] is byte-slice in, byte-slice out and infallible. RFC 4867
//! payloads are none of those things: they carry a codec mode request, a
//! table-of-contents chain describing one *or more* frames, per-frame type and
//! quality bits, and — in bandwidth-efficient mode — no octet alignment at all.
//! None of that survives a `&[u8] -> Bytes` signature.
//!
//! So this type handles only the degenerate case the trait can express: **one
//! speech frame per packet, no mode request**. That covers the common
//! single-frame path used by the existing pipeline and registry, and nothing
//! more.
//!
//! Anything that needs multi-frame packets, CMR-driven rate adaptation, DTX
//! (`NO_DATA` / SID) or loss signalling must use
//! [`codec_core::codecs::amr::AmrPayloadCodec`] directly — reachable here via
//! [`AmrPayloadFormat::codec`]. The relay and transcoding paths will use the
//! typed API; this shim exists so AMR appears in the payload registry
//! alongside the other codecs.
//!
//! Errors are swallowed, because the trait cannot report them: a malformed
//! payload unpacks to empty bytes. That is another reason to prefer the typed
//! API, which returns a `Result` explaining what was wrong.

use super::traits::PayloadFormat;
use bytes::Bytes;
use codec_core::codecs::amr::{
    AmrMode, AmrPacket, AmrPayloadCodec, AmrPayloadConfig, AmrPayloadFrame, AmrVariant,
};
use codec_core::codecs::amr::mode::AmrFrameType;
use std::any::Any;

/// RFC 4867 payload format handler for AMR-NB and AMR-WB.
pub struct AmrPayloadFormat {
    payload_type: u8,
    codec: AmrPayloadCodec,
    /// Mode assumed when sizing packets and when packing raw coded bits.
    mode: AmrMode,
}

impl AmrPayloadFormat {
    /// Create a handler for a dynamic payload type.
    ///
    /// `mode` is the mode used to size packets and to label frames handed to
    /// [`PayloadFormat::pack`], which carries no mode of its own. It should
    /// track the negotiated or currently selected mode.
    ///
    /// # Errors
    ///
    /// Returns an error when `config` is internally inconsistent — for example
    /// requesting CRC without octet alignment — or when `mode` belongs to a
    /// different variant than `config`.
    pub fn new(
        payload_type: u8,
        config: AmrPayloadConfig,
        mode: AmrMode,
    ) -> Result<Self, codec_core::error::CodecError> {
        if mode.variant() != config.variant {
            return Err(codec_core::error::CodecError::invalid_config(format!(
                "{} mode cannot be used with a {} payload format",
                mode.variant(),
                config.variant
            )));
        }
        Ok(Self {
            payload_type,
            codec: AmrPayloadCodec::new(config)?,
            mode,
        })
    }

    /// Create a handler with default parameters: bandwidth-efficient framing
    /// (the RFC 4867 default) at the variant's highest mode.
    ///
    /// # Errors
    ///
    /// Returns an error only if the variant has no modes, which cannot happen.
    pub fn with_defaults(
        payload_type: u8,
        variant: AmrVariant,
    ) -> Result<Self, codec_core::error::CodecError> {
        let top = variant.speech_mode_count() - 1;
        Self::new(
            payload_type,
            AmrPayloadConfig::bandwidth_efficient(variant),
            AmrMode::new(variant, top)?,
        )
    }

    /// The underlying typed codec, for callers that need the full RFC 4867
    /// surface rather than the single-frame shim.
    pub fn codec(&self) -> &AmrPayloadCodec {
        &self.codec
    }

    /// The variant this handler carries.
    pub fn variant(&self) -> AmrVariant {
        self.codec.config().variant
    }

    /// The mode used for sizing and for labelling packed frames.
    pub fn mode(&self) -> AmrMode {
        self.mode
    }

    /// Set the mode used for sizing and labelling.
    ///
    /// # Errors
    ///
    /// Returns an error when `mode` belongs to a different variant.
    pub fn set_mode(&mut self, mode: AmrMode) -> Result<(), codec_core::error::CodecError> {
        if mode.variant() != self.variant() {
            return Err(codec_core::error::CodecError::invalid_config(format!(
                "cannot set a {} mode on a {} payload format",
                mode.variant(),
                self.variant()
            )));
        }
        self.mode = mode;
        Ok(())
    }
}

impl PayloadFormat for AmrPayloadFormat {
    fn payload_type(&self) -> u8 {
        self.payload_type
    }

    fn clock_rate(&self) -> u32 {
        // 8000 for AMR, 16000 for AMR-WB. The two are always distinct payload
        // types, so a handler never has to switch between them.
        self.variant().clock_rate()
    }

    fn channels(&self) -> u8 {
        1
    }

    fn preferred_packet_duration(&self) -> u32 {
        20
    }

    fn packet_size_from_duration(&self, duration_ms: u32) -> usize {
        // AMR frames are fixed at 20 ms, so a packet holds ceil(duration / 20)
        // of them. Sizing assumes the current mode throughout.
        let frames = (duration_ms as usize).div_ceil(20).max(1);
        let frame_type = AmrFrameType::Speech(self.mode);
        let packet = AmrPacket {
            cmr: None,
            interleaving: None,
            frames: (0..frames)
                .map(|_| AmrPayloadFrame {
                    frame_type,
                    quality_ok: true,
                    data: vec![0u8; frame_type.octet_aligned_bytes()],
                })
                .collect(),
        };
        // pack only fails on inputs we just constructed correctly.
        self.codec.pack(&packet).map_or(0, |bytes| bytes.len())
    }

    fn duration_from_packet_size(&self, packet_size: usize) -> u32 {
        // Derived by counting frames rather than assumed, since a packet may
        // hold several. Falls back to one frame when the size matches nothing.
        for frames in 1..=8u32 {
            if self.packet_size_from_duration(frames * 20) == packet_size {
                return frames * 20;
            }
        }
        20
    }

    fn pack(&self, media_data: &[u8], _timestamp: u32) -> Bytes {
        // media_data is one frame of coded bits in the current mode, in the
        // left-aligned convention: `mode.bits()` bits starting at bit 7 of byte
        // 0, with any trailing bits of the final byte zero. A caller that
        // leaves junk in those trailing bits will not get it back, because only
        // `mode.bits()` bits are carried -- the mode's bit count, not the byte
        // count, is what defines the frame.
        //
        // Anything other than exactly one frame cannot be expressed through
        // this signature.
        let frame_type = AmrFrameType::Speech(self.mode);
        if media_data.len() != frame_type.octet_aligned_bytes() {
            return Bytes::new();
        }
        let Ok(frame) = AmrPayloadFrame::new(frame_type, true, media_data.to_vec()) else {
            return Bytes::new();
        };
        self.codec
            .pack(&AmrPacket::single(frame))
            .map_or_else(|_| Bytes::new(), Bytes::from)
    }

    fn unpack(&self, payload: &[u8], _timestamp: u32) -> Bytes {
        // Returns the first frame's coded bits. A multi-frame packet loses
        // every frame after the first, and a malformed one yields empty bytes —
        // both are limits of the trait, not of the parser underneath.
        self.codec.unpack(payload).map_or_else(
            |_| Bytes::new(),
            |packet| {
                packet
                    .frames
                    .first()
                    .map_or_else(Bytes::new, |frame| Bytes::from(frame.data.clone()))
            },
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wb_format() -> AmrPayloadFormat {
        AmrPayloadFormat::with_defaults(96, AmrVariant::WideBand).unwrap()
    }

    #[test]
    fn reports_the_variant_clock_rate() {
        assert_eq!(wb_format().clock_rate(), 16_000);
        let nb = AmrPayloadFormat::with_defaults(97, AmrVariant::NarrowBand).unwrap();
        assert_eq!(nb.clock_rate(), 8_000);
        assert_eq!(nb.channels(), 1);
        assert_eq!(nb.preferred_packet_duration(), 20);
    }

    /// Coded bits in the left-aligned convention: `bits` bits from bit 7 of
    /// byte 0, trailing bits of the final byte zeroed.
    fn coded_frame(mode: AmrMode, seed: u8) -> Vec<u8> {
        let bits = mode.bits();
        let mut data: Vec<u8> = (0..bits.div_ceil(8))
            .map(|i| (i as u8).wrapping_mul(23).wrapping_add(seed) | 0x40)
            .collect();
        let tail = bits % 8;
        if tail != 0 {
            let last = data.len() - 1;
            data[last] &= 0xFFu8 << (8 - tail);
        }
        data
    }

    #[test]
    fn single_frame_round_trip_through_the_shim() {
        let fmt = wb_format();
        let coded = coded_frame(fmt.mode(), 5);

        let payload = fmt.pack(&coded, 0);
        assert!(!payload.is_empty());
        assert_eq!(fmt.unpack(&payload, 0).as_ref(), coded.as_slice());
    }

    #[test]
    fn round_trips_every_mode_in_both_variants() {
        for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
            for mode in AmrMode::all(variant) {
                let mut fmt = AmrPayloadFormat::with_defaults(96, variant).unwrap();
                fmt.set_mode(mode).unwrap();
                let coded = coded_frame(mode, 9);
                let payload = fmt.pack(&coded, 0);
                assert_eq!(fmt.unpack(&payload, 0).as_ref(), coded.as_slice(), "{mode}");
            }
        }
    }

    #[test]
    fn trailing_bits_beyond_the_mode_bit_count_are_not_carried() {
        // A frame is `mode.bits()` bits, not `octet_aligned_bytes()` bytes.
        // AMR-WB mode 8 is 477 bits in 60 octets, so the last 3 bits of the
        // final octet are padding and do not survive a round trip. Asserting
        // this pins the convention rather than leaving it to be rediscovered.
        let fmt = wb_format();
        assert_eq!(fmt.mode().bits() % 8, 5);

        let mut coded = coded_frame(fmt.mode(), 1);
        let last = coded.len() - 1;
        coded[last] |= 0b0000_0111; // junk in the padding bits
        let back = fmt.unpack(&fmt.pack(&coded, 0), 0);
        assert_eq!(back[last], coded[last] & 0b1111_1000);
    }

    #[test]
    fn packet_sizing_matches_what_pack_produces() {
        let fmt = wb_format();
        let bits = AmrFrameType::Speech(fmt.mode()).octet_aligned_bytes();
        let coded = vec![0u8; bits];
        let packed = fmt.pack(&coded, 0);
        assert_eq!(fmt.packet_size_from_duration(20), packed.len());
        assert_eq!(fmt.duration_from_packet_size(packed.len()), 20);
    }

    #[test]
    fn malformed_input_yields_empty_rather_than_panicking() {
        // The trait has no way to report an error, so empty is the only
        // signal available. This is why the typed API is preferred.
        let fmt = wb_format();
        assert!(fmt.unpack(&[], 0).is_empty());
        assert!(fmt.unpack(&[0xFF; 3], 0).is_empty());
        // Wrong-sized input to pack.
        assert!(fmt.pack(&[0u8; 3], 0).is_empty());
    }

    #[test]
    fn mode_changes_resize_packets() {
        let mut fmt = wb_format();
        let big = fmt.packet_size_from_duration(20);
        fmt.set_mode(AmrMode::new(AmrVariant::WideBand, 0).unwrap())
            .unwrap();
        let small = fmt.packet_size_from_duration(20);
        assert!(small < big, "mode 0 packets must be smaller than mode 8");
    }

    #[test]
    fn cross_variant_modes_are_rejected() {
        let nb_mode = AmrMode::new(AmrVariant::NarrowBand, 0).unwrap();
        assert!(AmrPayloadFormat::new(
            96,
            AmrPayloadConfig::bandwidth_efficient(AmrVariant::WideBand),
            nb_mode
        )
        .is_err());

        let mut fmt = wb_format();
        assert!(fmt.set_mode(nb_mode).is_err());
    }

    #[test]
    fn the_typed_codec_is_reachable_for_everything_the_shim_cannot_do() {
        // Multi-frame packets, CMR and DTX all need the real API.
        let fmt = wb_format();
        let codec = fmt.codec();
        let frame_type = AmrFrameType::Speech(fmt.mode());
        let frame = AmrPayloadFrame::new(
            frame_type,
            true,
            vec![0u8; frame_type.octet_aligned_bytes()],
        )
        .unwrap();
        let packet = AmrPacket {
            cmr: Some(2),
            interleaving: None,
            frames: vec![frame.clone(), AmrPayloadFrame::no_data(), frame],
        };
        let bytes = codec.pack(&packet).unwrap();
        assert_eq!(codec.unpack(&bytes).unwrap(), packet);

        // The shim sees only the first frame of that same payload.
        assert_eq!(fmt.unpack(&bytes, 0).len(), frame_type.octet_aligned_bytes());
    }
}
