//! RFC 4733 telephone-event DTMF over WebRTC RTP tracks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use rtc::rtp;
use rtc::rtp::extension::HeaderExtension;
use rtc::shared::marshal::{Marshal, MarshalSize};
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;

use crate::errors::{Result, WebRtcError};
use crate::media::outbound::OutboundAudioRtpState;
use crate::peer::builder::HDREXT_SDES_MID;
pub use crate::peer::builder::TELEPHONE_EVENT_PAYLOAD_TYPE;
use crate::peer::RvoipPeerConnection;

const TICK: Duration = Duration::from_millis(20);
const END_OF_EVENT_RETRANSMITS: usize = 3;
const DEFAULT_VOLUME: u8 = 10;
const MIN_DURATION_MS: u32 = 40;
const MAX_DURATION_MS: u32 = 6_000;

/// RFC 8843/RFC 9335 SDES MID payload. The negotiated extension ID is applied
/// by `TrackLocalStaticRTP::write_rtp_with_extensions`; this value is only the
/// exact identification-tag bytes carried in the extension.
struct SdesMidExtension(Vec<u8>);

impl MarshalSize for SdesMidExtension {
    fn marshal_size(&self) -> usize {
        self.0.len()
    }
}

impl Marshal for SdesMidExtension {
    fn marshal_to(&self, buffer: &mut [u8]) -> rtc::shared::error::Result<usize> {
        if buffer.len() < self.0.len() {
            return Err(rtc::shared::error::Error::ErrBufferTooSmall);
        }
        buffer[..self.0.len()].copy_from_slice(&self.0);
        Ok(self.0.len())
    }
}

fn sdes_mid_header_extension(mid: &str) -> HeaderExtension {
    HeaderExtension::Custom {
        uri: HDREXT_SDES_MID.into(),
        extension: Box::new(SdesMidExtension(mid.as_bytes().to_vec())),
    }
}

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

/// Per-peer outbound RFC 4733 sequence/timestamp state.
///
/// The state is held behind the owning peer's async mutex for the complete
/// digit sequence. That keeps concurrent application calls from interleaving
/// events on one SSRC and preserves one RTP sequence/timestamp timeline across
/// successive `send_dtmf` calls.
#[derive(Debug)]
pub(crate) struct DtmfSenderState {
    next_sequence_number: u16,
    next_timestamp: u32,
}

impl DtmfSenderState {
    #[must_use]
    pub(crate) fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        Self {
            next_sequence_number: seed as u16,
            next_timestamp: ((seed >> 16) as u32) | 1,
        }
    }

    fn next_sequence_number(&mut self) -> u16 {
        let sequence_number = self.next_sequence_number;
        self.next_sequence_number = self.next_sequence_number.wrapping_add(1);
        sequence_number
    }

    fn reserve_event_timestamp(&mut self, timing: DtmfTiming) -> u32 {
        let timestamp = self.next_timestamp;
        // `send_single_digit` emits its final duration one tick before the
        // represented end of the event. `send_dtmf` waits that last tick
        // before starting another digit, so adjacent event timestamps advance
        // by the exact negotiated-clock duration without overlap.
        self.next_timestamp = self
            .next_timestamp
            .wrapping_add(u32::from(timing.final_duration_samples));
        timestamp
    }
}

trait DtmfTimeline {
    fn next_sequence_number(&mut self) -> u16;
    fn reserve_event_timestamp(&mut self, timing: DtmfTiming) -> u32;
}

impl DtmfTimeline for DtmfSenderState {
    fn next_sequence_number(&mut self) -> u16 {
        DtmfSenderState::next_sequence_number(self)
    }

    fn reserve_event_timestamp(&mut self, timing: DtmfTiming) -> u32 {
        DtmfSenderState::reserve_event_timestamp(self, timing)
    }
}

impl DtmfTimeline for OutboundAudioRtpState {
    fn next_sequence_number(&mut self) -> u16 {
        OutboundAudioRtpState::next_sequence_number(self)
    }

    fn reserve_event_timestamp(&mut self, timing: DtmfTiming) -> u32 {
        OutboundAudioRtpState::reserve_event_timestamp(
            self,
            timing.samples_per_tick,
            timing.final_duration_samples,
        )
    }
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
    track: &Arc<TrackLocalStaticRTP>,
    mid: &str,
    codec: TelephoneEventCodec,
    sequence_number: u16,
    ssrc: u32,
    event: u8,
    end_of_event: bool,
    volume: u8,
    duration: u16,
    timestamp: u32,
    marker: bool,
) -> Result<()> {
    let pkt = telephone_event_packet(
        codec,
        sequence_number,
        ssrc,
        event,
        end_of_event,
        volume,
        duration,
        timestamp,
        marker,
    );
    tracing::trace!(
        payload_type = codec.payload_type,
        clock_rate_hz = codec.clock_rate_hz,
        sequence_number,
        ssrc,
        mid,
        event,
        end_of_event,
        duration,
        "writing negotiated WebRTC telephone-event RTP"
    );
    track
        .write_rtp_with_extensions(pkt, &[sdes_mid_header_extension(mid)])
        .await
        .map_err(|e| WebRtcError::Webrtc(format!("DTMF write_rtp_with_extensions: {e}")))
}

async fn send_single_digit<S: DtmfTimeline>(
    track: &Arc<TrackLocalStaticRTP>,
    mid: &str,
    codec: TelephoneEventCodec,
    state: &mut S,
    ssrc: u32,
    digit: char,
    duration_ms: u32,
) -> Result<()> {
    let event_code = digit_to_event(digit)
        .ok_or_else(|| WebRtcError::Adapter(format!("invalid DTMF digit '{digit}'")))?;

    let timing = DtmfTiming::new(codec, duration_ms)?;
    let start_timestamp = state.reserve_event_timestamp(timing);
    let mut duration_samples = timing.samples_per_tick;

    write_telephone_event(
        track,
        mid,
        codec,
        state.next_sequence_number(),
        ssrc,
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
            track,
            mid,
            codec,
            state.next_sequence_number(),
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
    duration_samples = duration_samples.saturating_add(timing.samples_per_tick);
    for _ in 0..END_OF_EVENT_RETRANSMITS {
        write_telephone_event(
            track,
            mid,
            codec,
            state.next_sequence_number(),
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

/// Send one or more DTMF digits using the remote SDP's negotiated RFC 4733
/// payload type and clock rate.
///
/// Same-clock telephone events share the primary audio SSRC and its serialized
/// sequence/timestamp writer, which is what Chromium and RFC 4733 expect.
/// A differently-clocked event may use the hidden supplemental SSRC; changing
/// the RTP clock on one SSRC is rejected. Pending or unsupported final SDP
/// fails closed before any packet is written.
pub async fn send_dtmf(
    peer: &Arc<RvoipPeerConnection>,
    digits: &str,
    duration_ms: u32,
) -> Result<()> {
    let codec = outbound_codec_for_sender(peer.outbound_dtmf_negotiation())?;
    // A differently-clocked event uses a supplemental DTMF SSRC that is
    // intentionally not signalled in SDP, while a same-clock event shares the
    // primary audio SSRC. Require the exact mutually negotiated audio MID for
    // both paths so packet demux never falls back to payload-type or
    // first-m-line heuristics.
    let mid = peer
        .negotiated_outbound_audio_mid()
        .ok_or(WebRtcError::IncompatibleCapabilities)?;
    let digits = digits
        .chars()
        .filter(|digit| !digit.is_whitespace())
        .collect::<Vec<_>>();
    if let Some(writer) = peer
        .outbound_audio_writer()
        .filter(|writer| writer.clock_rate_hz() == codec.clock_rate_hz)
    {
        let mut state = writer.lock_state().await;
        for (index, digit) in digits.iter().copied().enumerate() {
            send_single_digit(
                writer.track(),
                &mid,
                codec,
                &mut *state,
                writer.ssrc(),
                digit,
                duration_ms,
            )
            .await?;
            if index + 1 < digits.len() {
                tokio::time::sleep(TICK).await;
            }
        }
        // The final end packet is emitted one tick before the duration it
        // represents. Keep primary audio serialized through that interval so
        // the next packet is sent at the event end, not 20 ms early.
        if !digits.is_empty() {
            tokio::time::sleep(TICK).await;
        }
        return Ok(());
    }

    let track = peer
        .local_dtmf_track()
        .ok_or(WebRtcError::IncompatibleCapabilities)?;
    let ssrc = peer
        .local_dtmf_ssrc_for_codec(codec)
        .ok_or(WebRtcError::IncompatibleCapabilities)?;
    let mut states = peer.dtmf_sender_states().lock().await;
    let state = states.entry(ssrc).or_insert_with(DtmfSenderState::new);
    for (index, digit) in digits.iter().copied().enumerate() {
        send_single_digit(&track, &mid, codec, state, ssrc, digit, duration_ms).await?;
        if index + 1 < digits.len() {
            tokio::time::sleep(TICK).await;
        }
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

        let mut state = DtmfSenderState {
            next_sequence_number: 400,
            next_timestamp: 48_000,
        };
        let timestamp = state.reserve_event_timestamp(timing);
        let first = telephone_event_packet(
            codec,
            state.next_sequence_number(),
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
            state.next_sequence_number(),
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
        assert_eq!(state.next_timestamp, 53_760);
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
