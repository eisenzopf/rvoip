//! RFC 4733 telephone-event DTMF over the local audio `RtpSender` track.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rtc::rtp;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::TrackLocal;

use crate::errors::{Result, WebRtcError};
use crate::peer::builder::TELEPHONE_EVENT_PAYLOAD_TYPE;
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
pub async fn send_dtmf(
    peer: &Arc<RvoipPeerConnection>,
    digits: &str,
    duration_ms: u32,
) -> Result<()> {
    let track = peer
        .local_audio_track()
        .ok_or_else(|| WebRtcError::Adapter("no local audio track for DTMF".into()))?;

    let ssrc = peer
        .local_audio_ssrc()
        .ok_or_else(|| WebRtcError::Adapter("no local audio SSRC for DTMF".into()))?;

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
    }

    #[test]
    fn telephone_event_payload_layout() {
        let wire = encode_telephone_event(1, true, 10, 800);
        assert_eq!(wire, [1, 0b1000_1010, 0x03, 0x20]);
    }
}
