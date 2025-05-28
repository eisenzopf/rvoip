//! Call Lifecycle Coordination
//!
//! This module manages the complete call lifecycle coordination between
//! dialog manager, transaction-core, and media-core. It handles the
//! application logic for call acceptance, rejection, and state management.

use std::sync::Arc;
use anyhow::Result;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};
use chrono;

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{HeaderName, TypedHeader};
use rvoip_transaction_core::TransactionKey;
use crate::dialog::transaction_coordination::TransactionCoordinator;
use crate::dialog::manager::DialogManager;
use crate::media::MediaManager;
use crate::session::SessionId;
use uuid;

/// Call lifecycle coordinator
///
/// This struct manages the complete call lifecycle from INVITE reception
/// to call establishment, coordinating between dialog state, transaction
/// responses, and media setup.
pub struct CallLifecycleCoordinator {
    transaction_coordinator: TransactionCoordinator,
    media_manager: Arc<MediaManager>,
    dialog_manager: Option<Arc<DialogManager>>,
}

impl CallLifecycleCoordinator {
    /// Create a new call lifecycle coordinator
    pub fn new(
        transaction_coordinator: TransactionCoordinator,
        media_manager: Arc<MediaManager>,
    ) -> Self {
        Self {
            transaction_coordinator,
            media_manager,
            dialog_manager: None,
        }
    }

    /// Set the dialog manager reference (for dialog creation)
    pub fn set_dialog_manager(&mut self, dialog_manager: Arc<DialogManager>) {
        self.dialog_manager = Some(dialog_manager);
    }

    /// Handle incoming INVITE request with complete call flow coordination
    ///
    /// This method manages the complete INVITE processing workflow:
    /// 1. Process the INVITE request
    /// 2. Send 180 Ringing (after brief delay)
    /// 3. Coordinate media setup
    /// 4. Send 200 OK with SDP and create dialog
    pub async fn handle_incoming_invite(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        session_id: &str,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "🎯 Starting complete INVITE call flow coordination"
        );

        // Step 1: Brief delay to simulate processing time (realistic behavior)
        sleep(Duration::from_millis(500)).await;

        // Step 2: Send 180 Ringing response
        self.send_ringing_response(transaction_id, request).await?;

        // Step 3: Simulate call acceptance decision time
        sleep(Duration::from_millis(1500)).await;

        // Step 4: Coordinate call acceptance with dialog creation
        self.coordinate_call_acceptance(transaction_id, request, session_id).await?;

        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "✅ Complete INVITE call flow coordination successful"
        );

        Ok(())
    }

    /// Send 180 Ringing response
    async fn send_ringing_response(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            "📞 Sending 180 Ringing response"
        );

        let ringing_response = self.transaction_coordinator.create_180_ringing_response(request);
        
        self.transaction_coordinator
            .send_provisional_response(transaction_id, ringing_response)
            .await?;

        info!(
            transaction_id = %transaction_id,
            "✅ 180 Ringing response sent successfully"
        );

        Ok(())
    }

    /// Coordinate call acceptance with media setup, 200 OK response, and dialog creation
    ///
    /// This method handles the complete call acceptance workflow:
    /// 1. Extract SDP offer from INVITE
    /// 2. Coordinate with media-core for SDP answer
    /// 3. Send 200 OK with SDP answer
    /// 4. **CRITICAL**: Create dialog from the transaction and response
    pub async fn coordinate_call_acceptance(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        session_id: &str,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "🎵 Coordinating call acceptance with media setup and dialog creation"
        );

        // Step 1: Extract SDP offer from INVITE request
        let offer_sdp = self.extract_sdp_from_request(request)?;
        debug!(
            transaction_id = %transaction_id,
            "📋 Extracted SDP offer from INVITE request"
        );

        // Step 2: Coordinate with media-core to create SDP answer
        let answer_sdp = self.create_media_answer(session_id, &offer_sdp).await?;
        debug!(
            transaction_id = %transaction_id,
            "🎵 Created SDP answer through media-core"
        );

        // Step 3: Send 200 OK response with SDP answer
        let ok_response = self.send_success_response(transaction_id, request, &answer_sdp).await?;

        // Step 4: **CRITICAL FIX**: Create dialog from the transaction and response
        if let Some(dialog_manager) = &self.dialog_manager {
            debug!(
                transaction_id = %transaction_id,
                "🔗 Creating dialog from INVITE transaction and 200 OK response"
            );

            // Extract session ID from the request (from a custom header or generate one)
            let session_id = if let Some(session_header) = request.header(&HeaderName::Other("X-Session-ID".to_string())) {
                // Get the header value as a string
                let session_id_str = match session_header {
                    TypedHeader::Other(_, header_value) => {
                        match header_value {
                            rvoip_sip_core::types::headers::HeaderValue::Raw(bytes) => {
                                std::str::from_utf8(&bytes).unwrap_or("")
                            }
                            _ => ""
                        }
                    }
                    _ => ""
                };
                
                // Try to parse the session ID as a UUID
                match uuid::Uuid::parse_str(session_id_str) {
                    Ok(uuid) => SessionId(uuid),
                    Err(_) => {
                        warn!("Invalid session ID format: {}, generating new one", session_id_str);
                        SessionId::new()
                    }
                }
            } else {
                // Generate a new session ID if not provided
                SessionId::new()
            };

            // Create dialog from the transaction (server side, so is_initiator = false)
            if let Some(dialog_id) = dialog_manager.create_dialog_from_transaction(
                transaction_id,
                request,
                &ok_response,
                false, // Server side - not initiator
            ).await {
                info!(
                    transaction_id = %transaction_id,
                    dialog_id = %dialog_id,
                    "✅ Dialog created successfully for call acceptance"
                );

                // Associate dialog with session
                if let Err(e) = dialog_manager.associate_with_session(&dialog_id, &session_id) {
                    warn!(
                        dialog_id = %dialog_id,
                        session_id = %session_id,
                        error = %e,
                        "Failed to associate dialog with session"
                    );
                } else {
                    info!(
                        dialog_id = %dialog_id,
                        session_id = %session_id,
                        "✅ Dialog associated with session successfully"
                    );
                }
            } else {
                error!(
                    transaction_id = %transaction_id,
                    "❌ Failed to create dialog from INVITE transaction"
                );
            }
        } else {
            warn!(
                transaction_id = %transaction_id,
                "⚠️ No dialog manager available - dialog will not be created"
            );
        }

        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "✅ Call acceptance coordination completed successfully"
        );

        Ok(())
    }

    /// Send 200 OK response with SDP and return the response for dialog creation
    async fn send_success_response(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        sdp: &str,
    ) -> Result<Response> {
        info!(
            transaction_id = %transaction_id,
            "📞 Sending 200 OK response with SDP"
        );

        let ok_response = self.transaction_coordinator.create_200_ok_response(request, Some(sdp))?;
        
        self.transaction_coordinator
            .send_success_response(transaction_id, ok_response.clone())
            .await?;

        info!(
            transaction_id = %transaction_id,
            "✅ 200 OK response with SDP sent successfully"
        );

        // Return the response for dialog creation
        Ok(ok_response)
    }

    /// Coordinate call rejection with error response
    ///
    /// This method handles call rejection by sending appropriate error responses.
    pub async fn coordinate_call_rejection(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        status_code: StatusCode,
        reason: Option<&str>,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            status_code = status_code.as_u16(),
            "📞 Coordinating call rejection"
        );

        let error_response = self.transaction_coordinator.create_error_response(
            request,
            status_code,
            reason,
        );

        self.transaction_coordinator
            .send_error_response(transaction_id, error_response)
            .await?;

        info!(
            transaction_id = %transaction_id,
            status_code = status_code.as_u16(),
            "✅ Call rejection coordinated successfully"
        );

        Ok(())
    }

    /// Handle ACK received for call establishment confirmation
    pub async fn handle_ack_received(
        &self,
        transaction_id: &TransactionKey,
        session_id: &str,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "🎯 ACK received - call establishment confirmed"
        );

        // For now, we'll just log the ACK received
        // In the future, this will coordinate with media-core to confirm media session
        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "✅ Call establishment confirmed - media coordination will be enhanced"
        );

        Ok(())
    }

    /// Handle incoming BYE request with complete call termination coordination
    ///
    /// This method manages the complete BYE processing workflow:
    /// 1. Process the BYE request
    /// 2. Find and terminate the associated dialog
    /// 3. Coordinate media session cleanup
    /// 4. Send 200 OK response
    /// 5. Emit session termination events
    pub async fn handle_incoming_bye(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        session_id: &str,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "🛑 Starting complete BYE call termination coordination"
        );

        // Step 1: Find and terminate the associated dialog
        self.terminate_dialog_for_bye(transaction_id, request, session_id).await?;

        // Step 2: Coordinate media session cleanup
        self.coordinate_media_cleanup(session_id).await?;

        // Step 3: Send 200 OK response to BYE
        self.send_bye_response(transaction_id, request).await?;

        // Step 4: Emit session termination events
        self.emit_session_termination_events(session_id).await?;

        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "✅ Complete BYE call termination coordination successful"
        );

        Ok(())
    }

    /// Find and terminate the dialog associated with the BYE request
    async fn terminate_dialog_for_bye(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        session_id: &str,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "🔗 Finding and terminating dialog for BYE request"
        );

        if let Some(dialog_manager) = &self.dialog_manager {
            // Try to find the dialog for this BYE request
            if let Some(dialog_id) = dialog_manager.find_dialog_for_request(request) {
                info!(
                    transaction_id = %transaction_id,
                    dialog_id = %dialog_id,
                    "🔗 Found dialog for BYE request - terminating"
                );

                // Terminate the dialog
                if let Err(e) = dialog_manager.terminate_dialog(&dialog_id).await {
                    error!(
                        transaction_id = %transaction_id,
                        dialog_id = %dialog_id,
                        error = %e,
                        "❌ Failed to terminate dialog"
                    );
                } else {
                    info!(
                        transaction_id = %transaction_id,
                        dialog_id = %dialog_id,
                        "✅ Dialog terminated successfully"
                    );
                }

                // Associate the BYE transaction with the dialog for cleanup
                dialog_manager.transaction_to_dialog.insert(transaction_id.clone(), dialog_id);
            } else {
                warn!(
                    transaction_id = %transaction_id,
                    session_id = session_id,
                    "⚠️ No dialog found for BYE request - call may have already been terminated"
                );
            }
        } else {
            warn!(
                transaction_id = %transaction_id,
                "⚠️ No dialog manager available for dialog termination"
            );
        }

        Ok(())
    }

    /// Send 200 OK response to BYE request
    async fn send_bye_response(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            "📞 Sending 200 OK response to BYE"
        );

        // **PROPER ARCHITECTURE**: Use transaction-core's helper to create response
        // and transaction-core's send_response method to send it
        let bye_response = rvoip_transaction_core::utils::create_ok_response_for_bye(request);
        
        // Send through transaction-core (not through transaction coordinator)
        self.transaction_coordinator
            .transaction_manager()
            .send_response(transaction_id, bye_response)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send BYE response: {}", e))?;

        info!(
            transaction_id = %transaction_id,
            "✅ 200 OK response to BYE sent successfully"
        );

        Ok(())
    }

    /// Coordinate media session cleanup
    async fn coordinate_media_cleanup(&self, session_id: &str) -> Result<()> {
        info!(
            session_id = session_id,
            "🎵 Coordinating media session cleanup"
        );

        // **REAL IMPLEMENTATION**: Coordinate with media-core to clean up media session
        // First, try to get the media session ID for this session
        if let Ok(session_uuid) = uuid::Uuid::parse_str(session_id) {
            let session_id_obj = SessionId(session_uuid);
            
            if let Some(media_session_id) = self.media_manager.get_media_session(&session_id_obj).await {
                // Stop the media session
                if let Err(e) = self.media_manager.stop_media(&media_session_id, "BYE received".to_string()).await {
                    warn!(
                        session_id = session_id,
                        media_session_id = %media_session_id,
                        error = %e,
                        "Failed to stop media session - continuing with call termination"
                    );
                } else {
                    debug!(
                        session_id = session_id,
                        media_session_id = %media_session_id,
                        "🎵 Media session stopped successfully"
                    );
                }
            } else {
                debug!(session_id = session_id, "No media session found for cleanup");
            }
        } else {
            warn!(session_id = session_id, "Invalid session ID format for media cleanup");
        }
        
        info!(
            session_id = session_id,
            "✅ Media session cleanup coordinated successfully"
        );

        Ok(())
    }

    /// Emit session termination events
    async fn emit_session_termination_events(&self, session_id: &str) -> Result<()> {
        info!(
            session_id = session_id,
            "📡 Emitting session termination events"
        );

        // **REAL IMPLEMENTATION**: Emit proper session termination events
        if let Some(dialog_manager) = &self.dialog_manager {
            // Parse session ID
            if let Ok(session_uuid) = uuid::Uuid::parse_str(session_id) {
                let session_id_obj = SessionId(session_uuid);
                
                // Emit session termination event through the dialog manager's event bus
                dialog_manager.event_bus.publish(crate::events::SessionEvent::Terminated {
                    session_id: session_id_obj,
                    reason: "BYE received".to_string(),
                }).await.map_err(|e| anyhow::anyhow!("Failed to publish termination event: {}", e))?;
                
                debug!(session_id = session_id, "📡 Session termination event published successfully");
            } else {
                warn!(session_id = session_id, "Invalid session ID format for event emission");
            }
        } else {
            warn!("No dialog manager available for event emission");
        }
        
        info!(
            session_id = session_id,
            "✅ Session termination events emitted successfully"
        );

        Ok(())
    }

    /// Extract SDP from INVITE request
    fn extract_sdp_from_request(&self, request: &Request) -> Result<String> {
        // Check if request has SDP content
        if let Some(content_type_header) = request.header(&HeaderName::ContentType) {
            if let TypedHeader::ContentType(content_type) = content_type_header {
                if content_type.to_string().contains("application/sdp") {
                    let sdp = String::from_utf8(request.body().to_vec())
                        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in SDP: {}", e))?;
                    return Ok(sdp);
                }
            }
        }

        // No SDP in request - create a basic offer
        warn!("No SDP found in INVITE request, creating basic audio offer");
        Ok(self.create_basic_audio_offer())
    }

    /// Create basic audio SDP offer
    fn create_basic_audio_offer(&self) -> String {
        // Create a basic audio SDP offer for testing
        "v=0\r\n\
         o=session-core 123456 654321 IN IP4 127.0.0.1\r\n\
         s=-\r\n\
         c=IN IP4 127.0.0.1\r\n\
         t=0 0\r\n\
         m=audio 8000 RTP/AVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n".to_string()
    }

    /// Create media answer through media-core coordination
    async fn create_media_answer(&self, session_id: &str, offer_sdp: &str) -> Result<String> {
        debug!(session_id = session_id, "🎵 Creating SDP answer through media-core coordination");
        
        // Parse session ID
        let session_uuid = uuid::Uuid::parse_str(session_id)
            .map_err(|e| anyhow::anyhow!("Invalid session ID format: {}", e))?;
        let session_id_obj = crate::session::SessionId(session_uuid);
        
        // Parse the offer SDP to extract media requirements
        let (remote_port, remote_codec) = self.parse_offer_sdp(offer_sdp)?;
        
        // Create media configuration based on the offer
        let media_config = crate::media::MediaConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(), // Will be updated with real port
            remote_addr: Some(format!("127.0.0.1:{}", remote_port).parse().unwrap()),
            media_type: crate::media::SessionMediaType::Audio,
            payload_type: remote_codec.to_payload_type(),
            clock_rate: remote_codec.clock_rate(),
            audio_codec: remote_codec,
            direction: crate::media::SessionMediaDirection::SendRecv,
        };
        
        // Create actual media session through media-core with real port allocation
        let media_session_id = self.media_manager.create_media_session(media_config.clone()).await
            .map_err(|e| anyhow::anyhow!("Failed to create media session: {}", e))?;
            
        debug!(
            session_id = session_id,
            media_session_id = %media_session_id,
            "🎵 Created media session through media-core"
        );
        
        // Start the media session
        self.media_manager.start_media(&session_id_obj, &media_session_id).await
            .map_err(|e| anyhow::anyhow!("Failed to start media session: {}", e))?;
            
        info!(
            session_id = session_id,
            media_session_id = %media_session_id,
            "🎵 Started media session - RTP streams should now be active"
        );
        
        // Get the REAL allocated RTP port directly from the media session
        // Use the full media session ID (including "media-" prefix) to query MediaSessionController
        let session_info = self.media_manager.media_controller().get_session_info(media_session_id.as_str()).await
            .ok_or_else(|| anyhow::anyhow!("Media session not found: {}", media_session_id))?;
        
        let actual_local_port = session_info.rtp_port
            .ok_or_else(|| anyhow::anyhow!("No RTP port allocated for session: {}", media_session_id))?;
        
        debug!(
            session_id = session_id,
            local_port = actual_local_port,
            remote_port = remote_port,
            "🎵 RTP streams configured - local_port={}, remote_port={}",
            actual_local_port, remote_port
        );
        
        // Generate SDP answer with actual RTP port from media session
        let answer_sdp = self.create_sdp_answer_with_real_port(actual_local_port, remote_codec);
        
        info!(
            session_id = session_id,
            media_session_id = %media_session_id,
            local_rtp_port = actual_local_port,
            "✅ Created SDP answer with real RTP port through media-core coordination"
        );
        
        Ok(answer_sdp)
    }
    
    /// Parse offer SDP to extract media requirements
    fn parse_offer_sdp(&self, offer_sdp: &str) -> Result<(u16, crate::media::AudioCodecType)> {
        debug!("🔍 Parsing offer SDP to extract media requirements");
        
        let mut remote_port = 6000u16; // Default fallback
        let mut codec = crate::media::AudioCodecType::PCMU; // Default fallback
        
        // Parse SDP lines
        for line in offer_sdp.lines() {
            // Extract media port from m= line
            if line.starts_with("m=audio ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(port) = parts[1].parse::<u16>() {
                        remote_port = port;
                        debug!("🔍 Extracted remote RTP port: {}", remote_port);
                    }
                }
                
                // Extract payload type from m= line
                if parts.len() >= 4 {
                    if let Ok(payload_type) = parts[3].parse::<u8>() {
                        codec = match payload_type {
                            0 => crate::media::AudioCodecType::PCMU,
                            8 => crate::media::AudioCodecType::PCMA,
                            9 => crate::media::AudioCodecType::G722,
                            111 => crate::media::AudioCodecType::Opus,
                            _ => crate::media::AudioCodecType::PCMU, // Default fallback
                        };
                        debug!("🔍 Extracted codec: {:?} (payload_type={})", codec, payload_type);
                    }
                }
            }
        }
        
        Ok((remote_port, codec))
    }
    
    /// Create SDP answer with real RTP port from media session
    fn create_sdp_answer_with_real_port(&self, local_port: u16, codec: crate::media::AudioCodecType) -> String {
        let payload_type = codec.to_payload_type();
        let clock_rate = codec.clock_rate();
        let codec_name = match codec {
            crate::media::AudioCodecType::PCMU => "PCMU",
            crate::media::AudioCodecType::PCMA => "PCMA", 
            crate::media::AudioCodecType::G722 => "G722",
            crate::media::AudioCodecType::Opus => "opus",
        };
        
        format!(
            "v=0\r\n\
             o=session-core {} {} IN IP4 127.0.0.1\r\n\
             s=-\r\n\
             c=IN IP4 127.0.0.1\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP {}\r\n\
             a=rtpmap:{} {}/{}\r\n\
             a=sendrecv\r\n",
            chrono::Utc::now().timestamp(),
            chrono::Utc::now().timestamp() + 1,
            local_port,
            payload_type,
            payload_type,
            codec_name,
            clock_rate
        )
    }
} 