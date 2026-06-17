//! RFC 4733 telephone-event DTMF over WebRTC RTP tracks.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rtc::rtp;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::TrackLocal;

use crate::errors::{Result, WebRtcError};
pub use crate::peer::builder::TELEPHONE_EVENT_PAYLOAD_TYPE;
use crate::peer::RvoipPeerConnection;

const TICK: Duration = Duration::from_millis(20);
const SAMPLES_PER_TICK: u16 = 160;
const END_OF_EVENT_RETRANSMITS: usize = 3;
const DEFAULT_VOLUME: u8 = 10;

/// Map a DTMF digit character to its RFC 4733 event code.
fn digit_to_event(digit: char) -> Option<u8> {
    match digit {
        '0'..='9' => Some(digit as u8 - b'0'),
        '*' => Some(10),
        '#' => Some(11),
        'A' | 'a' => Some(12),
        'B' | 'b' => Some(13),
        'C' | 'c' => Some(14),
        'D' | 'd' => Some(15),
        _ => None,
    }
}

fn encode_telephone_event(event: u8, end_of_event: bool, volume: u8, duration: u16) -> [u8; 4] {
    let e_bit = if end_of_event { 0b1000_0000 } else { 0 };
    let byte1 = e_bit | (volume & 0b0011_1111);
    let dur = duration.to_be_bytes();
    [event, byte1, dur[0], dur[1]]
}

fn event_to_digit(event: u8) -> Option<char> {
    match event {
        0..=9 => Some(char::from(b'0' + event)),
        10 => Some('*'),
        11 => Some('#'),
        12 => Some('A'),
        13 => Some('B'),
        14 => Some('C'),
        15 => Some('D'),
        _ => None,
    }
}

/// Parsed RFC 4733 telephone-event payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TelephoneEventFrame {
    pub event: u8,
    pub digit: char,
    pub end_of_event: bool,
    pub volume: u8,
    pub duration_samples: u16,
    pub duration_ms: u32,
}

/// Normalized receive-side DTMF event emitted by the inbound RTP pump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedDtmfEvent {
    pub digit: char,
    pub duration_ms: u32,
}

/// Decode the 4-byte RFC 4733 telephone-event payload.
pub fn decode_telephone_event_payload(payload: &[u8]) -> Option<TelephoneEventFrame> {
    if payload.len() < 4 {
        return None;
    }
    let event = payload[0];
    let digit = event_to_digit(event)?;
    let end_of_event = payload[1] & 0b1000_0000 != 0;
    let volume = payload[1] & 0b0011_1111;
    let duration_samples = u16::from_be_bytes([payload[2], payload[3]]);
    let duration_ms = ((duration_samples as u32) * 1000 + 7_999) / 8_000;
    Some(TelephoneEventFrame {
        event,
        digit,
        end_of_event,
        volume,
        duration_samples,
        duration_ms,
    })
}

/// Stateful RFC 4733 receive decoder.
///
/// Telephone events are retransmitted, especially the final end-of-event
/// packet. The decoder emits only once per `(rtp_timestamp, event)` and only
/// when the end bit is present, so consumers receive a normalized digit
/// duration instead of every low-level retransmission.
#[derive(Default)]
pub struct DtmfDecoder {
    emitted: HashSet<(u32, u8)>,
}

impl DtmfDecoder {
    pub fn decode_packet(
        &mut self,
        timestamp: u32,
        payload_type: u8,
        payload: &[u8],
    ) -> Option<DecodedDtmfEvent> {
        if payload_type != TELEPHONE_EVENT_PAYLOAD_TYPE {
            return None;
        }
        let frame = decode_telephone_event_payload(payload)?;
        if !frame.end_of_event || !self.emitted.insert((timestamp, frame.event)) {
            return None;
        }
        Some(DecodedDtmfEvent {
            digit: frame.digit,
            duration_ms: frame.duration_ms,
        })
    }
}

async fn write_telephone_event(
    track: &Arc<TrackLocalStaticRTP>,
    seq: &AtomicU16,
    ssrc: u32,
    event: u8,
    end_of_event: bool,
    volume: u8,
    duration: u16,
    timestamp: u32,
    marker: bool,
) -> Result<()> {
    let payload = encode_telephone_event(event, end_of_event, volume, duration);
    let pkt = rtp::Packet {
        header: rtp::Header {
            version: 2,
            padding: false,
            extension: false,
            marker,
            payload_type: TELEPHONE_EVENT_PAYLOAD_TYPE,
            sequence_number: seq.fetch_add(1, Ordering::Relaxed),
            timestamp,
            ssrc,
            ..Default::default()
        },
        payload: bytes::Bytes::copy_from_slice(&payload),
    };
    track
        .write_rtp(pkt)
        .await
        .map_err(|e| WebRtcError::Webrtc(format!("DTMF write_rtp: {e}")))
}

async fn send_single_digit(
    track: &Arc<TrackLocalStaticRTP>,
    ssrc: u32,
    digit: char,
    duration_ms: u32,
) -> Result<()> {
    let event_code = digit_to_event(digit)
        .ok_or_else(|| WebRtcError::Adapter(format!("invalid DTMF digit '{digit}'")))?;

    let seq = AtomicU16::new(1);
    let start_timestamp = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_millis()
        % u32::MAX) as u32
        | 1;

    let total_ticks = (duration_ms / 20).max(1);
    let mut duration_samples = SAMPLES_PER_TICK;

    write_telephone_event(
        track,
        &seq,
        ssrc,
        event_code,
        false,
        DEFAULT_VOLUME,
        duration_samples,
        start_timestamp,
        true,
    )
    .await?;

    let continuation_count = total_ticks.saturating_sub(2);
    for _ in 0..continuation_count {
        tokio::time::sleep(TICK).await;
        duration_samples = duration_samples.saturating_add(SAMPLES_PER_TICK);
        write_telephone_event(
            track,
            &seq,
            ssrc,
            event_code,
            false,
            DEFAULT_VOLUME,
            duration_samples,
            start_timestamp,
            false,
        )
        .await?;
    }

    tokio::time::sleep(TICK).await;
    duration_samples = duration_samples.saturating_add(SAMPLES_PER_TICK);
    for _ in 0..END_OF_EVENT_RETRANSMITS {
        write_telephone_event(
            track,
            &seq,
            ssrc,
            event_code,
            true,
            DEFAULT_VOLUME,
            duration_samples,
            start_timestamp,
            false,
        )
        .await?;
    }

    Ok(())
}

/// Send one or more DTMF digits using RFC 4733 telephone-event on PT 101.
///
/// D1 — prefers the dedicated DTMF track (separate SSRC, advertised in SDP
/// alongside the Opus track) so PT 101 packets survive SRTP filtering on the
/// remote. Falls back to the Opus audio track if no DTMF track was attached
/// (e.g. when an older `add_local_audio_track` call ran before D1 landed).
pub async fn send_dtmf(
    peer: &Arc<RvoipPeerConnection>,
    digits: &str,
    duration_ms: u32,
) -> Result<()> {
    let (track, ssrc) = match (peer.local_dtmf_track(), peer.local_dtmf_ssrc()) {
        (Some(track), Some(ssrc)) => (track, ssrc),
        _ => {
            let track = peer
                .local_audio_track()
                .ok_or_else(|| WebRtcError::Adapter("no local audio track for DTMF".into()))?;
            let ssrc = peer
                .local_audio_ssrc()
                .ok_or_else(|| WebRtcError::Adapter("no local audio SSRC for DTMF".into()))?;
            (track, ssrc)
        }
    };

    let duration_ms = duration_ms.clamp(40, 6000);

    for digit in digits.chars().filter(|c| !c.is_whitespace()) {
        send_single_digit(&track, ssrc, digit, duration_ms).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_mapping_matches_rfc4733() {
        assert_eq!(digit_to_event('5'), Some(5));
        assert_eq!(digit_to_event('#'), Some(11));
        assert_eq!(digit_to_event('x'), None);
        assert_eq!(event_to_digit(10), Some('*'));
        assert_eq!(event_to_digit(15), Some('D'));
        assert_eq!(event_to_digit(16), None);
    }

    #[test]
    fn telephone_event_payload_layout() {
        let wire = encode_telephone_event(1, true, 10, 800);
        assert_eq!(wire, [1, 0b1000_1010, 0x03, 0x20]);
    }

    #[test]
    fn decode_telephone_event_payload_normalizes_duration() {
        let wire = encode_telephone_event(11, true, 10, 800);
        let decoded = decode_telephone_event_payload(&wire).expect("decode");
        assert_eq!(decoded.digit, '#');
        assert!(decoded.end_of_event);
        assert_eq!(decoded.volume, 10);
        assert_eq!(decoded.duration_samples, 800);
        assert_eq!(decoded.duration_ms, 100);
    }

    #[test]
    fn decoder_emits_only_once_per_final_event() {
        let mut decoder = DtmfDecoder::default();
        let progress = encode_telephone_event(5, false, 10, 160);
        assert_eq!(
            decoder.decode_packet(123, TELEPHONE_EVENT_PAYLOAD_TYPE, &progress),
            None
        );

        let final_payload = encode_telephone_event(5, true, 10, 800);
        let event = decoder
            .decode_packet(123, TELEPHONE_EVENT_PAYLOAD_TYPE, &final_payload)
            .expect("final event");
        assert_eq!(event.digit, '5');
        assert_eq!(event.duration_ms, 100);

        assert_eq!(
            decoder.decode_packet(123, TELEPHONE_EVENT_PAYLOAD_TYPE, &final_payload),
            None,
            "RFC 4733 final retransmit should be suppressed"
        );
    }
}
