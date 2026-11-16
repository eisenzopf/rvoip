//! Short packet-loss concealment for inbound 20 ms Opus.
//!
//! The inbound pump forwards Opus codec payloads to a decoder downstream. A
//! lost RTP packet used to vanish without trace: the next received packet
//! followed the previous one directly, so the decoder skipped the missing
//! audio time. When at most [`MAX_CONCEALED_PACKETS`] packets are missing,
//! confirmed by both the RTP sequence gap and the matching RTP clock gap, the
//! pump now forwards one lost-frame indication per missing packet ahead of
//! the next packet: the previous packet's TOC byte with no frame data. RFC
//! 6716 §3.2.1 permits that zero-length frame, and libopus decodes it with
//! its packet-loss concealment for the TOC's frame duration (20 ms).
//!
//! Longer gaps, a clock jump without matching sequence loss, packets that are
//! not one 20 ms frame, and payload type or SSRC changes are not concealed.
//! A packet that arrives at or shortly behind the last forwarded packet of
//! its source is dropped: its audio time was already played or concealed.
//! RFC 4733 telephone-events share the audio sequence space and reset the
//! tracker, so a DTMF interval is never concealed as loss.

use bytes::Bytes;
use rtc::rtp;

/// RTP clock ticks in one 20 ms Opus frame (Opus RTP always uses 48 kHz).
const OPUS_FRAME_TICKS: u32 = 960;

/// Longest loss run that is concealed: 100 ms of 20 ms frames.
pub(crate) const MAX_CONCEALED_PACKETS: u16 = 5;

/// How far behind the last forwarded packet a packet is treated as late
/// (one second of 20 ms frames). Anything further back is taken as a new
/// sequence timeline of the same source and forwarded.
const LATE_PACKET_WINDOW: u16 = 50;

#[derive(Clone, Copy)]
struct LastPacket {
    ssrc: u32,
    payload_type: u8,
    sequence: u16,
    next_timestamp: u32,
    toc: u8,
}

/// A run of lost 20 ms frames to conceal ahead of a received packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConcealmentRun {
    /// RTP timestamp of the first lost frame.
    pub first_timestamp: u32,
    /// Number of lost frames, at most [`MAX_CONCEALED_PACKETS`].
    pub frames: u16,
    /// TOC byte carried by each lost-frame indication.
    pub toc: u8,
}

impl ConcealmentRun {
    /// `(rtp_timestamp, payload)` of each lost-frame indication, in order.
    pub fn frames(self) -> impl Iterator<Item = (u32, Bytes)> {
        (0..u32::from(self.frames)).map(move |index| {
            (
                self.first_timestamp.wrapping_add(index * OPUS_FRAME_TICKS),
                Bytes::copy_from_slice(&[self.toc]),
            )
        })
    }
}

/// What the pump does with a received audio packet.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Admission {
    /// The packet is a duplicate of, or late behind, the last forwarded
    /// packet of its source.
    pub discard: bool,
    /// Lost frames to forward ahead of the packet.
    pub concealment: Option<ConcealmentRun>,
}

/// Per-track loss tracker used by the inbound pump.
#[derive(Default)]
pub(crate) struct OpusLossConcealment {
    last: Option<LastPacket>,
}

/// TOC byte of a single-frame (code 0) 20 ms Opus packet with frame data.
fn single_twenty_ms_frame_toc(payload: &[u8]) -> Option<u8> {
    let toc = *payload.first()?;
    if payload.len() < 2 || toc & 0x03 != 0 {
        return None;
    }
    // RFC 6716 §3.1, Table 2: SILK-only configurations cycle 10/20/40/60 ms,
    // hybrid 10/20 ms and CELT-only 2.5/5/10/20 ms.
    let config = toc >> 3;
    let twenty_ms = match config {
        0..=11 => config % 4 == 1,
        12..=15 => config % 2 == 1,
        _ => config % 4 == 3,
    };
    twenty_ms.then_some(toc)
}

impl OpusLossConcealment {
    /// Forget the last packet, so the next one starts a new timeline.
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Classify `packet`. `negotiated_opus` says whether its SSRC carries
    /// the negotiated Opus codec; any other packet resets the tracker.
    pub fn admit(&mut self, packet: &rtp::Packet, negotiated_opus: bool) -> Admission {
        let Some(toc) = single_twenty_ms_frame_toc(&packet.payload).filter(|_| negotiated_opus)
        else {
            self.reset();
            return Admission::default();
        };
        let current = LastPacket {
            ssrc: packet.header.ssrc,
            payload_type: packet.header.payload_type,
            sequence: packet.header.sequence_number,
            next_timestamp: packet.header.timestamp.wrapping_add(OPUS_FRAME_TICKS),
            toc,
        };
        let mut admission = Admission::default();
        if let Some(last) = self
            .last
            .filter(|last| last.ssrc == current.ssrc && last.payload_type == current.payload_type)
        {
            let behind = last.sequence.wrapping_sub(current.sequence);
            if behind <= LATE_PACKET_WINDOW {
                admission.discard = true;
                return admission;
            }
            let lost = current.sequence.wrapping_sub(last.sequence).wrapping_sub(1);
            if (1..=MAX_CONCEALED_PACKETS).contains(&lost)
                && packet.header.timestamp.wrapping_sub(last.next_timestamp)
                    == u32::from(lost) * OPUS_FRAME_TICKS
            {
                admission.concealment = Some(ConcealmentRun {
                    first_timestamp: last.next_timestamp,
                    frames: lost,
                    toc: last.toc,
                });
            }
        }
        self.last = Some(current);
        admission
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::pump::silent_rtp_packet_for_ssrc;

    /// Silent 20 ms CELT Opus packet (TOC 0xF8) at `sequence`, starting on
    /// frame `frame` of the RTP clock.
    fn packet(sequence: u16, frame: u32) -> rtp::Packet {
        silent_rtp_packet_for_ssrc(7, sequence, 1000 + frame * OPUS_FRAME_TICKS)
    }

    fn concealed(recovery: &mut OpusLossConcealment, packet: &rtp::Packet) -> u16 {
        let admission = recovery.admit(packet, true);
        assert!(!admission.discard);
        admission.concealment.map_or(0, |run| run.frames)
    }

    #[test]
    fn a_short_gap_of_n_packets_yields_n_concealed_frames() {
        for lost in 1..=MAX_CONCEALED_PACKETS {
            let mut recovery = OpusLossConcealment::default();
            assert_eq!(concealed(&mut recovery, &packet(100, 0)), 0);
            let next = 101 + lost;
            let admission = recovery.admit(&packet(next, u32::from(lost) + 1), true);
            let run = admission.concealment.expect("short gap is concealed");
            assert_eq!(run.frames, lost);
            let frames = run.frames().collect::<Vec<_>>();
            assert_eq!(frames.len(), usize::from(lost));
            for (index, (timestamp, payload)) in frames.into_iter().enumerate() {
                assert_eq!(timestamp, 1000 + (index as u32 + 1) * OPUS_FRAME_TICKS);
                assert_eq!(&payload[..], &[0xF8], "TOC only: a lost-frame indication");
            }
        }
    }

    #[test]
    fn a_long_gap_or_mismatched_clock_is_not_concealed() {
        let mut recovery = OpusLossConcealment::default();
        concealed(&mut recovery, &packet(100, 0));
        // Six lost packets (120 ms) exceed the bound.
        assert_eq!(concealed(&mut recovery, &packet(107, 7)), 0);
        // A clock jump without missing sequence numbers is not loss.
        assert_eq!(concealed(&mut recovery, &packet(108, 10)), 0);
        // Missing sequence numbers without the matching clock gap.
        assert_eq!(concealed(&mut recovery, &packet(111, 11)), 0);
        // Contiguous packets conceal nothing.
        assert_eq!(concealed(&mut recovery, &packet(112, 12)), 0);
    }

    #[test]
    fn sequence_and_clock_wrap_are_handled() {
        let mut recovery = OpusLossConcealment::default();
        let start = u32::MAX - 479;
        recovery.admit(&silent_rtp_packet_for_ssrc(7, u16::MAX, start), true);
        let admission = recovery.admit(
            &silent_rtp_packet_for_ssrc(7, 1, start.wrapping_add(2 * OPUS_FRAME_TICKS)),
            true,
        );
        assert_eq!(
            admission.concealment,
            Some(ConcealmentRun {
                first_timestamp: start.wrapping_add(OPUS_FRAME_TICKS),
                frames: 1,
                toc: 0xF8,
            })
        );
    }

    #[test]
    fn late_and_duplicate_packets_are_dropped_but_a_restart_is_not() {
        let mut recovery = OpusLossConcealment::default();
        concealed(&mut recovery, &packet(100, 0));
        assert_eq!(concealed(&mut recovery, &packet(103, 3)), 2);
        // 101 and 102 were concealed; 103 is a duplicate.
        for late in [101, 102, 103] {
            assert!(
                recovery
                    .admit(&packet(late, u32::from(late - 100)), true)
                    .discard
            );
        }
        assert_eq!(concealed(&mut recovery, &packet(104, 4)), 0);
        // A sequence far behind the window is a restarted timeline.
        assert_eq!(concealed(&mut recovery, &packet(20_000, 50)), 0);
        assert_eq!(concealed(&mut recovery, &packet(20_002, 52)), 1);
    }

    #[test]
    fn other_codecs_sources_and_frame_shapes_reset_the_tracker() {
        for mutation in 0..5 {
            let mut recovery = OpusLossConcealment::default();
            recovery.admit(&packet(100, 0), true);
            let mut second = packet(102, 2);
            let mut negotiated = true;
            match mutation {
                0 => negotiated = false,
                1 => second.header.ssrc = 8,
                2 => second.header.payload_type = 112,
                3 => second.payload = Bytes::from_static(&[0xF0, 0xFF, 0xFE]), // 10 ms
                _ => second.payload = Bytes::from_static(&[0xF9, 0xFF, 0xFE]), // two frames
            }
            assert_eq!(recovery.admit(&second, negotiated), Admission::default());
        }
        assert!(single_twenty_ms_frame_toc(&[]).is_none());
        assert!(single_twenty_ms_frame_toc(&[0xF8]).is_none());
        // SILK NB 20 ms, hybrid SWB 20 ms, CELT NB 20 ms.
        for toc in [0x08, 0x68, 0x98] {
            assert_eq!(single_twenty_ms_frame_toc(&[toc, 0]), Some(toc));
        }
    }
}
