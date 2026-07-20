//! Serialized RTP ownership for one locally-originated WebRTC audio SSRC.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as SyncMutex, OnceLock, Weak};

use bytes::Bytes;
use rtc::rtp;
use tokio::sync::{Mutex, MutexGuard};
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::TrackLocal;

/// Sequence and timestamp state shared by primary audio and same-clock
/// telephone-event packets on one SSRC.
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
                // Media resumed after a serialized telephone event. Start at
                // the next audio tick rather than replaying a queued source
                // timestamp from before the event.
                let _ = source;
                last_wire.wrapping_add(samples_per_frame)
            }
            (source, _, None) => source,
        };
        self.last_source_timestamp = (source_timestamp != 0).then_some(source_timestamp);
        self.last_wire_timestamp = Some(timestamp);
        timestamp
    }

    /// Reserve a constant event timestamp and advance the shared media clock
    /// to the event's final 20 ms interval. The caller holds the writer lock
    /// while it emits every packet in the event, so primary audio cannot
    /// reorder them. The next audio timestamp then lands exactly at the event
    /// end rather than leaving an extra silent tick.
    pub(crate) fn reserve_event_timestamp(
        &mut self,
        samples_per_tick: u16,
        final_duration_samples: u16,
    ) -> u32 {
        let start = self.last_wire_timestamp.map_or_else(
            || {
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                ((seed >> 16) as u32) | 1
            },
            |last| last.wrapping_add(u32::from(samples_per_tick)),
        );
        let final_interval_start = final_duration_samples.saturating_sub(samples_per_tick);
        self.last_wire_timestamp = Some(start.wrapping_add(u32::from(final_interval_start)));
        self.last_source_timestamp = None;
        start
    }

    #[cfg(test)]
    fn last_wire_timestamp(&self) -> Option<u32> {
        self.last_wire_timestamp
    }
}

/// The single enqueue boundary for primary audio and same-clock DTMF RTP.
pub(crate) struct OutboundAudioRtpWriter {
    track: Arc<TrackLocalStaticRTP>,
    ssrc: u32,
    clock_rate_hz: u32,
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
            state: Mutex::new(OutboundAudioRtpState::default()),
        });
        writers.insert(key, Arc::downgrade(&writer));
        writer
    }

    pub(crate) fn track(&self) -> &Arc<TrackLocalStaticRTP> {
        &self.track
    }

    pub(crate) fn ssrc(&self) -> u32 {
        self.ssrc
    }

    pub(crate) fn clock_rate_hz(&self) -> u32 {
        self.clock_rate_hz
    }

    pub(crate) async fn lock_state(&self) -> MutexGuard<'_, OutboundAudioRtpState> {
        self.state.lock().await
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

        loop {
            match self.track.write_rtp(packet.clone()).await {
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
    fn audio_event_audio_timeline_is_monotonic_across_source_pause() {
        let mut state = OutboundAudioRtpState::default();
        assert_eq!(state.allocate_audio_timestamp(48_000, 960), 48_000);
        assert_eq!(state.next_sequence_number(), 1);
        let event = state.reserve_event_timestamp(960, 5_760);
        assert_eq!(event, 48_960);
        assert_eq!(state.last_wire_timestamp(), Some(53_760));
        assert_eq!(state.next_sequence_number(), 2);
        assert_eq!(
            state.allocate_audio_timestamp(48_960, 960),
            54_720,
            "queued media resumes after the event instead of moving backwards"
        );
        assert_eq!(state.next_sequence_number(), 3);
    }

    #[test]
    fn source_timestamp_deltas_are_preserved_until_a_reset() {
        let mut state = OutboundAudioRtpState::default();
        assert_eq!(state.allocate_audio_timestamp(10_000, 960), 10_000);
        assert_eq!(state.allocate_audio_timestamp(10_960, 960), 10_960);
        assert_eq!(state.allocate_audio_timestamp(100, 960), 11_920);
    }

    #[test]
    fn pre_audio_event_seeds_a_timeline_for_later_media() {
        let mut state = OutboundAudioRtpState::default();
        let event = state.reserve_event_timestamp(960, 5_760);
        assert_ne!(event, 0);
        assert_eq!(
            state.allocate_audio_timestamp(1_000, 960),
            event.wrapping_add(5_760)
        );
    }
}
