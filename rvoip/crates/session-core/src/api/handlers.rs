//! Event Handlers for Session Management
//!
//! Provides a simplified event handling system with pre-built handlers for common use cases.
//! 
//! # Overview
//! 
//! Call handlers allow you to customize how your application responds to VoIP events.
//! The system supports two main patterns:
//! 
//! 1. **Immediate Decision**: Make a decision synchronously in the callback
//! 2. **Deferred Decision**: Defer the decision for async processing
//! 
//! # Built-in Handlers
//! 
//! ## AutoAnswerHandler
//! 
//! Automatically accepts all incoming calls:
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! 
//! let handler = AutoAnswerHandler::default();
//! let coordinator = SessionManagerBuilder::new()
//!     .with_handler(Arc::new(handler))
//!     .build()
//!     .await?;
//! ```
//! 
//! ## QueueHandler
//! 
//! Queues incoming calls for later processing:
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use tokio::sync::mpsc;
//! 
//! // Create queue handler with max 10 calls
//! let queue_handler = Arc::new(QueueHandler::new(10));
//! 
//! // Set up notification channel
//! let (tx, mut rx) = mpsc::unbounded_channel();
//! queue_handler.set_notify_channel(tx);
//! 
//! // Process queued calls in another task
//! tokio::spawn(async move {
//!     while let Some(call) = rx.recv().await {
//!         // Process the queued call asynchronously
//!         process_queued_call(call).await;
//!     }
//! });
//! ```
//! 
//! ## RoutingHandler
//! 
//! Routes calls based on destination patterns:
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! 
//! let mut router = RoutingHandler::new();
//! 
//! // Add routing rules
//! router.add_route("sip:support@", "sip:queue@support.internal");
//! router.add_route("sip:sales@", "sip:queue@sales.internal");
//! router.add_route("sip:+1800", "sip:tollfree@gateway.com");
//! 
//! // Set default action for unmatched calls
//! router.set_default_action(CallDecision::Reject("Unknown destination"));
//! 
//! let coordinator = SessionManagerBuilder::new()
//!     .with_handler(Arc::new(router))
//!     .build()
//!     .await?;
//! ```
//! 
//! ## CompositeHandler
//! 
//! Combines multiple handlers in a chain:
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! 
//! let composite = CompositeHandler::new()
//!     .add_handler(Arc::new(SecurityHandler::new()))
//!     .add_handler(Arc::new(RateLimitHandler::new(10)))
//!     .add_handler(Arc::new(RoutingHandler::new()))
//!     .add_handler(Arc::new(QueueHandler::new(100)));
//! 
//! let coordinator = SessionManagerBuilder::new()
//!     .with_handler(Arc::new(composite))
//!     .build()
//!     .await?;
//! ```
//! 
//! # Custom Handlers
//! 
//! ## Example: Business Hours Handler
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use chrono::{Local, Timelike};
//! 
//! #[derive(Debug)]
//! struct BusinessHoursHandler {
//!     start_hour: u32,
//!     end_hour: u32,
//!     after_hours_message: String,
//! }
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for BusinessHoursHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         let hour = Local::now().hour();
//!         
//!         if hour >= self.start_hour && hour < self.end_hour {
//!             // During business hours, defer to next handler
//!             CallDecision::Defer
//!         } else {
//!             // After hours, reject with message
//!             CallDecision::Reject(self.after_hours_message.clone())
//!         }
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         // Log call duration for business analytics
//!         if let Some(started_at) = call.started_at {
//!             let duration = started_at.elapsed();
//!             log_call_duration(&call.id(), duration).await;
//!         }
//!     }
//! }
//! ```
//! 
//! ## Example: Database-Backed Handler
//! 
//! ```rust
//! #[derive(Debug)]
//! struct DatabaseHandler {
//!     db: Arc<Database>,
//!     coordinator: Arc<RwLock<Option<Arc<SessionCoordinator>>>>,
//! }
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for DatabaseHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // Defer for async database lookup
//!         CallDecision::Defer
//!     }
//!     
//!     async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
//!         // Record call in database
//!         self.db.record_call_start(&call, &local_sdp, &remote_sdp).await;
//!         
//!         // Set up media if we have the coordinator reference
//!         if let Some(coord) = self.coordinator.read().await.as_ref() {
//!             if let Some(sdp) = remote_sdp {
//!                 if let Ok(info) = parse_sdp_connection(&sdp) {
//!                     let _ = MediaControl::establish_media_flow(
//!                         coord,
//!                         call.id(),
//!                         &format!("{}:{}", info.ip, info.port)
//!                     ).await;
//!                 }
//!             }
//!         }
//!     }
//! }
//! 
//! // Process deferred calls from database handler
//! async fn process_database_calls(
//!     coordinator: &Arc<SessionCoordinator>,
//!     call: IncomingCall,
//!     db: &Database
//! ) -> Result<()> {
//!     // Check caller in database
//!     let caller_info = db.lookup_caller(&call.from).await?;
//!     
//!     if caller_info.is_blocked {
//!         SessionControl::reject_incoming_call(
//!             coordinator,
//!             &call,
//!             "Caller blocked"
//!         ).await?;
//!     } else if caller_info.is_vip {
//!         // VIP callers get priority handling
//!         let sdp_answer = generate_high_quality_sdp(&call.sdp);
//!         SessionControl::accept_incoming_call(
//!             coordinator,
//!             &call,
//!             Some(sdp_answer)
//!         ).await?;
//!     } else {
//!         // Regular callers
//!         SessionControl::accept_incoming_call(
//!             coordinator,
//!             &call,
//!             None
//!         ).await?;
//!     }
//!     
//!     Ok(())
//! }
//! ```

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::api::types::{IncomingCall, CallSession, CallDecision};
use crate::errors::Result;

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