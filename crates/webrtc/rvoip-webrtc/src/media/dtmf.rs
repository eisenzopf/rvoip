//! RFC 4733 telephone-event DTMF over WebRTC RTP tracks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
use rtc::rtp;

use crate::errors::{Result, WebRtcError};
use crate::media::outbound::OutboundAudioRtpWriter;
pub use crate::peer::builder::TELEPHONE_EVENT_PAYLOAD_TYPE;
use crate::peer::RvoipPeerConnection;

const TICK: Duration = Duration::from_millis(20);
const END_OF_EVENT_RETRANSMITS: usize = 3;
const DEFAULT_VOLUME: u8 = 10;
const MIN_DURATION_MS: u32 = 40;
const MAX_DURATION_MS: u32 = 6_000;

/// Negotiated RFC 4733 payload mapping for one WebRTC audio m-section.
///
/// Dynamic payload types and clock rates are selected by SDP negotiation.
/// Browsers are not required to use rvoip's preferred PT 101 / 8 kHz pair;
/// Chromium, for example, commonly offers telephone-event at both 48 kHz and
/// 8 kHz and may select the former. Receive-side classification must therefore
/// use this negotiated mapping rather than a process-wide constant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TelephoneEventCodec {
    pub payload_type: u8,
    pub clock_rate_hz: u32,
}

impl TelephoneEventCodec {
    #[must_use]
    pub const fn new(payload_type: u8, clock_rate_hz: u32) -> Self {
        Self {
            payload_type,
            clock_rate_hz,
        }
    }
}

impl Default for TelephoneEventCodec {
    fn default() -> Self {
        Self::new(TELEPHONE_EVENT_PAYLOAD_TYPE, 8_000)
    }
}

/// Final-SDP state for outbound RFC 4733 on one peer route.
///
/// `Pending` deliberately has no payload fallback: application DTMF must not
/// race offer/answer completion. `Unsupported` means final SDP (or a
/// receive-only route policy) rejected outbound telephone-event and therefore
/// fails closed without writing RTP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutboundDtmfNegotiation {
    Pending,
    Negotiated(TelephoneEventCodec),
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DtmfTiming {
    samples_per_tick: u16,
    total_ticks: u16,
    final_duration_samples: u16,
}

impl DtmfTiming {
    fn new(codec: TelephoneEventCodec, duration_ms: u32) -> Result<Self> {
        if codec.clock_rate_hz == 0 {
            return Err(WebRtcError::Adapter(
                "telephone-event clock rate must be non-zero".into(),
            ));
        }

        // WebRTC telephone-event clocks used for audio are integral at 20 ms
        // (8/16/32/48 kHz). Round a non-standard clock to the nearest sample
        // instead of silently retaining the legacy 160-sample assumption.
        let samples_per_tick = (u64::from(codec.clock_rate_hz) + 25) / 50;
        let samples_per_tick = u16::try_from(samples_per_tick).map_err(|_| {
            WebRtcError::Adapter(format!(
                "telephone-event clock rate {} is too large",
                codec.clock_rate_hz
            ))
        })?;
        if samples_per_tick == 0 {
            return Err(WebRtcError::Adapter(
                "telephone-event clock rate produces a zero-sample tick".into(),
            ));
        }

        let requested_ticks = duration_ms
            .clamp(MIN_DURATION_MS, MAX_DURATION_MS)
            .div_ceil(TICK.as_millis() as u32);
        let max_ticks = u32::from(u16::MAX / samples_per_tick);
        if max_ticks < 2 {
            return Err(WebRtcError::Adapter(format!(
                "telephone-event clock rate {} cannot represent the minimum tone duration",
                codec.clock_rate_hz
            )));
        }
        let total_ticks = requested_ticks.min(max_ticks).max(2) as u16;
        let final_duration_samples = samples_per_tick.saturating_mul(total_ticks);
        Ok(Self {
            samples_per_tick,
            total_ticks,
            final_duration_samples,
        })
    }
}

fn outbound_codec_for_sender(state: OutboundDtmfNegotiation) -> Result<TelephoneEventCodec> {
    match state {
        OutboundDtmfNegotiation::Negotiated(codec) => Ok(codec),
        OutboundDtmfNegotiation::Pending => Err(WebRtcError::InvalidState(
            "WebRTC DTMF requires completed SDP negotiation",
        )),
        OutboundDtmfNegotiation::Unsupported => Err(WebRtcError::IncompatibleCapabilities),
    }
}

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
    decode_telephone_event_payload_at_clock_rate(payload, 8_000)
}

/// Decode an RFC 4733 payload using the clock rate negotiated for its dynamic
/// payload type.
pub fn decode_telephone_event_payload_at_clock_rate(
    payload: &[u8],
    clock_rate_hz: u32,
) -> Option<TelephoneEventFrame> {
    if payload.len() < 4 {
        return None;
    }
    if clock_rate_hz == 0 {
        return None;
    }
    let event = payload[0];
    let digit = event_to_digit(event)?;
    let end_of_event = payload[1] & 0b1000_0000 != 0;
    let volume = payload[1] & 0b0011_1111;
    let duration_samples = u16::from_be_bytes([payload[2], payload[3]]);
    let duration_ms = ((u64::from(duration_samples) * 1_000
        + u64::from(clock_rate_hz).saturating_sub(1))
        / u64::from(clock_rate_hz))
    .min(u64::from(u32::MAX)) as u32;
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
pub struct DtmfDecoder {
    emitted: HashSet<(u32, u8)>,
    clock_rates_by_payload_type: HashMap<u8, u32>,
}

impl Default for DtmfDecoder {
    fn default() -> Self {
        Self::new([TelephoneEventCodec::default()])
    }
}

impl DtmfDecoder {
    /// Construct a decoder for the exact telephone-event mappings negotiated
    /// in remote SDP. An empty iterator deliberately disables DTMF decoding.
    #[must_use]
    pub fn new(codecs: impl IntoIterator<Item = TelephoneEventCodec>) -> Self {
        let clock_rates_by_payload_type = codecs
            .into_iter()
            .filter(|codec| codec.clock_rate_hz > 0)
            .map(|codec| (codec.payload_type, codec.clock_rate_hz))
            .collect();
        Self {
            emitted: HashSet::new(),
            clock_rates_by_payload_type,
        }
    }

    #[must_use]
    pub fn accepts_payload_type(&self, payload_type: u8) -> bool {
        self.clock_rates_by_payload_type.contains_key(&payload_type)
    }

    pub fn decode_packet(
        &mut self,
        timestamp: u32,
        payload_type: u8,
        payload: &[u8],
    ) -> Option<DecodedDtmfEvent> {
        let clock_rate_hz = *self.clock_rates_by_payload_type.get(&payload_type)?;
        let frame = decode_telephone_event_payload_at_clock_rate(payload, clock_rate_hz)?;
        if !frame.end_of_event || !self.emitted.insert((timestamp, frame.event)) {
            return None;
        }
        Some(DecodedDtmfEvent {
            digit: frame.digit,
            duration_ms: frame.duration_ms,
        })
    }
}

#[cfg(test)]
fn telephone_event_packet(
    codec: TelephoneEventCodec,
    sequence_number: u16,
    ssrc: u32,
    event: u8,
    end_of_event: bool,
    volume: u8,
    duration: u16,
    timestamp: u32,
    marker: bool,
) -> rtp::Packet {
    let payload = encode_telephone_event(event, end_of_event, volume, duration);
    rtp::Packet {
        header: rtp::Header {
            version: 2,
            padding: false,
            extension: false,
            marker,
            payload_type: codec.payload_type,
            sequence_number,
            timestamp,
            ssrc,
            ..Default::default()
        },
        payload: bytes::Bytes::copy_from_slice(&payload),
    }
}

#[allow(clippy::too_many_arguments)]
async fn write_telephone_event(
    writer: &Arc<OutboundAudioRtpWriter>,
    mid: Option<&str>,
    codec: TelephoneEventCodec,
    event: u8,
    end_of_event: bool,
    volume: u8,
    duration: u16,
    timestamp: u32,
    marker: bool,
) -> Result<()> {
    let payload = encode_telephone_event(event, end_of_event, volume, duration);
    tracing::trace!(
        payload_type = codec.payload_type,
        clock_rate_hz = codec.clock_rate_hz,
        ssrc = writer.ssrc(),
        ?mid,
        event,
        end_of_event,
        duration,
        "writing negotiated WebRTC telephone-event RTP"
    );
    writer
        .write_supplemental_audio(
            codec.payload_type,
            timestamp,
            marker,
            bytes::Bytes::copy_from_slice(&payload),
        )
        .await
        .map_err(|e| WebRtcError::Webrtc(format!("DTMF write_supplemental_audio: {e}")))
}

async fn send_single_digit(
    writer: &Arc<OutboundAudioRtpWriter>,
    mid: Option<&str>,
    codec: TelephoneEventCodec,
    digit: char,
    duration_ms: u32,
) -> Result<()> {
    let event_code = digit_to_event(digit)
        .ok_or_else(|| WebRtcError::Adapter(format!("invalid DTMF digit '{digit}'")))?;

    let timing = DtmfTiming::new(codec, duration_ms)?;
    let start_timestamp = writer
        .reserve_telephone_event_timestamp(codec.clock_rate_hz, timing.final_duration_samples)
        .await;
    let mut duration_samples = timing.samples_per_tick;

    write_telephone_event(
        writer,
        mid,
        codec,
        event_code,
        false,
        DEFAULT_VOLUME,
        duration_samples,
        start_timestamp,
        true,
    )
    .await?;

    let continuation_count = timing.total_ticks.saturating_sub(2);
    for _ in 0..continuation_count {
        tokio::time::sleep(TICK).await;
        duration_samples = duration_samples.saturating_add(timing.samples_per_tick);
        write_telephone_event(
            writer,
            mid,
            codec,
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
    duration_samples = duration_samples.saturating_add(timing.samples_per_tick);
    for _ in 0..END_OF_EVENT_RETRANSMITS {
        write_telephone_event(
            writer,
            mid,
            codec,
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

/// Send one or more DTMF digits using the remote SDP's negotiated RFC 4733
/// payload type and clock rate.
///
/// Telephone events use the primary negotiated audio SSRC, sequence-number
/// owner, and timestamp base as required by RFC 4733, and carry the negotiated
/// SDES MID when the peer agreed one. Pending or unsupported final SDP fails
/// closed before any packet is written; a peer that simply never offers the
/// MID extension does not, since primary audio to that peer is already written
/// without it.
pub async fn send_dtmf(
    peer: &Arc<RvoipPeerConnection>,
    digits: &str,
    duration_ms: u32,
) -> Result<()> {
    let codec = outbound_codec_for_sender(peer.outbound_dtmf_negotiation())?;
    // Carry the exact mutually negotiated audio MID when one exists so BUNDLE
    // demux never falls back to payload-type or first-m-line heuristics. A
    // peer that does not negotiate the SDES MID extension at all (Amazon
    // Connect) still receives events on the primary audio SSRC, exactly as it
    // receives primary audio.
    let mid = peer.negotiated_outbound_audio_mid();
    let digits = digits
        .chars()
        .filter(|digit| !digit.is_whitespace())
        .collect::<Vec<_>>();
    peer.local_dtmf_track()
        .ok_or(WebRtcError::IncompatibleCapabilities)?;
    let writer = peer
        .outbound_audio_writer()
        .ok_or(WebRtcError::IncompatibleCapabilities)?;
    for (index, digit) in digits.iter().copied().enumerate() {
        send_single_digit(&writer, mid.as_deref(), codec, digit, duration_ms).await?;
        if index + 1 < digits.len() {
            tokio::time::sleep(TICK).await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::outbound::sdes_mid_header_extension;
    use crate::peer::builder::HDREXT_SDES_MID;
    use rtc::shared::marshal::Marshal;

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
    fn sdes_mid_extension_marshals_exact_negotiated_bytes() {
        let extension = sdes_mid_header_extension("call-audio");
        assert_eq!(extension.uri(), HDREXT_SDES_MID);
        assert_eq!(
            extension.marshal().expect("marshal SDES MID").as_ref(),
            b"call-audio"
        );
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

    #[test]
    fn decoder_uses_negotiated_dynamic_payload_type_and_clock_rate() {
        let mut decoder = DtmfDecoder::new([
            TelephoneEventCodec::new(110, 48_000),
            TelephoneEventCodec::new(126, 8_000),
        ]);
        let final_payload = encode_telephone_event(6, true, 10, 5_760);

        assert_eq!(
            decoder.decode_packet(456, TELEPHONE_EVENT_PAYLOAD_TYPE, &final_payload),
            None,
            "the local preferred payload type was not negotiated"
        );
        let event = decoder
            .decode_packet(456, 110, &final_payload)
            .expect("Chrome-style negotiated mapping");
        assert_eq!(event.digit, '6');
        assert_eq!(event.duration_ms, 120);
    }

    #[test]
    fn sender_packet_uses_chromium_pt110_and_48khz_timeline() {
        let codec = TelephoneEventCodec::new(110, 48_000);
        let timing = DtmfTiming::new(codec, 120).expect("48 kHz timing");
        assert_eq!(timing.samples_per_tick, 960);
        assert_eq!(timing.total_ticks, 6);
        assert_eq!(timing.final_duration_samples, 5_760);

        let timestamp = 48_000;
        let first = telephone_event_packet(
            codec,
            400,
            7,
            6,
            false,
            DEFAULT_VOLUME,
            timing.samples_per_tick,
            timestamp,
            true,
        );
        let final_packet = telephone_event_packet(
            codec,
            401,
            7,
            6,
            true,
            DEFAULT_VOLUME,
            timing.final_duration_samples,
            timestamp,
            false,
        );

        assert_eq!(first.header.payload_type, 110);
        assert_eq!(first.header.sequence_number, 400);
        assert_eq!(first.header.timestamp, 48_000);
        assert_eq!(&first.payload[2..], &960_u16.to_be_bytes());
        assert_eq!(final_packet.header.payload_type, 110);
        assert_eq!(final_packet.header.sequence_number, 401);
        assert_eq!(final_packet.header.timestamp, first.header.timestamp);
        assert_eq!(&final_packet.payload[2..], &5_760_u16.to_be_bytes());
    }

    #[test]
    fn sender_packet_uses_pt126_and_eight_khz_timeline() {
        let codec = TelephoneEventCodec::new(126, 8_000);
        let timing = DtmfTiming::new(codec, 120).expect("8 kHz timing");
        assert_eq!(timing.samples_per_tick, 160);
        assert_eq!(timing.total_ticks, 6);
        assert_eq!(timing.final_duration_samples, 960);

        let packet = telephone_event_packet(
            codec,
            9,
            11,
            5,
            true,
            DEFAULT_VOLUME,
            timing.final_duration_samples,
            8_000,
            false,
        );
        assert_eq!(packet.header.payload_type, 126);
        assert_eq!(packet.header.timestamp, 8_000);
        assert_eq!(&packet.payload[2..], &960_u16.to_be_bytes());
    }

    #[test]
    fn sender_fails_closed_before_or_without_negotiation() {
        let chromium = TelephoneEventCodec::new(110, 48_000);
        assert_eq!(
            outbound_codec_for_sender(OutboundDtmfNegotiation::Negotiated(chromium))
                .expect("negotiated codec"),
            chromium
        );
        assert!(matches!(
            outbound_codec_for_sender(OutboundDtmfNegotiation::Pending),
            Err(WebRtcError::InvalidState(_))
        ));
        assert!(matches!(
            outbound_codec_for_sender(OutboundDtmfNegotiation::Unsupported),
            Err(WebRtcError::IncompatibleCapabilities)
        ));
    }
}
