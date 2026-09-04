//! UCTP media datagram framing per CONVERSATION_PROTOCOL.md §10.1.
//!
//! 8-byte header `ver | flags | stream_local_id (u16 BE) | datagram_seq (u32 BE)`
//! followed by one complete RTP packet (header and codec payload).
//!
//! Use [`pack_rtp_datagram`] and [`unpack_rtp_datagram`] on media paths. The
//! raw [`MediaDatagram`], [`pack`], and [`unpack`] surface remains public only
//! for compatibility with the alpha adapters and does not prove that its
//! opaque payload is a complete RTP packet.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::fmt;

use crate::compatibility::UCTP_DATAGRAM_VERSION;
use crate::errors::SubstrateError;

/// Lossless, parsed RTP observation before UCTP adapters normalize it to a
/// codec-payload `MediaFrame`.
#[derive(Clone, PartialEq, Eq)]
pub struct ObservedRtpPayload {
    packet: rvoip_rtp_core::RtpPacket,
}

impl ObservedRtpPayload {
    /// Inspect every validated RTP header field and the padding-free payload.
    pub fn packet(&self) -> &rvoip_rtp_core::RtpPacket {
        &self.packet
    }

    /// Consume the observation while preserving the complete packet identity.
    pub fn into_packet(self) -> rvoip_rtp_core::RtpPacket {
        self.packet
    }

    /// Normalize to the source-compatible UCTP media shape. Marker, CSRC,
    /// extension, and padding metadata are deliberately not forwarded here.
    pub fn into_media_payload(self) -> RtpMediaPayload {
        RtpMediaPayload {
            payload: self.packet.payload,
            payload_type: self.packet.header.payload_type,
            sequence_number: self.packet.header.sequence_number,
            timestamp: self.packet.header.timestamp,
            ssrc: self.packet.header.ssrc,
        }
    }
}

impl fmt::Debug for ObservedRtpPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObservedRtpPayload")
            .field("payload_type", &self.packet.header.payload_type)
            .field("sequence_number", &self.packet.header.sequence_number)
            .field("timestamp", &self.packet.header.timestamp)
            .field("marker", &self.packet.header.marker)
            .field("csrc_count", &self.packet.header.csrc.len())
            .field(
                "extension_count",
                &self
                    .packet
                    .header
                    .extensions
                    .as_ref()
                    .map_or(0, |extensions| extensions.elements.len()),
            )
            .field("payload_bytes", &self.packet.payload.len())
            .field("padding_bytes", &self.packet.padding_size)
            .finish()
    }
}

/// Lossless UCTP envelope plus parsed RTP observation. UCTP `seq` and RTP
/// sequence remain independently observable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedRtpDatagram {
    pub flags: u8,
    pub stream_local_id: u16,
    pub seq: u32,
    pub rtp: ObservedRtpPayload,
}

/// A lossless RTP observation associated with its authenticated core route.
///
/// QUIC and WebTransport adapters publish this value through their optional
/// bounded ingress receiver before converting the packet into a generic
/// codec-payload `MediaFrame`. Slow receivers may miss observations, but can
/// never delay or interrupt the live media path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RtpIngressObservation {
    pub route: super::PeerMediaRouteKey,
    pub datagram: ObservedRtpDatagram,
}

/// RTP fields recovered from a UCTP media datagram body.
#[derive(Clone, PartialEq, Eq)]
pub struct RtpMediaPayload {
    pub payload: Bytes,
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

impl fmt::Debug for RtpMediaPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RtpMediaPayload")
            .field("payload_bytes", &self.payload.len())
            .field("payload_type", &self.payload_type)
            .finish()
    }
}

/// A validated UCTP media datagram containing one complete RTP packet.
///
/// This is the typed shape applications and adapters should use. The codec
/// bytes belong in [`RtpMediaPayload::payload`]; the RTP header is generated
/// and parsed by [`pack_rtp_datagram`] and [`unpack_rtp_datagram`].
#[derive(Clone, PartialEq, Eq)]
pub struct RtpDatagram {
    pub flags: u8,
    pub stream_local_id: u16,
    pub seq: u32,
    pub rtp: RtpMediaPayload,
}

impl fmt::Debug for RtpDatagram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RtpDatagram")
            .field("flags", &self.flags)
            .field("stream_local_id", &self.stream_local_id)
            .field("sequence", &self.seq)
            .field("rtp", &self.rtp)
            .finish()
    }
}

/// Construct the complete RTP packet required by UCTP §10.1.
///
/// This codec-payload compatibility path deliberately creates a new RTP hop:
/// marker is false, CSRC and extension lists are empty, and no padding is
/// added. Use [`pack_observed_rtp_datagram`] only when an application has
/// explicitly decided that preserving the observed packet identity is safe.
pub fn pack_rtp(
    payload: Bytes,
    payload_type: u8,
    sequence_number: u16,
    timestamp: u32,
    ssrc: u32,
) -> Result<Bytes, SubstrateError> {
    if payload_type > 0x7f {
        return Err(SubstrateError::InvalidDatagram(
            "RTP payload type exceeds 7 bits",
        ));
    }
    rvoip_rtp_core::RtpPacket::new_with_payload(
        payload_type,
        sequence_number,
        timestamp,
        ssrc,
        payload,
    )
    .serialize()
    .map_err(|_| SubstrateError::InvalidDatagram("RTP serialization failed"))
}

/// Parse and validate the complete RTP packet carried after the UCTP header.
pub fn unpack_rtp(payload: Bytes) -> Result<RtpMediaPayload, SubstrateError> {
    Ok(observe_rtp(payload)?.into_media_payload())
}

/// Parse a complete RTP body without discarding any validated header metadata.
pub fn observe_rtp(payload: Bytes) -> Result<ObservedRtpPayload, SubstrateError> {
    let packet = rvoip_rtp_core::RtpPacket::parse_from_bytes(payload)
        .map_err(|_| SubstrateError::InvalidDatagram("invalid RTP packet"))?;
    Ok(ObservedRtpPayload { packet })
}

/// Serialize a typed media datagram as the eight-byte UCTP header followed by
/// one complete RTP packet.
pub fn pack_rtp_datagram(datagram: &RtpDatagram) -> Result<Bytes, SubstrateError> {
    let rtp = pack_rtp(
        datagram.rtp.payload.clone(),
        datagram.rtp.payload_type,
        datagram.rtp.sequence_number,
        datagram.rtp.timestamp,
        datagram.rtp.ssrc,
    )?;
    Ok(pack(&MediaDatagram {
        flags: datagram.flags,
        stream_local_id: datagram.stream_local_id,
        seq: datagram.seq,
        payload: rtp,
    }))
}

/// Parse the eight-byte UCTP header and require its payload to be a complete,
/// valid RTP packet.
pub fn unpack_rtp_datagram(input: &[u8]) -> Result<RtpDatagram, SubstrateError> {
    let observed = observe_rtp_datagram(input)?;
    Ok(RtpDatagram {
        flags: observed.flags,
        stream_local_id: observed.stream_local_id,
        seq: observed.seq,
        rtp: observed.rtp.into_media_payload(),
    })
}

/// Parse the UCTP envelope and expose its complete validated RTP packet before
/// adapter normalization. QUIC and WebTransport share this exact function.
pub fn observe_rtp_datagram(input: &[u8]) -> Result<ObservedRtpDatagram, SubstrateError> {
    let raw = unpack(input)?;
    Ok(ObservedRtpDatagram {
        flags: raw.flags,
        stream_local_id: raw.stream_local_id,
        seq: raw.seq,
        rtp: observe_rtp(raw.payload)?,
    })
}

/// Serialize a lossless observed datagram without regenerating RTP identity.
pub fn pack_observed_rtp_datagram(datagram: &ObservedRtpDatagram) -> Result<Bytes, SubstrateError> {
    let payload = datagram
        .rtp
        .packet
        .serialize()
        .map_err(|_| SubstrateError::InvalidDatagram("RTP serialization failed"))?;
    Ok(pack(&MediaDatagram {
        flags: datagram.flags,
        stream_local_id: datagram.stream_local_id,
        seq: datagram.seq,
        payload,
    }))
}

/// In-memory shape of a UCTP media datagram. Wire layout above.
///
/// This unchecked compatibility shape treats `payload` as opaque bytes. New
/// media code should use [`RtpDatagram`] instead.
#[derive(Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct MediaDatagram {
    pub flags: u8,
    pub stream_local_id: u16,
    pub seq: u32,
    pub payload: Bytes,
}

impl fmt::Debug for MediaDatagram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaDatagram")
            .field("flags", &self.flags)
            .field("stream_local_id", &self.stream_local_id)
            .field("sequence", &self.seq)
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}

/// Serialize a [`MediaDatagram`] to its wire bytes.
///
/// This compatibility helper does not validate that `payload` is RTP. New
/// media code should use [`pack_rtp_datagram`].
#[doc(hidden)]
pub fn pack(d: &MediaDatagram) -> Bytes {
    let mut buf = BytesMut::with_capacity(8 + d.payload.len());
    buf.put_u8(UCTP_DATAGRAM_VERSION);
    buf.put_u8(d.flags);
    buf.put_u16(d.stream_local_id);
    buf.put_u32(d.seq);
    buf.put(&d.payload[..]);
    buf.freeze()
}

/// Parse a wire-bytes datagram back to [`MediaDatagram`].
///
/// Returns [`SubstrateError::InvalidDatagram`] on:
/// - length < 8 bytes
/// - an unsupported `ver` byte
///
/// This compatibility helper leaves `payload` unchecked. New media code
/// should use [`unpack_rtp_datagram`].
#[doc(hidden)]
pub fn unpack(input: &[u8]) -> Result<MediaDatagram, SubstrateError> {
    if input.len() < 8 {
        return Err(SubstrateError::InvalidDatagram("length < 8"));
    }
    let mut b = input;
    let ver = b.get_u8();
    if !crate::compatibility::UCTP_COMPATIBILITY.supports_datagram(ver) {
        return Err(SubstrateError::InvalidDatagram(
            "unsupported datagram version",
        ));
    }
    let flags = b.get_u8();
    let stream_local_id = b.get_u16();
    let seq = b.get_u32();
    let payload = Bytes::copy_from_slice(b);
    Ok(MediaDatagram {
        flags,
        stream_local_id,
        seq,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_rtp_core::{RtpHeader, RtpHeaderExtensions, RtpPacket};

    #[test]
    fn pack_unpack_roundtrip() {
        let d = MediaDatagram {
            flags: 0,
            stream_local_id: 7,
            seq: 12345,
            payload: Bytes::from_static(b"\x80\x60\x00\x01rtp-body"),
        };
        let bytes = pack(&d);
        let d2 = unpack(&bytes).unwrap();
        assert_eq!(d, d2);
    }

    #[test]
    fn rtp_body_roundtrip_is_a_complete_packet() {
        let payload = Bytes::from_static(b"opus-frame");
        let rtp = pack_rtp(payload.clone(), 111, 7, 9_600, 0x1234_5678).unwrap();
        assert_eq!(rtp[0] >> 6, 2, "RTP version must be two");
        let parsed = unpack_rtp(rtp).unwrap();
        assert_eq!(parsed.payload, payload);
        assert_eq!(parsed.payload_type, 111);
        assert_eq!(parsed.sequence_number, 7);
        assert_eq!(parsed.timestamp, 9_600);
        assert_eq!(parsed.ssrc, 0x1234_5678);
    }

    #[test]
    fn observed_ingress_preserves_full_rtp_metadata_and_strips_padding_from_payload() {
        let mut header = RtpHeader::new(111, u16::MAX, 0xffff_ff00, 0x1234_5678);
        header.marker = true;
        header.csrc = vec![0x1111_1111, 0x2222_2222];
        header.cc = 2;
        let mut extensions = RtpHeaderExtensions::new_one_byte();
        extensions
            .add_extension(1, Bytes::from_static(b"vad"))
            .unwrap();
        header.extension = true;
        header.extensions = Some(extensions.clone());
        let mut packet = RtpPacket::new(header, Bytes::from_static(b"codec-only"));
        packet.set_padding(8);
        let wire = pack(&MediaDatagram {
            flags: 0x5a,
            stream_local_id: 7,
            seq: 900,
            payload: packet.serialize().unwrap(),
        });

        let observed = observe_rtp_datagram(&wire).unwrap();
        assert_eq!(observed.seq, 900, "UCTP sequence stays independent");
        let rtp = observed.rtp.packet();
        assert_eq!(rtp.header.sequence_number, u16::MAX);
        assert_eq!(rtp.header.timestamp, 0xffff_ff00);
        assert_eq!(rtp.header.ssrc, 0x1234_5678);
        assert!(rtp.header.marker);
        assert_eq!(rtp.header.csrc, vec![0x1111_1111, 0x2222_2222]);
        assert_eq!(rtp.header.extensions, Some(extensions));
        assert_eq!(rtp.padding_size, 8);
        assert_eq!(rtp.payload, Bytes::from_static(b"codec-only"));
        assert_eq!(pack_observed_rtp_datagram(&observed).unwrap(), wire);
        let debug = format!("{observed:?}");
        assert!(!debug.contains("codec-only"));
        assert!(!debug.contains("vad"));

        let normalized = observed.rtp.into_media_payload();
        assert_eq!(normalized.payload, Bytes::from_static(b"codec-only"));
        assert_eq!(normalized.sequence_number, u16::MAX);
        assert_eq!(normalized.ssrc, 0x1234_5678);
    }

    #[test]
    fn typed_pack_rejects_out_of_range_rtp_payload_type() {
        let datagram = RtpDatagram {
            flags: 0,
            stream_local_id: 1,
            seq: 1,
            rtp: RtpMediaPayload {
                payload: Bytes::from_static(b"audio"),
                payload_type: 128,
                sequence_number: 1,
                timestamp: 1,
                ssrc: 1,
            },
        };
        assert!(matches!(
            pack_rtp_datagram(&datagram),
            Err(SubstrateError::InvalidDatagram(
                "RTP payload type exceeds 7 bits"
            ))
        ));
    }

    #[test]
    fn complete_rtp_datagram_matches_fixed_golden_vector() {
        const GOLDEN: &[u8] = &[
            // UCTP: version, flags, stream_local_id, datagram_seq.
            0x01, 0xa5, 0x12, 0x34, 0x01, 0x02, 0x03, 0x04,
            // RTP: V=2/PT=111, sequence, timestamp, SSRC.
            0x80, 0x6f, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67,
            // RTP codec payload.
            0xde, 0xad, 0xbe, 0xef,
        ];

        let datagram = RtpDatagram {
            flags: 0xa5,
            stream_local_id: 0x1234,
            seq: 0x0102_0304,
            rtp: RtpMediaPayload {
                payload: Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
                payload_type: 111,
                sequence_number: 0x4567,
                timestamp: 0x89ab_cdef,
                ssrc: 0x0123_4567,
            },
        };

        assert_eq!(pack_rtp_datagram(&datagram).unwrap().as_ref(), GOLDEN);
        assert_eq!(unpack_rtp_datagram(GOLDEN).unwrap(), datagram);
    }

    #[test]
    fn normalized_outbound_deliberately_regenerates_hop_specific_rtp_metadata() {
        let datagram = RtpDatagram {
            flags: 0,
            stream_local_id: 9,
            seq: 10,
            rtp: RtpMediaPayload {
                payload: Bytes::from_static(b"codec-payload"),
                payload_type: 111,
                sequence_number: 22,
                timestamp: 960,
                ssrc: 0x1020_3040,
            },
        };

        let observed = observe_rtp_datagram(&pack_rtp_datagram(&datagram).unwrap()).unwrap();
        let packet = observed.rtp.packet();
        assert!(!packet.header.marker);
        assert!(packet.header.csrc.is_empty());
        assert!(packet.header.extensions.is_none());
        assert_eq!(packet.padding_size, 0);
        assert_eq!(packet.payload.as_ref(), b"codec-payload");
    }

    #[test]
    fn typed_unpack_rejects_payload_only_datagram() {
        let payload_only = [
            UCTP_DATAGRAM_VERSION,
            0,
            0,
            1,
            0,
            0,
            0,
            1,
            0xde,
            0xad,
            0xbe,
            0xef,
        ];
        assert!(matches!(
            unpack_rtp_datagram(&payload_only),
            Err(SubstrateError::InvalidDatagram("invalid RTP packet"))
        ));
    }

    #[test]
    fn unpack_rejects_short_input() {
        let err = unpack(b"abc").unwrap_err();
        matches!(err, SubstrateError::InvalidDatagram(_));
    }

    #[test]
    fn unpack_rejects_wrong_version() {
        let mut bad = vec![0u8; 8];
        bad[0] = 9; // bad ver
        let err = unpack(&bad).unwrap_err();
        matches!(err, SubstrateError::InvalidDatagram(_));
    }
}
