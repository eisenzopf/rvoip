//! Event Handlers for Session Management
//!
//! Provides a simplified event handling system with pre-built handlers for common use cases.

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
} 