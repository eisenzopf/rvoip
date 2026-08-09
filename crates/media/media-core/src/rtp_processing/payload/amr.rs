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
    AmrFmtp, AmrMode, AmrPacket, AmrPayloadCodec, AmrPayloadConfig, AmrPayloadFrame, AmrVariant,
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

    /// Build a handler from a negotiated SDP codec name and `a=fmtp` string.
    ///
    /// This is the path from signalling to the wire. `codec_name` is the
    /// `a=rtpmap` encoding name (`AMR` or `AMR-WB`) and `fmtp` is the raw
    /// parameter string, or `None` when the payload type carried no `a=fmtp`
    /// line — which is not missing information but a positive statement that
    /// every RFC 4867 default applies.
    ///
    /// **Using defaults instead of the negotiated parameters is not a
    /// degraded mode, it is a broken one:** `octet-align` selects the framing,
    /// so guessing it wrong produces a stream the peer cannot parse at all.
    ///
    /// The starting mode is the highest the negotiated `mode-set` permits; a
    /// peer can move it with a codec mode request.
    ///
    /// # Errors
    ///
    /// Returns an error when `codec_name` is not an AMR encoding name, or when
    /// `fmtp` is malformed or requests a configuration this build cannot
    /// produce.
    pub fn from_negotiated(
        payload_type: u8,
        codec_name: &str,
        fmtp: Option<&str>,
    ) -> Result<Self, codec_core::error::CodecError> {
        let variant = if codec_name.eq_ignore_ascii_case("AMR-WB") {
            AmrVariant::WideBand
        } else if codec_name.eq_ignore_ascii_case("AMR") {
            AmrVariant::NarrowBand
        } else {
            return Err(codec_core::error::CodecError::unsupported_codec(codec_name));
        };

        let parsed = AmrFmtp::parse(variant, fmtp.unwrap_or(""))?;
        let mode = parsed
            .active_modes()
            .highest()
            .ok_or_else(|| codec_core::error::CodecError::invalid_config("empty AMR mode-set"))?;
        Self::new(payload_type, parsed.payload_config(), mode)
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

    // ---- signalling -> wire ----

    #[test]
    fn negotiated_octet_align_actually_changes_the_framing() {
        // The reason the fmtp has to reach this layer at all. The two framings
        // produce different bytes for identical content, and a relay that
        // guesses emits a stream the peer cannot parse.
        let be = AmrPayloadFormat::from_negotiated(104, "AMR-WB", None).unwrap();
        let oa =
            AmrPayloadFormat::from_negotiated(105, "AMR-WB", Some("octet-align=1")).unwrap();

        let coded = coded_frame(be.mode(), 3);
        let be_bytes = be.pack(&coded, 0);
        let oa_bytes = oa.pack(&coded, 0);
        assert_ne!(be_bytes, oa_bytes, "framings must differ on the wire");
        assert_eq!(oa_bytes.len(), be_bytes.len() + 1);

        // Each parses its own and not the other's.
        assert_eq!(be.unpack(&be_bytes, 0).as_ref(), coded.as_slice());
        assert_eq!(oa.unpack(&oa_bytes, 0).as_ref(), coded.as_slice());
        assert_ne!(oa.unpack(&be_bytes, 0).as_ref(), coded.as_slice());
    }

    #[test]
    fn absent_fmtp_selects_the_rfc_defaults_rather_than_failing() {
        // No a=fmtp line is a positive statement, not missing data.
        let fmt = AmrPayloadFormat::from_negotiated(104, "AMR-WB", None).unwrap();
        assert_eq!(fmt.variant(), AmrVariant::WideBand);
        assert!(!fmt.codec().config().octet_aligned);
        // All modes allowed, so we start at the top.
        assert_eq!(fmt.mode().index(), 8);
    }

    #[test]
    fn negotiated_mode_set_bounds_the_starting_mode() {
        let fmt =
            AmrPayloadFormat::from_negotiated(104, "AMR-WB", Some("mode-set=0,1,2")).unwrap();
        assert_eq!(fmt.mode().index(), 2, "start at the highest permitted mode");

        let nb = AmrPayloadFormat::from_negotiated(106, "AMR", Some("mode-set=0")).unwrap();
        assert_eq!(nb.variant(), AmrVariant::NarrowBand);
        assert_eq!(nb.mode().index(), 0);
    }

    #[test]
    fn variant_comes_from_the_rtpmap_name_and_is_case_insensitive() {
        for name in ["AMR-WB", "amr-wb", "Amr-Wb"] {
            let fmt = AmrPayloadFormat::from_negotiated(104, name, None).unwrap();
            assert_eq!(fmt.variant(), AmrVariant::WideBand, "{name}");
            assert_eq!(fmt.clock_rate(), 16_000);
        }
        for name in ["AMR", "amr"] {
            let fmt = AmrPayloadFormat::from_negotiated(106, name, None).unwrap();
            assert_eq!(fmt.variant(), AmrVariant::NarrowBand, "{name}");
            assert_eq!(fmt.clock_rate(), 8_000);
        }
    }

    #[test]
    fn non_amr_and_malformed_fmtp_are_rejected() {
        assert!(AmrPayloadFormat::from_negotiated(104, "opus", None).is_err());
        assert!(AmrPayloadFormat::from_negotiated(104, "AMR-WB", Some("octet-align=2")).is_err());
        // Mode 8 does not exist for narrowband.
        assert!(AmrPayloadFormat::from_negotiated(106, "AMR", Some("mode-set=8")).is_err());
        // A configuration this build cannot produce.
        assert!(
            AmrPayloadFormat::from_negotiated(104, "AMR-WB", Some("octet-align=0; crc=1")).is_err()
        );
    }

    #[test]
    fn crc_and_robust_sorting_survive_negotiation_to_the_wire() {
        let fmt = AmrPayloadFormat::from_negotiated(
            104,
            "AMR-WB",
            Some("octet-align=1; crc=1; robust-sorting=1"),
        )
        .unwrap();
        let config = fmt.codec().config();
        assert!(config.octet_aligned && config.crc && config.robust_sorting);

        let coded = coded_frame(fmt.mode(), 7);
        assert_eq!(fmt.unpack(&fmt.pack(&coded, 0), 0).as_ref(), coded.as_slice());
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
