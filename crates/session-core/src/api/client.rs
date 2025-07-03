//! SIP Client API
//!
//! This module provides the SipClient trait for non-session SIP operations
//! like REGISTER, OPTIONS, MESSAGE, and SUBSCRIBE.

use async_trait::async_trait;
use std::time::Duration;
use std::collections::HashMap;
use crate::errors::{Result, SessionError};

/// Handle for tracking registration state
#[derive(Debug, Clone)]
pub struct RegistrationHandle {
    /// Transaction ID for the REGISTER
    pub transaction_id: String,
    /// Registration expiration in seconds
    pub expires: u32,
    /// Contact URI that was registered
    pub contact_uri: String,
    /// Registrar URI
    pub registrar_uri: String,
}

/// Response from a SIP request
#[derive(Debug, Clone)]
pub struct SipResponse {
    /// Status code (e.g., 200, 404, 401)
    pub status_code: u16,
    /// Reason phrase
    pub reason_phrase: String,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Response body
    pub body: Option<String>,
}

/// Handle for managing subscriptions
#[derive(Debug, Clone)]
pub struct SubscriptionHandle {
    /// Subscription dialog ID
    pub dialog_id: String,
    /// Event type
    pub event_type: String,
    /// Expiration time
    pub expires_at: std::time::Instant,
}

/// Trait for non-session SIP operations
/// 
/// This trait provides methods for SIP operations that don't create or manage
/// sessions/dialogs, such as registration, instant messaging, and presence.
/// 
/// # Example
/// 
/// ```rust,no_run
/// use rvoip_session_core::api::*;
/// 
/// # async fn example() -> Result<()> {
/// let coordinator = SessionManagerBuilder::new()
///     .with_sip_port(5060)
///     .with_local_address("sip:alice@192.168.1.100")
///     .enable_sip_client()
///     .build()
///     .await?;
/// 
/// // Register with a SIP server
/// let registration = coordinator.register(
///     "sip:registrar.example.com",
///     "sip:alice@example.com",
///     "sip:alice@192.168.1.100:5060",
///     3600
/// ).await?;
/// 
/// println!("Registered successfully: {}", registration.transaction_id);
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait SipClient: Send + Sync {
    /// Send a REGISTER request
    /// 
    /// Registers a SIP endpoint with a registrar server. This is used to
    /// tell the server where to route incoming calls for a particular AOR.
    /// 
    /// # Arguments
    /// * `registrar_uri` - The registrar server URI (e.g., "sip:registrar.example.com")
    /// * `from_uri` - The AOR being registered (e.g., "sip:alice@example.com")
    /// * `contact_uri` - Where to reach this endpoint (e.g., "sip:alice@192.168.1.100:5060")
    /// * `expires` - Registration duration in seconds (0 to unregister)
    /// 
    /// # Returns
    /// A handle tracking the registration or an error
    /// 
    /// # Example
    /// 
    /// ```rust,no_run
    /// # use rvoip_session_core::api::*;
    /// # async fn example(coordinator: Arc<SessionCoordinator>) -> Result<()> {
    /// // Register for 1 hour
    /// let reg = coordinator.register(
    ///     "sip:registrar.example.com",
    ///     "sip:alice@example.com",
    ///     "sip:alice@192.168.1.100:5060",
    ///     3600
    /// ).await?;
    /// 
    /// // Later, unregister
    /// coordinator.register(
    ///     "sip:registrar.example.com",
    ///     "sip:alice@example.com",
    ///     "sip:alice@192.168.1.100:5060",
    ///     0  // expires=0 means unregister
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<RegistrationHandle>;
    
    /// Send an OPTIONS request (keepalive/capability query)
    /// 
    /// OPTIONS is used to query the capabilities of a SIP endpoint or
    /// to keep NAT mappings alive.
    /// 
    /// # Arguments
    /// * `target_uri` - The target to query
    /// 
    /// # Returns
    /// The OPTIONS response containing supported methods and capabilities
    async fn send_options(&self, target_uri: &str) -> Result<SipResponse>;
    
    /// Send a MESSAGE request (instant message)
    /// 
    /// Sends a SIP instant message to another endpoint without establishing
    /// a session.
    /// 
    /// # Arguments
    /// * `to_uri` - Message recipient
    /// * `message` - Message content
    /// * `content_type` - MIME type (defaults to "text/plain")
    /// 
    /// # Returns
    /// The MESSAGE response indicating delivery status
    async fn send_message(
        &self,
        to_uri: &str,
        message: &str,
        content_type: Option<&str>,
    ) -> Result<SipResponse>;
    
    /// Send a SUBSCRIBE request
    /// 
    /// Subscribes to events from another endpoint, such as presence updates
    /// or dialog state changes.
    /// 
    /// # Arguments
    /// * `target_uri` - What to subscribe to
    /// * `event_type` - Event package (e.g., "presence", "dialog", "message-summary")
    /// * `expires` - Subscription duration in seconds
    /// 
    /// # Returns
    /// Subscription handle for managing the subscription
    async fn subscribe(
        &self,
        target_uri: &str,
        event_type: &str,
        expires: u32,
    ) -> Result<SubscriptionHandle>;
    
    /// Send a raw SIP request (advanced use)
    /// 
    /// For advanced users who need complete control over the SIP request.
    /// 
    /// # Arguments
    /// * `request` - Complete SIP request to send
    /// * `timeout` - Response timeout
    /// 
    /// # Returns
    /// The SIP response
    async fn send_raw_request(
        &self,
        request: rvoip_sip_core::Request,
        timeout: Duration,
    ) -> Result<SipResponse>;
} 