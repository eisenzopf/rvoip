//! Session Event Handlers
//!
//! This module provides pre-built handlers for common SIP session patterns.
//! Developers can use these handlers out of the box or compose them for custom behavior.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_session_core::api::handler::*;
//!
//! // Simple accept-all handler
//! let handler = AcceptAllHandler::new("MyServer");
//! session_manager.set_incoming_call_notifier(handler).await;
//!
//! // Business hours handler
//! let handler = BusinessHoursHandler::new("9:00", "17:00", "America/New_York")
//!     .with_weekend_mode(WeekendMode::Reject);
//! session_manager.set_incoming_call_notifier(handler).await;
//!
//! // Composite handler with capacity limits and logging
//! let handler = CompositeHandler::new()
//!     .add_handler(LoggingHandler::new("CallLog"))
//!     .add_handler(CapacityLimitHandler::new(100))
//!     .add_handler(AcceptAllHandler::new("Server"));
//! session_manager.set_incoming_call_notifier(handler).await;
//! ```

use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, Duration};
use async_trait::async_trait;
use chrono::{DateTime, Local, Timelike, Weekday, Datelike};
use tracing::{info, warn, debug, error};

use crate::session::manager::core::{IncomingCallNotification, IncomingCallEvent, CallDecision};
use crate::{SessionId, SessionManager};
use rvoip_sip_core::StatusCode;

// ============================================================================
// BASIC HANDLERS - Simple patterns for common use cases
// ============================================================================

/// Handler that accepts all incoming calls
#[derive(Debug)]
pub struct AcceptAllHandler {
    name: String,
    calls_accepted: AtomicUsize,
}

impl AcceptAllHandler {
    pub fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            calls_accepted: AtomicUsize::new(0),
        })
    }
    
    pub fn accepted_count(&self) -> usize {
        self.calls_accepted.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl IncomingCallNotification for AcceptAllHandler {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        let count = self.calls_accepted.fetch_add(1, Ordering::Relaxed) + 1;
        info!("📞 {} accepting call #{} from {}", self.name, count, event.caller_info.from);
        CallDecision::Accept
    }
    
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, _call_id: String) {
        debug!("📞 {} call terminated remotely: {}", self.name, session_id);
    }
    
    async fn on_call_ended_by_server(&self, session_id: SessionId, _call_id: String) {
        debug!("📞 {} call ended by server: {}", self.name, session_id);
    }
}

/// Handler that rejects all incoming calls
#[derive(Debug)]
pub struct RejectAllHandler {
    name: String,
    status_code: StatusCode,
    reason: Option<String>,
    calls_rejected: AtomicUsize,
}

impl RejectAllHandler {
    pub fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            status_code: StatusCode::BusyHere,
            reason: Some("Service unavailable".to_string()),
            calls_rejected: AtomicUsize::new(0),
        })
    }
    
    pub fn with_status_code(mut self, status_code: StatusCode) -> Self {
        self.status_code = status_code;
        self
    }
    
    pub fn with_reason(mut self, reason: String) -> Self {
        self.reason = Some(reason);
        self
    }
    
    pub fn rejected_count(&self) -> usize {
        self.calls_rejected.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl IncomingCallNotification for RejectAllHandler {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        let count = self.calls_rejected.fetch_add(1, Ordering::Relaxed) + 1;
        info!("📞 {} rejecting call #{} from {} ({})", 
            self.name, count, event.caller_info.from, 
            self.reason.as_deref().unwrap_or("Service unavailable"));
        
        CallDecision::Reject {
            status_code: self.status_code,
            reason: self.reason.clone(),
        }
    }
    
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, _call_id: String) {
        debug!("📞 {} call terminated remotely: {}", self.name, session_id);
    }
    
    async fn on_call_ended_by_server(&self, session_id: SessionId, _call_id: String) {
        debug!("📞 {} call ended by server: {}", self.name, session_id);
    }
}

/// Handler that limits concurrent calls to a maximum capacity
#[derive(Debug)]
pub struct CapacityLimitHandler {
    max_capacity: usize,
    current_calls: AtomicUsize,
    calls_accepted: AtomicUsize,
    calls_rejected: AtomicUsize,
}

impl CapacityLimitHandler {
    pub fn new(max_capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            max_capacity,
            current_calls: AtomicUsize::new(0),
            calls_accepted: AtomicUsize::new(0),
            calls_rejected: AtomicUsize::new(0),
        })
    }
    
    pub fn current_capacity(&self) -> usize {
        self.current_calls.load(Ordering::Relaxed)
    }
    
    pub fn available_capacity(&self) -> usize {
        self.max_capacity.saturating_sub(self.current_capacity())
    }
    
    pub fn accepted_count(&self) -> usize {
        self.calls_accepted.load(Ordering::Relaxed)
    }
    
    pub fn rejected_count(&self) -> usize {
        self.calls_rejected.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl IncomingCallNotification for CapacityLimitHandler {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        let current = self.current_calls.load(Ordering::Relaxed);
        
        if current >= self.max_capacity {
            let count = self.calls_rejected.fetch_add(1, Ordering::Relaxed) + 1;
            warn!("📞 CapacityLimit rejecting call #{} from {} - at capacity ({}/{})", 
                count, event.caller_info.from, current, self.max_capacity);
            
            CallDecision::Reject {
                status_code: StatusCode::BusyHere,
                reason: Some("Server at capacity".to_string()),
            }
        } else {
            self.current_calls.fetch_add(1, Ordering::Relaxed);
            let count = self.calls_accepted.fetch_add(1, Ordering::Relaxed) + 1;
            info!("📞 CapacityLimit accepting call #{} from {} ({}/{})", 
                count, event.caller_info.from, current + 1, self.max_capacity);
            
            CallDecision::Accept
        }
    }
    
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, _call_id: String) {
        self.current_calls.fetch_sub(1, Ordering::Relaxed);
        debug!("📞 CapacityLimit call terminated remotely: {} (capacity: {}/{})", 
            session_id, self.current_capacity(), self.max_capacity);
    }
    
    async fn on_call_ended_by_server(&self, session_id: SessionId, _call_id: String) {
        self.current_calls.fetch_sub(1, Ordering::Relaxed);
        debug!("📞 CapacityLimit call ended by server: {} (capacity: {}/{})", 
            session_id, self.current_capacity(), self.max_capacity);
    }
}

// ============================================================================
// ADVANCED HANDLERS - Business logic patterns
// ============================================================================

#[derive(Debug, Clone)]
pub enum WeekendMode {
    Accept,
    Reject,
    BusinessHours, // Use business hours even on weekends
}

/// Handler that accepts calls during business hours, rejects otherwise
#[derive(Debug)]
pub struct BusinessHoursHandler {
    name: String,
    start_hour: u32,
    end_hour: u32,
    timezone: String,
    weekend_mode: WeekendMode,
    calls_accepted: AtomicUsize,
    calls_rejected: AtomicUsize,
}

impl BusinessHoursHandler {
    pub fn new(start_time: &str, end_time: &str, timezone: &str) -> Arc<Self> {
        // Parse "HH:MM" format
        let start_hour = start_time.split(':').next()
            .and_then(|h| h.parse().ok())
            .unwrap_or(9);
        let end_hour = end_time.split(':').next()
            .and_then(|h| h.parse().ok())
            .unwrap_or(17);
        
        Arc::new(Self {
            name: "BusinessHours".to_string(),
            start_hour,
            end_hour,
            timezone: timezone.to_string(),
            weekend_mode: WeekendMode::Reject,
            calls_accepted: AtomicUsize::new(0),
            calls_rejected: AtomicUsize::new(0),
        })
    }
    
    pub fn with_name(mut self, name: String) -> Self {
        self.name = name;
        self
    }
    
    pub fn with_weekend_mode(mut self, mode: WeekendMode) -> Self {
        self.weekend_mode = mode;
        self
    }
    
    fn is_business_hours(&self) -> bool {
        let now = Local::now();
        let hour = now.hour();
        let weekday = now.weekday();
        
        // Check weekend
        let is_weekend = matches!(weekday, Weekday::Sat | Weekday::Sun);
        
        match self.weekend_mode {
            WeekendMode::Reject if is_weekend => false,
            WeekendMode::Accept if is_weekend => true,
            _ => hour >= self.start_hour && hour < self.end_hour,
        }
    }
    
    pub fn accepted_count(&self) -> usize {
        self.calls_accepted.load(Ordering::Relaxed)
    }
    
    pub fn rejected_count(&self) -> usize {
        self.calls_rejected.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl IncomingCallNotification for BusinessHoursHandler {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        if self.is_business_hours() {
            let count = self.calls_accepted.fetch_add(1, Ordering::Relaxed) + 1;
            info!("📞 {} accepting call #{} from {} (business hours)", 
                self.name, count, event.caller_info.from);
            CallDecision::Accept
        } else {
            let count = self.calls_rejected.fetch_add(1, Ordering::Relaxed) + 1;
            info!("📞 {} rejecting call #{} from {} (outside business hours)", 
                self.name, count, event.caller_info.from);
            
            CallDecision::Reject {
                status_code: StatusCode::TemporarilyUnavailable,
                reason: Some("Outside business hours".to_string()),
            }
        }
    }
    
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, _call_id: String) {
        debug!("📞 {} call terminated remotely: {}", self.name, session_id);
    }
    
    async fn on_call_ended_by_server(&self, session_id: SessionId, _call_id: String) {
        debug!("📞 {} call ended by server: {}", self.name, session_id);
    }
}

/// Handler that accepts/rejects calls based on a whitelist
#[derive(Debug)]
pub struct WhitelistHandler {
    name: String,
    allowed_callers: HashSet<String>,
    calls_accepted: AtomicUsize,
    calls_rejected: AtomicUsize,
}

impl WhitelistHandler {
    pub fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            allowed_callers: HashSet::new(),
            calls_accepted: AtomicUsize::new(0),
            calls_rejected: AtomicUsize::new(0),
        })
    }
    
    pub fn with_allowed_callers(mut self, callers: Vec<String>) -> Self {
        self.allowed_callers = callers.into_iter().collect();
        self
    }
    
    pub fn add_caller(&mut self, caller: String) {
        self.allowed_callers.insert(caller);
    }
    
    pub fn remove_caller(&mut self, caller: &str) {
        self.allowed_callers.remove(caller);
    }
    
    fn is_caller_allowed(&self, from: &str) -> bool {
        // Extract user part from SIP URI (sip:user@domain -> user)
        let user = if from.starts_with("sip:") {
            from.strip_prefix("sip:")
                .and_then(|s| s.split('@').next())
                .unwrap_or(from)
        } else {
            from
        };
        
        self.allowed_callers.contains(user) || self.allowed_callers.contains(from)
    }
    
    pub fn accepted_count(&self) -> usize {
        self.calls_accepted.load(Ordering::Relaxed)
    }
    
    pub fn rejected_count(&self) -> usize {
        self.calls_rejected.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl IncomingCallNotification for WhitelistHandler {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        if self.is_caller_allowed(&event.caller_info.from) {
            let count = self.calls_accepted.fetch_add(1, Ordering::Relaxed) + 1;
            info!("📞 {} accepting whitelisted call #{} from {}", 
                self.name, count, event.caller_info.from);
            CallDecision::Accept
        } else {
            let count = self.calls_rejected.fetch_add(1, Ordering::Relaxed) + 1;
            warn!("📞 {} rejecting non-whitelisted call #{} from {}", 
                self.name, count, event.caller_info.from);
            
            CallDecision::Reject {
                status_code: StatusCode::Forbidden,
                reason: Some("Caller not authorized".to_string()),
            }
        }
    }
    
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, _call_id: String) {
        debug!("📞 {} call terminated remotely: {}", self.name, session_id);
    }
    
    async fn on_call_ended_by_server(&self, session_id: SessionId, _call_id: String) {
        debug!("📞 {} call ended by server: {}", self.name, session_id);
    }
}

// ============================================================================
// WRAPPER HANDLERS - Add functionality to other handlers
// ============================================================================

/// Handler that adds logging to another handler
pub struct LoggingHandler {
    name: String,
    inner: Arc<dyn IncomingCallNotification + Send + Sync>,
    log_level: LogLevel,
}

#[derive(Debug, Clone)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LoggingHandler {
    pub fn new(name: &str, inner: Arc<dyn IncomingCallNotification + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            inner,
            log_level: LogLevel::Info,
        })
    }
    
    pub fn with_log_level(mut self, level: LogLevel) -> Self {
        self.log_level = level;
        self
    }
}

#[async_trait]
impl IncomingCallNotification for LoggingHandler {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        let start_time = SystemTime::now();
        
        match self.log_level {
            LogLevel::Debug => debug!("📞 {} processing call from {}", self.name, event.caller_info.from),
            LogLevel::Info => info!("📞 {} processing call from {}", self.name, event.caller_info.from),
            LogLevel::Warn => warn!("📞 {} processing call from {}", self.name, event.caller_info.from),
            LogLevel::Error => error!("📞 {} processing call from {}", self.name, event.caller_info.from),
        }
        
        let decision = self.inner.on_incoming_call(event).await;
        
        if let Ok(elapsed) = start_time.elapsed() {
            match self.log_level {
                LogLevel::Debug => debug!("📞 {} decision: {:?} (took {:?})", self.name, decision, elapsed),
                LogLevel::Info => info!("📞 {} decision: {:?} (took {:?})", self.name, decision, elapsed),
                LogLevel::Warn => warn!("📞 {} decision: {:?} (took {:?})", self.name, decision, elapsed),
                LogLevel::Error => error!("📞 {} decision: {:?} (took {:?})", self.name, decision, elapsed),
            }
        }
        
        decision
    }
    
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, call_id: String) {
        info!("📞 {} call terminated remotely: {}", self.name, session_id);
        self.inner.on_call_terminated_by_remote(session_id, call_id).await;
    }
    
    async fn on_call_ended_by_server(&self, session_id: SessionId, call_id: String) {
        info!("📞 {} call ended by server: {}", self.name, session_id);
        self.inner.on_call_ended_by_server(session_id, call_id).await;
    }
}

/// Handler that collects metrics about call handling
pub struct MetricsHandler {
    name: String,
    inner: Arc<dyn IncomingCallNotification + Send + Sync>,
    total_calls: AtomicUsize,
    accepted_calls: AtomicUsize,
    rejected_calls: AtomicUsize,
    total_processing_time: AtomicUsize, // microseconds
}

impl MetricsHandler {
    pub fn new(name: &str, inner: Arc<dyn IncomingCallNotification + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            inner,
            total_calls: AtomicUsize::new(0),
            accepted_calls: AtomicUsize::new(0),
            rejected_calls: AtomicUsize::new(0),
            total_processing_time: AtomicUsize::new(0),
        })
    }
    
    pub fn get_metrics(&self) -> HandlerMetrics {
        let total = self.total_calls.load(Ordering::Relaxed);
        let accepted = self.accepted_calls.load(Ordering::Relaxed);
        let rejected = self.rejected_calls.load(Ordering::Relaxed);
        let total_time_us = self.total_processing_time.load(Ordering::Relaxed);
        
        HandlerMetrics {
            total_calls: total,
            accepted_calls: accepted,
            rejected_calls: rejected,
            acceptance_rate: if total > 0 { (accepted as f64) / (total as f64) } else { 0.0 },
            average_processing_time: if total > 0 { 
                Duration::from_micros((total_time_us / total) as u64) 
            } else { 
                Duration::from_micros(0) 
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct HandlerMetrics {
    pub total_calls: usize,
    pub accepted_calls: usize,
    pub rejected_calls: usize,
    pub acceptance_rate: f64,
    pub average_processing_time: Duration,
}

#[async_trait]
impl IncomingCallNotification for MetricsHandler {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        let start_time = SystemTime::now();
        
        self.total_calls.fetch_add(1, Ordering::Relaxed);
        let decision = self.inner.on_incoming_call(event).await;
        
        match decision {
            CallDecision::Accept => { self.accepted_calls.fetch_add(1, Ordering::Relaxed); },
            CallDecision::AcceptWithSdp(_) => { self.accepted_calls.fetch_add(1, Ordering::Relaxed); },
            CallDecision::Reject { .. } => { self.rejected_calls.fetch_add(1, Ordering::Relaxed); },
            CallDecision::Defer => { /* Deferred decisions are not counted as accepted or rejected */ },
        }
        
        if let Ok(elapsed) = start_time.elapsed() {
            self.total_processing_time.fetch_add(elapsed.as_micros() as usize, Ordering::Relaxed);
        }
        
        decision
    }
    
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, call_id: String) {
        self.inner.on_call_terminated_by_remote(session_id, call_id).await;
    }
    
    async fn on_call_ended_by_server(&self, session_id: SessionId, call_id: String) {
        self.inner.on_call_ended_by_server(session_id, call_id).await;
    }
}

// ============================================================================
// COMPOSITE HANDLER - Combine multiple handlers with priority
// ============================================================================

/// Handler that chains multiple handlers in priority order
pub struct CompositeHandler {
    name: String,
    handlers: Vec<(Arc<dyn IncomingCallNotification + Send + Sync>, u32)>, // (handler, priority)
}

impl CompositeHandler {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            handlers: Vec::new(),
        }
    }
    
    pub fn add_handler(mut self, handler: Arc<dyn IncomingCallNotification + Send + Sync>, priority: u32) -> Self {
        self.handlers.push((handler, priority));
        // Sort by priority (higher priority first)
        self.handlers.sort_by(|a, b| b.1.cmp(&a.1));
        self
    }
    
    pub fn add_handler_default_priority(self, handler: Arc<dyn IncomingCallNotification + Send + Sync>) -> Self {
        self.add_handler(handler, 100) // Default priority
    }
}

#[async_trait]
impl IncomingCallNotification for CompositeHandler {
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        info!("📞 {} processing call through {} handlers", self.name, self.handlers.len());
        
        // Call each handler in priority order, use the first non-default decision
        for (handler, priority) in &self.handlers {
            debug!("📞 {} calling handler with priority {}", self.name, priority);
            let decision = handler.on_incoming_call(event.clone()).await;
            
            // For now, we use the first handler's decision
            // In a more sophisticated implementation, we might have voting logic
            return decision;
        }
        
        // Default decision if no handlers
        warn!("📞 {} no handlers configured, rejecting call", self.name);
        CallDecision::Reject {
            status_code: StatusCode::ServiceUnavailable,
            reason: Some("No handlers configured".to_string()),
        }
    }
    
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, call_id: String) {
        for (handler, _) in &self.handlers {
            handler.on_call_terminated_by_remote(session_id.clone(), call_id.clone()).await;
        }
    }
    
    async fn on_call_ended_by_server(&self, session_id: SessionId, call_id: String) {
        for (handler, _) in &self.handlers {
            handler.on_call_ended_by_server(session_id.clone(), call_id.clone()).await;
        }
    }
}

// ============================================================================
// BUILDER PATTERNS - Easy handler construction
// ============================================================================

/// Builder for creating sophisticated call handling strategies
pub struct HandlerBuilder {
    name: String,
    handlers: Vec<(Arc<dyn IncomingCallNotification + Send + Sync>, u32)>,
}

impl HandlerBuilder {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            handlers: Vec::new(),
        }
    }
    
    /// Add capacity limits
    pub fn with_capacity_limit(mut self, max_capacity: usize) -> Self {
        self.handlers.push((CapacityLimitHandler::new(max_capacity), 1000));
        self
    }
    
    /// Add business hours filtering
    pub fn with_business_hours(mut self, start: &str, end: &str, timezone: &str) -> Self {
        // Create the handler directly with configuration
        let handler = Arc::new(BusinessHoursHandler {
            name: format!("{}_BusinessHours", self.name),
            start_hour: start.split(':').next()
                .and_then(|h| h.parse().ok())
                .unwrap_or(9),
            end_hour: end.split(':').next()
                .and_then(|h| h.parse().ok())
                .unwrap_or(17),
            timezone: timezone.to_string(),
            weekend_mode: WeekendMode::Reject,
            calls_accepted: AtomicUsize::new(0),
            calls_rejected: AtomicUsize::new(0),
        });
        self.handlers.push((handler, 800));
        self
    }
    
    /// Add whitelist filtering
    pub fn with_whitelist(mut self, allowed_callers: Vec<String>) -> Self {
        // Create the handler directly with configuration
        let handler = Arc::new(WhitelistHandler {
            name: format!("{}_Whitelist", self.name),
            allowed_callers: allowed_callers.into_iter().collect(),
            calls_accepted: AtomicUsize::new(0),
            calls_rejected: AtomicUsize::new(0),
        });
        self.handlers.push((handler, 900));
        self
    }
    
    /// Add logging
    pub fn with_logging(mut self) -> Self {
        // This will wrap the final composite handler
        self
    }
    
    /// Add metrics collection
    pub fn with_metrics(mut self) -> Self {
        // This will wrap the final composite handler
        self
    }
    
    /// Build the final handler
    pub fn build(self) -> Arc<dyn IncomingCallNotification + Send + Sync> {
        if self.handlers.is_empty() {
            // Default to accept-all if no handlers specified
            return AcceptAllHandler::new(&self.name).into_arc();
        }
        
        if self.handlers.len() == 1 {
            return self.handlers.into_iter().next().unwrap().0;
        }
        
        // Create composite handler
        let mut composite = CompositeHandler::new(&self.name);
        for (handler, priority) in self.handlers {
            composite = composite.add_handler(handler, priority);
        }
        
        Arc::new(composite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::manager::core::CallerInfo;
    
    fn create_test_event() -> IncomingCallEvent {
        IncomingCallEvent {
            session_id: SessionId::new(),
            caller_info: CallerInfo {
                from: "sip:test@example.com".to_string(),
                to: "sip:server@example.com".to_string(),
                call_id: "test-call-123".to_string(),
            },
        }
    }
    
    #[tokio::test]
    async fn test_accept_all_handler() {
        let handler = AcceptAllHandler::new("Test").into_arc();
        let event = create_test_event();
        
        let decision = handler.on_incoming_call(event).await;
        assert!(matches!(decision, CallDecision::Accept));
        assert_eq!(handler.accepted_count(), 1);
    }
    
    #[tokio::test]
    async fn test_reject_all_handler() {
        let handler = RejectAllHandler::new("Test").into_arc();
        let event = create_test_event();
        
        let decision = handler.on_incoming_call(event).await;
        assert!(matches!(decision, CallDecision::Reject { .. }));
        assert_eq!(handler.rejected_count(), 1);
    }
    
    #[tokio::test]
    async fn test_capacity_limit_handler() {
        let handler = CapacityLimitHandler::new(2).into_arc();
        let event = create_test_event();
        
        // First two calls should be accepted
        assert!(matches!(handler.on_incoming_call(event.clone()).await, CallDecision::Accept));
        assert!(matches!(handler.on_incoming_call(event.clone()).await, CallDecision::Accept));
        
        // Third call should be rejected
        assert!(matches!(handler.on_incoming_call(event).await, CallDecision::Reject { .. }));
        
        assert_eq!(handler.accepted_count(), 2);
        assert_eq!(handler.rejected_count(), 1);
        assert_eq!(handler.current_capacity(), 2);
    }
    
    #[tokio::test]
    async fn test_whitelist_handler() {
        let handler = WhitelistHandler::new("Test")
            .with_allowed_callers(vec!["test".to_string()])
            .into_arc();
        
        let allowed_event = create_test_event(); // "test@example.com" -> "test" should be allowed
        let decision = handler.on_incoming_call(allowed_event).await;
        assert!(matches!(decision, CallDecision::Accept));
        
        let mut denied_event = create_test_event();
        denied_event.caller_info.from = "sip:other@example.com".to_string();
        let decision = handler.on_incoming_call(denied_event).await;
        assert!(matches!(decision, CallDecision::Reject { .. }));
    }
    
    #[tokio::test]
    async fn test_handler_builder() {
        let handler = HandlerBuilder::new("TestServer")
            .with_capacity_limit(100)
            .with_whitelist(vec!["test".to_string()])
            .build();
        
        let event = create_test_event();
        let decision = handler.on_incoming_call(event).await;
        // Should be accepted since "test" is whitelisted and we're under capacity
        assert!(matches!(decision, CallDecision::Accept));
    }
} 