//! Media session lifecycle (create/update/terminate), SDP, and ICE methods for MediaManager

use crate::api::types::SessionId;
use super::super::types::*;
use super::super::MediaError;
use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::Mutex;
use super::MediaManager;
use super::super::MediaResult;
use std::collections::HashSet;
use rvoip_media_core::MediaSessionId as MediaCoreSessionId;
use rvoip_rtp_core::transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};
use rvoip_rtp_core::ice::{IceAgentAdapter, IceRole, IceCandidate, IceConnectionState, CandidateType, ComponentId};

impl MediaManager {
    /// Create a new media session for a SIP session using real MediaSessionController
    pub async fn create_media_session(&self, session_id: &SessionId) -> super::super::MediaResult<MediaSessionInfo> {
        tracing::trace!("📹 create_media_session called for: {}", session_id);
        
        // Create dialog ID for media session (use session ID as base)
        let dialog_id = DialogId::new(format!("media-{}", session_id));
        tracing::trace!("📹 Using dialog_id: {}", dialog_id);
        
        // Check if this media session already exists
        if let Some(existing_info) = self.controller.get_session_info(&dialog_id).await {
            tracing::trace!("📹 Media session already exists in controller for {}, reusing", dialog_id);
            
            // Ensure session mapping exists
            {
                let mut mapping = self.session_mapping.write().await;
                mapping.insert(session_id.clone(), dialog_id.clone());
            }
            
            // Ensure zero-copy config exists
            {
                let mut configs = self.zero_copy_config.write().await;
                configs.insert(session_id.clone(), super::ZeroCopyConfig::default());
            }
            
            let session_info = MediaSessionInfo::from(existing_info);
            tracing::trace!("📹 Reused existing media session: {} for SIP session: {}", dialog_id, session_id);
            return Ok(session_info);
        }
        
        // Create media configuration using the manager's configured preferences
        let media_config = convert_to_media_core_config(
            &self.media_config,
            self.local_bind_addr,
            None, // Will be set later when remote SDP is processed
        );
        
        tracing::trace!("📹 Starting new media session in controller for {}", dialog_id);
        // Start media session using real MediaSessionController
        match self.controller.start_media(dialog_id.clone(), media_config).await {
            Ok(()) => {
                tracing::trace!("📹 MediaSessionController.start_media SUCCESS for {}", dialog_id);
            }
            Err(e) => {
                tracing::trace!("📹 MediaSessionController.start_media FAILED for {}: {}", dialog_id, e);
                return Err(MediaError::MediaEngine { source: Box::new(e) });
            }
        }
        
        tracing::trace!("📹 Getting session info from controller for {}", dialog_id);
        // Get session info from controller
        let media_session_info = self.controller.get_session_info(&dialog_id).await
            .ok_or_else(|| {
                tracing::trace!("📹 get_session_info returned None for {}", dialog_id);
                MediaError::SessionNotFound { session_id: dialog_id.to_string() }
            })?;
        
        // Store session mapping
        {
            let mut mapping = self.session_mapping.write().await;
            mapping.insert(session_id.clone(), dialog_id.clone());
            tracing::trace!("📹 Stored session mapping: {} -> {}", session_id, dialog_id);
        }
        
        // Initialize zero-copy configuration for new session
        {
            let mut configs = self.zero_copy_config.write().await;
            configs.insert(session_id.clone(), super::ZeroCopyConfig::default());
        }
        
        // Convert to our MediaSessionInfo type
        let session_info = MediaSessionInfo::from(media_session_info);

        // Create ICE agent and gather candidates if ICE is enabled
        if self.media_config.ice.enabled {
            let local_port = session_info.local_rtp_port.unwrap_or(0);
            let local_addr = SocketAddr::new(self.local_bind_addr.ip(), local_port);
            let mut agent = IceAgentAdapter::new(IceRole::Controlling);

            let stun_servers = &self.media_config.ice.stun_servers;
            let turn_configs = &self.media_config.ice.turn_servers;
            match agent.gather_candidates_with_turn(local_addr, stun_servers, turn_configs).await {
                Ok(candidates) => {
                    tracing::info!(
                        session = %session_id,
                        candidates = candidates.len(),
                        "ICE candidate gathering complete"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        session = %session_id,
                        error = %e,
                        "ICE candidate gathering failed, falling back to host-only"
                    );
                    // Gather host-only candidates without STUN/TURN
                    let empty_stun: Vec<SocketAddr> = Vec::new();
                    if let Err(e2) = agent.gather_candidates(local_addr, &empty_stun).await {
                        tracing::error!(
                            session = %session_id,
                            error = %e2,
                            "ICE host candidate gathering also failed"
                        );
                    }
                }
            }

            let mut agents = self.ice_agents.write().await;
            agents.insert(session_id.clone(), agent);
        }

        tracing::trace!("Successfully created NEW media session: {} for SIP session: {}", dialog_id, session_id);

        Ok(session_info)
    }

    /// Update a media session with new SDP (for re-INVITE, etc.)
    pub async fn update_media_session(&self, session_id: &SessionId, sdp: &str) -> super::super::MediaResult<()> {
        tracing::debug!("Updating media session for SIP session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        // Store the remote SDP
        {
            let mut sdp_storage = self.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.1 = Some(sdp.to_string());
        }
        
        // Parse SDP to extract remote address and codec information
        let remote_addr = self.parse_remote_address_from_sdp(sdp);
        let codec = self.parse_codec_from_sdp(sdp);
        
        if let Some(remote_addr) = remote_addr {
            // Create enhanced media configuration with remote address and codec
            let mut session_config = MediaConfig::default();
            if let Some(codec_name) = codec {
                session_config.preferred_codecs = vec![codec_name];
            }
            
            let updated_config = convert_to_media_core_config(
                &session_config,
                self.local_bind_addr,
                Some(remote_addr),
            );
            
            self.controller.update_media(dialog_id, updated_config).await
                .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
                
            tracing::info!("✅ Updated media session for SIP session: {} with remote: {} and codecs: {:?}", 
                          session_id, remote_addr, session_config.preferred_codecs);
        } else {
            tracing::warn!("Could not parse SDP for session: {}, skipping media update", session_id);
        }
        
        Ok(())
    }
    
    /// Terminate a media session
    pub async fn terminate_media_session(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        tracing::debug!("Terminating media session for SIP session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mut mapping = self.session_mapping.write().await;
            mapping.remove(session_id)
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        // Cleanup zero-copy configuration
        self.cleanup_zero_copy_config(session_id).await;

        // Cleanup SRTP bridge
        self.cleanup_srtp_bridge(session_id).await;

        // Cleanup ICE agent
        {
            let mut agents = self.ice_agents.write().await;
            if let Some(mut agent) = agents.remove(session_id) {
                agent.close().await;
            }
        }

        // Cleanup codec processing systems
        if let Err(e) = self.cleanup_codec_processing(session_id).await {
            tracing::warn!("Failed to cleanup codec processing for session {}: {}", session_id, e);
        }

        // Cleanup SDP storage
        {
            let mut sdp_storage = self.sdp_storage.write().await;
            sdp_storage.remove(session_id);
        }

        // Stop media session using real MediaSessionController
        self.controller.stop_media(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;

        tracing::info!("Terminated media session: {} for SIP session: {} (including zero-copy + SRTP cleanup)", dialog_id, session_id);
        Ok(())
    }
    
    /// Check if a session has a media mapping (for duplicate creation prevention)
    pub async fn has_session_mapping(&self, session_id: &SessionId) -> bool {
        let mapping = self.session_mapping.read().await;
        mapping.contains_key(session_id)
    }
    
    /// Get media information for a session
    pub async fn get_media_info(&self, session_id: &SessionId) -> super::super::MediaResult<Option<MediaSessionInfo>> {
        tracing::debug!("Getting media info for SIP session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
        };
        
        if let Some(dialog_id) = dialog_id {
            // Get session info from controller
            if let Some(media_session_info) = self.controller.get_session_info(&dialog_id).await {
                let mut session_info = MediaSessionInfo::from(media_session_info);
                
                // Add stored SDP
                let sdp_storage = self.sdp_storage.read().await;
                if let Some((local_sdp, remote_sdp)) = sdp_storage.get(session_id) {
                    session_info.local_sdp = local_sdp.clone();
                    session_info.remote_sdp = remote_sdp.clone();
                }
                
                Ok(Some(session_info))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
    
    /// Generate SDP offer for a session using real media session information
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> super::super::MediaResult<String> {
        tracing::debug!("Generating SDP offer for session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
        };
        
        // If we have a media session, get its info for SDP generation
        let media_info = if let Some(dialog_id) = dialog_id {
            self.controller.get_session_info(&dialog_id).await
        } else {
            None
        };
        
        // Generate SDP using MediaConfigConverter with configured preferences
        use crate::media::config::MediaConfigConverter;
        let converter = MediaConfigConverter::with_media_config(&self.media_config);
        
        let local_ip = self.local_bind_addr.ip().to_string();
        let local_port = if let Some(info) = media_info {
            info.rtp_port.unwrap_or(10000)
        } else {
            10000 // Default port if no media session exists yet
        };
        
        let mut sdp = converter.generate_sdp_offer(&local_ip, local_port)
            .map_err(|e| MediaError::Configuration { message: e.to_string() })?;

        // Append ICE attributes if an ICE agent exists for this session
        {
            let agents = self.ice_agents.read().await;
            if let Some(agent) = agents.get(session_id) {
                let creds = agent.local_credentials();
                let candidates = agent.local_candidates();

                // If we have a server-reflexive candidate, use its address
                // as the SDP connection address for better NAT traversal.
                if let Some(srflx) = candidates.iter().find(|c| c.candidate_type == CandidateType::ServerReflexive) {
                    sdp = sdp.replace(
                        &format!("c=IN IP4 {}", local_ip),
                        &format!("c=IN IP4 {}", srflx.address.ip()),
                    );
                }

                // Build ICE attribute lines to insert before a=sendrecv
                let mut ice_lines = String::new();
                ice_lines.push_str(&format!("a=ice-ufrag:{}\r\n", creds.ufrag));
                ice_lines.push_str(&format!("a=ice-pwd:{}\r\n", creds.pwd));

                // RFC 8840: signal trickle ICE support
                if agent.is_trickle_enabled() {
                    ice_lines.push_str("a=ice-options:trickle\r\n");
                }

                for candidate in candidates {
                    ice_lines.push_str(&format!("a=candidate:{}\r\n", candidate.to_sdp_attribute()));
                }

                // Insert before the sendrecv attribute
                if let Some(pos) = sdp.rfind("a=sendrecv") {
                    sdp.insert_str(pos, &ice_lines);
                } else {
                    sdp.push_str(&ice_lines);
                }

                tracing::debug!(
                    session = %session_id,
                    candidates = candidates.len(),
                    "included ICE attributes in SDP offer"
                );
            }
        }

        // Append DTLS-SRTP attributes when SRTP is enabled in config.
        if self.media_config.srtp.enabled {
            if let Some(ref fp) = self.media_config.srtp.local_fingerprint {
                // Upgrade transport from RTP/AVP to RTP/SAVP
                sdp = sdp.replace("RTP/AVP", "RTP/SAVP");

                let dtls_attrs =
                    super::super::srtp_bridge::generate_dtls_sdp_attributes(fp, true);

                // Insert before a=sendrecv
                if let Some(pos) = sdp.rfind("a=sendrecv") {
                    sdp.insert_str(pos, &dtls_attrs);
                } else {
                    sdp.push_str(&dtls_attrs);
                }

                tracing::debug!(
                    session = %session_id,
                    "Included DTLS-SRTP attributes in SDP offer"
                );
            } else {
                tracing::warn!(
                    session = %session_id,
                    "SRTP enabled but no local fingerprint configured -- \
                     SDP offer will not include DTLS attributes"
                );
            }
        }

        // Store the generated local SDP
        {
            let mut sdp_storage = self.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.0 = Some(sdp.clone());
        }

        tracing::info!("Generated SDP offer for session: {} with port: {} and codecs: {:?}",
                      session_id, local_port, self.media_config.preferred_codecs);
        Ok(sdp)
    }
    
    /// Helper method to parse remote address from SDP (improved implementation)
    fn parse_remote_address_from_sdp(&self, sdp: &str) -> Option<SocketAddr> {
        // Enhanced SDP parsing to extract remote address and port
        let mut remote_ip = None;
        let mut remote_port = None;
        
        for line in sdp.lines() {
            if line.starts_with("c=IN IP4 ") {
                if let Some(ip_str) = line.strip_prefix("c=IN IP4 ") {
                    remote_ip = ip_str.trim().parse().ok();
                }
            } else if line.starts_with("m=audio ") {
                // Parse m=audio line: "m=audio 10001 RTP/AVP 96"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    remote_port = parts[1].parse().ok();
                }
            }
        }
        
        if let (Some(ip), Some(port)) = (remote_ip, remote_port) {
            tracing::debug!("Parsed remote address from SDP: {}:{}", ip, port);
            Some(SocketAddr::new(ip, port))
        } else {
            tracing::warn!("Could not parse remote address from SDP - ip: {:?}, port: {:?}", remote_ip, remote_port);
            None
        }
    }
    
    /// Parse codec information from SDP
    fn parse_codec_from_sdp(&self, sdp: &str) -> Option<String> {
        for line in sdp.lines() {
            if line.starts_with("a=rtpmap:") {
                // Parse a=rtpmap:96 opus/48000/2 -> return "opus"
                if let Some(codec_part) = line.split_whitespace().nth(1) {
                    if let Some(codec_name) = codec_part.split('/').next() {
                        tracing::debug!("Parsed codec from SDP: {}", codec_name);
                        return Some(codec_name.to_string());
                    }
                }
            }
        }
        None
    }
    
    /// Process SDP answer and configure media session.
    ///
    /// This also inspects the answer for DTLS-SRTP indicators and, if
    /// present, creates an `SrtpMediaBridge` for the session.  The caller
    /// must subsequently call `perform_srtp_handshake` before starting media.
    ///
    /// When ICE is enabled, remote ICE credentials and candidates are parsed
    /// from the SDP answer and fed into the session's ICE agent.  Connectivity
    /// checks are started, and if a selected pair is found the media session's
    /// remote address is updated accordingly.
    pub async fn process_sdp_answer(&self, session_id: &SessionId, sdp: &str) -> super::super::MediaResult<()> {
        tracing::debug!("Processing SDP answer for session: {}", session_id);

        // Parse remote address from SDP and update media session
        if let Some(remote_addr) = self.parse_remote_address_from_sdp(sdp) {
            self.update_media_session(session_id, sdp).await?;
            tracing::info!("Processed SDP answer and updated remote address to: {}", remote_addr);
        } else {
            tracing::warn!("Could not parse remote address from SDP answer");
        }

        // Process remote ICE attributes if we have an agent for this session
        self.process_remote_ice(session_id, sdp).await?;

        // Check for DTLS-SRTP and initiate the full handshake if the
        // RTP socket is already available.  If the socket is not yet
        // allocated the bridge is still created; the handshake will be
        // driven later by the coordinator when the media session is ready.
        if let Some(remote_addr) = self.parse_remote_address_from_sdp(sdp) {
            if let Err(e) = self.initiate_srtp_for_session(session_id, sdp, remote_addr).await {
                tracing::warn!(
                    session = %session_id,
                    error = %e,
                    "DTLS-SRTP setup in process_sdp_answer failed -- plain RTP will be used"
                );
            }
        } else {
            // No remote address -- just create the bridge for later.
            let srtp = self.setup_srtp_from_sdp(session_id, sdp).await?;
            if srtp {
                tracing::info!(
                    session = %session_id,
                    "SDP answer indicates DTLS-SRTP -- bridge created, handshake pending"
                );
            }
        }

        Ok(())
    }

    /// Parse ICE credentials and candidates from remote SDP and feed them
    /// into the session's ICE agent.  Starts connectivity checks if
    /// sufficient information is available.
    async fn process_remote_ice(&self, session_id: &SessionId, sdp: &str) -> super::super::MediaResult<()> {
        let mut agents = self.ice_agents.write().await;
        let agent = match agents.get_mut(session_id) {
            Some(a) => a,
            None => return Ok(()), // ICE not active for this session
        };

        // Parse remote ICE credentials from SDP
        let mut remote_ufrag: Option<String> = None;
        let mut remote_pwd: Option<String> = None;
        let mut remote_candidates: Vec<IceCandidate> = Vec::new();

        for line in sdp.lines() {
            let trimmed = line.trim();

            if let Some(ufrag) = trimmed.strip_prefix("a=ice-ufrag:") {
                remote_ufrag = Some(ufrag.to_string());
            } else if let Some(pwd) = trimmed.strip_prefix("a=ice-pwd:") {
                remote_pwd = Some(pwd.to_string());
            } else if let Some(cand_str) = trimmed.strip_prefix("a=candidate:") {
                if let Some(candidate) = Self::parse_ice_candidate(cand_str) {
                    remote_candidates.push(candidate);
                }
            }
        }

        // Set remote credentials if present
        if let (Some(ufrag), Some(pwd)) = (remote_ufrag, remote_pwd) {
            agent.set_remote_credentials(ufrag, pwd);
        } else {
            tracing::debug!(
                session = %session_id,
                "no remote ICE credentials in SDP, skipping ICE processing"
            );
            return Ok(());
        }

        // Add remote candidates
        if !remote_candidates.is_empty() {
            tracing::info!(
                session = %session_id,
                count = remote_candidates.len(),
                "adding remote ICE candidates from SDP"
            );
            agent.add_remote_candidates(remote_candidates);
        }

        // Start connectivity checks (webrtc-ice handles the check loop internally)
        if let Err(e) = agent.start_checks().await {
            tracing::warn!(
                session = %session_id,
                error = %e,
                "failed to start ICE connectivity checks"
            );
            return Ok(());
        }

        // webrtc-ice drives connectivity checks internally via dial/accept.
        // Check if a selected pair is already available (e.g., from a
        // triggered check that succeeded). The on_selected_candidate_pair_change
        // callback will update the selected pair asynchronously as checks complete.
        if let Some(pair) = agent.selected_pair().await {
            let selected_remote = pair.remote.address;
            tracing::info!(
                session = %session_id,
                remote = %selected_remote,
                "ICE selected pair available, updating media remote address"
            );
            // Release the agents lock before updating media session
            drop(agents);
            self.update_media_remote_address(session_id, selected_remote).await?;
        }

        Ok(())
    }

    /// Parse a single ICE candidate from the value portion of an
    /// `a=candidate:` SDP attribute line.
    fn parse_ice_candidate(value: &str) -> Option<IceCandidate> {
        // Format: foundation component transport priority address port typ type [raddr addr rport port]
        let parts: Vec<&str> = value.split_whitespace().collect();
        if parts.len() < 8 {
            return None;
        }

        let foundation = parts[0].to_string();
        let component = match parts[1] {
            "1" => ComponentId::Rtp,
            "2" => ComponentId::Rtcp,
            _ => return None,
        };
        let transport = parts[2].to_lowercase();
        let priority: u32 = parts[3].parse().ok()?;
        let ip: std::net::IpAddr = parts[4].parse().ok()?;
        let port: u16 = parts[5].parse().ok()?;
        // parts[6] should be "typ"
        let candidate_type = match parts[7] {
            "host" => CandidateType::Host,
            "srflx" => CandidateType::ServerReflexive,
            "prflx" => CandidateType::PeerReflexive,
            "relay" => CandidateType::Relay,
            _ => return None,
        };

        let mut related_address: Option<SocketAddr> = None;
        let mut i = 8;
        while i + 1 < parts.len() {
            match parts[i] {
                "raddr" => {
                    if let Ok(rip) = parts[i + 1].parse::<std::net::IpAddr>() {
                        // Look for rport
                        if i + 3 < parts.len() && parts[i + 2] == "rport" {
                            if let Ok(rport) = parts[i + 3].parse::<u16>() {
                                related_address = Some(SocketAddr::new(rip, rport));
                                i += 4;
                                continue;
                            }
                        }
                        related_address = Some(SocketAddr::new(rip, 0));
                        i += 2;
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        Some(IceCandidate {
            foundation,
            component,
            transport,
            priority,
            address: SocketAddr::new(ip, port),
            candidate_type,
            related_address,
            ufrag: String::new(), // ufrag comes from session-level attribute
        })
    }

    /// Update the media session's remote address (e.g., after ICE
    /// connectivity checks select a candidate pair).
    async fn update_media_remote_address(
        &self,
        session_id: &SessionId,
        remote_addr: SocketAddr,
    ) -> super::super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        let updated_config = convert_to_media_core_config(
            &self.media_config,
            self.local_bind_addr,
            Some(remote_addr),
        );
        self.controller.update_media(dialog_id, updated_config).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        tracing::info!(
            session = %session_id,
            remote = %remote_addr,
            "media remote address updated via ICE"
        );
        Ok(())
    }

    /// Get the ICE connection state for a session, if ICE is active.
    pub async fn get_ice_state(&self, session_id: &SessionId) -> Option<IceConnectionState> {
        let agents = self.ice_agents.read().await;
        match agents.get(session_id) {
            Some(a) => Some(a.state_sync()),
            None => None,
        }
    }

    /// Get the selected ICE candidate pair for a session.
    pub async fn get_ice_selected_pair(&self, session_id: &SessionId) -> Option<(SocketAddr, SocketAddr)> {
        let agents = self.ice_agents.read().await;
        agents.get(session_id).and_then(|a| {
            a.selected_pair_sync().map(|p| (p.local.address, p.remote.address))
        })
    }

    // ---------------------------------------------------------------
    // Trickle ICE (RFC 8838 / RFC 8840)
    // ---------------------------------------------------------------

    /// Enable trickle ICE for a session's ICE agent.
    pub async fn enable_trickle_ice(&self, session_id: &SessionId) -> super::super::MediaResult<()> {
        let mut agents = self.ice_agents.write().await;
        let agent = agents.get_mut(session_id).ok_or_else(|| {
            super::super::MediaError::Ice {
                message: format!("No ICE agent for session {}", session_id),
            }
        })?;
        agent.enable_trickle();
        Ok(())
    }

    /// Check whether trickle ICE is enabled for a session.
    pub async fn is_trickle_ice_enabled(&self, session_id: &SessionId) -> bool {
        let agents = self.ice_agents.read().await;
        agents
            .get(session_id)
            .map_or(false, |a| a.is_trickle_enabled())
    }

    /// Add a remote ICE candidate received via trickle (SIP INFO).
    ///
    /// Parses the `a=candidate:` SDP attribute line and feeds it into
    /// the session's ICE agent. If connectivity checks are already
    /// running the agent will create a triggered check for the new pair.
    pub async fn add_remote_ice_candidate(
        &self,
        session_id: &SessionId,
        candidate_line: &str,
    ) -> super::super::MediaResult<()> {
        let candidate = IceCandidate::from_sdp_attribute(candidate_line).map_err(|e| {
            super::super::MediaError::Ice {
                message: format!("Failed to parse trickle candidate: {}", e),
            }
        })?;

        let mut agents = self.ice_agents.write().await;
        let agent = agents.get_mut(session_id).ok_or_else(|| {
            super::super::MediaError::Ice {
                message: format!("No ICE agent for session {}", session_id),
            }
        })?;

        tracing::info!(
            "Adding trickle remote candidate for session {}: {}",
            session_id,
            candidate
        );
        agent.add_remote_candidate(candidate);

        Ok(())
    }

    /// Signal that the remote side has finished sending trickle candidates.
    pub async fn set_remote_end_of_candidates(
        &self,
        session_id: &SessionId,
    ) -> super::super::MediaResult<()> {
        let mut agents = self.ice_agents.write().await;
        let agent = agents.get_mut(session_id).ok_or_else(|| {
            super::super::MediaError::Ice {
                message: format!("No ICE agent for session {}", session_id),
            }
        })?;

        agent.set_end_of_candidates();
        tracing::info!(
            "Remote end-of-candidates set for session {}",
            session_id
        );

        Ok(())
    }

    /// Gather only host candidates for trickle ICE (fast, synchronous).
    ///
    /// Returns the host candidates immediately. STUN/TURN gathering should
    /// be done in a background task and trickled to the remote side.
    pub async fn gather_host_candidates_for_trickle(
        &self,
        session_id: &SessionId,
    ) -> super::super::MediaResult<Vec<IceCandidate>> {
        let mut agents = self.ice_agents.write().await;
        let agent = agents.get_mut(session_id).ok_or_else(|| {
            super::super::MediaError::Ice {
                message: format!("No ICE agent for session {}", session_id),
            }
        })?;

        let local_port: u16 = {
            let mapping = self.session_mapping.read().await;
            if let Some(dialog_id) = mapping.get(session_id) {
                self.controller
                    .get_session_info(dialog_id)
                    .await
                    .and_then(|info| info.rtp_port)
                    .unwrap_or(0)
            } else {
                0
            }
        };

        let local_addr = SocketAddr::new(self.local_bind_addr.ip(), local_port);
        let candidates = agent.gather_host_candidates_only(local_addr);

        tracing::info!(
            "Trickle: gathered {} host candidates for session {}",
            candidates.len(),
            session_id
        );

        Ok(candidates)
    }

    /// List all active media sessions
    pub async fn list_active_sessions(&self) -> Vec<MediaSessionInfo> {
        let mut sessions = Vec::new();
        let mapping = self.session_mapping.read().await;
        
        for dialog_id in mapping.values() {
            if let Some(media_session_info) = self.controller.get_session_info(dialog_id).await {
                sessions.push(MediaSessionInfo::from(media_session_info));
            }
        }
        
        sessions
    }
    
    /// Get the local bind address
    pub fn get_local_bind_addr(&self) -> SocketAddr {
        self.local_bind_addr
    }
}
