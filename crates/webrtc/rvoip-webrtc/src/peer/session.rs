//! `RvoipPeerConnection` — offer/answer lifecycle on webrtc-rs 0.20.

use std::collections::HashMap;
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
use crate::media::dtmf::{DtmfSenderState, OutboundDtmfNegotiation, TelephoneEventCodec};
use crate::media::outbound::OutboundAudioRtpWriter;
use crate::peer::builder::{
    self, audio_track_rtcp_feedback, video_track_rtcp_feedback, MIME_TYPE_OPUS,
    MIME_TYPE_TELEPHONE_EVENT, MIME_TYPE_VP8, TELEPHONE_EVENT_MAPPINGS,
};
use crate::peer::handler::{
    ConnectionHandler, HandlerChannels, HandlerDropCounters, LocalIceEvent,
};
use crate::peer::ice::IceCandidateLog;
use crate::sdp::inspect::{
    negotiated_sdes_mid_for_outbound_audio, negotiated_telephone_event_codec,
    preferred_telephone_event_codec_in_sdp, sdp_has_media_line, NegotiatedAudioMid,
};
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
    data_channels_seen: Arc<SyncMutex<Vec<Arc<dyn DataChannel>>>>,
    local_ice_rx: Arc<AsyncMutex<mpsc::Receiver<LocalIceEvent>>>,
    local_ice_complete_pending: Arc<AtomicBool>,
    local_ice_overflowed: Arc<AtomicBool>,
    local_ice_overflow_reported: AtomicBool,
    local_audio: SyncMutex<Option<Arc<TrackLocalStaticRTP>>>,
    local_audio_ssrc: SyncMutex<Option<u32>>,
    outbound_audio_writer: SyncMutex<Option<Arc<OutboundAudioRtpWriter>>>,
    local_dtmf: SyncMutex<Option<Arc<TrackLocalStaticRTP>>>,
    local_dtmf_ssrcs_by_clock: SyncMutex<HashMap<u32, u32>>,
    local_dtmf_codec: SyncMutex<Option<TelephoneEventCodec>>,
    outbound_dtmf_negotiation: SyncMutex<OutboundDtmfNegotiation>,
    outbound_audio_mid: SyncMutex<Option<NegotiatedAudioMid>>,
    outbound_dtmf_send_capable: AtomicBool,
    dtmf_sender_states: AsyncMutex<HashMap<u32, DtmfSenderState>>,
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
            data_channels_seen: receivers.data_channels_seen,
            local_ice_rx: Arc::new(AsyncMutex::new(receivers.local_ice)),
            local_ice_complete_pending: receivers.local_ice_complete_pending,
            local_ice_overflowed: receivers.local_ice_overflowed,
            local_ice_overflow_reported: AtomicBool::new(false),
            local_audio: SyncMutex::new(None),
            local_audio_ssrc: SyncMutex::new(None),
            outbound_audio_writer: SyncMutex::new(None),
            local_dtmf: SyncMutex::new(None),
            local_dtmf_ssrcs_by_clock: SyncMutex::new(HashMap::new()),
            local_dtmf_codec: SyncMutex::new(None),
            outbound_dtmf_negotiation: SyncMutex::new(OutboundDtmfNegotiation::Pending),
            outbound_audio_mid: SyncMutex::new(None),
            outbound_dtmf_send_capable: AtomicBool::new(true),
            dtmf_sender_states: AsyncMutex::new(HashMap::new()),
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
    /// Also attaches a supplemental RFC 4733 encoding on the same negotiated
    /// audio sender using the remote offer's preferred telephone-event clock
    /// rate. The dynamic payload type is not
    /// committed until it appears unchanged in the completed answer. When the
    /// offer has no telephone-event mapping, no DTMF track is attached and
    /// outbound DTMF fails closed after negotiation.
    pub async fn prepare_answerer_media_for_offer(
        self: &Arc<Self>,
        remote_sdp: &str,
    ) -> Result<()> {
        let offered_dtmf = preferred_telephone_event_codec_in_sdp(remote_sdp);
        if sdp_has_media_line(remote_sdp, "audio") {
            self.add_local_audio_track_with_dtmf(offered_dtmf.into_iter().collect())
                .await?;
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

    /// Try to receive the next route-owned local ICE event without blocking.
    pub async fn try_recv_local_ice_event(&self) -> Option<LocalIceEvent> {
        if self.local_ice_overflowed.load(Ordering::Acquire)
            && !self
                .local_ice_overflow_reported
                .swap(true, Ordering::AcqRel)
        {
            return Some(LocalIceEvent::Overflow);
        }
        let event = self.local_ice_rx.lock().await.try_recv().ok();
        if event.is_some() {
            return event;
        }
        self.local_ice_complete_pending
            .swap(false, Ordering::AcqRel)
            .then_some(LocalIceEvent::Complete)
    }

    /// Receive the next route-owned local ICE event. Completion and overflow
    /// remain observable even when the bounded candidate channel was full.
    pub async fn recv_local_ice_event(&self) -> Option<LocalIceEvent> {
        loop {
            if self.local_ice_overflowed.load(Ordering::Acquire)
                && !self
                    .local_ice_overflow_reported
                    .swap(true, Ordering::AcqRel)
            {
                return Some(LocalIceEvent::Overflow);
            }
            let mut receiver = self.local_ice_rx.lock().await;
            if let Ok(event) = receiver.try_recv() {
                return Some(event);
            }
            if self
                .local_ice_complete_pending
                .swap(false, Ordering::AcqRel)
            {
                return Some(LocalIceEvent::Complete);
            }
            let event = receiver.recv().await;
            drop(receiver);
            if event.is_some() {
                return event;
            }
            if !(self.local_ice_overflowed.load(Ordering::Acquire)
                && !self.local_ice_overflow_reported.load(Ordering::Acquire))
                && !self.local_ice_complete_pending.load(Ordering::Acquire)
            {
                return None;
            }
        }
    }

    /// Drain all currently-buffered local ICE events without blocking.
    pub async fn drain_local_ice_events(&self) -> Vec<LocalIceEvent> {
        let mut out = Vec::new();
        if self.local_ice_overflowed.load(Ordering::Acquire)
            && !self
                .local_ice_overflow_reported
                .swap(true, Ordering::AcqRel)
        {
            out.push(LocalIceEvent::Overflow);
        }
        let mut rx = self.local_ice_rx.lock().await;
        while let Ok(event) = rx.try_recv() {
            out.push(event);
        }
        if self
            .local_ice_complete_pending
            .swap(false, Ordering::AcqRel)
        {
            out.push(LocalIceEvent::Complete);
        }
        out
    }

    /// Compatibility wrapper returning candidates only. New signalers should
    /// consume [`Self::recv_local_ice_event`] so completion/overflow cannot be
    /// lost.
    pub async fn recv_local_ice(&self) -> Option<RTCIceCandidate> {
        match self.recv_local_ice_event().await? {
            LocalIceEvent::Candidate(candidate) => Some(candidate),
            LocalIceEvent::Complete | LocalIceEvent::Overflow => None,
        }
    }

    /// Compatibility wrapper returning currently buffered candidates only.
    pub async fn drain_local_ice(&self) -> Vec<RTCIceCandidate> {
        self.drain_local_ice_events()
            .await
            .into_iter()
            .filter_map(|event| match event {
                LocalIceEvent::Candidate(candidate) => Some(candidate),
                LocalIceEvent::Complete | LocalIceEvent::Overflow => None,
            })
            .collect()
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
            // [`Self::recv_local_ice_event`] for the signaler to forward.
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

    /// Add one local audio sender with Opus plus a supplemental RFC 4733
    /// encoding when outbound DTMF is allowed.
    ///
    /// Opus and telephone-event use distinct SSRC encodings on one negotiated
    /// audio transceiver. RFC 4733 is a supplemental payload within the audio
    /// media section, not a second `MediaStreamTrack`; creating a second track
    /// for a one-m-line browser offer leaves that sender MID-less and its
    /// interceptor stream unbound.
    pub async fn add_local_audio_track(self: &Arc<Self>) -> Result<()> {
        let dtmf_codecs = if self.outbound_dtmf_send_capable.load(Ordering::Acquire) {
            unique_telephone_event_clock_mappings()
        } else {
            Vec::new()
        };
        self.add_local_audio_track_with_dtmf(dtmf_codecs).await
    }

    async fn add_local_audio_track_with_dtmf(
        self: &Arc<Self>,
        dtmf_codecs: Vec<TelephoneEventCodec>,
    ) -> Result<()> {
        if self.local_audio.lock().is_some() {
            return dtmf_codecs
                .into_iter()
                .try_for_each(|codec| self.validate_existing_dtmf_clock(codec));
        }

        let opus = RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: 48000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1".into(),
            rtcp_feedback: audio_track_rtcp_feedback(),
        };
        let ssrc = rand_ssrc();
        let mut encodings = vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(ssrc),
                ..Default::default()
            },
            codec: opus,
            ..Default::default()
        }];
        let mut dtmf_ssrcs_by_clock = HashMap::new();
        for codec in dtmf_codecs.iter().copied() {
            if codec.clock_rate_hz == 0 {
                return Err(WebRtcError::Sdp(
                    "telephone-event clock rate must be non-zero".into(),
                ));
            }
            if dtmf_ssrcs_by_clock.contains_key(&codec.clock_rate_hz) {
                continue;
            }
            let dtmf_ssrc = loop {
                let candidate = rand_ssrc();
                if candidate != ssrc && !dtmf_ssrcs_by_clock.values().any(|ssrc| *ssrc == candidate)
                {
                    break candidate;
                }
            };
            encodings.push(RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(dtmf_ssrc),
                    ..Default::default()
                },
                codec: RTCRtpCodec {
                    mime_type: MIME_TYPE_TELEPHONE_EVENT.to_owned(),
                    clock_rate: codec.clock_rate_hz,
                    channels: 1,
                    sdp_fmtp_line: "0-15".into(),
                    rtcp_feedback: vec![],
                },
                ..Default::default()
            });
            dtmf_ssrcs_by_clock.insert(codec.clock_rate_hz, dtmf_ssrc);
        }
        let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
            format!("rvoip-stream-{ssrc}"),
            format!("rvoip-track-{ssrc}"),
            "rvoip-audio".into(),
            RtpCodecKind::Audio,
            encodings,
        )));

        self.pc
            .add_track(track.clone() as Arc<dyn TrackLocal>)
            .await?;

        *self.local_audio.lock() = Some(Arc::clone(&track));
        *self.local_audio_ssrc.lock() = Some(ssrc);
        *self.outbound_audio_writer.lock() = Some(OutboundAudioRtpWriter::new(
            Arc::clone(&track),
            ssrc,
            48_000,
        ));
        if let Some(codec) = dtmf_codecs.first().copied() {
            // Both handles deliberately point at the same TrackLocal: distinct
            // SSRC encodings share one negotiated sender/transceiver.
            *self.local_dtmf.lock() = Some(track);
            *self.local_dtmf_ssrcs_by_clock.lock() = dtmf_ssrcs_by_clock;
            *self.local_dtmf_codec.lock() = Some(codec);
        }
        Ok(())
    }

    /// Attach one RFC 4733 encoding per unique clock that an offer may
    /// advertise (currently 8 kHz and 48 kHz).
    ///
    /// This compatibility entry point is used by offerers before remote SDP
    /// exists. Answerers use [`Self::add_local_dtmf_track_for_codec`] so the
    /// sender binding has the clock rate selected from the remote offer.
    ///
    /// Idempotent — safe to call multiple times.
    pub async fn add_local_dtmf_track(self: &Arc<Self>) -> Result<()> {
        if self.local_dtmf.lock().is_some() {
            return unique_telephone_event_clock_mappings()
                .into_iter()
                .try_for_each(|codec| self.validate_existing_dtmf_clock(codec));
        }
        if self.local_audio.lock().is_some() {
            return Err(WebRtcError::InvalidState(
                "telephone-event must be configured before adding the audio sender",
            ));
        }
        self.add_local_audio_track_with_dtmf(unique_telephone_event_clock_mappings())
            .await
    }

    /// Attach an RFC 4733 encoding whose codec clock matches the negotiated
    /// remote telephone-event mapping.
    ///
    /// Dynamic payload type is written per RTP packet; sender encoding carries
    /// MIME type and clock rate. Reusing an encoding across a payload-type remap
    /// at the same rate is safe, while silently retaining a differently
    /// clocked track would make SRTP/transceiver filtering nondeterministic.
    pub async fn add_local_dtmf_track_for_codec(
        self: &Arc<Self>,
        codec: TelephoneEventCodec,
    ) -> Result<()> {
        if codec.clock_rate_hz == 0 {
            return Err(WebRtcError::Sdp(
                "telephone-event clock rate must be non-zero".into(),
            ));
        }
        if self.local_dtmf.lock().is_some() {
            return self.validate_existing_dtmf_clock(codec);
        }
        if self.local_audio.lock().is_some() {
            return Err(WebRtcError::InvalidState(
                "telephone-event must be configured before adding the audio sender",
            ));
        }
        self.add_local_audio_track_with_dtmf(vec![codec]).await
    }

    fn validate_existing_dtmf_clock(&self, codec: TelephoneEventCodec) -> Result<()> {
        let clocks = self.local_dtmf_ssrcs_by_clock.lock();
        if clocks.contains_key(&codec.clock_rate_hz) {
            return Ok(());
        }
        let mut bound = clocks.keys().copied().collect::<Vec<_>>();
        bound.sort_unstable();
        Err(WebRtcError::Sdp(format!(
            "telephone-event encoding for {} Hz is unavailable; bound clocks: {bound:?}",
            codec.clock_rate_hz,
        )))
    }

    pub fn local_audio_track(&self) -> Option<Arc<TrackLocalStaticRTP>> {
        self.local_audio.lock().clone()
    }

    /// SSRC of the local audio encoding (for RFC 4733 DTMF).
    pub fn local_audio_ssrc(&self) -> Option<u32> {
        *self.local_audio_ssrc.lock()
    }

    pub(crate) fn outbound_audio_writer(&self) -> Option<Arc<OutboundAudioRtpWriter>> {
        self.outbound_audio_writer.lock().clone()
    }

    /// D1 — the shared audio track containing the RFC 4733 SSRC encoding.
    /// Returns `None` when that supplemental encoding was not attached.
    pub fn local_dtmf_track(&self) -> Option<Arc<TrackLocalStaticRTP>> {
        self.local_dtmf.lock().clone()
    }

    /// D1 — SSRC of the supplemental RFC 4733 encoding.
    pub fn local_dtmf_ssrc(&self) -> Option<u32> {
        self.local_dtmf_codec()
            .and_then(|codec| self.local_dtmf_ssrc_for_codec(codec))
    }

    pub(crate) fn local_dtmf_ssrc_for_codec(&self, codec: TelephoneEventCodec) -> Option<u32> {
        self.local_dtmf_ssrcs_by_clock
            .lock()
            .get(&codec.clock_rate_hz)
            .copied()
    }

    /// Codec descriptor selected for DTMF diagnostics and sending.
    ///
    /// This is a binding diagnostic, not proof that the current final SDP
    /// negotiated outbound DTMF. Call [`Self::negotiated_outbound_dtmf_codec`]
    /// for the active mapping; it returns `None` after a rejected remap even
    /// though the existing physical clock encodings remain attached.
    #[must_use]
    pub fn local_dtmf_codec(&self) -> Option<TelephoneEventCodec> {
        *self.local_dtmf_codec.lock()
    }

    /// Exact outbound telephone-event mapping selected from remote SDP.
    ///
    /// `None` means final SDP is either pending or explicitly unsupported.
    /// Use [`Self::outbound_dtmf_negotiation`] when that distinction matters.
    #[must_use]
    pub fn negotiated_outbound_dtmf_codec(&self) -> Option<TelephoneEventCodec> {
        match self.outbound_dtmf_negotiation() {
            OutboundDtmfNegotiation::Negotiated(codec) => Some(codec),
            OutboundDtmfNegotiation::Pending | OutboundDtmfNegotiation::Unsupported => None,
        }
    }

    /// Final-SDP state used by the outbound DTMF sender.
    #[must_use]
    pub fn outbound_dtmf_negotiation(&self) -> OutboundDtmfNegotiation {
        *self.outbound_dtmf_negotiation.lock()
    }

    /// Exact SDES MID value negotiated for locally-originated audio RTP.
    ///
    /// Supplemental SSRCs are intentionally absent from SDP, so RFC 4733
    /// packets require this header-extension value for browser demux. `None`
    /// means negotiation is pending, absent, or ambiguous and callers must not
    /// write supplemental RTP.
    #[must_use]
    pub fn negotiated_outbound_audio_mid(&self) -> Option<String> {
        self.outbound_audio_mid
            .lock()
            .as_ref()
            .map(|binding| binding.value.clone())
    }

    /// Negotiated RFC 8285 one-byte extension ID for the outbound audio MID.
    #[must_use]
    pub fn negotiated_outbound_audio_mid_extension_id(&self) -> Option<u8> {
        self.outbound_audio_mid
            .lock()
            .as_ref()
            .map(|binding| binding.extension_id)
    }

    pub(crate) fn dtmf_sender_states(&self) -> &AsyncMutex<HashMap<u32, DtmfSenderState>> {
        &self.dtmf_sender_states
    }

    fn disable_outbound_dtmf(&self) {
        self.outbound_dtmf_send_capable
            .store(false, Ordering::Release);
        *self.outbound_dtmf_negotiation.lock() = OutboundDtmfNegotiation::Unsupported;
        *self.outbound_audio_mid.lock() = None;
    }

    fn begin_outbound_dtmf_renegotiation(&self) {
        if self.outbound_dtmf_send_capable.load(Ordering::Acquire) {
            *self.outbound_dtmf_negotiation.lock() = OutboundDtmfNegotiation::Pending;
            *self.outbound_audio_mid.lock() = None;
        }
    }

    fn final_outbound_dtmf_state(
        &self,
        offer_sdp: &str,
        answer_sdp: &str,
    ) -> Result<OutboundDtmfNegotiation> {
        if !self.outbound_dtmf_send_capable.load(Ordering::Acquire) {
            return Ok(OutboundDtmfNegotiation::Unsupported);
        }
        let Some(codec) = negotiated_telephone_event_codec(offer_sdp, answer_sdp) else {
            return Ok(OutboundDtmfNegotiation::Unsupported);
        };
        if self.local_dtmf.lock().is_none() || self.local_dtmf_ssrc_for_codec(codec).is_none() {
            return Err(WebRtcError::IncompatibleCapabilities);
        }
        Ok(OutboundDtmfNegotiation::Negotiated(codec))
    }

    fn commit_outbound_dtmf_state(
        &self,
        state: OutboundDtmfNegotiation,
        audio_mid: Option<NegotiatedAudioMid>,
    ) {
        if let OutboundDtmfNegotiation::Negotiated(codec) = state {
            // Payload type is a final SDP result, not part of the alpha
            // track's codec binding. Update diagnostics only at this commit
            // boundary; a failed same-clock PT remap retains the old value.
            *self.local_dtmf_codec.lock() = Some(codec);
        }
        *self.outbound_dtmf_negotiation.lock() = state;
        *self.outbound_audio_mid.lock() = audio_mid;
    }

    async fn prepare_send_capable_dtmf_for_offer(self: &Arc<Self>) -> Result<()> {
        // A supplemental SSRC encoding must exist for every clock advertised
        // by a send-capable audio offer. This covers both initial and
        // subsequent offers, including callers that attached primary audio
        // directly.
        if self.outbound_dtmf_send_capable.load(Ordering::Acquire)
            && self.local_audio.lock().is_some()
            && self.local_dtmf.lock().is_none()
        {
            self.add_local_dtmf_track().await?;
        }
        Ok(())
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

    /// Snapshot every remotely-created DataChannel observed by the peer
    /// handler. Adapter-level pumps use this non-consuming view so no channel
    /// is lost when the compatibility receiver is full or another caller has
    /// already drained it.
    pub(crate) fn seen_data_channels(&self) -> Vec<Arc<dyn DataChannel>> {
        self.data_channels_seen.lock().clone()
    }

    /// Remove a closed/rejected channel from the bounded non-consuming
    /// registry so adapter scans cannot resurrect it and a replacement can
    /// use the released slot.
    pub(crate) fn forget_seen_data_channel(&self, target: &Arc<dyn DataChannel>) {
        self.data_channels_seen
            .lock()
            .retain(|channel| !Arc::ptr_eq(channel, target));
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

    /// Find a remote data channel by label without consuming the legacy
    /// `wait_data_channel` receiver. The handler records every inbound
    /// DataChannel before forwarding it through the older mpsc surface so
    /// adapter observers can coexist with tests and applications that still
    /// receive channels directly.
    pub async fn find_seen_data_channel_by_label(
        &self,
        label: &str,
        timeout: Duration,
    ) -> Option<Arc<dyn DataChannel>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let snapshot = self.data_channels_seen.lock().clone();
            for dc in snapshot {
                match dc.label().await {
                    Ok(dc_label) if dc_label == label => return Some(dc),
                    Ok(_) => {}
                    Err(err) => {
                        tracing::debug!(error = %err, "failed to read WebRTC data-channel label");
                    }
                }
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
        }
        // Any send-capable offerer with audio must own a dedicated RFC 4733
        // SSRC before SDP generation. This also covers clients/video routes
        // that pre-attached audio; there is no unsafe audio-track fallback.
        self.prepare_send_capable_dtmf_for_offer().await?;

        let offer = self.pc.create_offer(None).await?;
        self.pc.set_local_description(offer).await?;
        // Setting a new local offer invalidates the final mapping used by the
        // supplemental SSRC until an answer accepts both telephone-event and
        // SDES MID. Do not write using the previous exchange's binding while
        // this offer is pending.
        self.begin_outbound_dtmf_renegotiation();
        self.wait_gather().await?;

        let desc = self
            .pc
            .local_description()
            .await
            .ok_or_else(|| WebRtcError::Sdp("no local description after gather".into()))?;

        sdp_to_string(&desc)
    }

    /// Prepare an offerer for a playback-only exchange such as WHEP.
    ///
    /// A local audio track is retained so the alpha WebRTC engine creates the
    /// corresponding transceiver, but every audio/video transceiver is marked
    /// `recvonly` before SDP generation. The track therefore never authorizes
    /// outbound media while the peer can still receive RTP from the origin.
    pub async fn prepare_receive_only_offer(self: &Arc<Self>) -> Result<()> {
        self.require_role(PeerRole::Offerer)?;
        self.disable_outbound_dtmf();
        if self.local_audio.lock().is_none() {
            self.add_local_audio_track().await?;
        }
        self.set_media_direction(RTCRtpTransceiverDirection::Recvonly, None)
            .await
    }

    /// Prepare an offerer for the WHEP draft-04 server counter-offer path.
    ///
    /// The retained local track is the media source and every media
    /// transceiver is constrained to `sendonly` before the offer is created.
    /// This is deliberately separate from the player-side `recvonly` helper
    /// so signaling role never gets inferred from an application's media
    /// direction.
    pub async fn prepare_send_only_offer(self: &Arc<Self>) -> Result<()> {
        self.require_role(PeerRole::Offerer)?;
        // This path is reserved for WHEP counter-offer playback. Although its
        // media direction is sendonly, application DTMF is not part of WHEP's
        // playback contract and must fail closed.
        self.disable_outbound_dtmf();
        if self.local_audio.lock().is_none() {
            self.add_local_audio_track().await?;
        }
        self.set_media_direction(RTCRtpTransceiverDirection::Sendonly, None)
            .await
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

        let remote = self
            .pc
            .remote_description()
            .await
            .ok_or_else(|| WebRtcError::Sdp("no remote offer before answer".into()))?;
        let offer_sdp = sdp_to_string(&remote)?;
        let answer = self.pc.create_answer(None).await?;
        let provisional_answer_sdp = sdp_to_string(&answer)?;
        // Validate track binding against the actual offer/answer intersection
        // before mutating local signaling state.
        self.final_outbound_dtmf_state(&offer_sdp, &provisional_answer_sdp)?;
        self.pc.set_local_description(answer).await?;
        self.wait_gather().await?;

        let desc = self
            .pc
            .local_description()
            .await
            .ok_or_else(|| WebRtcError::Sdp("no local description after gather".into()))?;
        let answer_sdp = sdp_to_string(&desc)?;
        let final_dtmf = self.final_outbound_dtmf_state(&offer_sdp, &answer_sdp)?;
        let audio_mid = negotiated_sdes_mid_for_outbound_audio(&offer_sdp, &answer_sdp, false);
        self.commit_outbound_dtmf_state(final_dtmf, audio_mid);
        Ok(answer_sdp)
    }

    /// Answerer: apply remote offer, create answer, gather, return local SDP.
    pub async fn accept_offer_and_gather(self: &Arc<Self>, remote_sdp: &str) -> Result<String> {
        self.apply_remote_offer(remote_sdp).await?;
        self.create_answer_and_gather().await
    }

    /// Answerer: ICE restart / renegotiation — new remote offer → fresh answer SDP.
    pub async fn renegotiate_as_answerer(self: &Arc<Self>, remote_sdp: &str) -> Result<String> {
        self.require_role(PeerRole::Answerer)?;

        self.prepare_answerer_media_for_offer(remote_sdp).await?;
        let remote = parse_sdp(remote_sdp, RTCSdpType::Offer)?;
        self.pc.set_remote_description(remote).await?;
        self.create_answer_and_gather().await
    }

    /// Offerer: ICE restart — create a fresh offer SDP after the session is up.
    pub async fn renegotiate_as_offerer(self: &Arc<Self>) -> Result<String> {
        self.require_role(PeerRole::Offerer)?;
        self.prepare_send_capable_dtmf_for_offer().await?;

        let offer = self.pc.create_offer(None).await?;
        self.pc.set_local_description(offer).await?;
        self.begin_outbound_dtmf_renegotiation();
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
        let local = self
            .pc
            .local_description()
            .await
            .ok_or_else(|| WebRtcError::Sdp("no local offer before remote answer".into()))?;
        let offer_sdp = sdp_to_string(&local)?;
        let final_dtmf = self.final_outbound_dtmf_state(&offer_sdp, remote_sdp)?;
        let audio_mid = negotiated_sdes_mid_for_outbound_audio(&offer_sdp, remote_sdp, true);
        let remote = parse_sdp(remote_sdp, RTCSdpType::Answer)?;
        self.pc.set_remote_description(remote).await?;
        self.commit_outbound_dtmf_state(final_dtmf, audio_mid);
        Ok(())
    }

    /// Complete the WHEP draft-04 server counter-offer flow on the same peer.
    ///
    /// The player starts as JSEP offerer, rolls that pending offer back, then
    /// answers the server's offer as `recvonly`. `PeerRole` remains the stable
    /// application ownership role; this method is the one deliberate JSEP role
    /// transition supported on an outbound player connection.
    pub async fn answer_counter_offer_after_rollback(
        self: &Arc<Self>,
        remote_sdp: &str,
    ) -> Result<String> {
        self.require_role(PeerRole::Offerer)?;
        let remote = parse_sdp(remote_sdp, RTCSdpType::Offer)?;
        self.rollback_local().await?;
        self.pc.set_remote_description(remote).await?;
        self.set_media_direction(RTCRtpTransceiverDirection::Recvonly, None)
            .await?;
        let answer = self.pc.create_answer(None).await?;
        let provisional_answer_sdp = sdp_to_string(&answer)?;
        self.final_outbound_dtmf_state(remote_sdp, &provisional_answer_sdp)?;
        self.pc.set_local_description(answer).await?;
        self.wait_gather().await?;
        let description =
            self.pc.local_description().await.ok_or_else(|| {
                WebRtcError::Sdp("no local counter-offer answer after gather".into())
            })?;
        let answer_sdp = sdp_to_string(&description)?;
        let final_dtmf = self.final_outbound_dtmf_state(remote_sdp, &answer_sdp)?;
        let audio_mid = negotiated_sdes_mid_for_outbound_audio(remote_sdp, &answer_sdp, false);
        self.commit_outbound_dtmf_state(final_dtmf, audio_mid);
        Ok(answer_sdp)
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
        self.set_media_direction(direction, Some(RtpCodecKind::Audio))
            .await
    }

    async fn set_media_direction(
        &self,
        direction: RTCRtpTransceiverDirection,
        kind: Option<RtpCodecKind>,
    ) -> Result<()> {
        for tx in self.pc.get_transceivers().await {
            let transceiver_kind = if let Some(sender) = tx.sender().await? {
                Some(sender.track().kind().await)
            } else if let Some(receiver) = tx.receiver().await? {
                Some(receiver.track().kind().await)
            } else {
                None
            };
            if kind.is_none() || transceiver_kind == kind {
                tx.set_direction(direction).await?;
            }
        }
        Ok(())
    }
}

fn unique_telephone_event_clock_mappings() -> Vec<TelephoneEventCodec> {
    let mut mappings = Vec::new();
    for (payload_type, clock_rate_hz) in TELEPHONE_EVENT_MAPPINGS {
        if mappings
            .iter()
            .any(|codec: &TelephoneEventCodec| codec.clock_rate_hz == clock_rate_hz)
        {
            continue;
        }
        mappings.push(TelephoneEventCodec::new(payload_type, clock_rate_hz));
    }
    mappings
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

    let timeout = Duration::from_secs(config.connection_timeout_secs);
    tokio::try_join!(
        offerer.wait_connected(timeout),
        answerer.wait_connected(timeout)
    )?;

    Ok((offerer, answerer))
}
