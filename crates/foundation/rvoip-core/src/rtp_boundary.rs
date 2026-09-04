//! Checked conversion between transport-neutral media frames and RTP packets.
//!
//! A [`MediaFrame`] always contains codec payload bytes, never an RTP packet.
//! This module owns the explicit boundary so gateways do not each invent SSRC,
//! sequence, timestamp, payload-type, extension, or padding behavior.

use crate::stream::{MediaFrame, StreamKind};
use crate::StreamId;
use chrono::{DateTime, Utc};
use rvoip_rtp_core::{RtpHeader, RtpPacket};
use thiserror::Error;

/// Maximum codec payload accepted by the default checked boundary.
pub const DEFAULT_MAX_RTP_PAYLOAD_BYTES: usize = 64 * 1024;

/// Codec identity associated with one negotiated RTP payload type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RtpCodecKind {
    Pcmu,
    Pcma,
    Opus,
    TelephoneEvent,
}

/// Validated negotiated RTP payload-type mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NegotiatedRtpPayload {
    payload_type: u8,
    codec: RtpCodecKind,
    stream_kind: StreamKind,
    clock_rate_hz: u32,
}

impl NegotiatedRtpPayload {
    /// Validate one SDP-negotiated codec/PT association.
    pub fn new(
        payload_type: u8,
        codec: RtpCodecKind,
        clock_rate_hz: u32,
    ) -> Result<Self, RtpBoundaryError> {
        if payload_type > 127 {
            return Err(RtpBoundaryError::InvalidPayloadType);
        }
        let (stream_kind, expected_clock) = match codec {
            RtpCodecKind::Pcmu | RtpCodecKind::Pcma | RtpCodecKind::TelephoneEvent => {
                (StreamKind::Audio, 8_000)
            }
            RtpCodecKind::Opus => (StreamKind::Audio, 48_000),
        };
        if clock_rate_hz != expected_clock {
            return Err(RtpBoundaryError::InvalidClockRate);
        }
        Ok(Self {
            payload_type,
            codec,
            stream_kind,
            clock_rate_hz,
        })
    }

    pub const fn payload_type(self) -> u8 {
        self.payload_type
    }

    pub const fn codec(self) -> RtpCodecKind {
        self.codec
    }

    pub const fn stream_kind(self) -> StreamKind {
        self.stream_kind
    }

    pub const fn clock_rate_hz(self) -> u32 {
        self.clock_rate_hz
    }
}

/// Fixed, value-free failures for checked RTP/frame conversion.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum RtpBoundaryError {
    #[error("RTP payload type is outside the seven-bit range")]
    InvalidPayloadType,
    #[error("RTP clock rate does not match the negotiated codec")]
    InvalidClockRate,
    #[error("RTP packet payload type does not match negotiation")]
    PayloadTypeMismatch,
    #[error("media frame kind does not match negotiation")]
    StreamKindMismatch,
    #[error("media frame payload type does not match negotiation")]
    FramePayloadTypeMismatch,
    #[error("RTP payload exceeds the configured allocation bound")]
    PayloadTooLarge,
    #[error("RTP header invariants are invalid")]
    InvalidHeader,
    #[error("RTP packet changed and cannot use packet-preserving mode")]
    OriginalPacketChanged,
}

/// A transport-neutral frame paired with its immutable original RTP identity.
///
/// Borrow [`Self::frame`] for media processing. Consuming it with
/// [`Self::into_frame`] deliberately gives up packet-preserving re-emission.
#[derive(Clone)]
pub struct DepacketizedRtpFrame {
    frame: MediaFrame,
    original: RtpPacket,
    negotiated: NegotiatedRtpPayload,
}

impl DepacketizedRtpFrame {
    pub fn frame(&self) -> &MediaFrame {
        &self.frame
    }

    pub fn into_frame(self) -> MediaFrame {
        self.frame
    }

    /// Re-emit the exact original packet, including marker, CSRCs, extensions,
    /// padding, SSRC, and sequence number, after revalidating negotiation.
    pub fn preserve_packet(
        &self,
        negotiated: NegotiatedRtpPayload,
    ) -> Result<RtpPacket, RtpBoundaryError> {
        if negotiated != self.negotiated
            || self.frame.payload != self.original.payload
            || self.frame.timestamp_rtp != self.original.header.timestamp
            || self.frame.payload_type != Some(self.original.header.payload_type)
        {
            return Err(RtpBoundaryError::OriginalPacketChanged);
        }
        validate_packet(&self.original, negotiated, DEFAULT_MAX_RTP_PAYLOAD_BYTES)?;
        Ok(self.original.clone())
    }
}

impl std::fmt::Debug for DepacketizedRtpFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DepacketizedRtpFrame")
            .field("stream_kind", &self.frame.kind)
            .field("payload_type", &self.original.header.payload_type)
            .field("sequence_number", &self.original.header.sequence_number)
            .field("timestamp", &self.original.header.timestamp)
            .field("marker", &self.original.header.marker)
            .field("csrc_count", &self.original.header.csrc.len())
            .field(
                "extension_count",
                &self
                    .original
                    .header
                    .extensions
                    .as_ref()
                    .map_or(0, |extensions| extensions.elements.len()),
            )
            .field("payload_bytes", &self.original.payload.len())
            .field("padding_bytes", &self.original.padding_size)
            .finish()
    }
}

/// Convert a validated RTP packet into a payload-only [`MediaFrame`] while
/// retaining an optional exact-packet re-emission handle.
pub fn depacketize_rtp(
    packet: RtpPacket,
    stream_id: StreamId,
    captured_at: DateTime<Utc>,
    negotiated: NegotiatedRtpPayload,
    max_payload_bytes: usize,
) -> Result<DepacketizedRtpFrame, RtpBoundaryError> {
    validate_packet(&packet, negotiated, max_payload_bytes)?;
    let frame = MediaFrame {
        stream_id,
        kind: negotiated.stream_kind(),
        payload: packet.payload.clone(),
        timestamp_rtp: packet.header.timestamp,
        captured_at,
        payload_type: Some(packet.header.payload_type),
    };
    Ok(DepacketizedRtpFrame {
        frame,
        original: packet,
        negotiated,
    })
}

fn validate_packet(
    packet: &RtpPacket,
    negotiated: NegotiatedRtpPayload,
    max_payload_bytes: usize,
) -> Result<(), RtpBoundaryError> {
    if packet.payload.len() > max_payload_bytes {
        return Err(RtpBoundaryError::PayloadTooLarge);
    }
    if packet.header.version != 2
        || packet.header.payload_type > 127
        || packet.header.cc as usize != packet.header.csrc.len()
        || packet.header.csrc.len() > 15
        || packet.header.extension != packet.header.extensions.is_some()
        || packet.header.padding != (packet.padding_size != 0)
    {
        return Err(RtpBoundaryError::InvalidHeader);
    }
    if packet.header.payload_type != negotiated.payload_type() {
        return Err(RtpBoundaryError::PayloadTypeMismatch);
    }
    packet
        .serialize()
        .map(|_| ())
        .map_err(|_| RtpBoundaryError::InvalidHeader)
}

/// Stateful generator for media frames that need a new RTP packet identity.
pub struct RtpPacketizer {
    negotiated: NegotiatedRtpPayload,
    ssrc: u32,
    next_sequence: u16,
    next_timestamp: u32,
    timestamp_step: u32,
    max_payload_bytes: usize,
}

impl RtpPacketizer {
    /// Create a deterministic packet stream. Sequence and timestamp arithmetic
    /// uses RFC 3550 wrapping semantics.
    pub fn new(
        negotiated: NegotiatedRtpPayload,
        ssrc: u32,
        initial_sequence: u16,
        initial_timestamp: u32,
        timestamp_step: u32,
    ) -> Result<Self, RtpBoundaryError> {
        if timestamp_step == 0 || timestamp_step > negotiated.clock_rate_hz() {
            return Err(RtpBoundaryError::InvalidClockRate);
        }
        Ok(Self {
            negotiated,
            ssrc,
            next_sequence: initial_sequence,
            next_timestamp: initial_timestamp,
            timestamp_step,
            max_payload_bytes: DEFAULT_MAX_RTP_PAYLOAD_BYTES,
        })
    }

    pub fn with_max_payload_bytes(mut self, max_payload_bytes: usize) -> Self {
        self.max_payload_bytes = max_payload_bytes;
        self
    }

    /// Packetize one frame and advance state only after all validation passes.
    pub fn packetize(
        &mut self,
        frame: &MediaFrame,
        marker: bool,
    ) -> Result<RtpPacket, RtpBoundaryError> {
        if frame.kind != self.negotiated.stream_kind() {
            return Err(RtpBoundaryError::StreamKindMismatch);
        }
        if frame
            .payload_type
            .is_some_and(|payload_type| payload_type != self.negotiated.payload_type())
        {
            return Err(RtpBoundaryError::FramePayloadTypeMismatch);
        }
        if frame.payload.len() > self.max_payload_bytes {
            return Err(RtpBoundaryError::PayloadTooLarge);
        }
        let mut header = RtpHeader::new(
            self.negotiated.payload_type(),
            self.next_sequence,
            self.next_timestamp,
            self.ssrc,
        );
        header.marker = marker;
        let packet = RtpPacket::new(header, frame.payload.clone());
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.next_timestamp = self.next_timestamp.wrapping_add(self.timestamp_step);
        Ok(packet)
    }

    pub const fn next_sequence(&self) -> u16 {
        self.next_sequence
    }

    pub const fn next_timestamp(&self) -> u32 {
        self.next_timestamp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rvoip_rtp_core::{ExtensionElement, RtpHeaderExtensions};

    fn frame(pt: Option<u8>, kind: StreamKind, payload: Bytes) -> MediaFrame {
        MediaFrame {
            stream_id: StreamId::new(),
            kind,
            payload,
            timestamp_rtp: 7,
            captured_at: Utc::now(),
            payload_type: pt,
        }
    }

    #[test]
    fn preserves_csrc_extensions_padding_marker_and_shared_payload() {
        let mapping = NegotiatedRtpPayload::new(111, RtpCodecKind::Opus, 48_000).unwrap();
        let payload = Bytes::from_static(b"immutable-opus");
        let mut header = RtpHeader::new(111, u16::MAX, u32::MAX - 10, 0x1020_3040);
        header.marker = true;
        header.csrc = vec![7, 8];
        header.cc = 2;
        let mut extensions = RtpHeaderExtensions::new_one_byte();
        extensions
            .elements
            .push(ExtensionElement::new(1, Bytes::from_static(b"mid")));
        header.extension = true;
        header.extensions = Some(extensions);
        let mut packet = RtpPacket::new(header, payload.clone());
        packet.set_padding(4);
        let converted =
            depacketize_rtp(packet.clone(), StreamId::new(), Utc::now(), mapping, 1_200).unwrap();
        assert_eq!(converted.frame().payload, payload);
        let debug = format!("{converted:?}");
        assert!(!debug.contains("immutable-opus"));
        assert!(!debug.contains("mid"));
        assert!(!debug.contains("270544960"), "SSRC must stay redacted");
        assert_eq!(converted.preserve_packet(mapping).unwrap(), packet);
    }

    #[test]
    fn packetizer_is_deterministic_across_rollover_and_rejects_without_advancing() {
        let mapping = NegotiatedRtpPayload::new(101, RtpCodecKind::TelephoneEvent, 8_000).unwrap();
        let mut packetizer = RtpPacketizer::new(mapping, 9, u16::MAX, u32::MAX - 79, 160).unwrap();
        let bad = frame(Some(0), StreamKind::Audio, Bytes::from_static(b"bad"));
        assert_eq!(
            packetizer.packetize(&bad, false).unwrap_err(),
            RtpBoundaryError::FramePayloadTypeMismatch
        );
        assert_eq!(packetizer.next_sequence(), u16::MAX);
        let good = frame(Some(101), StreamKind::Audio, Bytes::from_static(b"dtmf"));
        let first = packetizer.packetize(&good, true).unwrap();
        let second = packetizer.packetize(&good, false).unwrap();
        assert_eq!(
            (first.header.sequence_number, second.header.sequence_number),
            (u16::MAX, 0)
        );
        assert_eq!(
            (first.header.timestamp, second.header.timestamp),
            (u32::MAX - 79, 80)
        );
        assert!(first.header.marker);

        let mut restarted = RtpPacketizer::new(mapping, 9, u16::MAX, u32::MAX - 79, 160).unwrap();
        assert_eq!(restarted.packetize(&good, true).unwrap(), first);
    }

    #[test]
    fn negotiated_mismatch_and_malformed_headers_fail_closed() {
        for mapping in [
            NegotiatedRtpPayload::new(0, RtpCodecKind::Pcmu, 8_000).unwrap(),
            NegotiatedRtpPayload::new(8, RtpCodecKind::Pcma, 8_000).unwrap(),
            NegotiatedRtpPayload::new(120, RtpCodecKind::Opus, 48_000).unwrap(),
            NegotiatedRtpPayload::new(101, RtpCodecKind::TelephoneEvent, 8_000).unwrap(),
        ] {
            let packet = RtpPacket::new_with_payload(
                mapping.payload_type(),
                1,
                2,
                3,
                Bytes::from_static(b"payload"),
            );
            depacketize_rtp(packet, StreamId::new(), Utc::now(), mapping, 1_200).unwrap();
        }
        let opus = NegotiatedRtpPayload::new(111, RtpCodecKind::Opus, 48_000).unwrap();
        let wrong = RtpPacket::new_with_payload(112, 1, 2, 3, Bytes::from_static(b"x"));
        assert_eq!(
            depacketize_rtp(wrong, StreamId::new(), Utc::now(), opus, 1_200).unwrap_err(),
            RtpBoundaryError::PayloadTypeMismatch
        );
        let mut malformed = RtpPacket::new_with_payload(111, 1, 2, 3, Bytes::from_static(b"x"));
        malformed.header.cc = 1;
        assert_eq!(
            depacketize_rtp(malformed, StreamId::new(), Utc::now(), opus, 1_200).unwrap_err(),
            RtpBoundaryError::InvalidHeader
        );

        let oversized = RtpPacket::new_with_payload(111, 1, 2, 3, Bytes::from_static(b"xx"));
        assert_eq!(
            depacketize_rtp(oversized, StreamId::new(), Utc::now(), opus, 1).unwrap_err(),
            RtpBoundaryError::PayloadTooLarge
        );
    }
}
