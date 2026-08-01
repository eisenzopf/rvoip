//! Serialized RTP ownership for one locally-originated WebRTC audio SSRC.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as SyncMutex, OnceLock, Weak};

use bytes::Bytes;
use rtc::rtp;
use rtc::rtp::extension::HeaderExtension;
use rtc::shared::marshal::{Marshal, MarshalSize};
use tokio::sync::Mutex;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;

use crate::peer::builder::HDREXT_SDES_MID;

/// RFC 8843/RFC 9335 SDES MID payload. The track applies the negotiated
/// extension ID; this value owns only the exact identification-tag bytes.
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

pub(crate) fn sdes_mid_header_extension(mid: &str) -> HeaderExtension {
    HeaderExtension::Custom {
        uri: HDREXT_SDES_MID.into(),
        extension: Box::new(SdesMidExtension(mid.as_bytes().to_vec())),
    }
}

/// Sequence and timestamp state for one primary audio SSRC.
#[derive(Debug)]
pub(crate) struct OutboundAudioRtpState {
    next_sequence_number: u16,
    last_wire_timestamp: Option<u32>,
    last_source_timestamp: Option<u32>,
}

impl Default for OutboundAudioRtpState {
    fn default() -> Self {
        Self {
            next_sequence_number: 1,
            last_wire_timestamp: None,
            last_source_timestamp: None,
        }
    }
}

impl OutboundAudioRtpState {
    pub(crate) fn next_sequence_number(&mut self) -> u16 {
        let sequence_number = self.next_sequence_number;
        self.next_sequence_number = self.next_sequence_number.wrapping_add(1);
        sequence_number
    }

    fn allocate_audio_timestamp(&mut self, source_timestamp: u32, samples_per_frame: u32) -> u32 {
        let timestamp = match (
            source_timestamp,
            self.last_source_timestamp,
            self.last_wire_timestamp,
        ) {
            (0, _, Some(last_wire)) => last_wire.wrapping_add(samples_per_frame),
            (0, _, None) => 0,
            (source, Some(last_source), Some(last_wire)) => {
                let delta = source.wrapping_sub(last_source);
                if delta < (1_u32 << 31) {
                    last_wire.wrapping_add(delta)
                } else {
                    // A source or handoff timestamp reset must not move the
                    // stable outbound SSRC backwards.
                    last_wire.wrapping_add(samples_per_frame)
                }
            }
            (source, None, Some(last_wire)) => {
                // Media resumed after a source handoff. Start at the next
                // audio tick rather than replaying a stale source timestamp.
                let _ = source;
                last_wire.wrapping_add(samples_per_frame)
            }
            (source, _, None) => source,
        };
        self.last_source_timestamp = (source_timestamp != 0).then_some(source_timestamp);
        self.last_wire_timestamp = Some(timestamp);
        timestamp
    }

    fn reserve_telephone_event_timestamp(
        &mut self,
        audio_clock_rate_hz: u32,
        event_clock_rate_hz: u32,
        event_duration_samples: u16,
    ) -> u32 {
        let samples_per_audio_frame = (audio_clock_rate_hz / 50).max(1);
        let start_audio = self
            .last_wire_timestamp
            .map_or_else(initial_rtp_timestamp, |last| {
                last.wrapping_add(samples_per_audio_frame)
            });
        let duration_audio = ((u64::from(event_duration_samples) * u64::from(audio_clock_rate_hz)
            + u64::from(event_clock_rate_hz).saturating_sub(1))
            / u64::from(event_clock_rate_hz))
        .max(1) as u32;
        self.last_wire_timestamp = Some(start_audio.wrapping_add(duration_audio));
        self.last_source_timestamp = None;

        // RFC 4733 requires named events to use the regular audio channel's
        // timestamp base. Only the duration field uses the telephone-event
        // clock when its negotiated rate differs from primary audio.
        start_audio
    }
}

fn initial_rtp_timestamp() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        | 1
}

/// The single enqueue boundary for primary audio RTP.
pub(crate) struct OutboundAudioRtpWriter {
    track: Arc<TrackLocalStaticRTP>,
    ssrc: u32,
    clock_rate_hz: u32,
    mid: SyncMutex<Option<String>>,
    state: Mutex<OutboundAudioRtpState>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct OutboundAudioRtpKey {
    track: usize,
    ssrc: u32,
}

type OutboundAudioRtpWriters = HashMap<OutboundAudioRtpKey, Weak<OutboundAudioRtpWriter>>;

fn outbound_audio_rtp_writers() -> &'static SyncMutex<OutboundAudioRtpWriters> {
    static WRITERS: OnceLock<SyncMutex<OutboundAudioRtpWriters>> = OnceLock::new();
    WRITERS.get_or_init(|| SyncMutex::new(HashMap::new()))
}

impl OutboundAudioRtpWriter {
    pub(crate) fn new(track: Arc<TrackLocalStaticRTP>, ssrc: u32, clock_rate_hz: u32) -> Arc<Self> {
        let clock_rate_hz = if clock_rate_hz == 0 {
            tracing::warn!(
                ssrc,
                "outbound WebRTC audio declared a zero RTP clock; retaining the 48 kHz compatibility default"
            );
            48_000
        } else {
            clock_rate_hz
        };
        let key = OutboundAudioRtpKey {
            track: Arc::as_ptr(&track) as usize,
            ssrc,
        };
        let mut writers = outbound_audio_rtp_writers()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        writers.retain(|_, writer| writer.strong_count() > 0);
        if let Some(writer) = writers.get(&key).and_then(Weak::upgrade) {
            if writer.clock_rate_hz != clock_rate_hz {
                // One SSRC has exactly one RTP clock. Reusing the existing
                // owner is safer than creating a second, racing timeline;
                // the mismatch remains visible to the caller in diagnostics.
                tracing::warn!(
                    ssrc,
                    requested_clock_rate_hz = clock_rate_hz,
                    existing_clock_rate_hz = writer.clock_rate_hz,
                    "reusing the existing outbound WebRTC RTP writer after a clock mismatch"
                );
            }
            return writer;
        }

        let writer = Arc::new(Self {
            track,
            ssrc,
            clock_rate_hz,
            mid: SyncMutex::new(None),
            state: Mutex::new(OutboundAudioRtpState::default()),
        });
        writers.insert(key, Arc::downgrade(&writer));
        writer
    }

    pub(crate) fn set_mid(&self, mid: Option<String>) {
        *self
            .mid
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = mid;
    }

    pub(crate) fn ssrc(&self) -> u32 {
        self.ssrc
    }

    pub(crate) async fn reserve_telephone_event_timestamp(
        &self,
        event_clock_rate_hz: u32,
        event_duration_samples: u16,
    ) -> u32 {
        self.state.lock().await.reserve_telephone_event_timestamp(
            self.clock_rate_hz,
            event_clock_rate_hz,
            event_duration_samples,
        )
    }

    pub(crate) async fn write_supplemental_audio(
        &self,
        payload_type: u8,
        timestamp: u32,
        marker: bool,
        payload: Bytes,
    ) -> webrtc::error::Result<()> {
        // Keep the owner locked through the actual enqueue so audio and
        // telephone-event packets cannot reach the track out of sequence.
        let mut state = self.state.lock().await;
        let sequence_number = state.next_sequence_number();
        let packet = rtp::Packet {
            header: rtp::Header {
                version: 2,
                padding: false,
                extension: false,
                marker,
                payload_type,
                sequence_number,
                timestamp,
                ssrc: self.ssrc,
                ..Default::default()
            },
            payload,
        };
        let mid = self
            .mid
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| {
                webrtc::error::Error::Other(
                    "outbound audio MID is not committed by final SDP".into(),
                )
            })?;
        let result = self
            .track
            .write_rtp_with_extensions(packet, &[sdes_mid_header_extension(&mid)])
            .await;
        drop(state);
        result
    }

    pub(crate) async fn write_audio(
        &self,
        payload_type: u8,
        source_timestamp: u32,
        payload: Bytes,
    ) -> webrtc::error::Result<()> {
        let mut state = self.state.lock().await;
        let samples_per_frame = (self.clock_rate_hz / 50).max(1);
        let timestamp = state.allocate_audio_timestamp(source_timestamp, samples_per_frame);
        let packet = rtp::Packet {
            header: rtp::Header {
                version: 2,
                padding: false,
                extension: false,
                marker: false,
                payload_type,
                sequence_number: state.next_sequence_number(),
                timestamp,
                ssrc: self.ssrc,
                ..Default::default()
            },
            payload,
        };

        let mid = self
            .mid
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| {
                webrtc::error::Error::Other(
                    "outbound audio MID is not committed by final SDP".into(),
                )
            })?;
        let extension = sdes_mid_header_extension(&mid);
        loop {
            match self
                .track
                .write_rtp_with_extensions(packet.clone(), std::slice::from_ref(&extension))
                .await
            {
                Err(error) if error.to_string().contains("not binding") => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                result => return result,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtc::rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
    };
    use webrtc::media_stream::MediaStreamTrack;

    fn audio_track(ssrc: u32) -> Arc<TrackLocalStaticRTP> {
        Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            format!("writer-test-stream-{ssrc}"),
            format!("writer-test-track-{ssrc}"),
            "writer-tests".into(),
            RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                codec: RTCRtpCodec {
                    mime_type: "audio/opus".into(),
                    clock_rate: 48_000,
                    channels: 2,
                    ..Default::default()
                },
                ..Default::default()
            }],
        )))
    }

    #[test]
    fn exact_track_and_ssrc_reuse_one_sequence_timeline() {
        let track = audio_track(0x1020_3040);
        let peer_writer = OutboundAudioRtpWriter::new(Arc::clone(&track), 0x1020_3040, 48_000);
        let public_stream_writer =
            OutboundAudioRtpWriter::new(Arc::clone(&track), 0x1020_3040, 48_000);
        assert!(Arc::ptr_eq(&peer_writer, &public_stream_writer));

        let different_ssrc = OutboundAudioRtpWriter::new(track, 0x1020_3041, 48_000);
        assert!(!Arc::ptr_eq(&peer_writer, &different_ssrc));
    }

    #[test]
    fn source_timestamp_deltas_are_preserved_until_a_reset() {
        let mut state = OutboundAudioRtpState::default();
        assert_eq!(state.allocate_audio_timestamp(10_000, 960), 10_000);
        assert_eq!(state.allocate_audio_timestamp(10_960, 960), 10_960);
        assert_eq!(state.allocate_audio_timestamp(100, 960), 11_920);
    }

    #[test]
    fn telephone_event_reservation_advances_the_primary_audio_timeline() {
        let mut state = OutboundAudioRtpState {
            next_sequence_number: 40,
            last_wire_timestamp: Some(48_000),
            last_source_timestamp: Some(48_000),
        };
        let event_timestamp = state.reserve_telephone_event_timestamp(48_000, 48_000, 5_760);
        assert_eq!(event_timestamp, 48_960);
        assert_eq!(state.last_wire_timestamp, Some(54_720));
        assert_eq!(state.last_source_timestamp, None);
        assert_eq!(state.next_sequence_number(), 40);

        let event_timestamp = state.reserve_telephone_event_timestamp(48_000, 8_000, 960);
        assert_eq!(
            event_timestamp, 55_680,
            "8 kHz events retain the audio timestamp base"
        );
        assert_eq!(state.last_wire_timestamp, Some(61_440));
    }
}
