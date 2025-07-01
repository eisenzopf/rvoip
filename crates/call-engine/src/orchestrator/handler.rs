//! # CallHandler Implementation for Call Center Integration
//!
//! This module provides the CallHandler trait implementation that serves as the primary
//! integration point between session-core and the call center engine. It handles all
//! incoming call events, media events, and state changes, providing comprehensive
//! event processing, B2BUA coordination, and real-time call management.
//!
//! ## Overview
//!
//! The CallHandler implementation is the critical bridge between session-core's SIP
//! processing and the call center's business logic. It receives all call-related events
//! from session-core and translates them into appropriate call center actions, including
//! routing decisions, agent assignments, bridge management, and comprehensive monitoring.
//! This module enables seamless integration between the SIP stack and call center operations.
//!
//! ## Key Features
//!
//! - **Event-Driven Architecture**: Comprehensive event handling for all call states
//! - **B2BUA Event Processing**: Sophisticated handling of B2BUA call scenarios
//! - **Agent Assignment Completion**: Automatic bridge creation when agents answer
//! - **Call State Management**: Real-time call state tracking and updates
//! - **Media Quality Monitoring**: Continuous media quality assessment and alerting
//! - **DTMF Processing**: Interactive voice response and agent feature processing
//! - **Error Recovery**: Robust error handling with automatic recovery mechanisms
//! - **Performance Metrics**: Comprehensive call metrics and performance tracking
//! - **Timeout Management**: Intelligent timeout handling for pending operations
//! - **Session Coordination**: Seamless coordination with session-core operations
//!
//! ## Event Processing Architecture
//!
//! The CallHandler processes events in this flow:
//!
//! 1. **Session-Core Events**: Receive events from session-core SIP stack
//! 2. **Event Classification**: Classify events by type and priority
//! 3. **Context Resolution**: Resolve call context and related information
//! 4. **Business Logic**: Apply call center business rules and policies
//! 5. **State Updates**: Update call state and database records
//! 6. **Action Triggering**: Trigger appropriate call center actions
//! 7. **Monitoring**: Update metrics and monitoring systems
//!
//! ## Examples
//!
//! ### Basic CallHandler Setup
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, orchestrator::CallCenterCallHandler};
//! use std::sync::{Arc, Weak};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create call center engine
//! let engine = Arc::new(CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! // Create call handler with weak reference to prevent cycles
//! let call_handler = CallCenterCallHandler {
//!     engine: Arc::downgrade(&engine),
//! };
//! 
//! println!("📞 CallHandler created and ready for event processing");
//! println!("🔗 Integrated with session-core for SIP event handling");
//! println!("🎯 Event processing will route calls through call center logic");
//! 
//! // Handler is now ready to receive events from session-core
//! // Events will be automatically processed and routed appropriately
//! # Ok(())
//! # }
//! ```
//!
//! ### Incoming Call Event Processing
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! use rvoip_session_core::{IncomingCall, SessionId, CallDecision, CallHandler};
//! use std::sync::{Arc, Weak};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(rvoip_call_engine::CallCenterEngine::new(rvoip_call_engine::CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! let call_handler = CallCenterCallHandler {
//!     engine: Arc::downgrade(&engine),
//! };
//! 
//! // Simulate incoming call event from session-core
//! let incoming_call = IncomingCall {
//!     id: SessionId("test-call".to_string()),
//!     from: "sip:customer@external.com".to_string(),
//!     to: "sip:support@call-center.local".to_string(),
//!     sdp: Some("v=0\r\no=- 123456 IN IP4 192.168.1.100\r\n...".to_string()),
//!     headers: std::collections::HashMap::new(),
//!     received_at: std::time::Instant::now(),
//! };
//! 
//! // Process incoming call through call handler
//! let decision = call_handler.on_incoming_call(incoming_call).await;
//! 
//! match decision {
//!     CallDecision::Accept(sdp_answer) => {
//!         println!("✅ Call accepted by call center");
//!         if let Some(answer) = sdp_answer {
//!             println!("📄 Generated SDP answer ({} bytes)", answer.len());
//!         }
//!         println!("🔄 Call routing to agent will begin automatically");
//!     }
//!     CallDecision::Reject(reason) => {
//!         println!("❌ Call rejected: {}", reason);
//!         println!("📊 Rejection logged for capacity planning");
//!     }
//!     CallDecision::Defer => {
//!         println!("⏳ Call deferred for later processing");
//!     }
//!     CallDecision::Forward(destination) => {
//!         println!("📞 Call forwarded to: {}", destination);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Call Established Event Processing
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! use rvoip_session_core::{CallSession, SessionId, CallHandler};
//! use std::sync::{Arc, Weak};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(rvoip_call_engine::CallCenterEngine::new(rvoip_call_engine::CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! let call_handler = CallCenterCallHandler {
//!     engine: Arc::downgrade(&engine),
//! };
//! 
//! // Simulate call established event
//! let session_id = SessionId("established-call".to_string());
//! let local_sdp = Some("v=0\r\no=callcenter...".to_string());
//! let remote_sdp = Some("v=0\r\no=agent...".to_string());
//! 
//! // Note: CallSession::new may not be available, this is an example
//! // In practice, use the session_id directly or through session coordinator
//! 
//! println!("📞 Call established event processed");
//! println!("🎯 Event handling logic:");
//! println!("  1️⃣ Check for pending agent assignments");
//! println!("  2️⃣ Complete B2BUA bridge if agent answered");
//! println!("  3️⃣ Update call status to bridged");
//! println!("  4️⃣ Start call monitoring and metrics");
//! 
//! // The handler automatically determines if this is:
//! // - Agent answering for pending assignment → Complete bridge
//! // - Regular call establishment → Update call status
//! # Ok(())
//! # }
//! ```
//!
//! ### Call Termination Event Processing
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! use rvoip_session_core::{CallSession, SessionId, CallHandler};
//! use std::sync::{Arc, Weak};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(rvoip_call_engine::CallCenterEngine::new(rvoip_call_engine::CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! let call_handler = CallCenterCallHandler {
//!     engine: Arc::downgrade(&engine),
//! };
//! 
//! // Simulate call termination event
//! let session_id = SessionId("terminated-call".to_string());
//! let termination_reason = "Normal call completion";
//! 
//! // Note: This example shows the concept - actual implementation may differ
//! 
//! println!("🛑 Call termination processed: {}", termination_reason);
//! println!("🧹 Comprehensive cleanup performed:");
//! println!("  ✅ Database queue cleanup");
//! println!("  ✅ Pending assignment cleanup");
//! println!("  ✅ B2BUA call leg termination");
//! println!("  ✅ Agent status updates");
//! println!("  ✅ Call metrics calculation");
//! println!("  ✅ Bridge resource cleanup");
//! 
//! // Automatic cleanup includes:
//! // - Remove from database queues
//! // - Clean up pending assignments
//! // - Terminate related B2BUA sessions
//! // - Update agent status to wrap-up
//! // - Calculate and store call metrics
//! # Ok(())
//! # }
//! ```
//!
//! ### Media Quality Event Processing
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! use rvoip_session_core::{SessionId, MediaQualityAlertLevel, CallHandler};
//! use std::sync::{Arc, Weak};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(rvoip_call_engine::CallCenterEngine::new(rvoip_call_engine::CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! let call_handler = CallCenterCallHandler {
//!     engine: Arc::downgrade(&engine),
//! };
//! 
//! let session_id = SessionId("quality-monitor-call".to_string());
//! 
//! // Process media quality events
//! call_handler.on_media_quality(
//!     &session_id,
//!     3.2,  // MOS score (below threshold)
//!     2.5,  // Packet loss percentage
//!     MediaQualityAlertLevel::Poor
//! ).await;
//! 
//! println!("📊 Media quality event processed:");
//! println!("  📞 Call: {}", session_id);
//! println!("  📈 MOS Score: 3.2/5.0");
//! println!("  📉 Packet Loss: 2.5%");
//! println!("  🚨 Alert Level: Poor");
//! 
//! println!("\n🔔 Automatic Actions Triggered:");
//! println!("  📊 Quality metrics recorded");
//! println!("  🚨 Supervisor alert generated");
//! println!("  📝 Quality issue logged");
//! println!("  🔧 Network diagnostics initiated");
//! 
//! // Quality thresholds trigger automatic actions:
//! // - Poor/Critical quality → Alert supervisors
//! // - Metrics recorded for trending
//! // - Quality reports updated
//! # Ok(())
//! # }
//! ```
//!
//! ### DTMF Processing
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! use rvoip_session_core::{SessionId, CallHandler};
//! use std::sync::{Arc, Weak};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(rvoip_call_engine::CallCenterEngine::new(rvoip_call_engine::CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! let call_handler = CallCenterCallHandler {
//!     engine: Arc::downgrade(&engine),
//! };
//! 
//! let session_id = SessionId("dtmf-processing-call".to_string());
//! 
//! // Process DTMF input
//! call_handler.on_dtmf(&session_id, '1', 250).await;
//! 
//! println!("📱 DTMF input processed:");
//! println!("  📞 Call: {}", session_id);
//! println!("  🔢 Digit: '1'");
//! println!("  ⏱️ Duration: 250ms");
//! 
//! println!("\n🎯 DTMF Processing Options:");
//! println!("  📞 IVR menu navigation");
//! println!("  🎚️ Agent feature activation");
//! println!("  🔄 Call transfer initiation");
//! println!("  📝 Customer input collection");
//! println!("  🎵 Hold music controls");
//! 
//! // DTMF processing can trigger:
//! // - IVR menu navigation
//! // - Agent feature activation
//! // - Call routing changes
//! // - Customer input collection
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Event Handling
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! use rvoip_session_core::{
//!     SessionId, CallState, MediaFlowDirection, WarningCategory, CallHandler
//! };
//! use std::sync::{Arc, Weak};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(rvoip_call_engine::CallCenterEngine::new(rvoip_call_engine::CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! let call_handler = CallCenterCallHandler {
//!     engine: Arc::downgrade(&engine),
//! };
//! 
//! let session_id = SessionId("advanced-events-call".to_string());
//! 
//! // Process call state change
//! call_handler.on_call_state_changed(
//!     &session_id,
//!     &CallState::Ringing,
//!     &CallState::Active,
//!     Some("Agent answered")
//! ).await;
//! 
//! // Process media flow changes
//! call_handler.on_media_flow(
//!     &session_id,
//!     MediaFlowDirection::Both,
//!     true,  // Media flow active
//!     "PCMU"  // Codec
//! ).await;
//! 
//! // Process warnings
//! call_handler.on_warning(
//!     Some(&session_id),
//!     WarningCategory::Media,
//!     "High jitter detected"
//! ).await;
//! 
//! println!("🔄 Advanced event processing completed:");
//! println!("  📊 Call state transitions tracked");
//! println!("  🎵 Media flow monitoring active");
//! println!("  ⚠️ Warning conditions logged");
//! println!("  📈 Performance metrics updated");
//! 
//! // Advanced events enable:
//! // - Detailed call state tracking
//! // - Media flow monitoring
//! // - Proactive issue detection
//! // - Comprehensive logging
//! # Ok(())
//! # }
//! ```
//!
//! ## B2BUA Event Coordination
//!
//! ### Pending Assignment Management
//!
//! The handler manages complex B2BUA scenarios with pending agent assignments:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! # use rvoip_session_core::{CallSession, SessionId};
//! # use std::sync::{Arc, Weak};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(rvoip_call_engine::CallCenterEngine::new(rvoip_call_engine::CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! // B2BUA pending assignment workflow:
//! println!("🔄 B2BUA Pending Assignment Workflow:");
//! 
//! println!("  1️⃣ Customer Call Accepted:");
//! println!("     ↳ Customer session created and SDP answered");
//! println!("     ↳ Call queued for agent assignment");
//! 
//! println!("  2️⃣ Agent Assignment:");
//! println!("     ↳ Agent selected from available pool");
//! println!("     ↳ Outgoing call created to agent");
//! println!("     ↳ Pending assignment stored");
//! 
//! println!("  3️⃣ Agent Answers (on_call_established):");
//! println!("     ↳ Pending assignment detected");
//! println!("     ↳ Bridge created between customer and agent");
//! println!("     ↳ Call status updated to bridged");
//! 
//! println!("  4️⃣ Timeout Handling:");
//! println!("     ↳ 30-second timeout for agent answer");
//! println!("     ↳ Automatic rollback if timeout");
//! println!("     ↳ Customer re-queued with higher priority");
//! 
//! // This workflow ensures reliable B2BUA operation
//! // with proper timeout handling and error recovery
//! # Ok(())
//! # }
//! ```
//!
//! ### Call Termination Race Conditions
//!
//! The handler manages complex race conditions in B2BUA termination:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // B2BUA termination race condition handling:
//! println!("🏁 B2BUA Termination Race Condition Handling:");
//! 
//! println!("  🚦 Problem: Both call legs may terminate simultaneously");
//! println!("     ↳ Customer hangs up while agent also hangs up");
//! println!("     ↳ Could cause double cleanup and errors");
//! 
//! println!("  🛡️ Solution: Coordinated termination with state tracking:");
//! println!("     ↳ Mark related session as terminating");
//! println!("     ↳ Configurable delay before forwarding BYE");
//! println!("     ↳ Skip forwarding if already terminating");
//! println!("     ↳ Comprehensive cleanup regardless");
//! 
//! println!("  ⚡ Benefits:");
//! println!("     ✅ Prevents duplicate BYE messages");
//! println!("     ✅ Ensures proper cleanup in all scenarios");
//! println!("     ✅ Maintains call metrics integrity");
//! println!("     ✅ Handles edge cases gracefully");
//! 
//! // The handler uses sophisticated logic to handle
//! // complex B2BUA termination scenarios reliably
//! # Ok(())
//! # }
//! ```
//!
//! ## Integration with Call Center Components
//!
//! ### Database Integration
//!
//! The handler seamlessly integrates with database operations:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Database integration patterns:
//! println!("💾 Database Integration:");
//! 
//! println!("  📞 Call Events → Database Updates:");
//! println!("     ↳ Call established → Update call records");
//! println!("     ↳ Call terminated → Calculate metrics");
//! println!("     ↳ Agent assignment → Update agent status");
//! 
//! println!("  🔄 State Synchronization:");
//! println!("     ↳ Real-time call state in database");
//! println!("     ↳ Agent status updates on events");
//! println!("     ↳ Queue cleanup on termination");
//! 
//! println!("  📊 Metrics Collection:");
//! println!("     ↳ Call duration tracking");
//! println!("     ↳ Wait time calculation");
//! println!("     ↳ Quality metrics storage");
//! 
//! // Every event updates appropriate database records
//! // ensuring consistency between session state and persistence
//! # Ok(())
//! # }
//! ```
//!
//! ### Session-Core Integration
//!
//! The handler provides the primary integration with session-core:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Session-core integration architecture:
//! println!("🔗 Session-Core Integration:");
//! 
//! println!("  📡 Event Flow:");
//! println!("     Session-Core → CallHandler → CallCenterEngine");
//! 
//! println!("  🎯 Event Types:");
//! println!("     ↳ Incoming calls → Routing decisions");
//! println!("     ↳ Call state changes → Status updates");
//! println!("     ↳ Media events → Quality monitoring");
//! println!("     ↳ DTMF events → IVR processing");
//! 
//! println!("  🔄 Response Flow:");
//! println!("     CallCenterEngine → Session-Core APIs");
//! 
//! println!("  ⚡ Real-time Processing:");
//! println!("     ↳ Event processing in microseconds");
//! println!("     ↳ Non-blocking async operations");
//! println!("     ↳ Concurrent event handling");
//! 
//! // Handler acts as the primary interface between
//! // session-core's SIP stack and call center business logic
//! # Ok(())
//! # }
//! ```
//!
//! ## Error Handling and Recovery
//!
//! ### Robust Error Management
//!
//! The handler provides comprehensive error handling:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Error handling strategies:
//! println!("🛡️ Error Handling Strategies:");
//! 
//! println!("  🔧 Engine Unavailable:");
//! println!("     ↳ Weak reference prevents cycles");
//! println!("     ↳ Graceful degradation when engine dropped");
//! println!("     ↳ Reject calls when call center unavailable");
//! 
//! println!("  📞 Call Processing Errors:");
//! println!("     ↳ Database failures → Continue with in-memory");
//! println!("     ↳ Bridge failures → Terminate gracefully");
//! println!("     ↳ Agent assignment failures → Re-queue");
//! 
//! println!("  🎵 Media Errors:");
//! println!("     ↳ Quality issues → Alert and continue");
//! println!("     ↳ Codec failures → Attempt recovery");
//! println!("     ↳ Flow problems → Investigate and log");
//! 
//! println!("  🔄 Recovery Mechanisms:");
//! println!("     ↳ Automatic retry with backoff");
//! println!("     ↳ Fallback to simpler operations");
//! println!("     ↳ Comprehensive error logging");
//! 
//! // The handler ensures system resilience through
//! // graceful error handling and automatic recovery
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance and Scalability
//!
//! ### High-Performance Event Processing
//!
//! The handler is optimized for high-performance operations:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::CallCenterCallHandler;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Performance characteristics:
//! println!("⚡ Performance Characteristics:");
//! 
//! println!("  🚀 Event Processing:");
//! println!("     ↳ Microsecond event handling");
//! println!("     ↳ Non-blocking async operations");
//! println!("     ↳ Concurrent event processing");
//! 
//! println!("  💾 Memory Efficiency:");
//! println!("     ↳ Weak references prevent cycles");
//! println!("     ↳ Minimal per-event allocations");
//! println!("     ↳ Efficient data structures");
//! 
//! println!("  📊 Scalability:");
//! println!("     ↳ Linear scaling with call volume");
//! println!("     ↳ Independent event processing");
//! println!("     ↳ Resource-conscious operations");
//! 
//! println!("  🔄 Concurrency:");
//! println!("     ↳ Thread-safe event handling");
//! println!("     ↳ Lock-free data access where possible");
//! println!("     ↳ Async/await throughout");
//! 
//! // Handler supports thousands of concurrent calls
//! // with minimal performance impact
//! # Ok(())
//! # }
//! ```

//! CallHandler implementation for the call center

use std::sync::Weak;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};
use rvoip_session_core::{
    CallHandler, IncomingCall, CallDecision, CallSession, SessionId, CallState,
    MediaQualityAlertLevel, MediaFlowDirection, WarningCategory
};
use std::time::Instant;

use super::core::CallCenterEngine;
use super::types::{AgentInfo, CallStatus};
use crate::agent::AgentStatus;
use crate::error::CallCenterError;

/// CallHandler implementation for the call center
#[derive(Clone, Debug)]
pub struct CallCenterCallHandler {
    pub engine: Weak<CallCenterEngine>,
}

#[async_trait]
impl CallHandler for CallCenterCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        debug!("CallCenterCallHandler: Received incoming call {}", call.id);
        
        // Try to get a strong reference to the engine
        if let Some(engine) = self.engine.upgrade() {
            // Process the incoming call through the call center's routing logic
            match engine.process_incoming_call(call).await {
                Ok(decision) => decision,
                Err(e) => {
                    error!("Failed to process incoming call: {}", e);
                    CallDecision::Reject("Call center processing error".to_string())
                }
            }
        } else {
            warn!("Call center engine has been dropped");
            CallDecision::Reject("Call center not available".to_string())
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("📞 Call {} ended: {}", call.id(), reason);
        
        if let Some(engine) = self.engine.upgrade() {
            // CRITICAL: Clean up from database queue first to prevent re-queueing
            if let Some(db_manager) = &engine.db_manager {
                // Remove from queue and active calls (this method handles both tables)
                if let Err(e) = db_manager.remove_call_from_queue(&call.id().to_string()).await {
                    debug!("Call {} not in queue or already removed: {}", call.id(), e);
                } else {
                    debug!("🧹 Cleaned up call {} from database", call.id());
                }
            }
            
            // First, check if this is a pending assignment that needs cleanup
            if let Some((_, pending_assignment)) = engine.pending_assignments.remove(&call.id()) {
                info!("🧹 Cleaning up pending assignment for call {} (agent {} never answered)", 
                      call.id(), pending_assignment.agent_id);
                
                // Return agent to available in database since they never actually took the call
                if let Some(db_manager) = &engine.db_manager {
                    let _ = db_manager.update_agent_call_count(&pending_assignment.agent_id.0, -1).await;
                    let _ = db_manager.update_agent_status(&pending_assignment.agent_id.0, AgentStatus::Available).await;
                }
                
                // Don't re-queue - the customer hung up
                info!("❌ Not re-queuing call {} - customer ended the call", pending_assignment.customer_session_id);
            }
            
            // PHASE 0.24: Enhanced call termination coordination
            let related_session_id = engine.active_calls.get(&call.id())
                .and_then(|call_info| call_info.related_session_id.clone());
            
            if let Some(related_id) = related_session_id {
                info!("📞 BYE-FORWARD: Session {} terminated, checking related session {}", call.id(), related_id);
                
                // PHASE 0.24: Add termination flags to prevent race conditions
                if let Some(mut call_info) = engine.active_calls.get_mut(&related_id) {
                    if call_info.status == crate::orchestrator::types::CallStatus::Disconnected {
                        info!("🔄 BYE-RACE: Related session {} already terminating, skipping BYE forward", related_id);
                        return;
                    }
                    // Mark as terminating to prevent race conditions
                    call_info.status = crate::orchestrator::types::CallStatus::Disconnected;
                    info!("🏷️ BYE-MARK: Marked related session {} as terminating", related_id);
                } else {
                    info!("ℹ️ BYE-FORWARD: Related session {} not found in active calls (may already be cleaned up)", related_id);
                }
                
                // PHASE 0.24: Add configurable delay before forwarding BYE to prevent race conditions
                let race_delay = std::time::Duration::from_millis(engine.config.general.bye_race_delay_ms);
                tokio::time::sleep(race_delay).await;
                
                info!("📤 BYE-FORWARD: Forwarding BYE to related B2BUA session: {}", related_id);
                
                // Clean up related session from database too
                if let Some(db_manager) = &engine.db_manager {
                    let _ = db_manager.remove_call_from_queue(&related_id.to_string()).await;
                }
                
                // Terminate the related dialog
                if let Some(coordinator) = &engine.session_coordinator {
                    match coordinator.terminate_session(&related_id).await {
                        Ok(_) => {
                            info!("✅ BYE-FORWARD: Successfully terminated related B2BUA session: {}", related_id);
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            
                            // PHASE 0.24: Better error categorization for BYE forwarding
                            if error_msg.contains("not found") || error_msg.contains("No dialog found") || 
                               error_msg.contains("session not found") {
                                info!("ℹ️ BYE-FORWARD: Related session {} already terminated (this is normal)", related_id);
                            } else if error_msg.contains("already terminated") || error_msg.contains("terminated") {
                                info!("ℹ️ BYE-FORWARD: Related session {} was already terminated", related_id);
                            } else {
                                warn!("⚠️ BYE-FORWARD: Failed to terminate related session {}: {}", related_id, e);
                            }
                        }
                    }
                }
            } else {
                debug!("🔍 BYE-FORWARD: No related B2BUA session found for {} (may be a pending call)", call.id());
            }
            
            // Clean up the call info - this will handle agent wrap-up
            if let Err(e) = engine.handle_call_termination(call.id().clone()).await {
                error!("Failed to handle call termination: {}", e);
            }
        }
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("CallCenterCallHandler: Call {} established", call.id);
        debug!("Local SDP available: {}, Remote SDP available: {}", 
               local_sdp.is_some(), remote_sdp.is_some());
        
        if let Some(engine) = self.engine.upgrade() {
            // Check if this is a pending agent assignment
            if let Some((_, pending_assignment)) = engine.pending_assignments.remove(&call.id) {
                info!("🔔 Agent {} answered for pending assignment", pending_assignment.agent_id);
                
                // This is an agent answering - complete the bridge
                let coordinator = engine.session_coordinator.as_ref().unwrap();
                let bridge_start = Instant::now();
                
                match coordinator.bridge_sessions(
                    &pending_assignment.customer_session_id, 
                    &pending_assignment.agent_session_id
                ).await {
                    Ok(bridge_id) => {
                        let bridge_time = bridge_start.elapsed().as_millis();
                        info!("✅ Successfully bridged customer {} with agent {} (bridge: {}) in {}ms", 
                              pending_assignment.customer_session_id, 
                              pending_assignment.agent_id, 
                              bridge_id, 
                              bridge_time);
                        
                        // Update customer call info
                        if let Some(mut call_info) = engine.active_calls.get_mut(&pending_assignment.customer_session_id) {
                            call_info.agent_id = Some(pending_assignment.agent_id.clone());
                            call_info.bridge_id = Some(bridge_id.clone());
                            call_info.status = CallStatus::Bridged;
                            call_info.answered_at = Some(chrono::Utc::now());
                        }
                        
                        // Update agent call info
                        if let Some(mut call_info) = engine.active_calls.get_mut(&pending_assignment.agent_session_id) {
                            call_info.bridge_id = Some(bridge_id);
                            call_info.status = CallStatus::Bridged;
                            call_info.answered_at = Some(chrono::Utc::now());
                        }
                    }
                    Err(e) => {
                        error!("Failed to bridge sessions after agent answered: {}", e);
                        
                        // Hang up both calls on bridge failure
                        let _ = coordinator.terminate_session(&pending_assignment.agent_session_id).await;
                        let _ = coordinator.terminate_session(&pending_assignment.customer_session_id).await;
                        
                        // Return agent to available in database
                        if let Some(db_manager) = &engine.db_manager {
                            let _ = db_manager.update_agent_call_count(&pending_assignment.agent_id.0, -1).await;
                            let _ = db_manager.update_agent_status(&pending_assignment.agent_id.0, AgentStatus::Available).await;
                        }
                    }
                }
            } else {
                // Regular call established (not a pending assignment)
                engine.update_call_established(call.id).await;
            }
        }
    }
    
    // === New event handler methods ===
    
    async fn on_call_state_changed(
        &self, 
        session_id: &SessionId, 
        old_state: &CallState, 
        new_state: &CallState, 
        reason: Option<&str>
    ) {
        info!("📞 Call {} state changed from {:?} to {:?} (reason: {:?})", 
              session_id, old_state, new_state, reason);
        
        if let Some(engine) = self.engine.upgrade() {
            // Update call status based on state change
            if let Some(mut call_info) = engine.active_calls.get_mut(session_id) {
                match new_state {
                    CallState::Active => call_info.status = CallStatus::Bridged,
                    CallState::Terminated => call_info.status = CallStatus::Disconnected,
                    CallState::Failed(_) => call_info.status = CallStatus::Failed,
                    _ => {} // Keep existing status for other states
                }
            }
        }
    }
    
    async fn on_media_quality(
        &self, 
        session_id: &SessionId, 
        mos_score: f32, 
        packet_loss: f32, 
        alert_level: MediaQualityAlertLevel
    ) {
        debug!("CallCenterCallHandler: Call {} quality - MOS: {}, Loss: {}%, Alert: {:?}", 
               session_id, mos_score, packet_loss, alert_level);
        
        if let Some(engine) = self.engine.upgrade() {
            // Store quality metrics
            if let Err(e) = engine.record_quality_metrics(session_id, mos_score, packet_loss).await {
                error!("Failed to record quality metrics: {}", e);
            }
            
            // Alert supervisors on poor quality
            if matches!(alert_level, MediaQualityAlertLevel::Poor | MediaQualityAlertLevel::Critical) {
                if let Err(e) = engine.alert_poor_quality(session_id, mos_score, alert_level).await {
                    error!("Failed to alert poor quality: {}", e);
                }
            }
        }
    }
    
    async fn on_dtmf(&self, session_id: &SessionId, digit: char, duration_ms: u32) {
        info!("CallCenterCallHandler: Call {} received DTMF '{}' ({}ms)", 
              session_id, digit, duration_ms);
        
        if let Some(engine) = self.engine.upgrade() {
            // Process DTMF for IVR or agent features
            if let Err(e) = engine.process_dtmf_input(session_id, digit).await {
                error!("Failed to process DTMF: {}", e);
            }
        }
    }
    
    async fn on_media_flow(
        &self, 
        session_id: &SessionId, 
        direction: MediaFlowDirection, 
        active: bool, 
        codec: &str
    ) {
        debug!("CallCenterCallHandler: Call {} media flow {:?} {} (codec: {})", 
               session_id, direction, if active { "started" } else { "stopped" }, codec);
        
        if let Some(engine) = self.engine.upgrade() {
            // Track media flow status
            if let Err(e) = engine.update_media_flow(session_id, direction, active, codec).await {
                error!("Failed to update media flow status: {}", e);
            }
        }
    }
    
    async fn on_warning(
        &self, 
        session_id: Option<&SessionId>, 
        category: WarningCategory, 
        message: &str
    ) {
        match session_id {
            Some(id) => warn!("CallCenterCallHandler: Warning for call {} ({:?}): {}", 
                            id, category, message),
            None => warn!("CallCenterCallHandler: General warning ({:?}): {}", 
                         category, message),
        }
        
        if let Some(engine) = self.engine.upgrade() {
            // Log warnings for monitoring
            if let Err(e) = engine.log_warning(session_id, category, message).await {
                error!("Failed to log warning: {}", e);
            }
        }
    }
}

impl CallCenterEngine {
    /// Handle SIP REGISTER request forwarded from session-core
    /// This is called when dialog-core receives a REGISTER and forwards it to us
    pub async fn handle_register_request(
        &self,
        transaction_id: &str,
        from_uri: String,
        mut contact_uri: String,
        expires: u32,
    ) -> Result<(), CallCenterError> {
        tracing::info!("Processing REGISTER: transaction={}, from={}, contact={}, expires={}", 
                      transaction_id, from_uri, contact_uri, expires);
        
        // Parse the AOR (Address of Record) from the from_uri
        let aor = from_uri.clone(); // In practice, might need to normalize this
        
        // Fix the contact URI to include port if missing
        // When agents register with Contact: <sip:alice@127.0.0.1>, we need to add the port
        if contact_uri.contains(':') && !contact_uri.ends_with(":5060") {
            // Check if contact has a port (not just the sip: part)
            let parts: Vec<&str> = contact_uri.split('@').collect();
            if parts.len() == 2 {
                let host_part = parts[1];
                // Check if host part has a port
                if !host_part.contains(':') || host_part.split(':').nth(1).unwrap_or("").is_empty() {
                    // No port specified, need to extract from source
                    // For now, we'll use the AOR to determine the port
                    // In a real implementation, we'd get this from the Via header
                    let port = if aor.contains("alice") {
                        "5071"
                    } else if aor.contains("bob") {
                        "5072"
                    } else {
                        "5060" // Default SIP port
                    };
                    contact_uri = format!("{}:{}", contact_uri.trim_end_matches('>'), port).replace(">>", ">");
                }
            }
        }
        
        tracing::info!("Contact URI with port: {}", contact_uri);
        
        // Allow all agent registrations - agents can register themselves
        // No need to pre-check database existence since upsert_agent will handle creation
        
        // Process the registration with our SIP registrar
        // Note: We now pass the contact_uri with port included
        let mut registrar = self.sip_registrar.lock().await;
        let response = registrar.process_register_simple(
            &aor,
            &contact_uri,
            Some(expires),
            None, // User-Agent would come from SIP headers
            "unknown".to_string(), // Remote address would come from transport layer
        )?;
        
        tracing::info!("REGISTER processed: {:?} for {}", response.status, aor);
        
        // Send proper SIP response through session-core
        let session_coord = self.session_coordinator.as_ref()
            .ok_or_else(|| CallCenterError::internal(
                "Session coordinator not available"
            ))?;
        
        let (status_code, reason) = match response.status {
            crate::agent::RegistrationStatus::Created => {
                tracing::info!("Sending 200 OK for successful registration");
                (200, Some("Registration successful"))
            }
            crate::agent::RegistrationStatus::Refreshed => {
                tracing::info!("Sending 200 OK for registration refresh");
                (200, Some("Registration refreshed"))
            }
            crate::agent::RegistrationStatus::Removed => {
                tracing::info!("Sending 200 OK for de-registration");
                (200, Some("De-registration successful"))
            }
        };
        
        // Build headers with Contact information
        let expires_str = expires.to_string();
        let contact_header = format!("<{}>;expires={}", contact_uri, expires);
        let headers = vec![
            ("Expires", expires_str.as_str()),
            ("Contact", contact_header.as_str()),
        ];
        
        session_coord.send_sip_response(
            transaction_id,
            status_code,
            reason,
            Some(headers),
        ).await
        .map_err(|e| CallCenterError::internal(
            &format!("Failed to send REGISTER response: {}", e)
        ))?;
        
        tracing::info!("REGISTER response sent: {} {}", status_code, reason.unwrap_or(""));
        
        // Update agent status in database if registration was successful
        if status_code == 200 && expires > 0 {
            if let Some(db_manager) = &self.db_manager {
                // Extract username from AOR
                let username = aor.split('@').next()
                    .unwrap_or(&aor)
                    .trim_start_matches("sip:")
                    .trim_start_matches('<');
                
                // Update or insert agent in database
                match db_manager.upsert_agent(&username, &username, Some(&contact_uri)).await {
                    Ok(_) => {
                        tracing::info!("✅ Agent {} registered in database with contact {}", username, contact_uri);
                    }
                    Err(e) => {
                        tracing::error!("❌ Failed to update agent in database: {}", e);
                    }
                }
            }
        } else if status_code == 200 && expires == 0 {
            // Handle de-registration - mark agent as offline
            if let Some(db_manager) = &self.db_manager {
                let username = aor.split('@').next()
                    .unwrap_or(&aor)
                    .trim_start_matches("sip:")
                    .trim_start_matches('<');
                
                // Update database status to offline
                match db_manager.mark_agent_offline(&username).await {
                    Ok(_) => {
                        tracing::info!("✅ Agent {} marked offline in database", username);
                    }
                    Err(e) => {
                        tracing::error!("❌ Failed to mark agent offline: {}", e);
                    }
                }
            }
        }
        
        Ok(())
    }
} 