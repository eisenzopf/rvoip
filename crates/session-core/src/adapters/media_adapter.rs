//! Simplified Media Adapter for session-core
//!
//! Thin translation layer between media-core and state machine.
//! Focuses only on essential media operations and events.

use std::sync::Arc;
use std::net::{IpAddr, SocketAddr};
use tokio::sync::mpsc;
use dashmap::DashMap;
use rvoip_media_core::{
    relay::controller::{
        MediaSessionController, MediaConfig, MediaSessionInfo,
        AudioSource, BridgeError, BridgeHandle,
    },
    DialogId,
};
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::types::sdp::{CryptoAttribute, CryptoSuite, ParsedAttribute, SdpSession};
use std::str::FromStr;
use crate::adapters::srtp_negotiator::{SrtpNegotiator, SrtpPair};
use crate::state_table::types::SessionId;
use crate::errors::{Result, SessionError};
use crate::session_store::SessionStore;
use rvoip_media_core::types::AudioFrame;

/// Audio format for recording
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum AudioFormat {
    Wav,
    Raw,
    Mp3,
}

/// Recording configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingConfig {
    /// Path where the recording should be saved
    pub file_path: String,

    /// Audio format for the recording
    pub format: AudioFormat,

    /// Sample rate in Hz (e.g., 8000, 16000, 48000)
    pub sample_rate: u32,

    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,

    /// Include mixed audio from both legs (for conference recording)
    pub include_mixed: bool,

    /// Save separate tracks for each leg
    pub separate_tracks: bool,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            file_path: "/tmp/recording.wav".to_string(),
            format: AudioFormat::Wav,
            sample_rate: 8000,
            channels: 1,
            include_mixed: false,
            separate_tracks: false,
        }
    }
}

/// Recording status information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordingStatus {
    pub is_recording: bool,
    pub is_paused: bool,
    pub duration_seconds: f64,
    pub file_size_bytes: u64,
}

/// Negotiated media configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NegotiatedConfig {
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub codec: String,
    pub payload_type: u8,
}

/// Minimal media adapter - just translates between media-core and state machine
pub struct MediaAdapter {
    /// Media-core controller
    pub(crate) controller: Arc<MediaSessionController>,
    
    /// Session store for updating IDs
    pub(crate) store: Arc<SessionStore>,
    
    /// Simple mapping of session IDs to dialog IDs (media-core uses DialogId)
    pub(crate) session_to_dialog: Arc<DashMap<SessionId, DialogId>>,
    pub(crate) dialog_to_session: Arc<DashMap<DialogId, SessionId>>,
    
    /// Store media session info for SDP generation
    media_sessions: Arc<DashMap<SessionId, MediaSessionInfo>>,
    
    /// Audio frame channels for receiving decoded audio from media-core
    audio_receivers: Arc<DashMap<SessionId, mpsc::Sender<AudioFrame>>>,
    
    /// Local IP for SDP generation
    local_ip: IpAddr,
    
    /// Port range for media
    media_port_start: u16,
    media_port_end: u16,
    
    /// Audio mixers for conferences
    audio_mixers: Arc<DashMap<crate::types::MediaSessionId, Vec<crate::types::MediaSessionId>>>,

    // ==== RFC 4568 SDES-SRTP state (Step 2B) ====

    /// Whether to attach `a=crypto:` lines to outgoing offers and to
    /// answer with `RTP/SAVP` when peer offers SRTP. When `false`,
    /// the adapter behaves like the pre-2B baseline (plain RTP/AVP).
    offer_srtp: bool,

    /// When `true`, refuse to fall back to plaintext RTP. UAC: a
    /// remote SDP without acceptable `a=crypto:` causes the
    /// negotiation function to return `Err`. UAS: an offer without
    /// `a=crypto:` is rejected with the same `Err`, which the state
    /// machine surfaces as `488 Not Acceptable Here`.
    srtp_required: bool,

    /// Crypto suites to offer in preference order when `offer_srtp`
    /// is set. Default: AES-CM-128 + HMAC-SHA1-80 then -32 per
    /// RFC 4568 §6.2.1 MTI plus low-bandwidth fallback.
    srtp_offered_suites: Vec<CryptoSuite>,

    /// UAC-side state held between `generate_sdp_offer` and
    /// `negotiate_sdp_as_uac`. The offerer-role `SrtpNegotiator`
    /// holds our locally-generated keys keyed by tag.
    pending_srtp_offerers: Arc<DashMap<SessionId, SrtpNegotiator>>,

    /// Negotiated SRTP context pairs keyed by session. Phase 2B.2
    /// will read these out and hand them to media-core's
    /// `start_secure_media`.
    pub(crate) negotiated_srtp: Arc<DashMap<SessionId, SrtpPair>>,

    /// Global event coordinator for publishing RFC 4733 DTMF events
    /// onto the session-core API event bus. Populated at boot via
    /// [`Self::set_global_coordinator`]; `None` in tests that bypass
    /// the full wiring.
    pub(crate) global_coordinator:
        Arc<tokio::sync::RwLock<Option<Arc<rvoip_infra_common::events::coordinator::GlobalEventCoordinator>>>>,

    /// Sprint 3 A6 — public RTP-side address advertised in SDP `c=` /
    /// `o=` / `m=audio` lines. Set at coordinator boot from either
    /// `Config::media_public_addr` (static override) or a successful
    /// `Config::stun_server` probe. `None` falls back to `local_ip` +
    /// the per-session local RTP port (today's behaviour).
    public_rtp_addr: std::sync::RwLock<Option<SocketAddr>>,

    /// Sprint 3 C1 — when `true`, generated offers and answers
    /// advertise PT 13 (RFC 3389 Comfort Noise) alongside the
    /// PCMU + PCMA + telephone-event format set. Set at coordinator
    /// boot from `Config::comfort_noise_enabled`.
    comfort_noise_enabled: bool,

    /// Sprint 3.5 C2 swap — when `true` (default), the answer's
    /// format list is the strict RFC 3264 §6 intersection of the
    /// offered formats and our supported set, in offerer-preference
    /// order. When `false`, the answer always advertises our full
    /// supported set (legacy pre-Sprint-3.5 behaviour). Set at
    /// coordinator boot from `Config::strict_codec_matching`.
    strict_codec_matching: bool,
}

impl MediaAdapter {
    /// Create a new media adapter (no SRTP — equivalent to the
    /// pre-Step-2B behaviour).
    pub fn new(
        controller: Arc<MediaSessionController>,
        store: Arc<SessionStore>,
        local_ip: IpAddr,
        port_start: u16,
        port_end: u16,
    ) -> Self {
        Self {
            controller,
            store,
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            media_sessions: Arc::new(DashMap::new()),
            audio_receivers: Arc::new(DashMap::new()),
            local_ip,
            media_port_start: port_start,
            media_port_end: port_end,
            audio_mixers: Arc::new(DashMap::new()),
            offer_srtp: false,
            srtp_required: false,
            srtp_offered_suites: vec![
                CryptoSuite::AesCm128HmacSha1_80,
                CryptoSuite::AesCm128HmacSha1_32,
            ],
            pending_srtp_offerers: Arc::new(DashMap::new()),
            negotiated_srtp: Arc::new(DashMap::new()),
            global_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
            public_rtp_addr: std::sync::RwLock::new(None),
            comfort_noise_enabled: false,
            strict_codec_matching: true,
        }
    }

    /// Sprint 3 C1 — enable RFC 3389 Comfort Noise advertisement on
    /// outgoing offers and answers. Wired from
    /// `Config::comfort_noise_enabled` at coordinator boot. Mutates
    /// in place, mirroring the `set_srtp_policy` shape.
    pub fn set_comfort_noise(&mut self, enabled: bool) {
        self.comfort_noise_enabled = enabled;
    }

    /// Sprint 3.5 C2 swap — enable strict RFC 3264 §6 SDP-answer
    /// matching. Wired from `Config::strict_codec_matching` at
    /// coordinator boot.
    pub fn set_strict_codec_matching(&mut self, strict: bool) {
        self.strict_codec_matching = strict;
    }

    /// Set the public RTP address advertised in SDP. Called at
    /// coordinator boot from `Config::media_public_addr` (static
    /// override) or a successful STUN probe. Idempotent — subsequent
    /// calls overwrite. The IP address goes into `c=`/`o=` lines and
    /// the port (when set) replaces `info.rtp_port` in `m=audio`.
    pub fn set_public_rtp_addr(&self, addr: Option<SocketAddr>) {
        if let Ok(mut guard) = self.public_rtp_addr.write() {
            *guard = addr;
        }
    }

    /// Read the current public RTP address override (used by tests
    /// and by SDP generation).
    pub(crate) fn public_rtp_addr(&self) -> Option<SocketAddr> {
        self.public_rtp_addr.read().ok().and_then(|g| *g)
    }

    /// Local IP address bound by the adapter. Used by the Sprint 3
    /// A6 STUN probe to bind its temp socket on the same interface.
    pub fn local_ip(&self) -> IpAddr {
        self.local_ip
    }

    /// Install the global event coordinator so the adapter can publish
    /// RFC 4733 DTMF events onto the session-core API event stream.
    /// Idempotent — a later call replaces any prior coordinator.
    pub async fn set_global_coordinator(
        &self,
        coordinator: Arc<rvoip_infra_common::events::coordinator::GlobalEventCoordinator>,
    ) {
        *self.global_coordinator.write().await = Some(coordinator);
    }

    /// Configure the SRTP offer policy. Called by `UnifiedCoordinator`
    /// when constructing the adapter from a [`Config`] that has
    /// `offer_srtp` / `srtp_required` / `srtp_offered_suites` set.
    /// Mutates in place rather than returning a new adapter so the
    /// existing constructor signature stays unchanged.
    pub fn set_srtp_policy(
        &mut self,
        offer_srtp: bool,
        srtp_required: bool,
        suites: Vec<CryptoSuite>,
    ) {
        self.offer_srtp = offer_srtp;
        self.srtp_required = srtp_required;
        if !suites.is_empty() {
            self.srtp_offered_suites = suites;
        }
    }
    
    // ===== Outbound Actions (from state machine) =====
    
    /// Start a media session
    pub async fn start_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session already exists
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            // Session already exists, check if it's started in media-core
            if self.controller.get_session_info(&dialog_id).await.is_some() {
                tracing::debug!("Media session already started for session {}", session_id.0);
                return Ok(());
            }
        }
        
        // If not, create it - delegate to create_session
        let _media_id = self.create_session(session_id).await?;
        Ok(())
    }
    
    /// Generate SDP offer (for UAC).
    ///
    /// Built via `sip-core`'s typed `SdpBuilder` (RFC 8866). The
    /// previous format-string implementation produced byte-identical
    /// output to this version when the `offer_srtp` knob is not set —
    /// Extract the `a=crypto:` attributes from the audio m-section of
    /// a parsed SDP. Empty result means the peer offered no SRTP.
    fn extract_audio_crypto(session: &SdpSession) -> Vec<CryptoAttribute> {
        session
            .media_descriptions
            .iter()
            .find(|m| m.media == "audio")
            .map(|m| {
                m.generic_attributes
                    .iter()
                    .filter_map(|a| match a {
                        ParsedAttribute::Crypto(c) => Some(c.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    
    /// Process SDP answer and negotiate (for UAC)
    pub async fn negotiate_sdp_as_uac(&self, session_id: &SessionId, remote_sdp: &str) -> Result<NegotiatedConfig> {
        // Parse remote SDP to extract IP and port
        let (remote_ip, remote_port) = self.parse_sdp_connection(remote_sdp)?;

        // SDES answer-side handling (RFC 4568 §7.5).
        // The state machine path that calls us doesn't expose a
        // failure-with-487 hook today; if `srtp_required` and the
        // answer can't satisfy SRTP, we surface `SDPNegotiationFailed`
        // which the executor turns into terminal `CallFailed`.
        if let Some((_, offerer_state)) =
            self.pending_srtp_offerers.remove(session_id)
        {
            // We did offer SRTP. Look for a matching `a=crypto:` in the answer.
            let parsed = SdpSession::from_str(remote_sdp).map_err(|e| {
                SessionError::SDPNegotiationFailed(format!(
                    "Failed to parse remote SDP for SDES answer extraction: {}",
                    e
                ))
            })?;
            let attrs = Self::extract_audio_crypto(&parsed);
            if let Some(chosen) = attrs.first() {
                let pair = offerer_state.accept_answer(chosen)?;
                self.negotiated_srtp.insert(session_id.clone(), pair);
                tracing::info!(
                    "SDES answer accepted for session {}: tag {} suite {:?}",
                    session_id.0,
                    chosen.tag,
                    chosen.suite
                );
            } else if self.srtp_required {
                return Err(SessionError::SDPNegotiationFailed(
                    "srtp_required is set but the SDP answer carries no a=crypto: line"
                        .into(),
                ));
            } else {
                tracing::warn!(
                    "Session {} offered SRTP but the answer didn't accept it; \
                     proceeding plaintext (Config::srtp_required = false)",
                    session_id.0
                );
                let _ = offerer_state; // dropped — keys discarded
            }
        }
        
        // Update media session with remote address. SRTP contexts (if
        // negotiated in 2B.1) must be installed *between* updating the
        // remote address and starting the audio transmitter — the
        // transmitter spawns a send loop and we don't want any
        // plaintext packets going out before the encrypt-side
        // SrtpContext is in place.
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            let remote_addr = SocketAddr::new(remote_ip, remote_port);

            self.controller.update_rtp_remote_addr(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to update RTP remote address: {}", e)))?;

            // RFC 4568 SDES: install per-direction contexts before the
            // first wire packet flows.
            if let Some((_, pair)) = self.negotiated_srtp.remove(session_id) {
                self.controller
                    .install_srtp_contexts(&dialog_id, pair.send_ctx, pair.recv_ctx)
                    .await
                    .map_err(|e| SessionError::MediaError(
                        format!("Failed to install SRTP contexts: {}", e)
                    ))?;
                tracing::info!(
                    "🔒 SRTP contexts installed for session {} (suite {:?})",
                    session_id.0,
                    pair.suite
                );
            }

            // Establish media flow (this starts audio transmission)
            self.controller.establish_media_flow(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to establish media flow: {}", e)))?;

            tracing::info!("✅ Updated RTP remote address to {} for session {}", remote_addr, session_id.0);
        }

        let config = NegotiatedConfig {
            local_addr: SocketAddr::new(self.local_ip, self.get_local_port(session_id)?),
            remote_addr: SocketAddr::new(remote_ip, remote_port),
            codec: "PCMU".to_string(),
            payload_type: 0,
        };

        // Event publishing will be handled by SessionCrossCrateEventHandler

        Ok(config)
    }

    /// Generate SDP answer and negotiate (for UAS)
    pub async fn negotiate_sdp_as_uas(&self, session_id: &SessionId, remote_sdp: &str) -> Result<(String, NegotiatedConfig)> {
        // Parse remote SDP — typed parse for both connection extraction
        // and SDES handling.
        let parsed_offer = SdpSession::from_str(remote_sdp).map_err(|e| {
            SessionError::SDPNegotiationFailed(format!("Failed to parse remote SDP: {}", e))
        })?;
        let (remote_ip, remote_port) = self.parse_sdp_connection(remote_sdp)?;

        // SDES UAS-side handling. Per RFC 4568 §7.3, if we require
        // SRTP and the offer doesn't include any `a=crypto:` lines we
        // must reject — the state-machine path turns the
        // `SDPNegotiationFailed` into a `488 Not Acceptable Here`
        // (decision D10).
        let offered_crypto = Self::extract_audio_crypto(&parsed_offer);
        let (answer_attr, srtp_pair) = if !offered_crypto.is_empty() && self.offer_srtp {
            // Both sides want SRTP — negotiate.
            let answerer = SrtpNegotiator::new_answerer();
            let (chosen, pair) = answerer.process_offer(&offered_crypto)?;
            self.negotiated_srtp.insert(session_id.clone(), pair);
            tracing::info!(
                "SDES offer accepted for session {}: tag {} suite {:?}",
                session_id.0,
                chosen.tag,
                chosen.suite
            );
            (Some(chosen), true)
        } else if offered_crypto.is_empty() && self.srtp_required {
            return Err(SessionError::SDPNegotiationFailed(
                "srtp_required is set but the SDP offer carries no a=crypto: line".into(),
            ));
        } else if !offered_crypto.is_empty() && !self.offer_srtp {
            // Peer offered SRTP but our policy is plain. Per RFC 4568
            // §7.3 the right answer here is `m=audio 0 RTP/SAVP …`
            // (port=0) signalling refusal. For now we keep the
            // simpler "answer plaintext on the same port" behavior;
            // expose the proper port=0 path when a real carrier
            // demands it.
            tracing::warn!(
                "Session {} received SRTP offer but local policy is offer_srtp=false; \
                 answering plaintext",
                session_id.0
            );
            (None, false)
        } else {
            (None, false)
        };
        let _ = srtp_pair; // suppress unused warning — value retained via DashMap insert
        
        // Get our local port
        let local_port = self.get_local_port(session_id)?;
        
        // Update media session with remote address. SRTP contexts must
        // be installed BEFORE establish_media_flow starts the audio
        // transmitter — see `negotiate_sdp_as_uac` for the same
        // ordering rationale.
        if let Some(dialog_id) = self.session_to_dialog.get(session_id) {
            let remote_addr = SocketAddr::new(remote_ip, remote_port);

            self.controller.update_rtp_remote_addr(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to update RTP remote address: {}", e)))?;

            if let Some((_, pair)) = self.negotiated_srtp.remove(session_id) {
                self.controller
                    .install_srtp_contexts(&dialog_id, pair.send_ctx, pair.recv_ctx)
                    .await
                    .map_err(|e| SessionError::MediaError(
                        format!("Failed to install SRTP contexts (UAS): {}", e)
                    ))?;
                tracing::info!(
                    "🔒 SRTP contexts installed for session {} (UAS, suite {:?})",
                    session_id.0,
                    pair.suite
                );
            }

            self.controller.establish_media_flow(&dialog_id, remote_addr)
                .await
                .map_err(|e| SessionError::MediaError(format!("Failed to establish media flow: {}", e)))?;

            tracing::info!("✅ Updated RTP remote address to {} for session {} (UAS)", remote_addr, session_id.0);
        }
        
        // Generate the SDP answer.
        //
        // Sprint 3.5 — `negotiate_sdp_as_uas` now consumes the
        // generic RFC 3264 §6 matcher in
        // `rvoip_dialog_core::sdp::match_offer`. The strict path
        // (default) intersects offered formats with our supported
        // set in offerer-preference order; the permissive path
        // (`Config::strict_codec_matching = false`) preserves the
        // pre-Sprint-3.5 "always answer with full set" behaviour
        // for deployments that depend on it.
        let sess_id = generate_session_id().to_string();
        // Sprint 3 A6 — same public-address override as the offer
        // path, so answers carry the discovered/configured public
        // mapping when one is set.
        let public = self.public_rtp_addr();
        let advertised_ip = public.map(|sa| sa.ip()).unwrap_or(self.local_ip);
        let local_ip_str = advertised_ip.to_string();
        let advertised_port = public
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or(local_port);
        let answer_transport = if answer_attr.is_some() { "RTP/SAVP" } else { "RTP/AVP" };

        let formats = compute_answer_formats(
            &parsed_offer,
            self.comfort_noise_enabled,
            self.strict_codec_matching,
            self.offer_srtp,
            self.srtp_required,
        )?;

        let formats_str: Vec<&str> = formats.iter().map(|s| s.as_str()).collect();
        let mut media_builder = SdpBuilder::new("Session")
            .origin("-", &sess_id, "0", "IN", "IP4", &local_ip_str)
            .connection("IN", "IP4", &local_ip_str)
            .time("0", "0")
            .media_audio(advertised_port, answer_transport)
                .formats(&formats_str);
        // Emit rtpmap/fmtp ONLY for the formats we kept. In the
        // permissive branch this is the full set; in the strict
        // branch the matcher's intersection has already filtered.
        for fmt in &formats {
            match fmt.as_str() {
                "0" => {
                    media_builder = media_builder.rtpmap("0", "PCMU/8000");
                }
                "8" => {
                    media_builder = media_builder.rtpmap("8", "PCMA/8000");
                }
                "13" => {
                    media_builder = media_builder.rtpmap("13", "CN/8000");
                }
                "101" => {
                    media_builder = media_builder
                        .rtpmap("101", "telephone-event/8000")
                        .fmtp("101", "0-15");
                }
                _ => {}
            }
        }
        if let Some(attr) = answer_attr {
            media_builder = media_builder.crypto_attribute(attr);
        }
        let session = media_builder
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .map_err(|e| SessionError::SDPNegotiationFailed(
                format!("SdpBuilder failed to build answer: {}", e)
            ))?;
        let sdp_answer = session.to_string();
        
        let config = NegotiatedConfig {
            local_addr: SocketAddr::new(self.local_ip, local_port),
            remote_addr: SocketAddr::new(remote_ip, remote_port),
            codec: "PCMU".to_string(),
            payload_type: 0,
        };
        
        // Event publishing will be handled by SessionCrossCrateEventHandler
        
        // Media flow is already represented by MediaStreamStarted above
        
        Ok((sdp_answer, config))
    }
    
    /// Play an audio file to the remote party
    pub async fn play_audio_file(&self, session_id: &SessionId, file_path: &str) -> Result<()> {
        // Get the dialog ID for this session
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        // Send play file command to media controller
        // Note: The actual media-core API might differ
        tracing::info!("Playing audio file {} for session {}", file_path, session_id.0);
        
        // In a real implementation, this would send the file path to the media relay
        // For now, we'll just log it
        // Send a media event (using MediaError as a workaround for now)
        // In production, we'd have proper event types for these
        tracing::debug!("Audio playback started: {}", file_path);
        
        Ok(())
    }
    
    /// Start recording the media session
    pub async fn start_recording_old(&self, session_id: &SessionId) -> Result<String> {
        // Get the dialog ID for this session
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        // Generate a unique recording filename
        let recording_path = format!("/tmp/recording_{}.wav", session_id.0);
        
        tracing::info!("Starting recording for session {} at {}", session_id.0, recording_path);
        
        // In a real implementation, this would start recording through the media relay
        // For now, just log the recording start
        tracing::debug!("Recording started at: {}", recording_path);
        
        // Store recording path in session if needed
        if let Ok(session) = self.store.get_session(session_id).await {
            // Could add a recording_path field to SessionState if needed
            let _ = self.store.update_session(session).await;
        }
        
        Ok(recording_path)
    }
    
    /// Create a media bridge between two sessions (for peer-to-peer conferencing)
    pub async fn create_bridge(&self, session1: &SessionId, session2: &SessionId) -> Result<()> {
        // Get dialog IDs for both sessions
        let _dialog1 = self.session_to_dialog.get(session1)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session1.0)))?
            .clone();
        let _dialog2 = self.session_to_dialog.get(session2)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session2.0)))?
            .clone();
        
        tracing::info!("Creating media bridge between {} and {}", session1.0, session2.0);
        
        // In a real implementation, this would configure the media relay to bridge RTP streams
        // For now, we'll just update the session states
        if let Ok(mut session1_state) = self.store.get_session(session1).await {
            session1_state.bridged_to = Some(session2.clone());
            let _ = self.store.update_session(session1_state).await;
        }
        
        if let Ok(mut session2_state) = self.store.get_session(session2).await {
            session2_state.bridged_to = Some(session1.clone());
            let _ = self.store.update_session(session2_state).await;
        }
        
        // Log bridge creation
        tracing::debug!("Bridge created between {} and {}", session1.0, session2.0);
        
        Ok(())
    }
    
    /// Swap the audio source on the running transmitter for a session.
    /// Used by early-media flows to replace silence with a ringback tone,
    /// hold announcement, or custom samples during `EarlyMedia`.
    ///
    /// The media session must already have an active transmitter — callers
    /// typically invoke this right after `send_early_media` (which has
    /// `PrepareEarlyMediaSDP` + `establish_media_flow` set one up).
    pub async fn set_audio_source(
        &self,
        session_id: &SessionId,
        source: AudioSource,
    ) -> Result<()> {
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!(
                "No media session for {}",
                session_id.0
            )))?
            .clone();

        self.controller
            .set_audio_source(&dialog_id, source)
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to set audio source: {}", e)))
    }

    /// Bridge the RTP streams of two sessions at the media-core layer.
    ///
    /// Resolves each `SessionId` to its underlying `DialogId` and delegates
    /// to `MediaSessionController::bridge_sessions`. Transparent packet-level
    /// relay — both legs must have negotiated the same codec and reached the
    /// `Active` state (remote RTP address known).
    ///
    /// Dropping the returned [`BridgeHandle`] tears the bridge down.
    pub async fn bridge_rtp_sessions(
        &self,
        session_a: &SessionId,
        session_b: &SessionId,
    ) -> std::result::Result<BridgeHandle, BridgeError> {
        let dialog_a = self
            .session_to_dialog
            .get(session_a)
            .ok_or_else(|| BridgeError::SessionNotFound(session_a.0.clone()))?
            .clone();
        let dialog_b = self
            .session_to_dialog
            .get(session_b)
            .ok_or_else(|| BridgeError::SessionNotFound(session_b.0.clone()))?
            .clone();

        let handle = self.controller.bridge_sessions(dialog_a, dialog_b).await?;

        // Keep the legacy session-store `bridged_to` pointers in sync so
        // anything that queries session state sees the pairing.
        if let Ok(mut a_state) = self.store.get_session(session_a).await {
            a_state.bridged_to = Some(session_b.clone());
            let _ = self.store.update_session(a_state).await;
        }
        if let Ok(mut b_state) = self.store.get_session(session_b).await {
            b_state.bridged_to = Some(session_a.clone());
            let _ = self.store.update_session(b_state).await;
        }

        Ok(handle)
    }

    /// Destroy a media bridge
    pub async fn destroy_bridge(&self, session_id: &SessionId) -> Result<()> {
        // Get the bridged session
        let bridged_session = if let Ok(session) = self.store.get_session(session_id).await {
            session.bridged_to.clone()
        } else {
            None
        };
        
        if let Some(other_session) = bridged_session {
            tracing::info!("Destroying bridge between {} and {}", session_id.0, other_session.0);
            
            // Clear bridge information from both sessions
            if let Ok(mut session1_state) = self.store.get_session(session_id).await {
                session1_state.bridged_to = None;
                let _ = self.store.update_session(session1_state).await;
            }
            
            if let Ok(mut session2_state) = self.store.get_session(&other_session).await {
                session2_state.bridged_to = None;
                let _ = self.store.update_session(session2_state).await;
            }
            
            // Log bridge destruction
            tracing::debug!("Bridge destroyed between {} and {}", session_id.0, other_session.0);
        }
        
        Ok(())
    }
    
    /// Stop recording the media session
    pub async fn stop_recording_old(&self, session_id: &SessionId) -> Result<()> {
        // Get the dialog ID for this session
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        tracing::info!("Stopping recording for session {}", session_id.0);
        
        // In a real implementation, this would stop recording through the media relay
        tracing::debug!("Recording stopped");
        
        Ok(())
    }
    
    // ===== AUDIO FRAME API - The Missing Core Functionality =====
    
    /// Send an audio frame for encoding and transmission
    /// This is the equivalent of the old session-core's MediaControl::send_audio_frame()
    pub async fn send_audio_frame(&self, session_id: &SessionId, audio_frame: AudioFrame) -> Result<()> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No media session for {}", session_id.0)))?
            .clone();
        
        tracing::info!("📤 Sending audio frame for session {} ({} samples) via RTP", session_id.0, audio_frame.samples.len());
        
        // Convert AudioFrame to PCM samples and call encode_and_send_audio_frame
        // This will encode the audio and send it via RTP to the remote peer
        let pcm_samples = audio_frame.samples.clone();
        let timestamp = audio_frame.timestamp;
        
        self.controller.encode_and_send_audio_frame(&dialog_id, pcm_samples, timestamp)
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to send audio frame via RTP: {}", e)))?;
        
        tracing::debug!("✅ Audio frame sent successfully via RTP for session {}", session_id.0);
        Ok(())
    }
    
    /// Create a new media session
    pub async fn create_session(&self, session_id: &SessionId) -> Result<crate::types::MediaSessionId> {
        // Create dialog ID for media-core
        let dialog_id = DialogId::new(format!("media-{}", session_id.0));
        
        tracing::info!("🚀 Creating media session for session {} with dialog ID {}", session_id.0, dialog_id);
        
        // Store mappings
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        
        // Create media config with our settings
        let media_config = MediaConfig {
            local_addr: SocketAddr::new(self.local_ip, 0), // Let media-core allocate port
            remote_addr: None, // Will be set when we get remote SDP
            preferred_codec: Some("PCMU".to_string()), // G.711 µ-law as default
            parameters: std::collections::HashMap::new(),
        };
        
        // Start the media session in media-core
        self.controller.start_media(dialog_id.clone(), media_config)
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to start media session: {}", e)))?;

        // Install RFC 4733 DTMF callback so incoming PT 101 frames
        // surface as `Event::DtmfReceived` on the public API bus.
        // Fires once per digit (media-core dedupes the three §2.5.1.3
        // retransmits) so app consumers don't see duplicate digits.
        self.install_dtmf_callback(session_id.clone(), dialog_id.clone()).await;

        // Get and store session info
        if let Some(info) = self.controller.get_session_info(&dialog_id).await {
            self.media_sessions.insert(session_id.clone(), info.clone());

            // `MediaSessionId` is a type alias for
            // `rvoip_media_core::DialogId` (P5), so the value we hand
            // back to session-core is identical to the controller's
            // dialog id — no `from_dialog` reconstruction needed.
            // `store_session_mapping` separately wants media-core's
            // **internal** `MediaSessionId` type (still distinct), so
            // that conversion stays.
            let media_id = dialog_id.clone();
            self.controller.store_session_mapping(
                session_id.0.clone(),
                rvoip_media_core::MediaSessionId::from_dialog(&dialog_id),
            );

            tracing::info!("✅ Media session created successfully for dialog {}", dialog_id);
            return Ok(media_id);
        }
        
        Err(SessionError::MediaError("Failed to get session info after creation".to_string()))
    }
    
    /// Generate the local SDP offer. **Sole** SDP-offer generator — this
    /// is the only entry point used by the state-machine's
    /// `Action::GenerateLocalSDP`. Always uses `SdpBuilder`, always
    /// advertises PCMU + PCMA + RFC 4733 telephone-event, and conditionally
    /// attaches `a=crypto:` lines when [`Config::offer_srtp`](crate::api::unified::Config) is set.
    ///
    /// Profile selection per RFC 4568 §3.1.4: `RTP/SAVP` when offering
    /// SDES, `RTP/AVP` otherwise. SRTP master keys are generated via
    /// [`SrtpNegotiator::new_offerer`] and stashed in
    /// `pending_srtp_offerers` keyed by `session_id` so
    /// [`Self::negotiate_sdp_as_uac`] can drive `accept_answer` against
    /// the matching answer.
    ///
    /// **DTMF advertisement (P2 fix).** Pre-Sprint 2.5 the non-SRTP path
    /// silently omitted PT 101 / `a=fmtp:101 0-15`, leaving plaintext
    /// callers unable to negotiate DTMF. The unified path always emits
    /// the telephone-event rtpmap + fmtp regardless of profile —
    /// `offer_advertises_telephone_event_on_plaintext` in the test
    /// module locks this in.
    pub async fn generate_local_sdp(&self, session_id: &SessionId) -> Result<String> {
        // Resolve dialog id and prime the cached session info. Both
        // the SRTP and plaintext paths used to do this; doing it once
        // up front lets the rest of the body share one shape.
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No dialog mapping for session {}", session_id.0)))?
            .clone();
        let info = self.controller.get_session_info(&dialog_id).await
            .ok_or_else(|| SessionError::MediaError(format!("Failed to get session info for dialog {}", dialog_id)))?;
        self.media_sessions.insert(session_id.clone(), info.clone());

        // Sprint 3 A6 — when a public RTP address has been configured
        // (static override or STUN-discovered), advertise that in the
        // SDP `c=` / `o=` / `m=audio` lines instead of the bind-address.
        // The static override's port wins when set; otherwise we keep
        // the per-session local RTP port (most NATs don't preserve
        // ports across the binding, but absent better info the local
        // port is our best guess and symmetric-RTP latching covers
        // the rest).
        let public = self.public_rtp_addr();
        let port = public
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or_else(|| info.rtp_port.unwrap_or(info.config.local_addr.port()));
        let elapsed_secs = info.created_at.elapsed().as_secs().to_string();
        let dialog_id_str = info.dialog_id.as_str().to_string();
        let advertised_ip = public.map(|sa| sa.ip()).unwrap_or(self.local_ip);
        let local_ip_str = advertised_ip.to_string();

        // Profile + crypto. RFC 4568 §3.1.4 — `RTP/SAVP` is mandatory
        // when offering SDES.
        let (transport, crypto_attrs) = if self.offer_srtp {
            let (negotiator, attrs) =
                SrtpNegotiator::new_offerer(&self.srtp_offered_suites)?;
            self.pending_srtp_offerers.insert(session_id.clone(), negotiator);
            ("RTP/SAVP", attrs)
        } else {
            ("RTP/AVP", Vec::new())
        };

        // Build the m-section. Always offer PCMU (0) + PCMA (8) +
        // telephone-event (101). Sprint 3 C1: append `13` + an
        // `a=rtpmap:13 CN/8000` line when comfort noise is enabled.
        // Crypto attrs follow rtpmap/fmtp so ordering matches what
        // carriers expect; sendrecv goes last so the byte-fixture
        // tests stay stable.
        let formats: &[&str] = if self.comfort_noise_enabled {
            &["0", "8", "13", "101"]
        } else {
            &["0", "8", "101"]
        };
        let mut media_builder = SdpBuilder::new("Session")
            .origin(
                "-",
                &dialog_id_str,
                &elapsed_secs,
                "IN",
                "IP4",
                &local_ip_str,
            )
            .connection("IN", "IP4", &local_ip_str)
            .time("0", "0")
            .media_audio(port, transport)
                .formats(formats)
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000");
        if self.comfort_noise_enabled {
            media_builder = media_builder.rtpmap("13", "CN/8000");
        }
        media_builder = media_builder
            .rtpmap("101", "telephone-event/8000")
            .fmtp("101", "0-15");
        for attr in crypto_attrs {
            media_builder = media_builder.crypto_attribute(attr);
        }
        let session = media_builder
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .map_err(|e| SessionError::SDPNegotiationFailed(
                format!("SdpBuilder failed to build offer: {}", e)
            ))?;

        let sdp = session.to_string();
        tracing::info!("✅ Generated SDP for session {} with local port {}", session_id.0, port);
        Ok(sdp)
    }
    
    /// Install the RFC 4733 DTMF bridge: registers a callback with
    /// media-core so PT 101 packets (already deduped to one-per-digit
    /// on the first end-of-event frame) are published as
    /// `Event::DtmfReceived { call_id, digit }` on the session-core
    /// public API event stream. No-op if the global coordinator has
    /// not been installed yet (e.g. isolated unit tests).
    async fn install_dtmf_callback(&self, session_id: SessionId, dialog_id: DialogId) {
        let Some(coordinator) = self.global_coordinator.read().await.clone() else {
            tracing::debug!(
                "DTMF callback install skipped for session {}: no global coordinator yet",
                session_id.0
            );
            return;
        };

        let (tx, mut rx) = mpsc::channel::<rvoip_media_core::DtmfNotification>(32);
        if let Err(e) = self.controller.set_dtmf_callback(dialog_id.clone(), tx).await {
            tracing::warn!(
                "Failed to register DTMF callback for session {} (dialog {}): {}",
                session_id.0, dialog_id, e
            );
            return;
        }

        // Consumer task: forwards each DTMF notification from media-core
        // onto the session-core API event bus. Exits cleanly when the
        // sender end of the channel is dropped (media session stopped).
        let sid = session_id.clone();
        let did = dialog_id.clone();
        tokio::spawn(async move {
            while let Some(notification) = rx.recv().await {
                let api_event = crate::api::events::Event::DtmfReceived {
                    call_id: sid.clone(),
                    digit: notification.digit,
                };
                let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(api_event);
                if let Err(e) = coordinator.publish(wrapped).await {
                    tracing::warn!(
                        "Failed to publish DtmfReceived for session {}: {}",
                        sid.0, e
                    );
                } else {
                    tracing::info!(
                        "📢 Published DtmfReceived digit='{}' for session {}",
                        notification.digit, sid.0
                    );
                }
            }
            tracing::debug!("DTMF bridge task exited for session {} (dialog {})", sid.0, did);
        });
    }

    /// Subscribe to receive decoded audio frames from RTP
    /// This is the equivalent of the old session-core's MediaControl::subscribe_to_audio_frames()
    pub async fn subscribe_to_audio_frames(&self, session_id: &SessionId) -> Result<crate::types::AudioFrameSubscriber> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No media session for {}", session_id.0)))?
            .clone();
        
        tracing::info!("🎧 Setting up audio subscription for session {} (dialog: {})", session_id.0, dialog_id);
        
        // Create channel for audio frames
        let (tx, rx) = mpsc::channel(1000); // Buffer up to 1000 frames (20 seconds at 50fps)
        
        // Register the callback with MediaSessionController to receive audio frames
        self.controller.set_audio_frame_callback(dialog_id.clone(), tx.clone())
            .await
            .map_err(|e| SessionError::MediaError(format!("Failed to set audio callback: {}", e)))?;
        
        // Store the sender for this session for cleanup
        self.audio_receivers.insert(session_id.clone(), tx);
        
        tracing::info!("🎧 Created audio frame subscriber for session {} with dialog {}", session_id.0, dialog_id);
        
        // Return our types::AudioFrameSubscriber
        Ok(crate::types::AudioFrameSubscriber::new(session_id.clone(), rx))
    }
    
    /// Internal method to forward received audio frames to subscribers
    /// This should be called by the media event handler when audio frames are received
    #[allow(dead_code)]
    pub(crate) async fn forward_audio_frame_to_subscriber(&self, session_id: &SessionId, audio_frame: rvoip_media_core::types::AudioFrame) -> Result<()> {
        if let Some(tx) = self.audio_receivers.get(session_id) {
            if let Err(_) = tx.send(audio_frame).await {
                // Receiver has been dropped, clean up
                self.audio_receivers.remove(session_id);
                tracing::debug!("Audio frame subscriber disconnected for session {}", session_id.0);
            }
        }
        Ok(())
    }
    
    // ===== New Methods for CallController and ConferenceManager =====
    
    /// Create a media session
    pub async fn create_media_session(&self) -> Result<crate::types::MediaSessionId> {
        let media_id = crate::types::MediaSessionId::new_v4();
        Ok(media_id)
    }
    
    /// Stop a media session
    pub async fn stop_media_session(&self, _media_id: crate::types::MediaSessionId) -> Result<()> {
        // For now, just return Ok
        Ok(())
    }
    
    /// Set media direction (for hold/resume)
    pub async fn set_media_direction(&self, _media_id: crate::types::MediaSessionId, _direction: crate::types::MediaDirection) -> Result<()> {
        // TODO: Implement actual media direction control
        Ok(())
    }
    
    /// Create hold SDP
    pub async fn create_hold_sdp(&self) -> Result<String> {
        // Create SDP with sendonly attribute
        let sdp = format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 {}\r\n\
             s=-\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio 0 RTP/AVP 0\r\n\
             a=sendonly\r\n",
            self.local_ip, self.local_ip
        );
        Ok(sdp)
    }
    
    /// Create active SDP
    pub async fn create_active_sdp(&self) -> Result<String> {
        // Create SDP with sendrecv attribute
        let port = self.media_port_start; // TODO: Allocate actual port
        let sdp = format!(
            "v=0\r\n\
             o=- 0 0 IN IP4 {}\r\n\
             s=-\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=sendrecv\r\n",
            self.local_ip, self.local_ip, port
        );
        Ok(sdp)
    }
    
    /// Send DTMF digit (legacy `media_id` signature used by the state
    /// machine's CallController path). Delegates to
    /// [`Self::send_dtmf_rfc4733`] with a 100 ms duration.
    pub async fn send_dtmf(
        &self,
        media_id: crate::types::MediaSessionId,
        digit: char,
    ) -> Result<()> {
        // `MediaSessionId` is now a type alias for media-core's
        // `DialogId` (P5), so the value passed in IS the dialog id we
        // need — no reconstruction.
        let dialog_id = media_id;
        self.controller
            .send_dtmf_packet(&dialog_id, digit, 100)
            .await
            .map_err(|e| SessionError::MediaError(format!("DTMF send failed: {}", e)))?;
        tracing::debug!("☎️  Queued DTMF '{}' for media_id {:?}", digit, dialog_id);
        Ok(())
    }

    /// Send RFC 4733 DTMF by session id — preferred public API, used
    /// by [`UnifiedCoordinator::send_dtmf`].
    pub async fn send_dtmf_rfc4733(
        &self,
        session_id: &SessionId,
        digit: char,
        duration_ms: u32,
    ) -> Result<()> {
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!(
                "No media session for {}",
                session_id.0
            )))?
            .clone();

        self.controller
            .send_dtmf_packet(&dialog_id, digit, duration_ms)
            .await
            .map_err(|e| SessionError::MediaError(format!("DTMF send failed: {}", e)))?;

        tracing::info!(
            "☎️  Queued DTMF '{}' for session {} (dialog {}, duration={}ms)",
            digit, session_id.0, dialog_id, duration_ms
        );
        Ok(())
    }
    
    /// Set mute state
    pub async fn set_mute(&self, media_id: crate::types::MediaSessionId, muted: bool) -> Result<()> {
        // TODO: Implement mute control
        tracing::debug!("Setting mute state to {} for media session {:?}", muted, media_id);
        Ok(())
    }
    
    /// Start recording for media session
    pub async fn start_recording_media(&self, media_id: crate::types::MediaSessionId) -> Result<()> {
        // TODO: Implement recording
        tracing::info!("Starting recording for media session {:?}", media_id);
        Ok(())
    }
    
    /// Stop recording for media session
    pub async fn stop_recording_media(&self, media_id: crate::types::MediaSessionId) -> Result<()> {
        // TODO: Implement recording stop
        tracing::info!("Stopping recording for media session {:?}", media_id);
        Ok(())
    }
    
    // ===== Conference Methods =====
    
    /// Create an audio mixer for a conference
    pub async fn create_audio_mixer(&self) -> Result<crate::types::MediaSessionId> {
        let mixer_id = crate::types::MediaSessionId::new_v4();
        self.audio_mixers.insert(mixer_id.clone(), Vec::new());
        tracing::info!("Created audio mixer {:?}", mixer_id);
        Ok(mixer_id)
    }
    
    /// Redirect audio to a mixer
    pub async fn redirect_to_mixer(&self, media_id: crate::types::MediaSessionId, mixer_id: crate::types::MediaSessionId) -> Result<()> {
        if let Some(mut mixer) = self.audio_mixers.get_mut(&mixer_id) {
            mixer.push(media_id.clone());
        }
        tracing::debug!("Redirected media {:?} to mixer {:?}", media_id, mixer_id);
        Ok(())
    }
    
    /// Remove audio from a mixer
    pub async fn remove_from_mixer(&self, media_id: crate::types::MediaSessionId, mixer_id: crate::types::MediaSessionId) -> Result<()> {
        if let Some(mut mixer) = self.audio_mixers.get_mut(&mixer_id) {
            mixer.retain(|id| id != &media_id);
        }
        tracing::debug!("Removed media {:?} from mixer {:?}", media_id, mixer_id);
        Ok(())
    }
    
    /// Destroy an audio mixer
    pub async fn destroy_mixer(&self, mixer_id: crate::types::MediaSessionId) -> Result<()> {
        self.audio_mixers.remove(&mixer_id);
        tracing::info!("Destroyed audio mixer {:?}", mixer_id);
        Ok(())
    }
    
    /// Clean up all mappings and resources for a session.
    ///
    /// Idempotent — safe to call multiple times. Always removes the audio
    /// frame callback from media-core (so subscriber `rx.recv()` calls can
    /// return `None` and exit their loops) as long as the dialog mapping is
    /// still present.
    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        // Resolve dialog_id — prefer the mapping populated by create_session
        // (which is authoritative once set) but fall back to the deterministic
        // form `media-<session_id>` when the mapping has been lost (e.g. on a
        // second CleanupMedia after a Dialog3xxRedirect transition's actions
        // already cleared it). The fallback ensures media-core's sessions map
        // is always freed so a subsequent CreateMediaSession can reuse the
        // same dialog_id.
        let removed = self.session_to_dialog.remove(session_id);
        let dialog_id = match removed {
            Some((_, d)) => Some(d),
            None => {
                // Fallback to deterministic form used by create_session.
                Some(DialogId::new(format!("media-{}", session_id.0)))
            }
        };

        if let Some(dialog_id) = dialog_id {
            let _ = self.controller.remove_audio_frame_callback(&dialog_id).await;
            // stop_media may return "session not found" on the fallback path
            // (media-core already cleaned up) — expected; ignore.
            let _ = self.controller.stop_media(&dialog_id).await;
            self.dialog_to_session.remove(&dialog_id);
        }

        self.media_sessions.remove(session_id);
        // Drop our own clone of the tx as well (the other lived in media-core).
        self.audio_receivers.remove(session_id);

        tracing::debug!("Cleaned up media adapter resources for session {}", session_id.0);
        Ok(())
    }
    
    // ===== Helper Methods =====
    
    /// Get local RTP port for a session
    fn get_local_port(&self, session_id: &SessionId) -> Result<u16> {
        self.media_sessions
            .get(session_id)
            .and_then(|info| info.rtp_port)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No local port for session {}", session_id.0)))
    }
    
    /// Parse SDP to extract connection info from the audio m= section.
    ///
    /// Uses sip-core's typed `SdpSession::from_str` parser instead of
    /// the previous bespoke line-scanner so that future SDP work
    /// (`a=crypto:` for SDES, `a=fingerprint:`/`a=setup:` for
    /// DTLS-SRTP, video m= sections, RFC 8866 conformance) gets
    /// validation for free.
    ///
    /// Per RFC 8866 §5.7 the m-section's own `c=` line (if present)
    /// overrides the session-level `c=`. We honour that.
    fn parse_sdp_connection(&self, sdp: &str) -> Result<(IpAddr, u16)> {
        let session = SdpSession::from_str(sdp).map_err(|e| {
            SessionError::SDPNegotiationFailed(format!("Failed to parse SDP: {}", e))
        })?;

        let media = session
            .media_descriptions
            .iter()
            .find(|m| m.media == "audio")
            .ok_or_else(|| SessionError::SDPNegotiationFailed(
                "SDP has no audio m= section".into()
            ))?;

        let port = media.port as u16;

        // Prefer the per-media c= line; fall back to session-level.
        let conn = media
            .connection_info
            .as_ref()
            .or(session.connection_info.as_ref())
            .ok_or_else(|| SessionError::SDPNegotiationFailed(
                "SDP has no c= line at session or audio level".into()
            ))?;

        let ip = match &conn.connection_address {
            host_str => host_str.parse::<IpAddr>().map_err(|e| {
                SessionError::SDPNegotiationFailed(format!(
                    "SDP c= address {:?} is not a valid IP: {}",
                    host_str, e
                ))
            })?,
        };

        Ok((ip, port))
    }
    
    // ===== Event handling removed - now centralized in SessionCrossCrateEventHandler ====="

    // ===== Recording Management =====

    /// Start recording for a session (simple version for backward compatibility)
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<String> {
        // Use default config for backward compatibility
        let config = RecordingConfig::default();
        self.start_recording_with_config(session_id, config).await
    }

    /// Start recording for a session with specific config
    pub async fn start_recording_with_config(
        &self,
        session_id: &SessionId,
        config: RecordingConfig,
    ) -> Result<String> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        tracing::info!("Starting recording for session {} with path: {}", session_id.0, config.file_path);

        // For now, we'll generate a simple recording ID
        // In a real implementation, this would interact with media-core's recording API
        let recording_id = format!("rec_{}_{}", session_id.0, chrono::Utc::now().timestamp());

        // TODO: When media-core adds recording support, implement this properly
        // self.controller.start_recording(&dialog_id, config).await

        tracing::info!("✅ Recording started for session {} with ID: {}", session_id.0, recording_id);
        Ok(recording_id)
    }

    /// Stop recording for a session (simple version for backward compatibility)
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        // For backward compatibility, we don't have a recording_id
        // Just log the action
        tracing::info!("Stopping recording for session {}", session_id.0);
        Ok(())
    }

    /// Stop recording for a session with specific recording ID
    pub async fn stop_recording_with_id(&self, session_id: &SessionId, recording_id: &str) -> Result<()> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        tracing::info!("Stopping recording {} for session {}", recording_id, session_id.0);

        // TODO: When media-core adds recording support, implement this properly
        // self.controller.stop_recording(&dialog_id, recording_id).await

        tracing::info!("✅ Recording stopped for session {}", session_id.0);
        Ok(())
    }

    /// Pause recording for a session
    pub async fn pause_recording(&self, session_id: &SessionId, recording_id: &str) -> Result<()> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        tracing::debug!("Pausing recording {} for session {}", recording_id, session_id.0);

        // TODO: When media-core adds recording support, implement this properly
        // self.controller.pause_recording(&dialog_id, recording_id).await

        Ok(())
    }

    /// Resume a paused recording
    pub async fn resume_recording(&self, session_id: &SessionId, recording_id: &str) -> Result<()> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        tracing::debug!("Resuming recording {} for session {}", recording_id, session_id.0);

        // TODO: When media-core adds recording support, implement this properly
        // self.controller.resume_recording(&dialog_id, recording_id).await

        Ok(())
    }

    /// Get recording status
    pub async fn get_recording_status(
        &self,
        session_id: &SessionId,
        _recording_id: &str,
    ) -> Result<RecordingStatus> {
        // Get dialog ID
        let _dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::MediaError(format!("No dialog for session {}", session_id.0)))?
            .clone();

        // TODO: When media-core adds recording support, implement this properly
        // For now, return a mock status
        Ok(RecordingStatus {
            is_recording: false,
            is_paused: false,
            duration_seconds: 0.0,
            file_size_bytes: 0,
        })
    }

    /// Start recording for a bridged session pair
    pub async fn start_bridge_recording(
        &self,
        session1: &SessionId,
        session2: &SessionId,
        config: RecordingConfig,
    ) -> Result<String> {
        tracing::info!("Starting bridge recording for sessions {} <-> {}", session1.0, session2.0);

        // Start recording on both sessions with mixed audio
        let mut recording_config = config;
        recording_config.include_mixed = true;
        recording_config.separate_tracks = true;

        // Start recording on the first session (will capture both legs if bridged)
        self.start_recording_with_config(session1, recording_config).await
    }

    /// Enable/disable recording for all conference sessions
    pub async fn set_conference_recording_enabled(&self, enabled: bool) -> Result<()> {
        // This would be stored in a shared configuration
        // For now, we'll just log the intent
        tracing::info!("Conference recording enabled: {}", enabled);

        // TODO: Store this in a shared configuration that conference sessions check
        // when they are created

        Ok(())
    }
}

impl Clone for MediaAdapter {
    fn clone(&self) -> Self {
        Self {
            controller: self.controller.clone(),
            store: self.store.clone(),
            session_to_dialog: self.session_to_dialog.clone(),
            dialog_to_session: self.dialog_to_session.clone(),
            media_sessions: self.media_sessions.clone(),
            audio_receivers: self.audio_receivers.clone(),
            audio_mixers: self.audio_mixers.clone(),
            local_ip: self.local_ip,
            media_port_start: self.media_port_start,
            media_port_end: self.media_port_end,
            offer_srtp: self.offer_srtp,
            srtp_required: self.srtp_required,
            srtp_offered_suites: self.srtp_offered_suites.clone(),
            pending_srtp_offerers: self.pending_srtp_offerers.clone(),
            negotiated_srtp: self.negotiated_srtp.clone(),
            global_coordinator: self.global_coordinator.clone(),
            public_rtp_addr: std::sync::RwLock::new(self.public_rtp_addr()),
            comfort_noise_enabled: self.comfort_noise_enabled,
            strict_codec_matching: self.strict_codec_matching,
        }
    }
}

/// Generate a random session ID for SDP
fn generate_session_id() -> u64 {
    use rand::Rng;
    rand::thread_rng().gen()
}

/// Sprint 3.5 — compute the answer's `m=audio` format list from the
/// offer + our policy flags. Pure (no `MediaAdapter` state) so unit
/// tests can exercise the strict-vs-permissive logic without standing
/// up a coordinator.
///
/// Returns the formats in the order they should appear on the wire.
/// Caller is responsible for emitting the matching `a=rtpmap:` /
/// `a=fmtp:` lines.
///
/// `Err(SDPNegotiationFailed)` when:
/// - Strict mode + offer carries no overlap with our supported set
///   → state machine surfaces this as `488 Not Acceptable Here`.
/// - Strict mode + matcher rejects on SRTP policy (e.g. `require_srtp`
///   set + offer is plain RTP/AVP).
pub(crate) fn compute_answer_formats(
    offer: &SdpSession,
    comfort_noise_enabled: bool,
    strict: bool,
    offer_srtp: bool,
    srtp_required: bool,
) -> Result<Vec<String>> {
    let mut supported = vec!["0".to_string(), "8".to_string()];
    if comfort_noise_enabled {
        supported.push("13".to_string());
    }
    supported.push("101".to_string());

    if !strict {
        // Permissive — answer with our full set regardless. Matches
        // the pre-Sprint-3.5 shape.
        return Ok(supported);
    }

    let caps = rvoip_dialog_core::sdp::AnswerCapabilities {
        supported_formats: supported,
        accept_srtp: offer_srtp,
        require_srtp: srtp_required,
    };
    let m = rvoip_dialog_core::sdp::match_offer(offer, &caps)
        .map_err(|e| SessionError::SDPNegotiationFailed(format!("{}", e)))?;
    let line = m
        .media_lines
        .iter()
        .find(|l| l.media == "audio")
        .ok_or_else(|| {
            SessionError::SDPNegotiationFailed(
                "matcher returned no audio media line".into(),
            )
        })?;
    if !line.accepted {
        return Err(SessionError::SDPNegotiationFailed(
            "no codec overlap with offer".into(),
        ));
    }
    Ok(line.negotiated_formats.clone())
}

#[cfg(test)]
mod sdp_format_tests {
    //! Byte-fixture regression tests for the format-strings →
    //! `SdpBuilder` refactor (Step 2B.1, decision D11). Builds the same
    //! SDP via the typed builder and asserts byte-identical output to
    //! what the previous `format!` block would have produced.
    //!
    //! When the SRTP offer landing in 2B.2 changes the m= transport to
    //! `RTP/SAVP` and adds `a=crypto:` lines, these tests will need a
    //! second fixture for that case.

    use super::*;

    /// Build the offer the same way `generate_local_sdp` does, but with
    /// fixed inputs so the output is deterministic. Mirrors the
    /// production shape: PCMU + PCMA + telephone-event, RTP/AVP profile.
    fn build_offer(dialog_id: &str, elapsed_secs: u64, ip: &str, port: u16) -> String {
        let elapsed = elapsed_secs.to_string();
        SdpBuilder::new("Session")
            .origin("-", dialog_id, &elapsed, "IN", "IP4", ip)
            .connection("IN", "IP4", ip)
            .time("0", "0")
            .media_audio(port, "RTP/AVP")
                .formats(&["0", "8", "101"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .rtpmap("101", "telephone-event/8000")
                .fmtp("101", "0-15")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("offer builds")
            .to_string()
    }

    /// Same as `build_offer` but for the answer (different sess_version).
    fn build_answer(sess_id: &str, ip: &str, port: u16) -> String {
        SdpBuilder::new("Session")
            .origin("-", sess_id, "0", "IN", "IP4", ip)
            .connection("IN", "IP4", ip)
            .time("0", "0")
            .media_audio(port, "RTP/AVP")
                .formats(&["0", "8", "101"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .rtpmap("101", "telephone-event/8000")
                .fmtp("101", "0-15")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("answer builds")
            .to_string()
    }

    /// Reference output for the unified offer (Sprint 2.5 P2): PCMU +
    /// PCMA + telephone-event on every offer regardless of SRTP. Pre-P2
    /// the non-SRTP path emitted only `0 8` (no DTMF) and the SRTP path
    /// only `0 101` (no PCMA); both have been merged into the unified
    /// shape below.
    fn legacy_offer(dialog_id: &str, elapsed_secs: u64, ip: &str, port: u16) -> String {
        format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 8 101\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=rtpmap:101 telephone-event/8000\r\n\
             a=fmtp:101 0-15\r\n\
             a=sendrecv\r\n",
            dialog_id, elapsed_secs, ip, ip, port,
        )
    }

    fn legacy_answer(sess_id: &str, ip: &str, port: u16) -> String {
        format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0 8 101\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=rtpmap:101 telephone-event/8000\r\n\
             a=fmtp:101 0-15\r\n\
             a=sendrecv\r\n",
            sess_id, 0u64, ip, ip, port,
        )
    }

    #[test]
    fn offer_matches_legacy_format_byte_for_byte() {
        let dialog_id = "test-dialog-uuid";
        let elapsed = 42u64;
        let ip = "127.0.0.1";
        let port = 16000;
        let new = build_offer(dialog_id, elapsed, ip, port);
        let old = legacy_offer(dialog_id, elapsed, ip, port);
        assert_eq!(new, old, "SdpBuilder offer drifted from legacy format-string output");
    }

    #[test]
    fn answer_matches_legacy_format_byte_for_byte() {
        let sess_id = "1234567890";
        let ip = "192.168.1.42";
        let port = 16002;
        let new = build_answer(sess_id, ip, port);
        let old = legacy_answer(sess_id, ip, port);
        assert_eq!(new, old, "SdpBuilder answer drifted from legacy format-string output");
    }

    #[test]
    fn offer_round_trips_through_typed_parser() {
        // Build → parse → assert key fields. Catches CRLF / spacing
        // issues that would also break peer interop.
        let sdp_str = build_offer("d", 0, "10.0.0.1", 5004);
        let parsed = SdpSession::from_str(&sdp_str).expect("parses back");
        assert_eq!(parsed.session_name, "Session");
        assert_eq!(parsed.media_descriptions.len(), 1);
        let m = &parsed.media_descriptions[0];
        assert_eq!(m.media, "audio");
        assert_eq!(m.port, 5004);
        assert_eq!(m.protocol, "RTP/AVP");
        assert_eq!(m.formats, vec!["0", "8", "101"]);
    }

    /// Sprint 2.5 P2 regression: every plaintext (RTP/AVP) offer must
    /// advertise PT 101 telephone-event + the RFC 4733 fmtp param
    /// range. Pre-P2 the non-SRTP code path emitted `m=audio … RTP/AVP
    /// 0 8` with no DTMF rtpmap, which silently broke DTMF negotiation
    /// for any plaintext call. The unified `generate_local_sdp` emits
    /// the full PCMU + PCMA + 101 set on every offer.
    #[test]
    fn offer_advertises_telephone_event_on_plaintext() {
        let sdp = build_offer("d", 0, "127.0.0.1", 16000);
        assert!(
            sdp.contains("m=audio 16000 RTP/AVP 0 8 101\r\n"),
            "plaintext offer must advertise PT 101 alongside PCMU + PCMA:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=rtpmap:101 telephone-event/8000\r\n"),
            "plaintext offer must carry the RFC 4733 telephone-event rtpmap:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=fmtp:101 0-15\r\n"),
            "plaintext offer must carry the RFC 4733 fmtp 0-15 range:\n{}",
            sdp
        );
    }

    /// Build an SRTP-flavoured offer (RFC 4568 §3.1.4: m= profile is
    /// `RTP/SAVP`) directly via the builder so we can assert the
    /// shape without standing up a full MediaAdapter. Mirrors the
    /// unified `generate_local_sdp` shape — PCMU + PCMA + telephone-event
    /// — with crypto lines appended.
    fn build_srtp_offer(ip: &str, port: u16, attrs: Vec<CryptoAttribute>) -> String {
        let mut media_builder = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", ip)
            .connection("IN", "IP4", ip)
            .time("0", "0")
            .media_audio(port, "RTP/SAVP")
                .formats(&["0", "8", "101"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .rtpmap("101", "telephone-event/8000")
                .fmtp("101", "0-15");
        for attr in attrs {
            media_builder = media_builder.crypto_attribute(attr);
        }
        media_builder
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("srtp offer builds")
            .to_string()
    }

    #[test]
    fn srtp_offer_uses_savp_profile_and_carries_crypto_lines() {
        // RFC 4568 §3.1.4 — m= line MUST be RTP/SAVP when offering SDES.
        use crate::adapters::srtp_negotiator::SrtpNegotiator;
        let suites = vec![
            CryptoSuite::AesCm128HmacSha1_80,
            CryptoSuite::AesCm128HmacSha1_32,
        ];
        let (_, attrs) = SrtpNegotiator::new_offerer(&suites).unwrap();
        let sdp = build_srtp_offer("127.0.0.1", 16000, attrs);

        // Wire-level checks.
        assert!(
            sdp.contains("m=audio 16000 RTP/SAVP 0 8 101\r\n"),
            "SRTP offer should use RTP/SAVP profile per RFC 4568 §3.1.4 \
             with the unified PCMU+PCMA+telephone-event format set:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:"),
            "SRTP offer should carry tag-1 _80 crypto line:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=crypto:2 AES_CM_128_HMAC_SHA1_32 inline:"),
            "SRTP offer should carry tag-2 _32 crypto line:\n{}",
            sdp
        );

        // Round-trip: parse back via the typed parser, assert the
        // crypto attributes survive both directions.
        let parsed = SdpSession::from_str(&sdp).expect("parses");
        let m = &parsed.media_descriptions[0];
        assert_eq!(m.protocol, "RTP/SAVP");
        let crypto_count = m
            .generic_attributes
            .iter()
            .filter(|a| matches!(a, ParsedAttribute::Crypto(_)))
            .count();
        assert_eq!(crypto_count, 2);
    }

    #[test]
    fn extract_audio_crypto_finds_both_offered_lines() {
        use crate::adapters::srtp_negotiator::SrtpNegotiator;
        let (_, attrs) = SrtpNegotiator::new_offerer(&[
            CryptoSuite::AesCm128HmacSha1_80,
            CryptoSuite::AesCm128HmacSha1_32,
        ])
        .unwrap();
        let sdp = build_srtp_offer("127.0.0.1", 16000, attrs);
        let parsed = SdpSession::from_str(&sdp).expect("parses");
        let extracted = MediaAdapter::extract_audio_crypto(&parsed);
        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].tag, 1);
        assert_eq!(extracted[1].tag, 2);
    }

    /// Sprint 3 A6 — when a public RTP address is configured (static
    /// or STUN-discovered), the offer's c=/o=/m= lines must advertise
    /// it instead of the local interface IP/port. Mirrors what the
    /// generate_local_sdp body does — `local_ip_str` resolves to
    /// `public.ip()` when set, and `port` to `public.port()` when
    /// non-zero, else falls back to the per-session local port.
    #[test]
    fn public_rtp_addr_override_replaces_local_ip_and_port_in_offer() {
        let public: SocketAddr = "203.0.113.42:30000".parse().unwrap();
        let local_fallback: std::net::IpAddr = "192.168.1.10".parse().unwrap();
        let local_port_fallback: u16 = 16000;

        // Replicate the override branch the way generate_local_sdp does it.
        let public_opt = Some(public);
        let advertised_ip = public_opt.map(|sa| sa.ip()).unwrap_or(local_fallback);
        let port = public_opt
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or(local_port_fallback);

        let sdp = build_offer("dlg", 0, &advertised_ip.to_string(), port);
        assert!(
            sdp.contains("c=IN IP4 203.0.113.42\r\n"),
            "c= must carry public IP when override set:\n{}",
            sdp
        );
        assert!(
            sdp.contains("o=- dlg 0 IN IP4 203.0.113.42\r\n"),
            "o= must carry public IP when override set:\n{}",
            sdp
        );
        assert!(
            sdp.contains("m=audio 30000 RTP/AVP"),
            "m=audio must carry public port when override set:\n{}",
            sdp
        );
    }

    #[test]
    fn public_rtp_addr_unset_falls_back_to_local_ip_and_local_port() {
        let public_opt: Option<SocketAddr> = None;
        let local_fallback: std::net::IpAddr = "192.168.1.10".parse().unwrap();
        let local_port_fallback: u16 = 16000;

        let advertised_ip = public_opt.map(|sa| sa.ip()).unwrap_or(local_fallback);
        let port = public_opt
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or(local_port_fallback);

        let sdp = build_offer("dlg", 0, &advertised_ip.to_string(), port);
        assert!(
            sdp.contains("c=IN IP4 192.168.1.10\r\n"),
            "c= falls back to local_ip when no override:\n{}",
            sdp
        );
        assert!(
            sdp.contains("m=audio 16000 RTP/AVP"),
            "m=audio falls back to local_port when no override:\n{}",
            sdp
        );
    }

    /// Sprint 3 C1 — when `comfort_noise_enabled` is set, the SDP
    /// offer's `m=audio` line lists `13` and the body carries an
    /// `a=rtpmap:13 CN/8000` line. The order must be `0 8 13 101` so
    /// telephone-event remains last (Sprint 2.5 P2 fixture stability).
    #[test]
    fn cn_enabled_offer_advertises_pt13_and_rtpmap() {
        let ip = "127.0.0.1";
        let port = 16000;
        let sdp = SdpBuilder::new("Session")
            .origin("-", "dlg", "0", "IN", "IP4", ip)
            .connection("IN", "IP4", ip)
            .time("0", "0")
            .media_audio(port, "RTP/AVP")
                .formats(&["0", "8", "13", "101"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .rtpmap("13", "CN/8000")
                .rtpmap("101", "telephone-event/8000")
                .fmtp("101", "0-15")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("offer builds")
            .to_string();

        assert!(
            sdp.contains("m=audio 16000 RTP/AVP 0 8 13 101\r\n"),
            "format list must include 13 between PCMA and telephone-event:\n{}",
            sdp
        );
        assert!(
            sdp.contains("a=rtpmap:13 CN/8000\r\n"),
            "RFC 3389 CN rtpmap must appear:\n{}",
            sdp
        );
        // Sanity: existing PTs still present in the right shape.
        assert!(sdp.contains("a=rtpmap:0 PCMU/8000\r\n"));
        assert!(sdp.contains("a=rtpmap:8 PCMA/8000\r\n"));
        assert!(sdp.contains("a=rtpmap:101 telephone-event/8000\r\n"));
    }

    /// Sprint 3.5 — strict matching answers with the intersection
    /// only. Offer = `0 101` (no PCMA); answer must carry `0 101`,
    /// not the legacy full `0 8 101` set.
    #[test]
    fn strict_default_answers_with_intersection_only() {
        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
                .formats(&["0", "101"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("101", "telephone-event/8000")
                .fmtp("101", "0-15")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("offer builds")
            .to_string();
        let offer = SdpSession::from_str(&offer_sdp).expect("offer parses");

        let formats = compute_answer_formats(
            &offer,
            /*comfort_noise_enabled*/ false,
            /*strict*/ true,
            /*offer_srtp*/ false,
            /*srtp_required*/ false,
        )
        .expect("strict-mode match succeeds");
        assert_eq!(
            formats,
            vec!["0".to_string(), "101".to_string()],
            "strict answer must drop PCMA when the offer didn't list it"
        );
    }

    /// Sprint 3.5 — permissive mode preserves the legacy
    /// pre-Sprint-3.5 "always full set" answer shape.
    #[test]
    fn permissive_mode_answers_with_full_set() {
        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
                .formats(&["0", "101"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("101", "telephone-event/8000")
                .fmtp("101", "0-15")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("offer builds")
            .to_string();
        let offer = SdpSession::from_str(&offer_sdp).expect("offer parses");

        let formats = compute_answer_formats(
            &offer,
            /*comfort_noise_enabled*/ false,
            /*strict*/ false,
            /*offer_srtp*/ false,
            /*srtp_required*/ false,
        )
        .expect("permissive mode never errors on overlap");
        assert_eq!(
            formats,
            vec!["0".to_string(), "8".to_string(), "101".to_string()],
            "permissive answer keeps the full legacy set"
        );
    }

    /// Sprint 3.5 — strict mode + zero overlap returns
    /// `SDPNegotiationFailed`. The state machine turns this into
    /// `488 Not Acceptable Here` (the same path `srtp_required`
    /// already uses on a plain offer).
    #[test]
    fn strict_default_no_overlap_returns_negotiation_failed() {
        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
                .formats(&["97", "98"])
                .rtpmap("97", "VP8/90000")
                .rtpmap("98", "VP9/90000")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("offer builds")
            .to_string();
        let offer = SdpSession::from_str(&offer_sdp).expect("offer parses");

        let err = compute_answer_formats(
            &offer,
            /*comfort_noise_enabled*/ false,
            /*strict*/ true,
            /*offer_srtp*/ false,
            /*srtp_required*/ false,
        )
        .unwrap_err();
        assert!(
            matches!(err, SessionError::SDPNegotiationFailed(_)),
            "no overlap must surface as SDPNegotiationFailed → 488 NAH; got {:?}",
            err
        );
    }

    /// Sprint 3.5 — strict matching preserves CN advertisement
    /// when both peers offer it. Offer `0 13 101`, our caps include
    /// `13` (comfort_noise_enabled=true), answer carries `0 13 101`.
    #[test]
    fn strict_with_cn_enabled_keeps_cn_in_intersection() {
        let offer_sdp = SdpBuilder::new("Session")
            .origin("-", "1", "0", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(16000, "RTP/AVP")
                .formats(&["0", "13", "101"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("13", "CN/8000")
                .rtpmap("101", "telephone-event/8000")
                .fmtp("101", "0-15")
                .attribute("sendrecv", None::<String>)
                .done()
            .build()
            .expect("offer builds")
            .to_string();
        let offer = SdpSession::from_str(&offer_sdp).expect("offer parses");

        let formats = compute_answer_formats(
            &offer,
            /*comfort_noise_enabled*/ true,
            /*strict*/ true,
            /*offer_srtp*/ false,
            /*srtp_required*/ false,
        )
        .expect("CN-on-both-sides match succeeds");
        assert_eq!(
            formats,
            vec!["0".to_string(), "13".to_string(), "101".to_string()],
            "intersection must include 13 when both sides advertise it"
        );
    }

    #[test]
    fn cn_disabled_offer_omits_pt13_and_rtpmap() {
        // The pre-Sprint-3 baseline shape — no `13`, no CN rtpmap.
        let sdp = build_offer("dlg", 0, "127.0.0.1", 16000);
        assert!(
            sdp.contains("m=audio 16000 RTP/AVP 0 8 101\r\n"),
            "default offer must keep the pre-Sprint-3 format set:\n{}",
            sdp
        );
        assert!(!sdp.contains("CN/8000"), "default offer must not advertise CN: \n{}", sdp);
    }

    #[test]
    fn public_rtp_addr_with_zero_port_keeps_local_port() {
        // The override semantics: when `media_public_addr` carries an
        // IP-only mapping (port 0), advertise the public IP but keep
        // the per-session local port. Useful for SBC-fronted setups
        // where the port doesn't change but the IP does.
        let public: SocketAddr = "203.0.113.42:0".parse().unwrap();
        let public_opt = Some(public);
        let local_fallback: std::net::IpAddr = "192.168.1.10".parse().unwrap();
        let local_port_fallback: u16 = 16000;

        let advertised_ip = public_opt.map(|sa| sa.ip()).unwrap_or(local_fallback);
        let port = public_opt
            .filter(|sa| sa.port() != 0)
            .map(|sa| sa.port())
            .unwrap_or(local_port_fallback);

        assert_eq!(advertised_ip.to_string(), "203.0.113.42");
        assert_eq!(port, 16000, "zero port must defer to local_port_fallback");
    }
}