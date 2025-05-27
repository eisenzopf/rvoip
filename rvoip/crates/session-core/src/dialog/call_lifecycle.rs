//! Call Lifecycle Coordination
//!
//! This module manages the complete call lifecycle coordination between
//! dialog manager, transaction-core, and media-core. It handles the
//! application logic for call acceptance, rejection, and state management.

use std::sync::Arc;
use anyhow::Result;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{HeaderName, TypedHeader};
use rvoip_transaction_core::TransactionKey;
use crate::dialog::transaction_coordination::TransactionCoordinator;
use crate::media::MediaManager;

/// Call lifecycle coordinator
///
/// This struct manages the complete call lifecycle from INVITE reception
/// to call establishment, coordinating between dialog state, transaction
/// responses, and media setup.
pub struct CallLifecycleCoordinator {
    transaction_coordinator: TransactionCoordinator,
    media_manager: Arc<MediaManager>,
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
        }
    }

    /// Handle incoming INVITE request with complete call flow coordination
    ///
    /// This method manages the complete INVITE processing workflow:
    /// 1. Process the INVITE request
    /// 2. Send 180 Ringing (after brief delay)
    /// 3. Coordinate media setup
    /// 4. Send 200 OK with SDP
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

        // Step 4: Coordinate call acceptance
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

    /// Coordinate call acceptance with media setup and 200 OK response
    ///
    /// This method handles the complete call acceptance workflow:
    /// 1. Extract SDP offer from INVITE
    /// 2. Coordinate with media-core for SDP answer
    /// 3. Send 200 OK with SDP answer
    pub async fn coordinate_call_acceptance(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        session_id: &str,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "🎵 Coordinating call acceptance with media setup"
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
        self.send_success_response(transaction_id, request, &answer_sdp).await?;

        info!(
            transaction_id = %transaction_id,
            session_id = session_id,
            "✅ Call acceptance coordination completed successfully"
        );

        Ok(())
    }

    /// Send 200 OK response with SDP
    async fn send_success_response(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
        sdp: &str,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            "📞 Sending 200 OK response with SDP"
        );

        let ok_response = self.transaction_coordinator.create_200_ok_response(request, sdp);
        
        self.transaction_coordinator
            .send_success_response(transaction_id, ok_response)
            .await?;

        info!(
            transaction_id = %transaction_id,
            "✅ 200 OK response with SDP sent successfully"
        );

        Ok(())
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
    async fn create_media_answer(&self, session_id: &str, _offer_sdp: &str) -> Result<String> {
        // For now, we'll create a basic SDP answer
        // In the future, this will coordinate with media-core to:
        // 1. Parse the offer SDP
        // 2. Create a media session with appropriate configuration
        // 3. Generate an SDP answer based on media-core capabilities
        
        debug!(session_id = session_id, "🎵 Creating SDP answer (basic implementation)");
        
        // Return a basic SDP answer for now
        Ok(self.create_basic_audio_answer())
    }

    /// Create basic audio SDP answer
    fn create_basic_audio_answer(&self) -> String {
        // Create a basic audio SDP answer for testing
        "v=0\r\n\
         o=session-core 654321 123456 IN IP4 127.0.0.1\r\n\
         s=-\r\n\
         c=IN IP4 127.0.0.1\r\n\
         t=0 0\r\n\
         m=audio 8001 RTP/AVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n".to_string()
    }
} 