//! `MediaStream` implementation over webrtc-rs tracks.

use std::collections::HashSet;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::TrackRemote;

use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaReceiverReservation, MediaStream, QualitySnapshot, StreamKind};

use crate::media::dtmf::{DecodedDtmfEvent, TelephoneEventCodec};
use crate::media::outbound::OutboundAudioRtpWriter;
use crate::media::pump::{
    spawn_inbound_pump_tracked, spawn_outbound_pump_with_writer_tracked, InboundStats,
    WebRtcStatsSnapshot, DEFAULT_INBOUND_FRAME_QUEUE_CAPACITY, DEFAULT_INBOUND_SEND_DEADLINE_MS,
    FRAME_CHANNEL_CAP,
};

/// Bound and overflow deadline of a stream's inbound frame queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InboundFrameQueue {
    pub capacity: usize,
    pub send_deadline_ms: u64,
}

impl Default for InboundFrameQueue {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_INBOUND_FRAME_QUEUE_CAPACITY,
            send_deadline_ms: DEFAULT_INBOUND_SEND_DEADLINE_MS,
        }
    }
}

impl InboundFrameQueue {
    pub(crate) fn from_config(config: &crate::config::WebRtcConfig) -> Self {
        Self {
            capacity: config.inbound_frame_queue_capacity,
            send_deadline_ms: config.inbound_send_deadline_ms,
        }
    }
}
use crate::media::stats::spawn_webrtc_stats_collector_tracked;

struct WebRtcMediaStreamInner {
    id: StreamId,
    codec: CodecInfo,
    direction: Direction,
    frames_in_tx: mpsc::Sender<rvoip_core::stream::MediaFrame>,
    frames_in_rx: Mutex<Option<mpsc::Receiver<rvoip_core::stream::MediaFrame>>>,
    frames_out_tx: mpsc::Sender<rvoip_core::stream::MediaFrame>,
    pumps: Mutex<Vec<JoinHandle<()>>>,
    /// Remote track identities already assigned an inbound pump. Audio and
    /// RFC 4733 telephone-event commonly arrive on distinct WebRTC tracks,
    /// so a boolean cannot represent the route correctly.
    remote_tracks: Mutex<HashSet<usize>>,
    inbound_stats: Arc<InboundStats>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
    dtmf_codecs: Vec<TelephoneEventCodec>,
    /// Local cancel used to stop owned pump tasks on `close()`. The adapter may
    /// also pass a route-level Notify into `enable_webrtc_stats` for global cancel.
    cancel: Arc<Notify>,
    send_deadline_ms: u64,
    task_counter: Option<Arc<AtomicUsize>>,
}

/// Concrete media stream — supports late remote-track attachment.
pub struct WebRtcMediaStream {
    inner: Arc<WebRtcMediaStreamInner>,
}

impl WebRtcMediaStream {
    /// Attach a remote track after the stream was created (late `on_track`).
    pub fn attach_remote(&self, track: Arc<dyn TrackRemote>) {
        let identity = track_identity(&track);
        if !self.inner.remote_tracks.lock().insert(identity) {
            return;
        }

        let handle = spawn_inbound_pump_tracked(
            track,
            self.inner.id.clone(),
            self.inner.frames_in_tx.clone(),
            Arc::clone(&self.inner.inbound_stats),
            self.inner.send_deadline_ms,
            Some(Arc::clone(&self.inner.cancel)),
            self.inner.dtmf_tx.clone(),
            self.inner.dtmf_codecs.clone(),
            self.inner.task_counter.clone(),
        );
        self.inner.pumps.lock().push(handle);
    }

    /// Poll webrtc-rs `get_stats` in the background to enrich [`QualitySnapshot`].
    /// `cancel` is honored so the loop exits cleanly when the route is closed.
    pub fn enable_webrtc_stats(
        &self,
        peer: Arc<dyn webrtc::peer_connection::PeerConnection>,
        cancel: Arc<Notify>,
    ) {
        let handle = spawn_webrtc_stats_collector_tracked(
            Arc::clone(&self.inner.inbound_stats),
            peer,
            cancel,
            self.inner.task_counter.clone(),
        );
        self.inner.pumps.lock().push(handle);
    }

    /// Typed inbound RTP stats snapshot (packets/bytes/loss/jitter/MOS).
    /// Richer than the core `QualitySnapshot` returned by `quality_snapshot()`.
    pub fn webrtc_stats_snapshot(&self) -> WebRtcStatsSnapshot {
        self.inner.inbound_stats.webrtc_snapshot()
    }

    /// Most recently observed non-DTMF RTP payload type on the remote wire.
    /// This is diagnostic evidence, not a substitute for negotiated codec
    /// policy.
    pub fn last_inbound_media_payload_type(&self) -> Option<u8> {
        self.inner.inbound_stats.last_media_payload_type()
    }

    /// Cancel, abort, and join every task owned by this stream.
    ///
    /// Route teardown uses this directly so retained stream Arcs cannot keep
    /// detached RTP or stats work alive after the peer route is gone.
    pub(crate) async fn shutdown_background_tasks(&self) {
        self.inner.cancel.notify_waiters();
        let tasks: Vec<_> = self.inner.pumps.lock().drain(..).collect();
        for mut task in tasks {
            if !task.is_finished() {
                task.abort();
            }
            let _ = (&mut task).await;
        }
    }

    /// Best-effort synchronous cancellation for adapter Drop paths where an
    /// async join is unavailable.
    pub(crate) fn abort_background_tasks(&self) {
        self.inner.cancel.notify_waiters();
        for task in self.inner.pumps.lock().drain(..) {
            task.abort();
        }
    }
}

/// Build a bidirectional audio stream from local + optional remote track.
///
/// D4 follow-up — `local_ssrc` and `payload_type` are passed to the
/// outbound pump so it can wrap codec-payload `MediaFrame`s (the
/// orchestrator-side `Transcoder` output) in fresh RTP headers.
pub fn from_tracks(
    id: StreamId,
    codec: CodecInfo,
    local: Arc<TrackLocalStaticRTP>,
    local_ssrc: u32,
    payload_type: u8,
    remote: Option<Arc<dyn TrackRemote>>,
) -> Arc<WebRtcMediaStream> {
    from_tracks_with_dtmf_events(id, codec, local, local_ssrc, payload_type, remote, None)
}

/// Build a media stream and route inbound RFC 4733 telephone-events to
/// `dtmf_tx` instead of forwarding them as ordinary audio frames.
pub fn from_tracks_with_dtmf_events(
    id: StreamId,
    codec: CodecInfo,
    local: Arc<TrackLocalStaticRTP>,
    local_ssrc: u32,
    payload_type: u8,
    remote: Option<Arc<dyn TrackRemote>>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
) -> Arc<WebRtcMediaStream> {
    from_tracks_with_dtmf_codecs(
        id,
        codec,
        local,
        local_ssrc,
        payload_type,
        remote,
        dtmf_tx,
        [TelephoneEventCodec::default()],
    )
}

/// Build a media stream and decode RFC 4733 using the exact negotiated
/// telephone-event payload mappings.
///
/// Dynamic WebRTC payload types and their clock rates come from the final SDP;
/// callers that have that negotiation result must use this constructor rather
/// than assuming the compatibility default of PT 101 at 8 kHz.
///
/// When `local` and `local_ssrc` come from an
/// [`RvoipPeerConnection`](crate::peer::RvoipPeerConnection), this constructor
/// reuses that peer's serialized outbound RTP writer. Audio sent through the
/// returned stream therefore shares sequence/timestamp ownership with
/// [`crate::media::dtmf::send_dtmf`] without requiring a new public handle.
pub fn from_tracks_with_dtmf_codecs(
    id: StreamId,
    codec: CodecInfo,
    local: Arc<TrackLocalStaticRTP>,
    local_ssrc: u32,
    payload_type: u8,
    remote: Option<Arc<dyn TrackRemote>>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
    dtmf_codecs: impl IntoIterator<Item = TelephoneEventCodec>,
) -> Arc<WebRtcMediaStream> {
    from_tracks_with_dtmf_events_and_task_counter(
        id,
        codec,
        local,
        local_ssrc,
        payload_type,
        remote,
        dtmf_tx,
        dtmf_codecs.into_iter().collect(),
        None,
        None,
        InboundFrameQueue::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn from_tracks_with_dtmf_events_and_task_counter(
    id: StreamId,
    codec: CodecInfo,
    local: Arc<TrackLocalStaticRTP>,
    local_ssrc: u32,
    payload_type: u8,
    remote: Option<Arc<dyn TrackRemote>>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
    dtmf_codecs: Vec<TelephoneEventCodec>,
    task_counter: Option<Arc<AtomicUsize>>,
    outbound_writer: Option<Arc<OutboundAudioRtpWriter>>,
    inbound_queue: InboundFrameQueue,
) -> Arc<WebRtcMediaStream> {
    let (frames_in_tx, frames_in_rx) = mpsc::channel(inbound_queue.capacity.max(1));
    let (frames_out_tx, frames_out_rx) = mpsc::channel(FRAME_CHANNEL_CAP);
    let inbound_stats = Arc::new(InboundStats::default());
    let cancel = Arc::new(Notify::new());
    let send_deadline_ms = inbound_queue.send_deadline_ms;

    let mut pumps = Vec::new();
    let outbound_writer = outbound_writer
        .unwrap_or_else(|| OutboundAudioRtpWriter::new(local, local_ssrc, codec.clock_rate_hz));
    pumps.push(spawn_outbound_pump_with_writer_tracked(
        outbound_writer,
        frames_out_rx,
        payload_type,
        task_counter.clone(),
    ));

    let remote_tracks = Mutex::new(HashSet::new());
    if let Some(remote_track) = remote {
        remote_tracks.lock().insert(track_identity(&remote_track));
        pumps.push(spawn_inbound_pump_tracked(
            remote_track,
            id.clone(),
            frames_in_tx.clone(),
            Arc::clone(&inbound_stats),
            send_deadline_ms,
            Some(Arc::clone(&cancel)),
            dtmf_tx.clone(),
            dtmf_codecs.clone(),
            task_counter.clone(),
        ));
    }

    Arc::new(WebRtcMediaStream {
        inner: Arc::new(WebRtcMediaStreamInner {
            id,
            codec,
            direction: Direction::Outbound,
            frames_in_tx,
            frames_in_rx: Mutex::new(Some(frames_in_rx)),
            frames_out_tx,
            pumps: Mutex::new(pumps),
            remote_tracks,
            inbound_stats,
            dtmf_tx,
            dtmf_codecs,
            cancel,
            send_deadline_ms,
            task_counter,
        }),
    })
}

fn track_identity(track: &Arc<dyn TrackRemote>) -> usize {
    Arc::as_ptr(track) as *const () as usize
}

#[async_trait]
impl MediaStream for WebRtcMediaStream {
    fn id(&self) -> StreamId {
        self.inner.id.clone()
    }

    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }

    fn codec(&self) -> CodecInfo {
        self.inner.codec.clone()
    }

    fn direction(&self) -> Direction {
        self.inner.direction
    }

    fn frames_in(&self) -> mpsc::Receiver<rvoip_core::stream::MediaFrame> {
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> RvoipResult<mpsc::Receiver<rvoip_core::stream::MediaFrame>> {
        Ok(self.reserve_frames_in()?.commit())
    }

    fn reserve_frames_in(&self) -> RvoipResult<MediaReceiverReservation> {
        let receiver = self
            .inner
            .frames_in_rx
            .lock()
            .take()
            .ok_or(RvoipError::InvalidState(
                "WebRTC media receiver has already been acquired",
            ))?;
        let inner = Arc::clone(&self.inner);
        Ok(MediaReceiverReservation::new(receiver, move |receiver| {
            let mut slot = inner.frames_in_rx.lock();
            debug_assert!(slot.is_none(), "reserved WebRTC receiver slot was replaced");
            if slot.is_none() {
                *slot = Some(receiver);
            }
        }))
    }

    fn frames_out(&self) -> mpsc::Sender<rvoip_core::stream::MediaFrame> {
        self.inner.frames_out_tx.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        self.inner.inbound_stats.snapshot()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        self.shutdown_background_tasks().await;
        Ok(())
    }
}

#[cfg(test)]
mod receiver_ownership_tests {
    use super::*;

    #[test]
    fn second_receiver_acquisition_is_a_typed_error() {
        let (frames_in_tx, frames_in_rx) = mpsc::channel(1);
        let (frames_out_tx, _frames_out_rx) = mpsc::channel(1);
        let stream = WebRtcMediaStream {
            inner: Arc::new(WebRtcMediaStreamInner {
                id: StreamId::new(),
                codec: CodecInfo {
                    name: "opus".into(),
                    clock_rate_hz: 48_000,
                    channels: 1,
                    fmtp: None,
                    payload_type: None,
                },
                direction: Direction::Inbound,
                frames_in_tx,
                frames_in_rx: Mutex::new(Some(frames_in_rx)),
                frames_out_tx,
                pumps: Mutex::new(Vec::new()),
                remote_tracks: Mutex::new(HashSet::new()),
                inbound_stats: Arc::new(InboundStats::default()),
                dtmf_tx: None,
                dtmf_codecs: Vec::new(),
                cancel: Arc::new(Notify::new()),
                send_deadline_ms: DEFAULT_INBOUND_SEND_DEADLINE_MS,
                task_counter: None,
            }),
        };

        let reservation = stream.reserve_frames_in().expect("reserve receiver");
        assert!(matches!(
            stream.try_frames_in(),
            Err(RvoipError::InvalidState(_))
        ));
        drop(reservation);
        assert!(stream.try_frames_in().is_ok());
        assert!(matches!(
            stream.try_frames_in(),
            Err(RvoipError::InvalidState(_))
        ));
    }
}
#[cfg(test)]
mod inbound_queue_tests {
    use super::*;
    use std::time::Duration;

    use rvoip_core::ids::StreamId;

    use rtc::media_stream::{
        MediaStreamId, MediaStreamTrackId, MediaStreamTrackState, MediaTrackCapabilities,
        MediaTrackConstraints, MediaTrackSettings,
    };
    use rtc::rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
    };
    use rtc::rtp_transceiver::RtpStreamId;
    use webrtc::media_stream::track_remote::TrackRemoteEvent;
    use webrtc::media_stream::{MediaStreamTrack, Track};

    use crate::media::pump::silent_rtp_packet_for_ssrc;

    /// Remote track fed by the test. Like the peer-connection driver, the
    /// producer `try_send`s into a bounded event queue and loses packets
    /// when the inbound pump does not keep up.
    struct ScriptedRemoteTrack {
        track: parking_lot::Mutex<MediaStreamTrack>,
        events: tokio::sync::Mutex<mpsc::Receiver<TrackRemoteEvent>>,
    }

    impl ScriptedRemoteTrack {
        fn opus(ssrc: u32, capacity: usize) -> (Arc<Self>, mpsc::Sender<TrackRemoteEvent>) {
            let (tx, rx) = mpsc::channel(capacity);
            let track = MediaStreamTrack::new(
                "remote-stream".into(),
                "remote-audio".into(),
                "remote-audio".into(),
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
            );
            let remote = Arc::new(Self {
                track: parking_lot::Mutex::new(track),
                events: tokio::sync::Mutex::new(rx),
            });
            (remote, tx)
        }
    }

    #[async_trait]
    impl Track for ScriptedRemoteTrack {
        async fn stream_id(&self) -> MediaStreamId {
            self.track.lock().stream_id().to_owned()
        }
        async fn track_id(&self) -> MediaStreamTrackId {
            self.track.lock().track_id().to_owned()
        }
        async fn label(&self) -> String {
            self.track.lock().label().to_owned()
        }
        async fn kind(&self) -> RtpCodecKind {
            self.track.lock().kind()
        }
        async fn rid(&self, ssrc: u32) -> Option<RtpStreamId> {
            self.track.lock().rid(ssrc).cloned()
        }
        async fn codec(&self, ssrc: u32) -> Option<RTCRtpCodec> {
            self.track.lock().codec(ssrc).cloned()
        }
        async fn ssrcs(&self) -> Vec<u32> {
            self.track.lock().ssrcs().collect()
        }
        async fn enabled(&self) -> bool {
            self.track.lock().enabled()
        }
        async fn set_enabled(&self, enabled: bool) {
            self.track.lock().set_enabled(enabled);
        }
        async fn muted(&self) -> bool {
            self.track.lock().muted()
        }
        async fn set_muted(&self, muted: bool) {
            self.track.lock().set_muted(muted);
        }
        async fn ready_state(&self) -> MediaStreamTrackState {
            self.track.lock().ready_state()
        }
        async fn stop(&self) {
            self.track.lock().stop();
        }
        async fn get_capabilities(&self) -> MediaTrackCapabilities {
            self.track.lock().get_capabilities().clone()
        }
        async fn get_constraints(&self) -> MediaTrackConstraints {
            self.track.lock().get_constraints().clone()
        }
        async fn get_settings(&self) -> MediaTrackSettings {
            self.track.lock().get_settings().clone()
        }
        async fn apply_constraints(&self, constraints: Option<MediaTrackConstraints>) {
            self.track.lock().apply_constraints(constraints);
        }
        async fn codings(&self) -> Vec<RTCRtpEncodingParameters> {
            self.track.lock().codings().to_vec()
        }
        async fn add_coding(&self, coding: RTCRtpEncodingParameters) {
            self.track.lock().add_coding(coding);
        }
    }

    #[async_trait]
    impl TrackRemote for ScriptedRemoteTrack {
        async fn write_rtcp(
            &self,
            _packets: Vec<Box<dyn rtc::rtcp::Packet>>,
        ) -> webrtc::error::Result<()> {
            Ok(())
        }

        async fn poll(&self) -> Option<TrackRemoteEvent> {
            self.events.lock().await.recv().await
        }
    }

    fn local_track() -> Arc<TrackLocalStaticRTP> {
        Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            "local-stream".into(),
            "local-audio".into(),
            "local-audio".into(),
            RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(0x0102_0304),
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

    fn opus() -> CodecInfo {
        CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48_000,
            channels: 1,
            fmtp: None,
            payload_type: Some(111),
        }
    }

    fn from_tracks_with_inbound_queue(
        remote: Arc<ScriptedRemoteTrack>,
        queue: InboundFrameQueue,
    ) -> Arc<WebRtcMediaStream> {
        from_tracks_with_dtmf_events_and_task_counter(
            StreamId::new(),
            opus(),
            local_track(),
            0x0102_0304,
            111,
            Some(remote),
            None,
            Vec::new(),
            None,
            None,
            queue,
        )
    }

    /// Driver-side event queue of the remote track (the stack uses 256).
    const TRACK_EVENT_CAPACITY: usize = 256;
    const REMOTE_SSRC: u32 = 0x0a0b_0c0d;

    struct Run {
        /// Packets the remote track lost because the pump fell behind.
        lost_at_track: usize,
        /// RTP sequence numbers delivered within 100 ms after the consumer
        /// started reading: the queued backlog plus about five live frames.
        burst: Vec<u16>,
        /// Every delivered sequence number, in order.
        delivered: Vec<u16>,
        frames_dropped: u64,
    }

    /// 400 packets at 20 ms pacing; the consumer starts reading after 7 s.
    async fn stalled_consumer_run(queue: InboundFrameQueue) -> Run {
        let (remote, events) = ScriptedRemoteTrack::opus(REMOTE_SSRC, TRACK_EVENT_CAPACITY);
        let stream = from_tracks_with_inbound_queue(remote, queue);
        let mut frames_in = stream.try_frames_in().expect("receiver");

        let started = tokio::time::Instant::now();
        let producer = tokio::spawn(async move {
            let mut lost = 0;
            for index in 0..400u16 {
                tokio::time::sleep_until(started + Duration::from_millis(u64::from(index) * 20))
                    .await;
                let packet = silent_rtp_packet_for_ssrc(REMOTE_SSRC, index, u32::from(index) * 960);
                if events
                    .try_send(TrackRemoteEvent::OnRtpPacket(packet))
                    .is_err()
                {
                    lost += 1;
                }
            }
            // Keep the track open: a closed event queue ends the pump.
            (lost, events)
        });

        tokio::time::sleep_until(started + Duration::from_secs(7)).await;
        let attached = tokio::time::Instant::now();
        let mut burst = Vec::new();
        let mut delivered = Vec::new();
        while let Ok(Some(frame)) =
            tokio::time::timeout(Duration::from_millis(500), frames_in.recv()).await
        {
            let sequence = u16::try_from(frame.timestamp_rtp / 960).unwrap();
            if attached.elapsed() < Duration::from_millis(100) {
                burst.push(sequence);
            }
            delivered.push(sequence);
        }
        let (lost_at_track, _events) = producer.await.unwrap();
        let frames_dropped = stream.webrtc_stats_snapshot().frames_dropped;
        stream.shutdown_background_tasks().await;
        Run {
            lost_at_track,
            burst,
            delivered,
            frames_dropped,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_consumer_keeps_the_reader_live_and_the_backlog_bounded() {
        let queue = InboundFrameQueue::default();
        let run = stalled_consumer_run(queue).await;
        assert_eq!(
            run.lost_at_track, 0,
            "the RTP reader fell behind the wire while the consumer was stalled"
        );
        assert!(run.frames_dropped > 0, "overflow was not exercised");
        assert!(
            run.burst.len() <= queue.capacity + 6,
            "{} stale frames were released at once after the consumer attached: {:?}",
            run.burst.len(),
            run.burst
        );
        assert_eq!(
            run.delivered.last(),
            Some(&399),
            "live audio did not recover"
        );
        assert!(
            run.delivered.windows(2).all(|pair| pair[0] < pair[1]),
            "frames were reordered"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn configured_capacity_bounds_the_backlog() {
        let queue = InboundFrameQueue {
            capacity: 3,
            send_deadline_ms: 1,
        };
        let run = stalled_consumer_run(queue).await;
        assert_eq!(run.lost_at_track, 0);
        assert!(run.burst.len() <= 3 + 6, "burst {:?}", run.burst);
        assert_eq!(run.delivered.last(), Some(&399));
    }

    #[test]
    fn adapter_streams_use_the_configured_queue() {
        let mut config = crate::config::WebRtcConfig::default();
        assert_eq!(
            InboundFrameQueue::from_config(&config),
            InboundFrameQueue::default()
        );
        config.inbound_frame_queue_capacity = 32;
        config.inbound_send_deadline_ms = 15;
        assert_eq!(
            InboundFrameQueue::from_config(&config),
            InboundFrameQueue {
                capacity: 32,
                send_deadline_ms: 15,
            }
        );
    }
}
