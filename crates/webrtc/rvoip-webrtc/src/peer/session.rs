//! `RvoipPeerConnection` — offer/answer lifecycle on webrtc-rs 0.20.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex as SyncMutex;
use rtc::media_stream::MediaStreamTrack;
use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters,
};
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use webrtc::data_channel::{DataChannel, DataChannelEvent, RTCDataChannelState};
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_remote::TrackRemote;
use webrtc::peer_connection::PeerConnection;
use webrtc::peer_connection::{RTCIceCandidate, RTCIceCandidateInit, RTCSdpType};
use webrtc::rtp_transceiver::RTCRtpTransceiverDirection;

use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};
use crate::peer::builder::{
    self, audio_track_rtcp_feedback, video_track_rtcp_feedback, MIME_TYPE_OPUS,
    MIME_TYPE_TELEPHONE_EVENT, MIME_TYPE_VP8,
};
use crate::peer::handler::{ConnectionHandler, HandlerChannels, HandlerDropCounters};
use crate::peer::ice::IceCandidateLog;
use crate::sdp::inspect::{sdp_advertises_telephone_event, sdp_has_media_line};
use crate::sdp::session::{parse_sdp, sdp_to_string};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerRole {
    Offerer,
    Answerer,
}

impl PeerRole {
    fn name(self) -> &'static str {
        match self {
            PeerRole::Offerer => "offerer",
            PeerRole::Answerer => "answerer",
        }
    }
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
    local_ice_rx: Arc<AsyncMutex<mpsc::Receiver<RTCIceCandidate>>>,
    local_audio: SyncMutex<Option<Arc<TrackLocalStaticRTP>>>,
    local_audio_ssrc: SyncMutex<Option<u32>>,
    local_dtmf: SyncMutex<Option<Arc<TrackLocalStaticRTP>>>,
    local_dtmf_ssrc: SyncMutex<Option<u32>>,
    local_video: SyncMutex<Option<Arc<TrackLocalStaticRTP>>>,
    local_video_ssrc: SyncMutex<Option<u32>>,
    gather_timeout: Duration,
    trickle_ice: bool,
    drops: Arc<HandlerDropCounters>,
}

impl RvoipPeerConnection {
    pub async fn new(config: &WebRtcConfig, role: PeerRole) -> Result<Arc<Self>> {
        let (channels, receivers, connected_flag, failed_flag, ice_candidates, drops) =
            HandlerChannels::pair(config.handler_channel_capacity);
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
            local_ice_rx: Arc::new(AsyncMutex::new(receivers.local_ice)),
            local_audio: SyncMutex::new(None),
            local_audio_ssrc: SyncMutex::new(None),
            local_dtmf: SyncMutex::new(None),
            local_dtmf_ssrc: SyncMutex::new(None),
            local_video: SyncMutex::new(None),
            local_video_ssrc: SyncMutex::new(None),
            gather_timeout,
            trickle_ice: config.trickle_ice,
            drops,
        }))
    }

    pub fn role(&self) -> PeerRole {
        self.role
    }

    /// Drop counters for handler event channels (remote_track, data_channel, state).
    pub fn handler_drop_counters(&self) -> &HandlerDropCounters {
        &self.drops
    }

    fn require_role(&self, expected: PeerRole) -> Result<()> {
        if self.role != expected {
            return Err(WebRtcError::WrongRole {
                expected: expected.name(),
                actual: self.role.name(),
            });
        }
        Ok(())
    }

    /// Blocking receive of the next remote track. Returns `None` when the
    /// underlying channel is closed (peer dropped).
    pub async fn recv_remote_track(&self) -> Option<Arc<dyn TrackRemote>> {
        self.remote_track_rx.lock().await.recv().await
    }

    pub fn peer_connection(&self) -> &Arc<dyn PeerConnection> {
        &self.pc
    }

    /// ICE candidates observed during local gathering (full SDP embeds these inline).
    pub fn gathered_ice_candidates(&self) -> Vec<String> {
        self.ice_candidates.candidates()
    }

    /// Add local tracks on an answerer to mirror the remote offer's `m=` sections.
    ///
    /// D1 — also attaches the dedicated PT 101 track when the offer
    /// advertises `telephone-event`, so the answerer can send DTMF back to
    /// the offerer. When the offer is Opus-only (typical Firefox / Chrome
    /// audio-only offer) we skip the DTMF attach to keep the local
    /// transceiver count matched to the offer's m-lines.
    pub async fn prepare_answerer_media_for_offer(
        self: &Arc<Self>,
        remote_sdp: &str,
    ) -> Result<()> {
        if sdp_has_media_line(remote_sdp, "audio") {
            self.add_local_audio_track().await?;
            if sdp_advertises_telephone_event(remote_sdp) {
                self.add_local_dtmf_track().await?;
            }
        }
        if sdp_has_media_line(remote_sdp, "video") {
            self.add_local_video_track().await?;
        }
        Ok(())
    }

    /// Apply a remote trickle ICE candidate.
    pub async fn add_remote_ice_candidate(
        self: &Arc<Self>,
        candidate: RTCIceCandidateInit,
    ) -> Result<()> {
        self.pc
            .add_ice_candidate(candidate)
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("add_ice_candidate: {e}")))
    }

    /// Try to receive the next locally-gathered ICE candidate (non-blocking).
    /// Used by trickle signalers to forward outbound candidates to the remote.
    pub async fn try_recv_local_ice(&self) -> Option<RTCIceCandidate> {
        self.local_ice_rx.lock().await.try_recv().ok()
    }

    /// Receive the next locally-gathered ICE candidate; awaits until one
    /// arrives or the channel closes (peer dropped).
    pub async fn recv_local_ice(&self) -> Option<RTCIceCandidate> {
        self.local_ice_rx.lock().await.recv().await
    }

    /// Drain all currently-buffered local ICE candidates without blocking.
    /// Helpful right after `create_offer_and_gather` in trickle mode for the
    /// first batch.
    pub async fn drain_local_ice(&self) -> Vec<RTCIceCandidate> {
        let mut out = Vec::new();
        let mut rx = self.local_ice_rx.lock().await;
        while let Ok(c) = rx.try_recv() {
            out.push(c);
        }
        out
    }

    /// Ask webrtc-rs to restart ICE (fresh ufrag/pwd on the next offer).
    pub async fn restart_ice(self: &Arc<Self>) -> Result<()> {
        self.pc
            .restart_ice()
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("restart_ice: {e}")))
    }

    async fn wait_gather(&self) -> Result<()> {
        if self.trickle_ice {
            // Trickle mode: return immediately; candidates will arrive via
            // [`Self::recv_local_ice`] for the signaler to forward.
            return Ok(());
        }
        let mut rx = self.gather_rx.lock().await;
        tokio::time::timeout(self.gather_timeout, rx.recv())
            .await
            .map_err(|_| WebRtcError::Timeout("ICE gathering"))?
            .ok_or_else(|| WebRtcError::Webrtc("gather channel closed".into()))?;
        Ok(())
    }

    /// Returns true if this peer is in trickle mode.
    pub fn is_trickle(&self) -> bool {
        self.trickle_ice
    }

    /// Add a local Opus audio track (required before offer/answer for media).
    ///
    /// D1 — this attaches **only the Opus** track. The dedicated RFC 4733
    /// telephone-event track (PT 101, separate SSRC) is opt-in via
    /// [`Self::add_local_dtmf_track`] — it's auto-attached on the offerer's
    /// `create_offer_and_gather` (so outbound DTMF works by default) and on
    /// the answerer when the remote offer advertises `telephone-event`.
    /// Splitting these keeps the local-transceiver count matched to the
    /// remote m-lines when the peer offers Opus-only (typical browser).
    pub async fn add_local_audio_track(self: &Arc<Self>) -> Result<()> {
        if self.local_audio.lock().is_some() {
            return Ok(());
        }

        let opus = RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: 48000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1".into(),
            rtcp_feedback: audio_track_rtcp_feedback(),
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

    /// D1 — attach a dedicated RFC 4733 telephone-event track (PT 101) on
    /// its own SSRC, bound to the peer connection via a second `add_track`
    /// call. This is what [`crate::media::dtmf::send_dtmf`] writes to, so
    /// PT 101 packets carry their negotiated codec on the wire and survive
    /// SRTP filtering on the remote side.
    ///
    /// The naive two-encoding-per-track approach broke the inbound
    /// round-trip because webrtc-rs 0.20-alpha dispatches inbound packets
    /// by encoding order, not payload type; using separate tracks keeps
    /// receive-side demux one-PT-per-encoding.
    ///
    /// Idempotent — safe to call multiple times.
    pub async fn add_local_dtmf_track(self: &Arc<Self>) -> Result<()> {
        if self.local_dtmf.lock().is_some() {
            return Ok(());
        }

        let telephone_event = RTCRtpCodec {
            mime_type: MIME_TYPE_TELEPHONE_EVENT.to_owned(),
            clock_rate: 8000,
            channels: 1,
            sdp_fmtp_line: "0-15".into(),
            rtcp_feedback: vec![],
        };
        let dtmf_ssrc = rand_ssrc();
        let dtmf_track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            format!("rvoip-dtmf-stream-{dtmf_ssrc}"),
            format!("rvoip-dtmf-track-{dtmf_ssrc}"),
            "rvoip-audio".into(),
            RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(dtmf_ssrc),
                    ..Default::default()
                },
                codec: telephone_event,
                ..Default::default()
            }],
        )));

        self.pc
            .add_track(dtmf_track.clone() as Arc<dyn TrackLocal>)
            .await?;

        *self.local_dtmf.lock() = Some(dtmf_track);
        *self.local_dtmf_ssrc.lock() = Some(dtmf_ssrc);
        Ok(())
    }

    pub fn local_audio_track(&self) -> Option<Arc<TrackLocalStaticRTP>> {
        self.local_audio.lock().clone()
    }

    /// SSRC of the local audio encoding (for RFC 4733 DTMF).
    pub fn local_audio_ssrc(&self) -> Option<u32> {
        *self.local_audio_ssrc.lock()
    }

    /// D1 — the dedicated RFC 4733 telephone-event track (PT 101). Returns
    /// `None` when no audio has been negotiated yet, or when the underlying
    /// `add_track` for the secondary track was rejected by webrtc-rs.
    pub fn local_dtmf_track(&self) -> Option<Arc<TrackLocalStaticRTP>> {
        self.local_dtmf.lock().clone()
    }

    /// D1 — SSRC of the dedicated RFC 4733 track.
    pub fn local_dtmf_ssrc(&self) -> Option<u32> {
        *self.local_dtmf_ssrc.lock()
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
            rtcp_feedback: video_track_rtcp_feedback(),
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
    ///
    /// `opts` controls reliability/ordering/protocol per RFC 8832 §5.1 — see
    /// [`DataChannelOptions`](crate::peer::DataChannelOptions) for the
    /// available knobs (use `DataChannelOptions::reliable()` to match the
    /// previous default behavior).
    pub async fn create_data_channel(
        self: &Arc<Self>,
        label: &str,
        opts: crate::peer::DataChannelOptions,
    ) -> Result<Arc<dyn DataChannel>> {
        opts.validate()?;
        self.pc
            .create_data_channel(label, Some(opts.to_rtc_init()))
            .await
            .map_err(|e| WebRtcError::Webrtc(format!("create_data_channel: {e}")))
    }

    /// Create a labeled SCTP data channel and return the typed
    /// [`RvoipDataChannel`](crate::peer::RvoipDataChannel) wrapper. Use this
    /// in new code; the raw [`Self::create_data_channel`] is preserved for
    /// callers that need the bare trait object.
    pub async fn create_data_channel_typed(
        self: &Arc<Self>,
        label: &str,
        opts: crate::peer::DataChannelOptions,
    ) -> Result<crate::peer::RvoipDataChannel> {
        let dc = self.create_data_channel(label, opts).await?;
        Ok(crate::peer::RvoipDataChannel::new(dc, label.to_string()))
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
        tokio::time::timeout(timeout, dc.poll())
            .await
            .ok()
            .flatten()
    }

    /// Wait until a data channel reports `OnOpen`.
    pub async fn wait_data_channel_open(
        dc: &Arc<dyn DataChannel>,
        timeout: Duration,
    ) -> Result<()> {
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
                return Err(WebRtcError::Webrtc(
                    "data channel closed before open".into(),
                ));
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
                        return Err(WebRtcError::Webrtc(
                            "data channel closed before open".into(),
                        ));
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
        self.require_role(PeerRole::Offerer)?;

        if self.local_audio.lock().is_none() && self.local_video.lock().is_none() {
            self.add_local_audio_track().await?;
            // D1 — auto-attach the DTMF track on the offerer so outbound
            // PT 101 telephone-event packets are advertised in the offer.
            // Best-effort: a webrtc-rs build that rejects the second audio
            // track must still ship Opus, so we log + carry on.
            if let Err(e) = self.add_local_dtmf_track().await {
                tracing::warn!(target: "rvoip_webrtc", error = %e, "failed to attach DTMF track; outbound DTMF will use the Opus track and may not survive SRTP filtering");
            }
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
        self.require_role(PeerRole::Answerer)?;

        self.prepare_answerer_media_for_offer(remote_sdp).await?;

        let remote = parse_sdp(remote_sdp, RTCSdpType::Offer)?;
        self.pc.set_remote_description(remote).await?;
        Ok(())
    }

    /// Answerer: create answer, gather ICE, return SDP (after [`Self::apply_remote_offer`]).
    pub async fn create_answer_and_gather(self: &Arc<Self>) -> Result<String> {
        self.require_role(PeerRole::Answerer)?;

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
        self.require_role(PeerRole::Answerer)?;

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
        self.require_role(PeerRole::Offerer)?;

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

    /// G3 — true when the underlying peer connection has no pending local or
    /// remote description (W3C "stable" signaling state). Used by the
    /// perfect-negotiation helper to decide whether an incoming offer
    /// collides with our own pending offer.
    pub async fn signaling_is_stable(&self) -> bool {
        self.pc.pending_local_description().await.is_none()
            && self.pc.pending_remote_description().await.is_none()
    }

    /// G11 — SDP rollback (JSEP §4.1.10.2).
    ///
    /// Used by the perfect-negotiation pattern to undo an unfinished local
    /// offer when a competing remote offer arrives. Calls
    /// `setLocalDescription({ type: "rollback" })` on the underlying
    /// webrtc-rs peer connection. Returns `Err(InvalidState)` if the
    /// signaling state is `stable` (nothing to rollback).
    pub async fn rollback_local(self: &Arc<Self>) -> Result<()> {
        let rollback = rtc::peer_connection::sdp::RTCSessionDescription::rollback(None)
            .map_err(|e| WebRtcError::Webrtc(format!("build rollback: {e}")))?;
        self.pc.set_local_description(rollback).await.map_err(|e| {
            // Translate the "already stable" error into our typed variant.
            let msg = e.to_string();
            if msg.contains("InvalidModificationError")
                || msg.contains("InvalidState")
                || msg.contains("stable")
            {
                WebRtcError::InvalidState("rollback called in stable signaling state")
            } else {
                WebRtcError::Webrtc(format!("rollback: {e}"))
            }
        })?;
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
        let audio = self
            .discover_remote_track(RtpCodecKind::Audio)
            .await
            .is_some();
        let video = self
            .discover_remote_track(RtpCodecKind::Video)
            .await
            .is_some();
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

            let pkt = crate::media::pump::silent_rtp_packet_for_ssrc(ssrc, seq, seq as u32 * 960);
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
