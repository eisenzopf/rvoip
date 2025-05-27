//! Transaction Coordination Interface
//!
//! This module provides the interface for dialog manager to coordinate with
//! transaction-core for sending SIP responses. It maintains proper separation
//! of concerns where dialog manager makes application decisions and coordinates
//! with transaction-core for SIP protocol handling.

use std::sync::Arc;
use anyhow::Result;
use tracing::{debug, error, info};

use rvoip_sip_core::prelude::*;
use rvoip_transaction_core::{TransactionManager, TransactionKey};
use uuid;

/// Transaction coordination interface for dialog manager
///
/// This struct provides methods for the dialog manager to coordinate with
/// transaction-core for sending SIP responses while maintaining proper
/// architectural separation.
#[derive(Debug, Clone)]
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

    /// Send a provisional response (1xx) through transaction-core
    ///
    /// This method coordinates with transaction-core to send provisional responses
    /// like 180 Ringing while maintaining proper SIP protocol handling.
    pub async fn send_provisional_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> Result<()> {
        debug!(
            transaction_id = %transaction_id,
            status_code = response.status_code(),
            "Coordinating provisional response with transaction-core"
        );

        // Coordinate with transaction-core to send the response
        match self.transaction_manager.send_response(transaction_id, response.clone()).await {
            Ok(()) => {
                info!(
                    transaction_id = %transaction_id,
                    status_code = response.status_code(),
                    "✅ Provisional response coordinated successfully"
                );
                Ok(())
            }
            Err(e) => {
                error!(
                    transaction_id = %transaction_id,
                    status_code = response.status_code(),
                    error = %e,
                    "❌ Failed to coordinate provisional response"
                );
                Err(anyhow::anyhow!("Failed to send provisional response: {}", e))
            }
        }
    }

    /// Send a success response (2xx) through transaction-core
    ///
    /// This method coordinates with transaction-core to send success responses
    /// like 200 OK with SDP for call establishment.
    pub async fn send_success_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> Result<()> {
        debug!(
            transaction_id = %transaction_id,
            status_code = response.status_code(),
            "Coordinating success response with transaction-core"
        );

        // Coordinate with transaction-core to send the response
        match self.transaction_manager.send_response(transaction_id, response.clone()).await {
            Ok(()) => {
                info!(
                    transaction_id = %transaction_id,
                    status_code = response.status_code(),
                    "✅ Success response coordinated successfully"
                );
                Ok(())
            }
            Err(e) => {
                error!(
                    transaction_id = %transaction_id,
                    status_code = response.status_code(),
                    error = %e,
                    "❌ Failed to coordinate success response"
                );
                Err(anyhow::anyhow!("Failed to send success response: {}", e))
            }
        }
    }

    /// Send an error response (4xx/5xx/6xx) through transaction-core
    ///
    /// This method coordinates with transaction-core to send error responses
    /// for call rejection or server errors.
    pub async fn send_error_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> Result<()> {
        debug!(
            transaction_id = %transaction_id,
            status_code = response.status_code(),
            "Coordinating error response with transaction-core"
        );

        // Coordinate with transaction-core to send the response
        match self.transaction_manager.send_response(transaction_id, response.clone()).await {
            Ok(()) => {
                info!(
                    transaction_id = %transaction_id,
                    status_code = response.status_code(),
                    "✅ Error response coordinated successfully"
                );
                Ok(())
            }
            Err(e) => {
                error!(
                    transaction_id = %transaction_id,
                    status_code = response.status_code(),
                    error = %e,
                    "❌ Failed to coordinate error response"
                );
                Err(anyhow::anyhow!("Failed to send error response: {}", e))
            }
        }
    }

    /// Get reference to the transaction manager
    ///
    /// This provides access to the underlying transaction manager for
    /// advanced coordination scenarios.
    pub fn transaction_manager(&self) -> &Arc<TransactionManager> {
        &self.transaction_manager
    }
}

/// Helper functions for creating SIP responses using transaction-core utilities
impl TransactionCoordinator {
    /// Create a 180 Ringing response from the original request
    ///
    /// Uses transaction-core's helper function that properly handles To-tags
    /// and other SIP requirements for dialog establishment.
    pub fn create_180_ringing_response(&self, request: &Request) -> Response {
        // Use transaction-core's helper that properly handles To-tags
        rvoip_transaction_core::utils::create_ringing_response_with_tag(request)
    }

    /// Create a 200 OK response with SDP from the original request
    ///
    /// Uses transaction-core's helper function that properly handles To-tags,
    /// Contact headers, and other SIP requirements for dialog establishment.
    pub fn create_200_ok_response(&self, request: &Request, sdp: &str) -> Response {
        // Use transaction-core's helper that properly handles To-tags and Contact
        let mut response = rvoip_transaction_core::utils::create_ok_response_with_dialog_info(
            request,
            "server",      // contact_user
            "127.0.0.1",   // contact_host - TODO: make this configurable
            Some(5060),    // contact_port - TODO: make this configurable
        );
        
        // Add SDP content using proper ContentType creation
        use rvoip_sip_core::parser::headers::content_type::ContentTypeValue;
        use std::collections::HashMap;
        
        let content_type = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: HashMap::new(),
        });
        
        response.headers.push(TypedHeader::ContentType(content_type));
        response.headers.retain(|h| !matches!(h, TypedHeader::ContentLength(_)));
        response.headers.push(TypedHeader::ContentLength(
            ContentLength::new(sdp.len() as u32)
        ));
        response.body = bytes::Bytes::from(sdp.as_bytes().to_vec());
        
        response
    }

    /// Create an error response from the original request
    ///
    /// Uses transaction-core's helper function for proper response creation.
    pub fn create_error_response(&self, request: &Request, status_code: StatusCode, reason: Option<&str>) -> Response {
        // Use transaction-core's basic response helper
        rvoip_transaction_core::utils::create_response(request, status_code)
    }
} 