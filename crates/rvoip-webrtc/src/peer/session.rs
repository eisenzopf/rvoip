//! `RvoipPeerConnection` — offer/answer lifecycle on webrtc-rs 0.20.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex as SyncMutex;
use rtc::media_stream::MediaStreamTrack;
use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters};
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use webrtc::data_channel::{DataChannel, DataChannelEvent, RTCDataChannelState};
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_remote::TrackRemote;
use webrtc::peer_connection::PeerConnection;
use webrtc::peer_connection::{RTCSdpType, RTCSessionDescription};
use webrtc::rtp_transceiver::{RtpTransceiver, RTCRtpTransceiverDirection};

use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};
use crate::peer::builder::{self, MIME_TYPE_OPUS, MIME_TYPE_VP8};
use crate::peer::handler::{ConnectionHandler, HandlerChannels};
use crate::peer::ice::IceCandidateLog;
use crate::sdp::inspect::sdp_has_media_line;
use crate::sdp::session::{parse_sdp, sdp_to_string};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerRole {
    Offerer,
    Answerer,
}

/// One Participant's WebRTC attach — wraps `Arc<dyn PeerConnection>` plus
/// signaling state and handler channels.
pub struct RvoipPeerConnection {
    pc: Arc<dyn PeerConnection>,
    role: PeerRole,
    gather_rx: Arc<AsyncMutex<mpsc::Receiver<()>>>,
    connected_rx: Arc<AsyncMutex<mpsc::Receiver<()>>>,
    connected_flag: Arc<AtomicBool>,
    failed_rx: Arc<AsyncMutex<mpsc::Receiver<()>>>,
    failed_flag: Arc<AtomicBool>,
    ice_candidates: IceCandidateLog,
    remote_track_rx: Arc<AsyncMutex<mpsc::Receiver<Arc<dyn TrackRemote>>>>,
    data_channel_rx: Arc<AsyncMutex<mpsc::Receiver<Arc<dyn DataChannel>>>>,
    local_audio: SyncMutex<Option<Arc<TrackLocalStaticRTP>>>,
    local_audio_ssrc: SyncMutex<Option<u32>>,
    local_video: SyncMutex<Option<Arc<TrackLocalStaticRTP>>>,
    local_video_ssrc: SyncMutex<Option<u32>>,
    gather_timeout: Duration,
}

impl RvoipPeerConnection {
    pub async fn new(config: &WebRtcConfig, role: PeerRole) -> Result<Arc<Self>> {
        let (channels, receivers, connected_flag, failed_flag, ice_candidates) =
            HandlerChannels::pair(8);
        let handler = ConnectionHandler::new(channels);
        let pc = builder::build_peer_connection(config, handler).await?;
        let gather_timeout = Duration::from_secs(config.gather_timeout_secs);

        Ok(Arc::new(Self {
            pc,
            role,
            gather_rx: Arc::new(AsyncMutex::new(receivers.gather_complete)),
            connected_rx: Arc::new(AsyncMutex::new(receivers.connected)),
            connected_flag,
            failed_rx: Arc::new(AsyncMutex::new(receivers.failed)),
            failed_flag,
            ice_candidates,
            remote_track_rx: Arc::new(AsyncMutex::new(receivers.remote_track)),
            data_channel_rx: Arc::new(AsyncMutex::new(receivers.data_channel)),
            local_audio: SyncMutex::new(None),
            local_audio_ssrc: SyncMutex::new(None),
            local_video: SyncMutex::new(None),
            local_video_ssrc: SyncMutex::new(None),
            gather_timeout,
        }))
    }

    pub fn role(&self) -> PeerRole {
        self.role
    }

    pub fn peer_connection(&self) -> &Arc<dyn PeerConnection> {
        &self.pc
    }

    /// ICE candidates observed during local gathering (full SDP embeds these inline).
    pub fn gathered_ice_candidates(&self) -> Vec<String> {
        self.ice_candidates.candidates()
    }

    /// Add local tracks on an answerer to mirror the remote offer's `m=` sections.
    pub async fn prepare_answerer_media_for_offer(self: &Arc<Self>, remote_sdp: &str) -> Result<()> {
        if sdp_has_media_line(remote_sdp, "audio") {
            self.add_local_audio_track().await?;
        }
        if sdp_has_media_line(remote_sdp, "video") {
            self.add_local_video_track().await?;
        }
        Ok(())
    }

    /// Apply a remote trickle ICE candidate (signaling not wired in v1).
    pub async fn add_remote_ice_candidate(
        self: &Arc<Self>,
        candidate: webrtc::peer_connection::RTCIceCandidateInit,
    ) -> Result<()> {
        self.pc
            .add_ice_candidate(candidate)
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("add_ice_candidate: {e}")))
    }

    async fn wait_gather(&self) -> Result<()> {
        let mut rx = self.gather_rx.lock().await;
        tokio::time::timeout(self.gather_timeout, rx.recv())
            .await
            .map_err(|_| WebRtcError::Timeout("ICE gathering"))?
            .ok_or_else(|| WebRtcError::Webrtc("gather channel closed".into()))?;
        Ok(())
    }

    /// Add a local Opus audio track (required before offer/answer for media).
    pub async fn add_local_audio_track(self: &Arc<Self>) -> Result<()> {
        if self.local_audio.lock().is_some() {
            return Ok(());
        }

        let opus = RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: 48000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1".into(),
            rtcp_feedback: vec![],
        };
        let ssrc = rand_ssrc();
        let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            format!("rvoip-stream-{ssrc}"),
            format!("rvoip-track-{ssrc}"),
            "rvoip-audio".into(),
            RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                codec: opus,
                ..Default::default()
            }],
        )));

        self.pc
            .add_track(track.clone() as Arc<dyn TrackLocal>)
            .await?;

        *self.local_audio.lock() = Some(track);
        *self.local_audio_ssrc.lock() = Some(ssrc);
        Ok(())
    }

    pub fn local_audio_track(&self) -> Option<Arc<TrackLocalStaticRTP>> {
        self.local_audio.lock().clone()
    }

    /// SSRC of the local audio encoding (for RFC 4733 DTMF).
    pub fn local_audio_ssrc(&self) -> Option<u32> {
        *self.local_audio_ssrc.lock()
    }

    /// Add a local VP8 video track (call before offer/answer when video is requested).
    pub async fn add_local_video_track(self: &Arc<Self>) -> Result<()> {
        if self.local_video.lock().is_some() {
            return Ok(());
        }

        let vp8 = RTCRtpCodec {
            mime_type: MIME_TYPE_VP8.to_owned(),
            clock_rate: 90000,
            channels: 0,
            sdp_fmtp_line: String::new(),
            rtcp_feedback: vec![],
        };
        let ssrc = rand_ssrc();
        let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            format!("rvoip-vstream-{ssrc}"),
            format!("rvoip-vtrack-{ssrc}"),
            "rvoip-video".into(),
            RtpCodecKind::Video,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                codec: vp8,
                ..Default::default()
            }],
        )));

        self.pc
            .add_track(track.clone() as Arc<dyn TrackLocal>)
            .await?;

        *self.local_video.lock() = Some(track);
        *self.local_video_ssrc.lock() = Some(ssrc);
        Ok(())
    }

    pub fn local_video_track(&self) -> Option<Arc<TrackLocalStaticRTP>> {
        self.local_video.lock().clone()
    }

    pub fn local_video_ssrc(&self) -> Option<u32> {
        *self.local_video_ssrc.lock()
    }

    /// Create a labeled SCTP data channel (must be called before offer for offerer).
    pub async fn create_data_channel(self: &Arc<Self>, label: &str) -> Result<Arc<dyn DataChannel>> {
        self.pc
            .create_data_channel(label, None)
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("create_data_channel: {e}")))
    }

    pub async fn try_recv_data_channel(&self) -> Option<Arc<dyn DataChannel>> {
        self.data_channel_rx.lock().await.try_recv().ok()
    }

    pub async fn wait_data_channel(&self, timeout: Duration) -> Option<Arc<dyn DataChannel>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(dc) = self.try_recv_data_channel().await {
                return Some(dc);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Poll for the next data-channel event with a bounded wait.
    ///
    /// webrtc-rs `DataChannel::poll` blocks on an internal channel; use this
    /// helper so callers can enforce deadlines.
    pub async fn poll_data_channel(
        dc: &Arc<dyn DataChannel>,
        timeout: Duration,
    ) -> Option<DataChannelEvent> {
        tokio::time::timeout(timeout, dc.poll()).await.ok().flatten()
    }

    /// Wait until a data channel reports `OnOpen`.
    pub async fn wait_data_channel_open(dc: &Arc<dyn DataChannel>, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let state = dc
                .ready_state()
                .await
                .map_err(|e| WebRtcError::Webrtc(format!("data channel ready_state: {e}")))?;
            if state == RTCDataChannelState::Open {
                return Ok(());
            }
            if matches!(
                state,
                RTCDataChannelState::Closing | RTCDataChannelState::Closed
            ) {
                return Err(WebRtcError::Webrtc("data channel closed before open".into()));
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(WebRtcError::Timeout("data channel open"));
            }
            let remaining = deadline - tokio::time::Instant::now();
            if let Some(event) =
                Self::poll_data_channel(dc, remaining.min(Duration::from_millis(50))).await
            {
                match event {
                    DataChannelEvent::OnOpen => return Ok(()),
                    DataChannelEvent::OnError => {
                        return Err(WebRtcError::Webrtc("data channel error".into()));
                    }
                    DataChannelEvent::OnClose => {
                        return Err(WebRtcError::Webrtc("data channel closed before open".into()));
                    }
                    _ => {}
                }
            }
        }
    }

    /// Receive the next text message on a data channel.
    pub async fn recv_data_channel_text(
        dc: &Arc<dyn DataChannel>,
        timeout: Duration,
    ) -> Result<String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(WebRtcError::Timeout("data channel message"));
            }
            let remaining = deadline - tokio::time::Instant::now();
            if let Some(event) =
                Self::poll_data_channel(dc, remaining.min(Duration::from_millis(100))).await
            {
                if let DataChannelEvent::OnMessage(msg) = event {
                    if msg.is_string {
                        return Ok(String::from_utf8_lossy(&msg.data).into_owned());
                    }
                }
            }
        }
    }

    /// Offerer: create offer, set local description, wait for ICE, return SDP string.
    pub async fn create_offer_and_gather(self: &Arc<Self>) -> Result<String> {
        assert_eq!(self.role, PeerRole::Offerer);

        if self.local_audio.lock().is_none() && self.local_video.lock().is_none() {
            self.add_local_audio_track().await?;
        }

        let offer = self.pc.create_offer(None).await?;
        self.pc.set_local_description(offer).await?;
        self.wait_gather().await?;

        let desc = self
            .pc
            .local_description()
            .await
            .ok_or_else(|| WebRtcError::Sdp("no local description after gather".into()))?;

        sdp_to_string(&desc)
    }

    /// Answerer: apply remote offer SDP without creating the answer yet.
    pub async fn apply_remote_offer(self: &Arc<Self>, remote_sdp: &str) -> Result<()> {
        assert_eq!(self.role, PeerRole::Answerer);

        self.prepare_answerer_media_for_offer(remote_sdp).await?;

        let remote = parse_sdp(remote_sdp, RTCSdpType::Offer)?;
        self.pc.set_remote_description(remote).await?;
        Ok(())
    }

    /// Answerer: create answer, gather ICE, return SDP (after [`Self::apply_remote_offer`]).
    pub async fn create_answer_and_gather(self: &Arc<Self>) -> Result<String> {
        assert_eq!(self.role, PeerRole::Answerer);

        let answer = self.pc.create_answer(None).await?;
        self.pc.set_local_description(answer).await?;
        self.wait_gather().await?;

        let desc = self
            .pc
            .local_description()
            .await
            .ok_or_else(|| WebRtcError::Sdp("no local description after gather".into()))?;

        sdp_to_string(&desc)
    }

    /// Answerer: apply remote offer, create answer, gather, return local SDP.
    pub async fn accept_offer_and_gather(self: &Arc<Self>, remote_sdp: &str) -> Result<String> {
        self.apply_remote_offer(remote_sdp).await?;
        self.create_answer_and_gather().await
    }

    /// Answerer: ICE restart / renegotiation — new remote offer → fresh answer SDP.
    pub async fn renegotiate_as_answerer(self: &Arc<Self>, remote_sdp: &str) -> Result<String> {
        assert_eq!(self.role, PeerRole::Answerer);

        let remote = parse_sdp(remote_sdp, RTCSdpType::Offer)?;
        self.pc.set_remote_description(remote).await?;
        let answer = self.pc.create_answer(None).await?;
        self.pc.set_local_description(answer).await?;
        self.wait_gather().await?;

        let desc = self
            .pc
            .local_description()
            .await
            .ok_or_else(|| WebRtcError::Sdp("no local description after gather".into()))?;

        sdp_to_string(&desc)
    }

    /// Offerer: ICE restart — create a fresh offer SDP after the session is up.
    pub async fn renegotiate_as_offerer(self: &Arc<Self>) -> Result<String> {
        assert_eq!(self.role, PeerRole::Offerer);

        let offer = self.pc.create_offer(None).await?;
        self.pc.set_local_description(offer).await?;
        self.wait_gather().await?;

        let desc = self
            .pc
            .local_description()
            .await
            .ok_or_else(|| WebRtcError::Sdp("no local description after gather".into()))?;

        sdp_to_string(&desc)
    }

    /// Offerer: apply remote answer after signaling exchange.
    pub async fn set_remote_answer(self: &Arc<Self>, remote_sdp: &str) -> Result<()> {
        let remote = parse_sdp(remote_sdp, RTCSdpType::Answer)?;
        self.pc.set_remote_description(remote).await?;
        Ok(())
    }

    /// Returns true once ICE/DTLS has reached `Connected`.
    pub fn is_connected(&self) -> bool {
        self.connected_flag.load(Ordering::Acquire)
    }

    /// Wait until `RTCPeerConnectionState::Connected` or timeout.
    ///
    /// Multiple concurrent waiters are supported — connection state is tracked
    /// via a shared flag in addition to the one-shot handler channel.
    pub async fn wait_connected(self: &Arc<Self>, timeout: Duration) -> Result<()> {
        if self.is_connected() {
            return Ok(());
        }
        if self.failed_flag.load(Ordering::Acquire) {
            return Err(WebRtcError::Webrtc("peer connection failed".into()));
        }

        let connected_rx = Arc::clone(&self.connected_rx);
        let failed_rx = Arc::clone(&self.failed_rx);
        let connected_flag = Arc::clone(&self.connected_flag);
        let failed_flag = Arc::clone(&self.failed_flag);

        tokio::time::timeout(timeout, async move {
            loop {
                if connected_flag.load(Ordering::Acquire) {
                    return Ok(());
                }
                if failed_flag.load(Ordering::Acquire) {
                    return Err(WebRtcError::Webrtc("peer connection failed".into()));
                }

                tokio::select! {
                    msg = async {
                        connected_rx.lock().await.recv().await
                    } => {
                        if msg.is_some() {
                            connected_flag.store(true, Ordering::Release);
                            return Ok(());
                        }
                    }
                    msg = async {
                        failed_rx.lock().await.recv().await
                    } => {
                        if msg.is_some() {
                            failed_flag.store(true, Ordering::Release);
                            return Err(WebRtcError::Webrtc("peer connection failed".into()));
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                }
            }
        })
        .await
        .map_err(|_| WebRtcError::Timeout("peer connection"))?
    }

    pub async fn close(&self) -> Result<()> {
        self.pc.close().await?;
        Ok(())
    }

    /// Poll the handler for the next remote track (non-blocking).
    pub async fn try_recv_remote_track(&self) -> Option<Arc<dyn TrackRemote>> {
        self.remote_track_rx.lock().await.try_recv().ok()
    }

    /// Discover a remote track of `kind` from transceivers.
    pub async fn discover_remote_track(&self, kind: RtpCodecKind) -> Option<Arc<dyn TrackRemote>> {
        use webrtc::media_stream::Track;

        for tx in self.pc.get_transceivers().await {
            let Ok(Some(receiver)) = tx.receiver().await else {
                continue;
            };
            let track = receiver.track().clone();
            if track.kind().await == kind {
                return Some(track);
            }
        }
        None
    }

    /// Discover a remote audio track from transceivers (fallback when `on_track`
    /// fired before the consumer started listening).
    pub async fn discover_remote_audio_track(&self) -> Option<Arc<dyn TrackRemote>> {
        self.discover_remote_track(RtpCodecKind::Audio).await
    }

    pub async fn discover_remote_video_track(&self) -> Option<Arc<dyn TrackRemote>> {
        self.discover_remote_track(RtpCodecKind::Video).await
    }

    /// Wait up to `timeout` for the first remote track of `kind`.
    pub async fn wait_remote_track_kind(
        &self,
        kind: RtpCodecKind,
        timeout: Duration,
    ) -> Option<Arc<dyn TrackRemote>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(track) = self.try_recv_remote_track().await {
                if track.kind().await == kind {
                    return Some(track);
                }
            }
            if let Some(track) = self.discover_remote_track(kind).await {
                return Some(track);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Wait up to `timeout` for the first remote audio track.
    pub async fn wait_remote_track(&self, timeout: Duration) -> Option<Arc<dyn TrackRemote>> {
        self.wait_remote_track_kind(RtpCodecKind::Audio, timeout)
            .await
    }

    pub async fn wait_remote_video_track(&self, timeout: Duration) -> Option<Arc<dyn TrackRemote>> {
        self.wait_remote_track_kind(RtpCodecKind::Video, timeout)
            .await
    }

    /// Returns whether a remote track of each kind has been observed.
    pub async fn remote_media_ready(&self) -> (bool, bool) {
        let audio = self.discover_remote_track(RtpCodecKind::Audio).await.is_some();
        let video = self.discover_remote_track(RtpCodecKind::Video).await.is_some();
        (audio, video)
    }

    /// Send silent RTP until `receiver` observes a remote track (webrtc-rs fires
    /// `on_track` only after the first inbound RTP packet).
    pub async fn prime_remote_track(
        sender: &Arc<Self>,
        receiver: &Arc<Self>,
        timeout: Duration,
    ) -> Option<Arc<dyn TrackRemote>> {
        use webrtc::media_stream::track_local::TrackLocal;

        let local = sender.local_audio_track()?;
        let ssrc = sender.local_audio_ssrc()?;
        let mut seq: u16 = 1;
        let deadline = tokio::time::Instant::now() + timeout;

        while tokio::time::Instant::now() < deadline {
            if let Some(track) = receiver.wait_remote_track(Duration::from_millis(50)).await {
                return Some(track);
            }

            let pkt =
                crate::media::pump::silent_rtp_packet_for_ssrc(ssrc, seq, seq as u32 * 960);
            let _ = local.write_rtp(pkt).await;
            seq = seq.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        None
    }

    /// Send silent audio + optional VP8 RTP until the receiver observes remote tracks.
    pub async fn prime_remote_media(
        sender: &Arc<Self>,
        receiver: &Arc<Self>,
        include_video: bool,
        timeout: Duration,
    ) -> (Option<Arc<dyn TrackRemote>>, Option<Arc<dyn TrackRemote>>) {
        use webrtc::media_stream::track_local::TrackLocal;

        let audio_local = sender.local_audio_track();
        let audio_ssrc = sender.local_audio_ssrc();
        let video_local = if include_video {
            sender.local_video_track()
        } else {
            None
        };
        let video_ssrc = sender.local_video_ssrc();

        let mut seq: u16 = 1;
        let deadline = tokio::time::Instant::now() + timeout;

        while tokio::time::Instant::now() < deadline {
            let (audio_ready, video_ready) = receiver.remote_media_ready().await;
            if audio_ready && (!include_video || video_ready) {
                break;
            }

            if let (Some(track), Some(ssrc)) = (&audio_local, audio_ssrc) {
                let pkt =
                    crate::media::pump::silent_rtp_packet_for_ssrc(ssrc, seq, seq as u32 * 960);
                let _ = track.write_rtp(pkt).await;
            }
            if let (Some(track), Some(ssrc)) = (&video_local, video_ssrc) {
                let pkt = crate::media::pump::silent_vp8_rtp_packet_for_ssrc(
                    ssrc,
                    seq,
                    seq as u32 * 3000,
                );
                let _ = track.write_rtp(pkt).await;
            }
            seq = seq.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        (
            receiver.discover_remote_track(RtpCodecKind::Audio).await,
            if include_video {
                receiver.discover_remote_track(RtpCodecKind::Video).await
            } else {
                None
            },
        )
    }

    /// Block until the peer connection reports `Failed` (polls; does not hold locks across await).
    pub async fn wait_failed(&self) {
        loop {
            if self.failed_flag.load(Ordering::Acquire) {
                break;
            }
            if self.failed_rx.lock().await.try_recv().is_ok() {
                self.failed_flag.store(true, Ordering::Release);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Hold: mute local track; best-effort recvonly on audio transceivers.
    pub async fn hold_audio(&self) -> Result<()> {
        use webrtc::media_stream::Track;

        if let Some(track) = self.local_audio_track() {
            let _ = tokio::time::timeout(Duration::from_millis(500), track.set_muted(true)).await;
        }
        let _ = tokio::time::timeout(
            Duration::from_millis(500),
            self.set_audio_direction(RTCRtpTransceiverDirection::Recvonly),
        )
        .await;
        Ok(())
    }

    /// Resume: unmute local track; best-effort sendrecv on audio transceivers.
    pub async fn resume_audio(&self) -> Result<()> {
        use webrtc::media_stream::Track;

        if let Some(track) = self.local_audio_track() {
            let _ = tokio::time::timeout(Duration::from_millis(500), track.set_muted(false)).await;
        }
        let _ = tokio::time::timeout(
            Duration::from_millis(500),
            self.set_audio_direction(RTCRtpTransceiverDirection::Sendrecv),
        )
        .await;
        Ok(())
    }

    async fn set_audio_direction(&self, direction: RTCRtpTransceiverDirection) -> Result<()> {
        use webrtc::media_stream::Track;

        for tx in self.pc.get_transceivers().await {
            let Some(sender) = tx.sender().await? else {
                continue;
            };
            if sender.track().kind().await == RtpCodecKind::Audio {
                tx.set_direction(direction).await?;
            }
        }
        Ok(())
    }
}

fn rand_ssrc() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        % u32::MAX) as u32
        | 1
}

/// In-process offer/answer between two peer connections (loopback UDP).
pub async fn connect_loopback(
    config: &WebRtcConfig,
) -> Result<(Arc<RvoipPeerConnection>, Arc<RvoipPeerConnection>)> {
    let offerer = RvoipPeerConnection::new(config, PeerRole::Offerer).await?;
    let answerer = RvoipPeerConnection::new(config, PeerRole::Answerer).await?;

    let offer_sdp = offerer.create_offer_and_gather().await?;
    let answer_sdp = answerer.accept_offer_and_gather(&offer_sdp).await?;
    offerer.set_remote_answer(&answer_sdp).await?;

    let timeout = Duration::from_secs(config.gather_timeout_secs + 10);
    tokio::try_join!(
        offerer.wait_connected(timeout),
        answerer.wait_connected(timeout)
    )?;

    Ok((offerer, answerer))
}
