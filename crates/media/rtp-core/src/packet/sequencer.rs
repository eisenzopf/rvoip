//! Session/transport-independent RTP packet construction.
//!
//! [`RtpPacketSequencer`] builds successive [`RtpPacket`]s for one
//! synchronization source without an [`RtpSession`](crate::session::RtpSession),
//! a socket, a channel, or the async runtime — callers that already own their
//! own I/O (e.g. a `mio` reactor) can use it to get correct SSRC/sequence-number
//! bookkeeping without adopting the library's transport.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use bytes::Bytes;

use super::header::RtpHeader;
use super::rtp::RtpPacket;
use crate::{RtpSequenceNumber, RtpSsrc, RtpTimestamp};

/// Builds successive RTP packets for one SSRC.
///
/// Maintains a single monotonic sequence-number space shared by every
/// payload type packetized through the same instance — audio and RFC 4733
/// `telephone-event` on the same track are expected to share one instance,
/// per RFC 3550 §5.1. The sequence number wraps at `u16::MAX`.
///
/// Timestamps are supplied by the caller on every call; this type has no
/// opinion on clock rate, codec, or per-media-type timestamp policy (RFC
/// 4733 events, for example, keep the same timestamp across every packet
/// of one event while the sequence number keeps advancing).
#[derive(Debug, Clone)]
pub struct RtpPacketSequencer {
    ssrc: RtpSsrc,
    next_sequence: RtpSequenceNumber,
}

impl RtpPacketSequencer {
    /// Create a sequencer for `ssrc`, starting at `initial_sequence`.
    pub fn new(ssrc: RtpSsrc, initial_sequence: RtpSequenceNumber) -> Self {
        Self {
            ssrc,
            next_sequence: initial_sequence,
        }
    }

    /// SSRC this sequencer stamps on every packet it builds.
    pub fn ssrc(&self) -> RtpSsrc {
        self.ssrc
    }

    /// Sequence number the next call to [`packetize`](Self::packetize) will use.
    pub fn next_sequence(&self) -> RtpSequenceNumber {
        self.next_sequence
    }

    /// Build the next RTP packet in this SSRC's sequence-number space.
    ///
    /// Does not send the packet — serialization (see
    /// [`RtpPacket::serialize`]) and transport are entirely up to the caller.
    pub fn packetize(
        &mut self,
        payload_type: u8,
        timestamp: RtpTimestamp,
        marker: bool,
        payload: Bytes,
    ) -> RtpPacket {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        build_packet(
            payload_type,
            sequence,
            timestamp,
            self.ssrc,
            marker,
            payload,
        )
    }
}

/// A cheaply-clonable, thread-safe variant of [`RtpPacketSequencer`].
///
/// [`RtpPacketSequencer::packetize`] takes `&mut self`, which is fine when
/// a single task owns the stream, but doesn't fit a caller where two
/// concurrent producers (for example an audio sender and a DTMF sender)
/// need to packetize into the *same* SSRC and sequence-number space.
/// `SharedRtpPacketSequencer` reserves each sequence number with a single
/// atomic fetch-add, the same lock-free mechanism
/// [`RtpSendHandle`](crate::session::RtpSendHandle) already uses
/// internally, so packetizing never blocks on a mutex.
///
/// Reserving a sequence number atomically only guarantees the number
/// itself is handed out exactly once, in increasing order of reservation.
/// It does not guarantee that packets reach the wire in that same order if
/// multiple callers race to send right after packetizing; a caller that
/// needs strict wire ordering across concurrent producers still needs to
/// serialize the actual send (e.g. funnel through one channel), same as
/// callers of `RtpSendHandle` already have to.
#[derive(Debug, Clone)]
pub struct SharedRtpPacketSequencer {
    ssrc: RtpSsrc,
    next_sequence: Arc<AtomicU16>,
}

impl SharedRtpPacketSequencer {
    /// Create a sequencer for `ssrc`, starting at `initial_sequence`.
    pub fn new(ssrc: RtpSsrc, initial_sequence: RtpSequenceNumber) -> Self {
        Self {
            ssrc,
            next_sequence: Arc::new(AtomicU16::new(initial_sequence)),
        }
    }

    /// SSRC this sequencer stamps on every packet it builds.
    pub fn ssrc(&self) -> RtpSsrc {
        self.ssrc
    }

    /// Sequence number the next call to [`packetize`](Self::packetize) will
    /// reserve. Racy the instant it's read if other clones are packetizing
    /// concurrently; useful for diagnostics, not for predicting the next
    /// value.
    pub fn next_sequence(&self) -> RtpSequenceNumber {
        self.next_sequence.load(Ordering::Relaxed)
    }

    /// Atomically reserve the next sequence number and build the packet.
    /// Safe to call concurrently from clones of the same sequencer; see
    /// the type-level docs for what concurrent calls do and don't
    /// guarantee about wire order.
    pub fn packetize(
        &self,
        payload_type: u8,
        timestamp: RtpTimestamp,
        marker: bool,
        payload: Bytes,
    ) -> RtpPacket {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        build_packet(
            payload_type,
            sequence,
            timestamp,
            self.ssrc,
            marker,
            payload,
        )
    }
}

/// Header + packet construction shared by [`RtpPacketSequencer::packetize`]
/// and the session-internal send paths, so the two can't drift apart.
pub(crate) fn build_packet(
    payload_type: u8,
    sequence_number: RtpSequenceNumber,
    timestamp: RtpTimestamp,
    ssrc: RtpSsrc,
    marker: bool,
    payload: Bytes,
) -> RtpPacket {
    let mut header = RtpHeader::new(payload_type, sequence_number, timestamp, ssrc);
    header.marker = marker;
    RtpPacket::new(header, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_packet_uses_given_ssrc_and_sequence() {
        let mut seq = RtpPacketSequencer::new(0xabcdef01, 1000);
        let packet = seq.packetize(96, 12345, false, Bytes::from_static(b"payload"));

        assert_eq!(packet.header.ssrc, 0xabcdef01);
        assert_eq!(packet.header.sequence_number, 1000);
        assert_eq!(packet.header.payload_type, 96);
        assert_eq!(packet.header.timestamp, 12345);
        assert!(!packet.header.marker);
        assert_eq!(packet.payload, Bytes::from_static(b"payload"));
    }

    #[test]
    fn sequence_increments_across_payload_types() {
        let mut seq = RtpPacketSequencer::new(1, 0);

        let audio = seq.packetize(0, 100, false, Bytes::new());
        let dtmf = seq.packetize(101, 100, true, Bytes::new());
        let audio2 = seq.packetize(0, 160, false, Bytes::new());

        assert_eq!(audio.header.sequence_number, 0);
        assert_eq!(dtmf.header.sequence_number, 1);
        assert_eq!(audio2.header.sequence_number, 2);
        // Both packets share one SSRC/sequence space regardless of PT.
        assert_eq!(audio.header.ssrc, dtmf.header.ssrc);
    }

    #[test]
    fn sequence_wraps_from_max_to_zero() {
        let mut seq = RtpPacketSequencer::new(1, u16::MAX);

        let last = seq.packetize(0, 0, false, Bytes::new());
        let wrapped = seq.packetize(0, 0, false, Bytes::new());

        assert_eq!(last.header.sequence_number, u16::MAX);
        assert_eq!(wrapped.header.sequence_number, 0);
    }

    #[test]
    fn marker_and_timestamp_are_preserved() {
        let mut seq = RtpPacketSequencer::new(1, 0);
        let packet = seq.packetize(8, 999, true, Bytes::new());

        assert!(packet.header.marker);
        assert_eq!(packet.header.timestamp, 999);
    }

    #[test]
    fn serialize_parse_roundtrip_preserves_header_and_payload() {
        let mut seq = RtpPacketSequencer::new(0x11223344, 42);
        let packet = seq.packetize(96, 5000, true, Bytes::from_static(b"roundtrip"));

        let bytes = packet.serialize().expect("serialize");
        let parsed = RtpPacket::parse(&bytes).expect("parse");

        assert_eq!(parsed.header, packet.header);
        assert_eq!(parsed.payload, packet.payload);
    }

    #[test]
    fn rfc4733_event_packets_share_timestamp_with_distinct_sequences() {
        let mut seq = RtpPacketSequencer::new(1, 0);
        let event_timestamp = 8000;

        let first = seq.packetize(101, event_timestamp, false, Bytes::from_static(&[1]));
        let middle = seq.packetize(101, event_timestamp, false, Bytes::from_static(&[2]));
        let last = seq.packetize(101, event_timestamp, true, Bytes::from_static(&[3]));

        assert_eq!(first.header.timestamp, event_timestamp);
        assert_eq!(middle.header.timestamp, event_timestamp);
        assert_eq!(last.header.timestamp, event_timestamp);
        assert_eq!(
            [
                first.header.sequence_number,
                middle.header.sequence_number,
                last.header.sequence_number
            ],
            [0, 1, 2]
        );
        assert!(last.header.marker);
    }

    #[test]
    fn ssrc_and_next_sequence_accessors_reflect_state() {
        let mut seq = RtpPacketSequencer::new(7, 500);
        assert_eq!(seq.ssrc(), 7);
        assert_eq!(seq.next_sequence(), 500);

        seq.packetize(0, 0, false, Bytes::new());
        assert_eq!(seq.next_sequence(), 501);
    }

    #[test]
    fn shared_first_packet_uses_given_ssrc_and_sequence() {
        let seq = SharedRtpPacketSequencer::new(0xabcdef01, 1000);
        let packet = seq.packetize(96, 12345, false, Bytes::from_static(b"payload"));

        assert_eq!(packet.header.ssrc, 0xabcdef01);
        assert_eq!(packet.header.sequence_number, 1000);
        assert_eq!(packet.header.payload_type, 96);
        assert_eq!(packet.header.timestamp, 12345);
        assert!(!packet.header.marker);
    }

    #[test]
    fn shared_sequence_wraps_from_max_to_zero() {
        let seq = SharedRtpPacketSequencer::new(1, u16::MAX);

        let last = seq.packetize(0, 0, false, Bytes::new());
        let wrapped = seq.packetize(0, 0, false, Bytes::new());

        assert_eq!(last.header.sequence_number, u16::MAX);
        assert_eq!(wrapped.header.sequence_number, 0);
    }

    #[test]
    fn shared_clones_advance_the_same_sequence_space() {
        // Simulates the audio-sender-plus-DTMF-sender case: two independent
        // clones packetizing into what must behave as one shared SSRC and
        // sequence-number space.
        let audio = SharedRtpPacketSequencer::new(1, 0);
        let dtmf = audio.clone();

        let a1 = audio.packetize(0, 0, false, Bytes::new());
        let d1 = dtmf.packetize(101, 0, false, Bytes::new());
        let a2 = audio.packetize(0, 160, false, Bytes::new());

        assert_eq!(a1.header.ssrc, d1.header.ssrc);
        let mut sequences = [
            a1.header.sequence_number,
            d1.header.sequence_number,
            a2.header.sequence_number,
        ];
        sequences.sort_unstable();
        assert_eq!(
            sequences,
            [0, 1, 2],
            "no duplicate or skipped sequence numbers across clones"
        );
    }

    #[test]
    fn shared_concurrent_packetize_never_reserves_a_duplicate_sequence_number() {
        use std::thread;

        let seq = SharedRtpPacketSequencer::new(1, 0);
        const PACKETS_PER_THREAD: usize = 200;
        const THREADS: usize = 8;

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let seq = seq.clone();
                thread::spawn(move || {
                    (0..PACKETS_PER_THREAD)
                        .map(|_| {
                            seq.packetize(0, 0, false, Bytes::new())
                                .header
                                .sequence_number
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        let mut all_sequences: Vec<u16> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("thread panicked"))
            .collect();
        all_sequences.sort_unstable();

        let expected: Vec<u16> = (0..(THREADS * PACKETS_PER_THREAD) as u16).collect();
        assert_eq!(
            all_sequences, expected,
            "every sequence number in range must be reserved exactly once, wire order is a separate concern"
        );
    }
}
