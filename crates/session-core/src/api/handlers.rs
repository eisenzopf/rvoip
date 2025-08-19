//! Event Handlers for Session Management
//!
//! This module provides the `CallHandler` trait and pre-built implementations for
//! handling incoming calls and call lifecycle events in your VoIP applications.
//! 
//! # Overview
//! 
//! Call handlers are the primary mechanism for customizing how your application
//! responds to incoming calls. They support two decision patterns:
//! 
//! 1. **Immediate Decision**: Synchronously decide to accept/reject/forward
//! 2. **Deferred Decision**: Defer for asynchronous processing with external systems
//! 
//! # The CallHandler Trait
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use async_trait::async_trait;
//! 
//! #[async_trait]
//! pub trait CallHandler: Send + Sync + std::fmt::Debug {
//!     /// Decide what to do with an incoming call
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision;
//!
//!     /// Handle when a call ends
//!     async fn on_call_ended(&self, call: CallSession, reason: &str);
//!     
//!     /// Handle when a call is established (optional)
//!     async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
//!         // Default implementation
//!     }
//! }
//! ```
//! 
//! # Built-in Handlers
//! 
//! ## AutoAnswerHandler
//! 
//! Automatically accepts all incoming calls. Useful for testing or simple scenarios.
//! 
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder, SessionControl, Result};
//! use rvoip_session_core::handlers::AutoAnswerHandler;
//! use std::sync::Arc;
//! 
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_handler(Arc::new(AutoAnswerHandler))
//!         .build()
//!         .await?;
//!     
//!     SessionControl::start(&coordinator).await?;
//!     
//!     // All incoming calls will be automatically accepted
//!     Ok(())
//! }
//! ```
//! 
//! ## QueueHandler
//! 
//! Queues incoming calls for later processing. Perfect for call centers or when
//! you need to process calls asynchronously.
//! 
//! ```rust
//! use rvoip_session_core::{SessionManagerBuilder, SessionControl, MediaControl, Result};
//! use rvoip_session_core::handlers::QueueHandler;
//! use tokio::sync::mpsc;
//! use std::sync::Arc;
//! 
//! async fn setup_queue_handler() -> Result<()> {
//!     // Create queue handler with max 100 calls
//!     let queue_handler = Arc::new(QueueHandler::new(100));
//!     
//!     // Set up notification channel
//!     let (tx, mut rx) = mpsc::unbounded_channel();
//!     queue_handler.set_notify_channel(tx);
//!     
//!     let coordinator = SessionManagerBuilder::new()
//!         .with_sip_port(5060)
//!         .with_handler(queue_handler.clone())
//!         .build()
//!         .await?;
//!     
//!     SessionControl::start(&coordinator).await?;
//!     
//!     // Process queued calls in background
//!     let coord_clone = coordinator.clone();
//!     tokio::spawn(async move {
//!         while let Some(call) = rx.recv().await {
//!             // Perform async operations (database lookup, etc.)
//!             // In a real application, you would check permissions here
//!             let allowed = !call.from.contains("blocked");
//!             
//!             if allowed {
//!                 if let Some(ref offer) = call.sdp {
//!                     let sdp = MediaControl::generate_sdp_answer(
//!                         &coord_clone,
//!                         &call.id,
//!                         offer
//!                     ).await.unwrap();
//!                     
//!                     SessionControl::accept_incoming_call(
//!                         &coord_clone,
//!                         &call,
//!                         Some(sdp)
//!                     ).await.unwrap();
//!                 }
//!             } else {
//!                 SessionControl::reject_incoming_call(
//!                     &coord_clone,
//!                     &call,
//!                     "Not authorized"
//!                 ).await.unwrap();
//!             }
//!         }
//!     });
//!     
//!     Ok(())
//! }
//! ```
//! 
//! ## RoutingHandler
//! 
//! Routes calls based on destination patterns. Supports prefix matching and
//! default actions for unmatched calls.
//! 
//! ```rust
//! use rvoip_session_core::CallDecision;
//! use rvoip_session_core::handlers::RoutingHandler;
//! 
//! fn create_pbx_router() -> RoutingHandler {
//!     let mut router = RoutingHandler::new();
//!     
//!     // Department routing
//!     router.add_route("sip:support@", "sip:queue@support.internal");
//!     router.add_route("sip:sales@", "sip:queue@sales.internal");
//!     router.add_route("sip:billing@", "sip:queue@billing.internal");
//!     
//!     // Geographic routing
//!     router.add_route("sip:+1212", "sip:nyc@gateway.com");
//!     router.add_route("sip:+1415", "sip:sf@gateway.com");
//!     
//!     // Toll-free routing
//!     router.add_route("sip:+1800", "sip:tollfree@special.gateway.com");
//!     router.add_route("sip:+1888", "sip:tollfree@special.gateway.com");
//!     
//!     // Set default for unknown destinations
//!     router.set_default_action(
//!         CallDecision::Forward("sip:operator@default.internal".to_string())
//!     );
//!     
//!     router
//! }
//! ```
//! 
//! ## CompositeHandler
//! 
//! Chains multiple handlers together. Handlers are tried in order until one
//! makes a decision (doesn't defer).
//! 
//! ```rust
//! use rvoip_session_core::handlers::{CompositeHandler, QueueHandler, RoutingHandler, CallHandler};
//! use rvoip_session_core::{CallDecision, IncomingCall, CallSession};
//! use std::sync::Arc;
//! use std::time::Duration;
//! 
//! // Example of building advanced handler
//! // In a real application, you would implement the other handlers
//! fn create_advanced_handler() -> Arc<CompositeHandler> {
//!     let composite = CompositeHandler::new()
//!         // Add a simple routing handler
//!         .add_handler(Arc::new(create_pbx_router()))
//!         // Finally, queue any remaining calls
//!         .add_handler(Arc::new(QueueHandler::new(50)));
//!     
//!     Arc::new(composite)
//! }
//! 
//! fn create_pbx_router() -> RoutingHandler {
//!     let mut router = RoutingHandler::new();
//!     router.add_route("sip:support@", "sip:queue@support.internal");
//!     router
//! }
//! ```
//! 
//! # Custom Handler Examples
//! 
//! ## Example: Business Hours Handler
//! 
//! ```rust
//! use rvoip_session_core::{CallHandler, IncomingCall, CallDecision, CallSession};
//! use chrono::{Local, Timelike, Datelike, Weekday};
//! 
//! #[derive(Debug)]
//! struct BusinessHoursHandler {
//!     start_hour: u32,
//!     end_hour: u32,
//!     timezone: String,
//! }
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for BusinessHoursHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         let now = Local::now();
//!         let hour = now.hour();
//!         let weekday = now.weekday();
//!         
//!         // Check if weekend
//!         if weekday == Weekday::Sat || weekday == Weekday::Sun {
//!             return CallDecision::Forward("sip:weekend@voicemail.com".to_string());
//!         }
//!         
//!         // Check business hours
//!         if hour >= self.start_hour && hour < self.end_hour {
//!             // During business hours, let next handler decide
//!             CallDecision::Defer
//!         } else {
//!             // After hours, send to voicemail
//!             CallDecision::Forward("sip:afterhours@voicemail.com".to_string())
//!         }
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         // Log for business analytics
//!         println!(
//!             "Call {} ended after {} seconds: {}",
//!             call.id(),
//!             call.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0),
//!             reason
//!         );
//!     }
//! }
//! ```
//! 
//! ## Example: Database-Backed VIP Handler
//! 
//! ```rust
//! use rvoip_session_core::{CallHandler, IncomingCall, CallDecision, CallSession, SessionCoordinator, SessionControl};
//! use std::sync::Arc;
//! use tokio::sync::RwLock;
//! use std::time::Duration;
//! 
//! // Example database stub - in real code this would be your actual database
//! #[derive(Debug)]
//! struct Database;
//! 
//! impl Database {
//!     async fn get_caller_info(&self, from: &str) -> Result<CallerInfo, Box<dyn std::error::Error>> {
//!         Ok(CallerInfo { is_vip: from.contains("vip") })
//!     }
//!     
//!     async fn record_vip_call(&self, call: &CallSession, local_sdp: &Option<String>, remote_sdp: &Option<String>) {
//!         // Record call details
//!     }
//!     
//!     async fn update_vip_call_stats(&self, session_id: &str, reason: &str) {
//!         // Update statistics
//!     }
//! }
//! 
//! struct CallerInfo {
//!     is_vip: bool,
//! }
//! 
//! #[derive(Debug)]
//! struct VipHandler {
//!     db: Arc<Database>,
//!     vip_queue: Arc<RwLock<Vec<IncomingCall>>>,
//! }
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for VipHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // Always defer for async database lookup
//!         self.vip_queue.write().await.push(call);
//!         CallDecision::Defer
//!     }
//!     
//!     async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
//!         // Record high-priority call establishment
//!         self.db.record_vip_call(&call, &local_sdp, &remote_sdp).await;
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         // Update VIP call statistics
//!         self.db.update_vip_call_stats(&call.id().to_string(), reason).await;
//!     }
//! }
//! 
//! // Background processor for VIP calls
//! async fn process_vip_calls(
//!     handler: Arc<VipHandler>,
//!     coordinator: Arc<SessionCoordinator>
//! ) {
//!     loop {
//!         let calls = handler.vip_queue.write().await.drain(..).collect::<Vec<_>>();
//!         
//!         for call in calls {
//!             // Check VIP status in database
//!             let caller_info = handler.db.get_caller_info(&call.from).await;
//!             
//!             if let Ok(info) = caller_info {
//!                 if info.is_vip {
//!                     // VIP gets high-quality codec and priority routing
//!                     // In real code, you would generate proper HD audio SDP
//!                     let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 96\r\na=rtpmap:96 opus/48000/2\r\n".to_string();
//!                     SessionControl::accept_incoming_call(
//!                         &coordinator,
//!                         &call,
//!                         Some(sdp)
//!                     ).await.unwrap();
//!                 } else {
//!                     // Non-VIP goes to regular queue
//!                     SessionControl::reject_incoming_call(
//!                         &coordinator,
//!                         &call,
//!                         "Please use regular support line"
//!                     ).await.unwrap();
//!                 }
//!             }
//!         }
//!         
//!         tokio::time::sleep(Duration::from_millis(100)).await;
//!     }
//! }
//! ```
//! 
//! ## Example: Geographic Load Balancer
//! 
//! ```rust
//! use rvoip_session_core::{CallHandler, IncomingCall, CallDecision, CallSession};
//! use std::collections::HashMap;
//! use std::sync::{Arc, Mutex};
//! 
//! #[derive(Debug)]
//! struct GeoLoadBalancer {
//!     regions: HashMap<String, Vec<String>>,
//!     current_index: Arc<Mutex<HashMap<String, usize>>>,
//! }
//! 
//! impl GeoLoadBalancer {
//!     fn new() -> Self {
//!         let mut regions = HashMap::new();
//!         regions.insert("US-East".to_string(), vec![
//!             "sip:server1@east.example.com".to_string(),
//!             "sip:server2@east.example.com".to_string(),
//!         ]);
//!         regions.insert("US-West".to_string(), vec![
//!             "sip:server1@west.example.com".to_string(),
//!             "sip:server2@west.example.com".to_string(),
//!         ]);
//!         
//!         Self {
//!             regions,
//!             current_index: Arc::new(Mutex::new(HashMap::new())),
//!         }
//!     }
//!     
//!     fn get_region_from_number(&self, number: &str) -> &str {
//!         if number.starts_with("sip:+1212") || number.starts_with("sip:+1646") {
//!             "US-East"
//!         } else if number.starts_with("sip:+1415") || number.starts_with("sip:+1650") {
//!             "US-West"
//!         } else {
//!             "US-East" // default
//!         }
//!     }
//! }
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for GeoLoadBalancer {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         let region = self.get_region_from_number(&call.from);
//!         
//!         if let Some(servers) = self.regions.get(region) {
//!             // Round-robin within region
//!             let mut indices = self.current_index.lock().unwrap();
//!             let index = indices.entry(region.to_string()).or_insert(0);
//!             let server = &servers[*index % servers.len()];
//!             *index = (*index + 1) % servers.len();
//!             
//!             CallDecision::Forward(server.clone())
//!         } else {
//!             CallDecision::Reject("No servers available in region".to_string())
//!         }
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         // Could track server performance metrics here
//!     }
//! }
//! ```
//! 
//! # Best Practices
//! 
//! 1. **Use Defer for Async Operations**: Don't block in `on_incoming_call`
//! 2. **Chain Handlers**: Use CompositeHandler for complex logic
//! 3. **Handle Errors Gracefully**: Always have a fallback decision
//! 4. **Log Important Events**: Use the callbacks for monitoring
//! 5. **Clean Up Resources**: Use `on_call_ended` for cleanup
//! 
//! # Thread Safety
//! 
//! All handlers must be `Send + Sync` as they're called from multiple async tasks.
//! Use `Arc<Mutex<>>` or `Arc<RwLock<>>` for shared mutable state.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::api::types::{IncomingCall, CallSession, CallDecision, SessionId, CallState};
use crate::errors::Result;
use crate::manager::events::{MediaQualityAlertLevel, MediaFlowDirection, WarningCategory};

/// Main trait for handling call events
#[async_trait]
pub trait CallHandler: Send + Sync + std::fmt::Debug {
    /// Handle an incoming call and decide what to do with it
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision;

    /// Handle when a call ends
    async fn on_call_ended(&self, call: CallSession, reason: &str);
    
    /// Handle when a call is established (200 OK received/sent)
    /// This is called when the call is fully established and media can flow
    /// 
    /// # Arguments
    /// * `call` - The established call session
    /// * `local_sdp` - The local SDP (offer or answer)
    /// * `remote_sdp` - The remote SDP (answer or offer)
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        // Default implementation does nothing
        tracing::info!("Call {} established", call.id());
        if let Some(remote) = remote_sdp {
            tracing::debug!("Remote SDP: {}", remote);
        }
    }

    // === New optional methods with default implementations ===
    
    /// Called on any session state change
    /// 
    /// This method is called for all state transitions, allowing fine-grained
    /// monitoring of call progress. The default implementation does nothing.
    /// 
    /// # Arguments
    /// * `session_id` - The session that changed state
    /// * `old_state` - The previous state
    /// * `new_state` - The new state
    /// * `reason` - Optional reason for the state change
    async fn on_call_state_changed(
        &self, 
        session_id: &SessionId, 
        old_state: &CallState, 
        new_state: &CallState, 
        reason: Option<&str>
    ) {
        // Default: do nothing - maintains backward compatibility
        tracing::debug!(
            "Call {} state changed from {:?} to {:?} (reason: {:?})",
            session_id, old_state, new_state, reason
        );
    }
    
    /// Called when media quality metrics are available
    /// 
    /// This method provides real-time quality metrics for active calls.
    /// The alert level is calculated based on the MOS score.
    /// 
    /// # Arguments
    /// * `session_id` - The session with quality metrics
    /// * `mos_score` - Mean Opinion Score (1.0-5.0)
    /// * `packet_loss` - Packet loss percentage
    /// * `alert_level` - Calculated quality alert level
    async fn on_media_quality(
        &self, 
        session_id: &SessionId, 
        mos_score: f32, 
        packet_loss: f32, 
        alert_level: MediaQualityAlertLevel
    ) {
        // Default: do nothing
        if matches!(alert_level, MediaQualityAlertLevel::Poor | MediaQualityAlertLevel::Critical) {
            tracing::warn!(
                "Call {} has poor quality: MOS={}, packet_loss={}%",
                session_id, mos_score, packet_loss
            );
        }
    }
    
    /// Called when DTMF digit is received
    /// 
    /// This method is called for each DTMF digit received during a call.
    /// 
    /// # Arguments
    /// * `session_id` - The session that received DTMF
    /// * `digit` - The DTMF digit (0-9, *, #, A-D)
    /// * `duration_ms` - Duration of the digit press in milliseconds
    async fn on_dtmf(&self, session_id: &SessionId, digit: char, duration_ms: u32) {
        // Default: do nothing
        tracing::debug!(
            "Call {} received DTMF digit '{}' (duration: {}ms)",
            session_id, digit, duration_ms
        );
    }
    
    /// Called when media starts/stops flowing
    /// 
    /// This method notifies when media flow changes for a session.
    /// 
    /// # Arguments
    /// * `session_id` - The session with media flow change
    /// * `direction` - Direction of media flow (Send/Receive/Both)
    /// * `active` - Whether media is now flowing or stopped
    /// * `codec` - The codec being used
    async fn on_media_flow(
        &self, 
        session_id: &SessionId, 
        direction: MediaFlowDirection, 
        active: bool, 
        codec: &str
    ) {
        // Default: do nothing
        tracing::debug!(
            "Call {} media flow {:?} {} (codec: {})",
            session_id, 
            direction,
            if active { "started" } else { "stopped" },
            codec
        );
    }
    
    /// Called on non-fatal warnings
    /// 
    /// This method is called for warnings that don't prevent operation
    /// but might indicate issues that should be addressed.
    /// 
    /// # Arguments
    /// * `session_id` - Optional session ID if warning is session-specific
    /// * `category` - Category of the warning
    /// * `message` - Warning message
    async fn on_warning(
        &self, 
        session_id: Option<&SessionId>, 
        category: WarningCategory, 
        message: &str
    ) {
        // Default: do nothing
        match session_id {
            Some(id) => tracing::warn!("Warning for call {} ({:?}): {}", id, category, message),
            None => tracing::warn!("Warning ({:?}): {}", category, message),
        }
    }
    
    /// Called when an incoming transfer request (REFER) is received
    /// 
    /// This method is called when a SIP REFER request is received, requesting
    /// that this session be transferred to another party.
    /// 
    /// # Arguments
    /// * `session_id` - The session being requested to transfer
    /// * `target_uri` - The SIP URI to transfer to
    /// * `referred_by` - Optional URI of who requested the transfer
    /// 
    /// # Returns
    /// * `true` to accept the transfer (sends 202 Accepted and initiates new call)
    /// * `false` to reject the transfer (sends 603 Decline)
    async fn on_incoming_transfer_request(
        &self,
        session_id: &SessionId,
        target_uri: &str,
        referred_by: Option<&str>
    ) -> bool {
        // Default: accept transfers
        tracing::info!(
            "Call {} received transfer request to {} (referred by: {:?})",
            session_id, target_uri, referred_by
        );
        true
    }
    
    /// Called when transfer progress updates occur
    /// 
    /// This method provides updates on the progress of an outgoing transfer
    /// that was initiated via transfer_session().
    /// 
    /// # Arguments
    /// * `session_id` - The session that initiated the transfer
    /// * `status` - The current transfer status
    async fn on_transfer_progress(
        &self,
        session_id: &SessionId,
        status: &crate::manager::events::SessionTransferStatus
    ) {
        // Default: log the progress
        tracing::info!(
            "Call {} transfer progress: {:?}",
            session_id, status
        );
    }
}

/// Automatically accepts all incoming calls
#[derive(Debug, Default)]
pub struct AutoAnswerHandler;

#[async_trait]
impl CallHandler for AutoAnswerHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Accept(None) // Auto-accept without SDP answer
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Call {} ended: {}", call.id(), reason);
    }
}

/// Queues incoming calls up to a maximum limit
#[derive(Debug)]
pub struct QueueHandler {
    max_queue_size: usize,
    queue: Arc<Mutex<Vec<IncomingCall>>>,
    notify: Arc<Mutex<Option<mpsc::UnboundedSender<IncomingCall>>>>,
}

impl QueueHandler {
    /// Create a new queue handler with the specified maximum queue size
    pub fn new(max_queue_size: usize) -> Self {
        Self {
            max_queue_size,
            queue: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Mutex::new(None)),
        }
    }

    /// Set up a notification channel for when calls are queued
    pub fn set_notify_channel(&self, sender: mpsc::UnboundedSender<IncomingCall>) {
        *self.notify.lock().unwrap() = Some(sender);
    }

    /// Get the next call from the queue
    pub fn dequeue(&self) -> Option<IncomingCall> {
        self.queue.lock().unwrap().pop()
    }

    /// Get the current queue size
    pub fn queue_size(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    /// Add a call to the queue (internal use)
    pub async fn enqueue(&self, call: IncomingCall) {
        let mut queue = self.queue.lock().unwrap();
        queue.push(call.clone());
        
        // Notify if there's a listener
        if let Some(sender) = self.notify.lock().unwrap().as_ref() {
            let _ = sender.send(call);
        }
    }
}

#[async_trait]
impl CallHandler for QueueHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        let queue_size = {
            let queue = self.queue.lock().unwrap();
            queue.len()
        };

        if queue_size >= self.max_queue_size {
            CallDecision::Reject("Queue full".to_string())
        } else {
            self.enqueue(call).await;
            CallDecision::Defer
        }
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Queued call {} ended: {}", call.id(), reason);
    }
}

/// Routes calls based on destination patterns
#[derive(Debug)]
pub struct RoutingHandler {
    routes: HashMap<String, String>,
    default_action: CallDecision,
}

impl RoutingHandler {
    /// Create a new routing handler
    pub fn new() -> Self {
        Self {
            routes: HashMap::new(),
            default_action: CallDecision::Reject("No route found".to_string()),
        }
    }

    /// Add a routing rule (pattern -> target)
    pub fn add_route(&mut self, pattern: &str, target: &str) {
        self.routes.insert(pattern.to_string(), target.to_string());
    }

    /// Set the default action when no route matches
    pub fn set_default_action(&mut self, action: CallDecision) {
        self.default_action = action;
    }

    /// Find a route for the given destination
    fn find_route(&self, destination: &str) -> Option<&str> {
        // Simple prefix matching for now
        for (pattern, target) in &self.routes {
            if destination.starts_with(pattern) {
                return Some(target);
            }
        }
        None
    }
}

impl Default for RoutingHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CallHandler for RoutingHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        if let Some(target) = self.find_route(&call.to) {
            CallDecision::Forward(target.to_string())
        } else {
            self.default_action.clone()
        }
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Routed call {} ended: {}", call.id(), reason);
    }
}

/// Combines multiple handlers using a chain-of-responsibility pattern
#[derive(Debug)]
pub struct CompositeHandler {
    handlers: Vec<Arc<dyn CallHandler>>,
}

impl CompositeHandler {
    /// Create a new composite handler
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    /// Add a handler to the chain
    pub fn add_handler(mut self, handler: Arc<dyn CallHandler>) -> Self {
        self.handlers.push(handler);
        self
    }
}

impl Default for CompositeHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CallHandler for CompositeHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Try each handler in sequence
        for handler in &self.handlers {
            let decision = handler.on_incoming_call(call.clone()).await;
            
            // Return the decision from the first handler that doesn't defer
            // OR if any handler explicitly defers (like queue handler), return that
            match decision {
                CallDecision::Defer => return CallDecision::Defer,
                CallDecision::Accept(sdp) => return CallDecision::Accept(sdp),
                CallDecision::Reject(_) => return decision,
                CallDecision::Forward(_) => return decision,
            }
        }
        
        // If no handlers, reject the call
        CallDecision::Reject("No handlers configured".to_string())
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        // Notify all handlers
        for handler in &self.handlers {
            handler.on_call_ended(call.clone(), reason).await;
        }
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        // Notify all handlers
        for handler in &self.handlers {
            handler.on_call_established(call.clone(), local_sdp.clone(), remote_sdp.clone()).await;
        }
    }
} 