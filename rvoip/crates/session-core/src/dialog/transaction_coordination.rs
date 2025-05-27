//! Transaction Coordination
//!
//! This module provides the interface between dialog manager and transaction-core,
//! allowing the dialog manager to coordinate SIP responses through transaction-core
//! while maintaining proper architectural separation.

use std::sync::Arc;
use anyhow::Result;
use tracing::{debug, info, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{HeaderName, TypedHeader};
use rvoip_sip_core::builder::ContentLengthBuilderExt;
use rvoip_transaction_core::{TransactionManager, TransactionKey};
use bytes::Bytes;

/// Transaction coordinator
///
/// This struct provides the interface between dialog manager and transaction-core,
/// allowing dialog manager to coordinate SIP responses without directly handling
/// SIP protocol details.
pub struct TransactionCoordinator {
    transaction_manager: Arc<TransactionManager>,
}

impl TransactionCoordinator {
    /// Create a new transaction coordinator
    pub fn new(transaction_manager: Arc<TransactionManager>) -> Self {
        Self {
            transaction_manager,
        }
    }

    /// Get access to the transaction manager
    pub fn transaction_manager(&self) -> &Arc<TransactionManager> {
        &self.transaction_manager
    }

    /// Send a provisional response (e.g., 180 Ringing)
    pub async fn send_provisional_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            status_code = response.status().as_u16(),
            "📞 Sending provisional response through transaction-core"
        );

        self.transaction_manager
            .send_response(transaction_id, response)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send provisional response: {}", e))?;

        info!(
            transaction_id = %transaction_id,
            "✅ Provisional response sent successfully"
        );

        Ok(())
    }

    /// Send a success response (e.g., 200 OK) and create dialog
    pub async fn send_success_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            status_code = response.status().as_u16(),
            "📞 Sending success response through transaction-core"
        );

        self.transaction_manager
            .send_response(transaction_id, response)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send success response: {}", e))?;

        info!(
            transaction_id = %transaction_id,
            "✅ Success response sent successfully"
        );

        Ok(())
    }

    /// Send an error response (e.g., 486 Busy Here)
    pub async fn send_error_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> Result<()> {
        info!(
            transaction_id = %transaction_id,
            status_code = response.status().as_u16(),
            "📞 Sending error response through transaction-core"
        );

        self.transaction_manager
            .send_response(transaction_id, response)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send error response: {}", e))?;

        info!(
            transaction_id = %transaction_id,
            "✅ Error response sent successfully"
        );

        Ok(())
    }

    /// Create 180 Ringing response using transaction-core helpers
    pub fn create_180_ringing_response(&self, request: &Request) -> Response {
        debug!("Creating 180 Ringing response using transaction-core helpers");
        
        // **PROPER ARCHITECTURE**: Use transaction-core's helper function
        rvoip_transaction_core::utils::create_ringing_response_with_tag(request)
    }

    /// Create 200 OK response using transaction-core helpers
    pub fn create_200_ok_response(&self, request: &Request, sdp: Option<&str>) -> Result<Response> {
        debug!("Creating 200 OK response with SDP using transaction-core helpers");
        
        // **PROPER ARCHITECTURE**: Use transaction-core's helper function
        // Extract local contact information from the request's destination
        let contact_host = "127.0.0.1"; // In production, this should be the actual local IP
        let contact_port = 5060; // In production, this should be the actual local port
        let contact_user = "session-core"; // In production, this could be extracted from config
        
        let mut response = rvoip_transaction_core::utils::create_ok_response_with_dialog_info(
            request,
            &contact_user,
            &contact_host,
            Some(contact_port),
        );
        
        // Add Content-Type header for SDP
        if let Some(sdp) = sdp {
            use rvoip_sip_core::parser::headers::content_type::ContentTypeValue;
            use std::collections::HashMap;
            
            let ct = rvoip_sip_core::types::content_type::ContentType::new(ContentTypeValue {
                m_type: "application".to_string(),
                m_subtype: "sdp".to_string(),
                parameters: HashMap::new(),
            });
            response.headers.push(TypedHeader::ContentType(ct));
            response.body = Bytes::from(sdp.as_bytes().to_vec());
        }
        
        Ok(response)
    }

    /// Create error response using transaction-core helpers
    pub fn create_error_response(
        &self,
        request: &Request,
        status_code: StatusCode,
        reason: Option<&str>,
    ) -> Response {
        debug!(
            status_code = status_code.as_u16(),
            "Creating error response using transaction-core helpers"
        );
        
        // **PROPER ARCHITECTURE**: Use sip-core's response builder
        // (transaction-core doesn't have specific helpers for all error codes)
        let mut builder = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            request,
            status_code,
            reason,
        );
        
        // Add Content-Length: 0 for error responses
        builder = builder.content_length(0);
        
        builder.build()
    }
} 