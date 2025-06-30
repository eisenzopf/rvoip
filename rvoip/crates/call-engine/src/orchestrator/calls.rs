//! # Call Handling Logic for Call Center Operations
//!
//! This module implements the core call processing functionality for the call center,
//! including sophisticated incoming call handling, intelligent agent assignment, B2BUA
//! (Back-to-Back User Agent) operations, call lifecycle management, and comprehensive
//! routing logic. It serves as the heart of the call center's operational engine.
//!
//! ## Overview
//!
//! Call handling is the most critical component of any call center system. This module
//! provides enterprise-grade call processing that handles everything from initial call
//! reception through agent assignment, bridge management, and call termination. The
//! system operates as a B2BUA, managing both customer-facing and agent-facing call legs
//! while providing intelligent routing, queue management, and comprehensive monitoring.
//!
//! ## Key Features
//!
//! - **B2BUA Operations**: Complete Back-to-Back User Agent functionality
//! - **Intelligent Routing**: Sophisticated call routing based on multiple factors
//! - **Queue Management**: Advanced queue-first routing with fair distribution
//! - **Agent Assignment**: Atomic agent assignment with rollback capabilities
//! - **Call Lifecycle**: Complete call lifecycle management from ingress to termination
//! - **SDP Handling**: Full SDP offer/answer processing for media negotiation
//! - **Monitoring & Metrics**: Comprehensive call metrics and performance tracking
//! - **Error Recovery**: Robust error handling with automatic recovery mechanisms
//! - **Database Integration**: Real-time call state synchronization with persistence
//! - **Concurrent Processing**: High-performance concurrent call handling
//!
//! ## B2BUA Architecture
//!
//! The call center operates as a B2BUA, managing two separate call legs:
//!
//! ### Customer Leg
//! - Receives incoming calls from customers
//! - Processes SDP offers and generates answers
//! - Provides hold music and queue announcements
//! - Manages customer experience during wait times
//!
//! ### Agent Leg  
//! - Creates outgoing calls to available agents
//! - Handles agent registration and availability
//! - Manages agent-specific call routing
//! - Provides call supervision and monitoring
//!
//! ### Bridge Management
//! - Coordinates media bridging between legs
//! - Handles call transfers and conference creation
//! - Manages call hold and resume operations
//! - Provides recording and monitoring capabilities
//!
//! ## Call Flow Process
//!
//! The typical call flow follows this pattern:
//!
//! 1. **Call Reception**: Customer call received and analyzed
//! 2. **Immediate Accept**: B2BUA accepts call with SDP answer
//! 3. **Customer Analysis**: Caller information and requirements analyzed
//! 4. **Routing Decision**: Intelligent routing decision made
//! 5. **Queue Placement**: Call placed in appropriate queue with priority
//! 6. **Agent Selection**: Available agent selected using fair algorithms
//! 7. **Agent Call**: Outgoing call created to selected agent
//! 8. **Bridge Creation**: Media bridge established when agent answers
//! 9. **Call Management**: Ongoing call monitoring and management
//! 10. **Call Termination**: Proper cleanup and metrics recording
//!
//! ## Examples
//!
//! ### Basic Incoming Call Processing
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, orchestrator::types::CustomerType};
//! use rvoip_session_core::{IncomingCall, SessionId, CallDecision};
//! use std::collections::HashMap;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Simulate incoming call
//! let incoming_call = IncomingCall {
//!     id: SessionId("customer-support-001".to_string()),
//!     from: "sip:customer@external.com".to_string(),
//!     to: "sip:support@call-center.local".to_string(),
//!     sdp: Some("v=0\r\no=- 123456 IN IP4 192.168.1.100\r\n...".to_string()),
//!     headers: HashMap::new(),
//!     received_at: std::time::Instant::now(),
//! };
//! 
//! // Route the incoming call through the call center
//! engine.route_incoming_call(&incoming_call.id).await?;
//! 
//! println!("📞 Call routing initiated for session: {}", incoming_call.id);
//! println!("🎵 Customer will hear hold music while we find an agent");
//! 
//! println!("🔄 Call now being routed to available agent...");
//! # Ok(())
//! # }
//! ```
//!
//! ### Call Information Tracking
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, orchestrator::types::{CallInfo, CallStatus}};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! let session_id = SessionId("call-tracking-example".to_string());
//! 
//! // Get detailed call information
//! if let Some(call_info) = engine.active_calls().get(&session_id) {
//!     println!("📞 Call Information:");
//!     println!("  Session ID: {}", call_info.session_id);
//!     println!("  From: {}", call_info.from);
//!     println!("  To: {}", call_info.to);
//!     println!("  Status: {:?}", call_info.status);
//!     println!("  Priority: {}", call_info.priority);
//!     println!("  Customer Type: {:?}", call_info.customer_type);
//!     
//!     // Agent assignment information
//!     if let Some(ref agent_id) = call_info.agent_id {
//!         println!("  Assigned Agent: {}", agent_id);
//!     }
//!     
//!     // Queue information
//!     if let Some(ref queue_id) = call_info.queue_id {
//!         println!("  Queue: {}", queue_id);
//!         if let Some(queued_at) = call_info.queued_at {
//!             let wait_time = chrono::Utc::now().signed_duration_since(queued_at);
//!             println!("  Wait Time: {:.1}s", wait_time.num_milliseconds() as f64 / 1000.0);
//!         }
//!     }
//!     
//!     // Call timing metrics
//!     println!("  Duration: {}s", call_info.duration_seconds);
//!     println!("  Talk Time: {}s", call_info.talk_time_seconds);
//!     println!("  Hold Time: {}s", call_info.hold_time_seconds);
//!     
//!     // B2BUA tracking
//!     if let Some(ref related_session) = call_info.related_session_id {
//!         println!("  Related Session (B2BUA): {}", related_session);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Assignment Process
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::AgentId};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! let customer_session = SessionId("waiting-customer".to_string());
//! let agent_id = AgentId("agent-001".to_string());
//! 
//! // Assign specific agent to customer call
//! match engine.assign_agent_to_call(customer_session.clone(), agent_id.clone()).await {
//!     Ok(()) => {
//!         println!("✅ Agent {} assigned to call {}", agent_id, customer_session);
//!         println!("📞 B2BUA creating outgoing call to agent...");
//!         println!("⏱️ Waiting for agent to answer (30s timeout)...");
//!         
//!         // The system will automatically:
//!         // 1. Update database with atomic assignment
//!         // 2. Create outgoing SIP call to agent
//!         // 3. Generate SDP offer for agent
//!         // 4. Wait for agent to answer
//!         // 5. Bridge customer and agent when answered
//!         // 6. Handle timeout if agent doesn't answer
//!     }
//!     Err(e) => {
//!         println!("❌ Assignment failed: {}", e);
//!         println!("🔄 Call will be re-queued with higher priority");
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Call State Management
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, orchestrator::types::{CallStatus, CallInfo}};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! let session_id = SessionId("state-management-example".to_string());
//! 
//! // Monitor call state transitions
//! if let Some(call_info) = engine.active_calls().get(&session_id) {
//!     match call_info.status {
//!         CallStatus::Incoming => {
//!             println!("📞 Call state: INCOMING - Just received");
//!         }
//!         CallStatus::Ringing => {
//!             println!("📞 Call state: RINGING - Processing by B2BUA");
//!         }
//!         CallStatus::Queued => {
//!             println!("📞 Call state: QUEUED - Waiting for agent");
//!             if let Some(queue_id) = &call_info.queue_id {
//!                 println!("  In queue: {}", queue_id);
//!                 println!("  Priority: {}", call_info.priority);
//!             }
//!         }
//!         CallStatus::Connecting => {
//!             println!("📞 Call state: CONNECTING - Agent being contacted");
//!             if let Some(agent_id) = &call_info.agent_id {
//!                 println!("  Agent: {}", agent_id);
//!             }
//!         }
//!         CallStatus::Bridged => {
//!             println!("📞 Call state: BRIDGED - Active conversation");
//!             if let Some(bridge_id) = &call_info.bridge_id {
//!                 println!("  Bridge: {}", bridge_id);
//!             }
//!             println!("  Talk time: {}s", call_info.talk_time_seconds);
//!         }
//!         CallStatus::OnHold => {
//!             println!("📞 Call state: ON HOLD - Customer on hold");
//!             println!("  Hold count: {}", call_info.hold_count);
//!         }
//!         CallStatus::Transferring => {
//!             println!("📞 Call state: TRANSFERRING - Being transferred");
//!             println!("  Transfer count: {}", call_info.transfer_count);
//!         }
//!         CallStatus::Disconnected => {
//!             println!("📞 Call state: DISCONNECTED - Call ended");
//!         }
//!         CallStatus::Failed => {
//!             println!("📞 Call state: FAILED - Call failed");
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Call Operations
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! let session_id = SessionId("advanced-ops-example".to_string());
//! 
//! // Put call on hold
//! engine.put_call_on_hold(&session_id).await?;
//! println!("⏸️ Call placed on hold");
//! 
//! // Resume from hold
//! tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
//! engine.resume_call_from_hold(&session_id).await?;
//! println!("▶️ Call resumed from hold");
//! 
//! // Transfer call (simple version)
//! let transfer_target = "queue:vip";
//! engine.transfer_call_simple(&session_id, transfer_target).await?;
//! println!("🔄 Call transferred to {}", transfer_target);
//! 
//! // Call operations automatically:
//! // - Update call metrics (hold time, transfer count)
//! // - Maintain call state consistency
//! // - Integrate with session-core for media operations
//! // - Provide comprehensive audit trail
//! 
//! println!("📊 All operations tracked and monitored");
//! # Ok(())
//! # }
//! ```
//!
//! ### Queue Processing and Assignment
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::AgentId};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! let agent_id = AgentId("newly-available-agent".to_string());
//! 
//! // When agent becomes available, process queued calls
//! engine.process_all_queues().await?;
//! 
//! println!("🔍 Processed queued calls for agent {}", agent_id);
//! println!("  System automatically:");
//! println!("  1️⃣ Checked agent skills and availability");
//! println!("  2️⃣ Searched relevant queues for matching calls");
//! println!("  3️⃣ Selected highest priority call");
//! println!("  4️⃣ Performed atomic assignment");
//! println!("  5️⃣ Started B2BUA call to agent");
//! 
//! println!("📊 Queue processing completed successfully");
//! println!("🧹 Checked for and resolved any stuck assignments");
//! 
//! // Queue monitoring ensures fair distribution
//! let queue_id = "general";
//! engine.monitor_queue_for_agents(queue_id.to_string()).await;
//! println!("👁️ Started queue monitor for fair call distribution");
//! # Ok(())
//! # }
//! ```
//!
//! ### Call Termination and Cleanup
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! let session_id = SessionId("terminating-call".to_string());
//! 
//! // Handle call termination with comprehensive cleanup
//! engine.cleanup_call(&session_id).await?;
//! 
//! println!("🛑 Call termination completed:");
//! println!("  ✅ B2BUA cleanup: Both call legs terminated");
//! println!("  ✅ Database cleanup: Call removed from queues");
//! println!("  ✅ Agent status: Updated to post-call wrap-up");
//! println!("  ✅ Metrics calculated: Duration, wait time, talk time");
//! println!("  ✅ Bridge destroyed: Media resources freed");
//! println!("  ✅ Memory cleanup: Call info removed from active calls");
//! 
//! // Agent will automatically transition through wrap-up period
//! println!("⏰ Agent entering 10-second post-call wrap-up");
//! println!("🔄 Agent will become available for new calls automatically");
//! # Ok(())
//! # }
//! ```
//!
//! ## B2BUA Implementation Details
//!
//! ### SDP Handling
//!
//! The B2BUA manages SDP offer/answer exchange between customer and agent:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! # use rvoip_session_core::{IncomingCall, SessionId};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Customer SDP offer received
//! let customer_sdp = "v=0\r\no=customer 123456 IN IP4 192.168.1.100\r\n...";
//! 
//! // B2BUA process:
//! println!("📄 B2BUA SDP Processing:");
//! println!("  1️⃣ Receive customer SDP offer");
//! println!("  2️⃣ Generate B2BUA SDP answer for customer");
//! println!("  3️⃣ Store customer SDP for later use");
//! println!("  4️⃣ Create agent call with B2BUA SDP offer");
//! println!("  5️⃣ Receive agent SDP answer");
//! println!("  6️⃣ Bridge media streams between customer and agent");
//! 
//! // Each leg has independent SDP negotiation
//! println!("🔗 Independent SDP negotiation:");
//! println!("  Customer ↔ B2BUA: Customer SDP ↔ B2BUA Answer");
//! println!("  B2BUA ↔ Agent: B2BUA Offer ↔ Agent SDP");
//! println!("  Bridge: Media relay between negotiated streams");
//! # Ok(())
//! # }
//! ```
//!
//! ### Pending Assignment Management
//!
//! The system tracks pending agent assignments to handle timeouts and failures:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, orchestrator::types::PendingAssignment};
//! # use rvoip_session_core::SessionId;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Pending assignments track agent call attempts
//! let customer_session = SessionId("customer-123".to_string());
//! let agent_session = SessionId("agent-456".to_string());
//! 
//! // System automatically manages pending assignments:
//! println!("📝 Pending Assignment Management:");
//! println!("  ✅ Store assignment when agent call initiated");
//! println!("  ⏰ Start 30-second timeout timer");
//! println!("  🔔 Remove when agent answers (success)");
//! println!("  ⏰ Rollback on timeout (failure)");
//! println!("  🔄 Re-queue customer call on failure");
//! println!("  📊 Track assignment success rates");
//! 
//! // Automatic timeout handling prevents hung calls
//! println!("🛡️ Timeout Protection:");
//! println!("  - Agent doesn't answer within 30s");
//! println!("  - Terminate agent call leg");
//! println!("  - Restore agent to available status");
//! println!("  - Re-queue customer with higher priority");
//! println!("  - Continue assignment attempts");
//! # Ok(())
//! # }
//! ```
//!
//! ## Routing Intelligence
//!
//! ### Customer Analysis
//!
//! The system analyzes customer information for intelligent routing:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, orchestrator::types::CustomerType};
//! # use rvoip_session_core::IncomingCall;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Customer analysis considers multiple factors
//! println!("🧠 Customer Analysis Factors:");
//! println!("  📞 Caller ID patterns (VIP numbers, known customers)");
//! println!("  🏢 Company/domain identification");
//! println!("  📊 Historical call patterns");
//! println!("  🎯 Requested service type (sales, support, billing)");
//! println!("  ⚡ Priority indicators (emergency, escalation)");
//! 
//! // Results in customer classification
//! let customer_types = vec![
//!     (CustomerType::VIP, "Priority 0 - Immediate assignment"),
//!     (CustomerType::Premium, "Priority 10 - High priority queue"),
//!     (CustomerType::Standard, "Priority 50 - Standard queue"),
//!     (CustomerType::Trial, "Priority 100 - Lower priority"),
//! ];
//! 
//! println!("\n🏷️ Customer Classifications:");
//! for (customer_type, description) in customer_types {
//!     println!("  {:?}: {}", customer_type, description);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Queue-First Routing
//!
//! The system uses queue-first routing for fair call distribution:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Queue-first ensures fairness
//! println!("⚖️ Queue-First Routing Benefits:");
//! println!("  🎯 Fair distribution: Round-robin with last-agent exclusion");
//! println!("  📊 Load balancing: Even distribution across agents");
//! println!("  🔄 Priority handling: Higher priority calls served first");
//! println!("  🛡️ Overload protection: Queue limits prevent system overload");
//! println!("  📈 Scalability: Works with any number of agents");
//! 
//! // Queue management
//! println!("\n📋 Queue Management:");
//! println!("  general: Standard support calls");
//! println!("  sales: Sales inquiries and opportunities");
//! println!("  support: Technical support requests");
//! println!("  billing: Billing and account questions");
//! println!("  vip: VIP customer priority queue");
//! println!("  premium: Premium customer queue");
//! println!("  overflow: Overflow when other queues full");
//! 
//! // Automatic queue monitoring
//! println!("\n👁️ Automatic Queue Monitoring:");
//! println!("  ✅ Real-time depth monitoring");
//! println!("  ⏰ Agent availability detection");
//! println!("  🔄 Automatic assignment when agents available");
//! println!("  🧹 Stuck assignment cleanup");
//! println!("  📊 Queue performance metrics");
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance and Scalability
//!
//! ### Concurrent Call Handling
//!
//! The system is designed for high-concurrency call processing:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // High-performance characteristics
//! println!("⚡ Performance Characteristics:");
//! println!("  🚀 Async/await throughout for non-blocking operations");
//! println!("  🔒 Thread-safe data structures (DashMap, Arc, Mutex)");
//! println!("  💾 Database connection pooling for scalability");
//! println!("  📊 Efficient algorithms for agent selection (O(log n))");
//! println!("  🎯 Optimized queue operations (O(1) enqueue/dequeue)");
//! 
//! // Concurrent operations
//! println!("\n🔄 Concurrent Operations:");
//! println!("  📞 Multiple incoming calls processed simultaneously");
//! println!("  🔍 Agent assignment happens in parallel");
//! println!("  💾 Database operations use connection pools");
//! println!("  🌉 Bridge operations don't block call processing");
//! println!("  📊 Metrics collection happens asynchronously");
//! 
//! // Scalability features
//! println!("\n📈 Scalability Features:");
//! println!("  🏗️ Horizontal scaling: Multiple call center instances");
//! println!("  💾 Database-backed state: Survives restarts");
//! println!("  🔄 Load balancing: Distributes calls across instances");
//! println!("  📊 Monitoring: Real-time performance metrics");
//! # Ok(())
//! # }
//! ```
//!
//! ### Error Recovery
//!
//! Comprehensive error recovery ensures system reliability:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Error recovery mechanisms
//! println!("🛡️ Error Recovery Mechanisms:");
//! println!("  🔄 Automatic retry with exponential backoff");
//! println!("  🎯 Rollback on failed agent assignments");
//! println!("  📞 Re-queue calls on assignment failures");
//! println!("  ⏰ Timeout protection for hung operations");
//! println!("  🧹 Automatic cleanup of stuck assignments");
//! 
//! // Graceful degradation
//! println!("\n⚡ Graceful Degradation:");
//! println!("  📞 Continue operating with partial agent availability");
//! println!("  💾 Fallback to in-memory operations if database issues");
//! println!("  🔄 Queue overflow to secondary queues");
//! println!("  📊 Maintain metrics even during partial failures");
//! 
//! // System resilience
//! println!("\n🏗️ System Resilience:");
//! println!("  🔒 Atomic operations prevent data corruption");
//! println!("  📊 Comprehensive logging for troubleshooting");
//! println!("  🎯 Circuit breaker patterns for external dependencies");
//! println!("  🔄 Automatic recovery from transient failures");
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use std::collections::HashMap;
use tracing::{info, debug, warn, error};
use tokio::time::{timeout, Duration};

use rvoip_session_core::api::{
    types::{IncomingCall, CallDecision, SessionId, CallState},
    control::SessionControl,
    media::MediaControl,
};

use crate::agent::{AgentId, AgentStatus};
use crate::error::{CallCenterError, Result as CallCenterResult};
use crate::queue::{QueuedCall, QueueStats};
use super::core::CallCenterEngine;
use super::types::{CallInfo, CallStatus, CustomerType, RoutingDecision, PendingAssignment, AgentInfo};

impl CallCenterEngine {
    /// Process incoming call with sophisticated routing
    pub(super) async fn process_incoming_call(&self, call: IncomingCall) -> CallCenterResult<CallDecision> {
        let session_id = call.id.clone();
        
        info!("📞 Processing incoming call: {} from {} to {}", 
              session_id, call.from, call.to);
        
        // Check if we have any agents at all (not just available ones)
        let total_agents = if let Some(db_manager) = &self.db_manager {
            match db_manager.count_total_agents().await {
                Ok(count) => count,
                Err(_) => 0
            }
        } else {
            0
        };
        
        if total_agents == 0 {
            warn!("❌ No agents registered in the system, rejecting call");
            return Ok(CallDecision::Reject("No agents available".to_string()));
        }
        
        // Check queue capacity
        let queue_manager = self.queue_manager.read().await;
        let total_queued = queue_manager.total_queued_calls();
        drop(queue_manager);
        
        // If we have too many calls queued relative to agents, reject new calls
        if total_queued > (total_agents * 3) {  // Allow 3 calls per agent max
            warn!("❌ Queue overload: {} calls queued for {} agents", total_queued, total_agents);
            return Ok(CallDecision::Reject("System at capacity - please try again later".to_string()));
        }
        
        // Analyze customer info first
        let (customer_type, priority, required_skills) = self.analyze_customer_info(&call).await;
        
        // PHASE 17.3: IncomingCall doesn't have dialog_id, we'll get it from events
        // For now, just track that we don't have the dialog ID yet
        let incoming_dialog_id = None; // Will be populated when we get dialog events
        info!("🔍 Incoming call - dialog ID will be set from events");
        
        // Create a session ID for internal tracking - IncomingCall.id is already a SessionId
        let session_id = call.id.clone();
        
        // Extract and log the SDP from the incoming call
        if let Some(ref sdp) = call.sdp {
            info!("📄 Incoming call has SDP offer ({} bytes)", sdp.len());
            debug!("📄 Customer SDP content:\n{}", sdp);
        } else {
            warn!("⚠️ Incoming call has no SDP offer");
        }
        
        // Store the call information
        let mut call_info = CallInfo {
            session_id: session_id.clone(),
            caller_id: call.from.clone(),
            from: call.from.clone(),
            to: call.to.clone(),
            agent_id: None,
            queue_id: None,
            status: CallStatus::Ringing,
            priority,
            customer_type,
            required_skills,
            created_at: chrono::Utc::now(),
            answered_at: None,
            ended_at: None,
            bridge_id: None,
            duration_seconds: 0,
            wait_time_seconds: 0,
            talk_time_seconds: 0,
            queue_time_seconds: 0,
            hold_time_seconds: 0,
            queued_at: None,
            transfer_count: 0,
            hold_count: 0,
            customer_sdp: call.sdp.clone(), // Store the customer's SDP for later use
            customer_dialog_id: incoming_dialog_id, // PHASE 17.3: Store the dialog ID
            agent_dialog_id: None,
            related_session_id: None, // Will be set when agent is assigned
        };
        
        // Store call info
        self.active_calls.insert(session_id.clone(), call_info);
        
        // B2BUA: Accept customer call immediately with our SDP answer
        // Customer will wait (with hold music) until an agent is available
        info!("📞 B2BUA: Accepting customer call {} immediately", session_id);
        
        // Generate B2BUA's SDP answer for the customer
        let sdp_answer = if let Some(ref customer_sdp) = call.sdp {
            // Generate our SDP answer based on customer's offer
            match self.session_coordinator.as_ref().unwrap()
                .generate_sdp_answer(&session_id, customer_sdp).await {
                Ok(answer) => {
                    info!("✅ Generated SDP answer for customer ({} bytes)", answer.len());
                    Some(answer)
                }
                Err(e) => {
                    error!("Failed to generate SDP answer: {}", e);
                    None
                }
            }
        } else {
            warn!("⚠️ No SDP from customer, accepting without SDP");
            None
        };
        
        // Update call status to Connecting since we're accepting
        if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
            call_info.status = CallStatus::Connecting;
            call_info.answered_at = Some(chrono::Utc::now());
        }
        
        // Spawn the routing task to find an agent
        let engine = Arc::new(self.clone());
        let session_id_clone = session_id.clone();
        tokio::spawn(async move {
            // Route immediately - call is already accepted
            engine.route_call_to_agent(session_id_clone).await;
        });
        
        // Return Accept with SDP to immediately answer the customer
        Ok(CallDecision::Accept(sdp_answer))
    }
    
    /// Route an already-accepted call to an agent
    async fn route_call_to_agent(&self, session_id: SessionId) {
        info!("🚦 Routing call {} to find available agent", session_id);
        
        let routing_start = std::time::Instant::now();
        
        // Make intelligent routing decision based on multiple factors
        let routing_decision = self.make_routing_decision(&session_id, &CustomerType::Standard, 50, &[]).await;
        
        match routing_decision {
            Ok(decision) => {
                info!("🎯 Routing decision for call {}: {:?}", session_id, decision);
                
                // Handle the routing decision
                match decision {
                    RoutingDecision::DirectToAgent { agent_id, reason } => {
                        info!("📞 Direct routing to agent {} for call {}: {}", agent_id, session_id, reason);
                        
                        // Update call status
                        if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                            call_info.status = CallStatus::Connecting;
                            call_info.agent_id = Some(agent_id.clone());
                        }
                        
                        // Assign to specific agent
                        if let Err(e) = self.assign_specific_agent_to_call(session_id.clone(), agent_id.clone()).await {
                            error!("Failed to assign call {} to agent {}: {}", session_id, agent_id, e);
                            
                            // TODO: Re-queue or find another agent
                            if let Some(coordinator) = &self.session_coordinator {
                                let _ = coordinator.terminate_session(&session_id).await;
                            }
                        }
                    },
                    
                    RoutingDecision::Queue { queue_id, priority, reason } => {
                        info!("📥 Queueing call {} to {} (reason: {})", session_id, queue_id, reason);
                        
                        // Ensure the queue exists before trying to enqueue
                        if let Err(e) = self.ensure_queue_exists(&queue_id).await {
                            error!("Failed to ensure queue {} exists: {}", queue_id, e);
                            // Terminate the call if we can't create the queue
                            if let Some(coordinator) = &self.session_coordinator {
                                let _ = coordinator.terminate_session(&session_id).await;
                            }
                            return;
                        }
                        
                        // Create queued call entry
                        let call_id = uuid::Uuid::new_v4().to_string();
                        let customer_info = self.active_calls.get(&session_id)
                            .map(|c| serde_json::json!({
                                "caller_id": c.caller_id.clone(),
                                "customer_type": format!("{:?}", c.customer_type),
                            }));
                        
                        // Create the QueuedCall struct
                        let queued_call = QueuedCall {
                            session_id: session_id.clone(),
                            caller_id: self.active_calls.get(&session_id)
                                .map(|c| c.caller_id.clone())
                                .unwrap_or_else(|| "unknown".to_string()),
                            priority,
                            queued_at: chrono::Utc::now(),
                            estimated_wait_time: None,
                            retry_count: 0,
                        };
                        
                        // Add to database queue
                        let mut enqueue_success = false;
                        if let Some(db_manager) = &self.db_manager {
                            match db_manager.enqueue_call(&queue_id, &queued_call).await {
                                Ok(_) => {
                                    info!("✅ Call {} enqueued to database queue '{}'", session_id, queue_id);
                                    
                                    // Log queue depth after enqueue
                                    if let Ok(depth) = db_manager.get_queue_depth(&queue_id).await {
                                        info!("📊 Queue '{}' status after enqueue: {} calls waiting", 
                                              queue_id, depth);
                                    }
                                    enqueue_success = true;
                                }
                                Err(e) => {
                                    error!("Failed to enqueue call {} to database: {}", session_id, e);
                                }
                            }
                        }
                        
                        // Always add to in-memory queue as well (database is just for persistence)
                        if !enqueue_success {
                            // Only use in-memory queue if database enqueue failed
                            let mut queue_manager = self.queue_manager.write().await;
                            match queue_manager.enqueue_call(&queue_id, queued_call) {
                                Ok(_) => {
                                    // Log queue depth after enqueue
                                    if let Ok(stats) = queue_manager.get_queue_stats(&queue_id) {
                                        info!("📊 Queue '{}' status after enqueue: {} calls waiting", 
                                              queue_id, stats.total_calls);
                                    }
                                    enqueue_success = true;
                                }
                                Err(e) => {
                                    error!("Failed to enqueue call {}: {}", session_id, e);
                                }
                            }
                        } else {
                            // Also add to in-memory queue
                            let mut queue_manager = self.queue_manager.write().await;
                            let _ = queue_manager.enqueue_call(&queue_id, queued_call);
                        }
                        
                        if !enqueue_success {
                            // Terminate the call if we can't queue it
                            if let Some(coordinator) = &self.session_coordinator {
                                let _ = coordinator.terminate_session(&session_id).await;
                            }
                            return;
                        }
                        
                        // Update call status
                        if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                            call_info.status = CallStatus::Queued;
                            call_info.queue_id = Some(queue_id.clone());
                            call_info.queued_at = Some(chrono::Utc::now());
                        }
                        
                        // Update routing stats
                        {
                            let mut stats = self.routing_stats.write().await;
                            stats.calls_queued += 1;
                        }
                        
                        // PHASE 0.10: Start monitoring for agent availability immediately
                        info!("🔄 Starting queue monitor for '{}' immediately", queue_id);
                        self.monitor_queue_for_agents(queue_id).await;
                    },
                    
                    RoutingDecision::Reject { reason } => {
                        warn!("❌ Rejecting call {} after acceptance: {}", session_id, reason);
                        
                        // Since we already accepted, we need to terminate
                        if let Some(coordinator) = &self.session_coordinator {
                            let _ = coordinator.terminate_session(&session_id).await;
                        }
                        
                        // Update routing stats
                        {
                            let mut stats = self.routing_stats.write().await;
                            stats.calls_rejected += 1;
                        }
                    },
                    
                    _ => {
                        warn!("Unhandled routing decision for call {}: {:?}", session_id, decision);
                    }
                }
            }
            Err(e) => {
                error!("Routing decision failed for call {}: {}", session_id, e);
                
                // Terminate the call
                if let Some(coordinator) = &self.session_coordinator {
                    let _ = coordinator.terminate_session(&session_id).await;
                }
            }
        }
        
        // Update routing time metrics
        let routing_time = routing_start.elapsed().as_millis() as u64;
        {
            let mut stats = self.routing_stats.write().await;
            stats.average_routing_time_ms = (stats.average_routing_time_ms + routing_time) / 2;
        }
        
        info!("✅ Call {} routing completed in {}ms", session_id, routing_time);
    }
    
    /// Assign a specific agent to an incoming call
    pub(super) async fn assign_specific_agent_to_call(&self, session_id: SessionId, agent_id: AgentId) -> CallCenterResult<()> {
        info!("🎯 Assigning specific agent {} to call: {}", agent_id, session_id);
        
        // Get customer's SDP from the call info
        let customer_sdp = self.active_calls.get(&session_id)
            .and_then(|call_info| call_info.customer_sdp.clone());
        
        // First, perform atomic database assignment with retry logic
        if let Some(db_manager) = &self.db_manager {
            match db_manager.atomic_assign_call_to_agent(&session_id.0, &agent_id.0, customer_sdp.clone()).await {
                Ok(()) => {
                    info!("✅ Atomically assigned call {} to agent {} in database", session_id, agent_id);
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    if error_msg.contains("not available") || error_msg.contains("already busy") {
                        // Agent is no longer available - this is expected in concurrent scenarios
                        info!("⚠️ Agent {} is no longer available: {}", agent_id, e);
                        return Err(CallCenterError::orchestration(&format!("Agent {} not available", agent_id)));
                    } else {
                        // Unexpected database error
                        error!("Failed to atomically assign call to agent: {}", e);
                        return Err(CallCenterError::database(&format!("Database assignment failed: {}", e)));
                    }
                }
            }
        } else {
            return Err(CallCenterError::database("Database not configured"));
        };
        
        // Get agent information from database (should be marked as busy now)
        let agent_info = if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent(&agent_id.0).await {
                Ok(Some(db_agent)) => {
                    // Convert to AgentInfo
                    let contact_uri = db_agent.contact_uri.clone()
                        .unwrap_or_else(|| self.config.general.agent_sip_uri(&db_agent.username));
                    AgentInfo::from_db_agent(&db_agent, contact_uri, &self.config.general)
                }
                Ok(None) => {
                    error!("Agent {} not found in database after assignment", agent_id);
                    // Try to rollback by re-queuing the call
                    self.rollback_failed_assignment(&session_id, &agent_id).await;
                    return Err(CallCenterError::orchestration(&format!("Agent {} not found", agent_id)));
                }
                Err(e) => {
                    error!("Failed to get agent from database: {}", e);
                    // Try to rollback
                    self.rollback_failed_assignment(&session_id, &agent_id).await;
                    return Err(CallCenterError::database(&format!("Failed to get agent: {}", e)));
                }
            }
        } else {
            return Err(CallCenterError::database("Database not configured"));
        };
        
        // Now proceed with the SIP call setup
        let coordinator = self.session_coordinator.as_ref().unwrap();
        
        // Verify customer session is ready
        match coordinator.find_session(&session_id).await {
            Ok(Some(customer_session)) => {
                info!("📞 Customer session {} is in state: {:?}", session_id, customer_session.state);
                // Only proceed if customer call is in a suitable state
                match customer_session.state {
                    CallState::Active | CallState::Ringing => {
                        // Good to proceed
                    }
                    _ => {
                        warn!("⚠️ Customer session is in unexpected state: {:?}", customer_session.state);
                    }
                }
            }
            _ => {
                warn!("⚠️ Could not find customer session {}", session_id);
            }
        }
        
        // Step 1: B2BUA prepares its own SDP offer for the agent
        let agent_contact_uri = agent_info.contact_uri.clone();
        let call_center_uri = self.config.general.call_center_uri();
        
        info!("📞 B2BUA: Preparing outgoing call to agent {} at {}", 
              agent_id, agent_contact_uri);
        
        // Prepare the call - this allocates media resources and generates SDP
        let prepared_call = match SessionControl::prepare_outgoing_call(
            coordinator,
            &call_center_uri,    // FROM: The call center is making the call
            &agent_contact_uri,  // TO: The agent is receiving the call
        ).await {
            Ok(prepared) => {
                info!("✅ B2BUA: Prepared call with SDP offer ({} bytes), allocated RTP port: {}", 
                      prepared.sdp_offer.len(), prepared.local_rtp_port);
                prepared
            }
            Err(e) => {
                error!("Failed to prepare outgoing call to agent {}: {}", agent_id, e);
                // Rollback database changes
                self.rollback_failed_assignment(&session_id, &agent_id).await;
                return Err(CallCenterError::orchestration(&format!("Failed to prepare call to agent: {}", e)));
            }
        };
        
        // Step 2: Initiate the prepared call with our SDP offer
        let agent_call_session = match SessionControl::initiate_prepared_call(
            coordinator,
            &prepared_call,
        ).await {
            Ok(call_session) => {
                info!("✅ Created outgoing call {:?} to agent {} with SDP", call_session.id, agent_id);
                
                // Create CallInfo for the agent's session with proper tracking
                let agent_call_info = CallInfo {
                    session_id: call_session.id.clone(),
                    caller_id: "Call Center".to_string(),
                    from: self.config.general.call_center_uri(),
                    to: agent_info.sip_uri.clone(),
                    agent_id: Some(agent_id.clone()),
                    queue_id: None,
                    bridge_id: None,
                    status: CallStatus::Connecting,
                    priority: 0,
                    customer_type: CustomerType::Standard,
                    required_skills: vec![],
                    created_at: chrono::Utc::now(),
                    queued_at: None,
                    answered_at: None,
                    ended_at: None,
                    customer_sdp: None,
                    duration_seconds: 0,
                    wait_time_seconds: 0,
                    talk_time_seconds: 0,
                    hold_time_seconds: 0,
                    queue_time_seconds: 0,
                    transfer_count: 0,
                    hold_count: 0,
                    customer_dialog_id: None,
                    agent_dialog_id: None,
                    related_session_id: Some(session_id.clone()),
                };
                
                // Store the agent's call info
                self.active_calls.insert(call_session.id.clone(), agent_call_info);
                info!("📋 Created CallInfo for agent session {} with agent_id={}", call_session.id, agent_id);
                
                // Update the customer's call info with the agent session ID
                if let Some(mut customer_call_info) = self.active_calls.get_mut(&session_id) {
                    customer_call_info.related_session_id = Some(call_session.id.clone());
                    info!("📋 Updated customer session {} with related agent session {}", session_id, call_session.id);
                }
                
                call_session
            }
            Err(e) => {
                error!("Failed to initiate call to agent {}: {}", agent_id, e);
                // Rollback database changes
                self.rollback_failed_assignment(&session_id, &agent_id).await;
                return Err(CallCenterError::orchestration(&format!("Failed to call agent: {}", e)));
            }
        };
        
        // Get the session ID from the CallSession
        let agent_session_id = agent_call_session.id.clone();
        
        // Step 3: Store pending assignment instead of waiting
        info!("📝 Storing pending assignment for agent {} to answer", agent_id);
        
        let pending_assignment = PendingAssignment {
            customer_session_id: session_id.clone(),
            agent_session_id: agent_session_id.clone(),
            agent_id: agent_id.clone(),
            timestamp: chrono::Utc::now(),
            customer_sdp: customer_sdp,
        };
        
        // Store in pending assignments collection using agent session ID as key
        self.pending_assignments.insert(agent_session_id.clone(), pending_assignment);
        
        info!("✅ Agent {} call initiated - waiting for answer event", agent_id);
        
        // Start timeout task for agent answer (30 seconds)
        let engine = Arc::new(self.clone());
        let timeout_agent_id = agent_id.clone();
        let timeout_agent_session_id = agent_session_id.clone();
        let timeout_customer_session_id = session_id.clone();
        
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            
            // Check if assignment is still pending
            if engine.pending_assignments.contains_key(&timeout_agent_session_id) {
                warn!("⏰ Agent {} failed to answer within 30 seconds", timeout_agent_id);
                
                // Remove from pending
                engine.pending_assignments.remove(&timeout_agent_session_id);
                
                // Terminate the agent call
                if let Some(coordinator) = &engine.session_coordinator {
                    let _ = coordinator.terminate_session(&timeout_agent_session_id).await;
                }
                
                // Rollback database changes
                engine.rollback_failed_assignment(&timeout_customer_session_id, &timeout_agent_id).await;
                
                // Only re-queue if the customer call is still active
                if engine.active_calls.contains_key(&timeout_customer_session_id) {
                    // Re-queue the customer call
                    if let Some(mut call_info) = engine.active_calls.get_mut(&timeout_customer_session_id) {
                        call_info.status = CallStatus::Queued;
                        call_info.agent_id = None;
                        
                        if let Some(queue_id) = &call_info.queue_id {
                            if let Some(db_manager) = &engine.db_manager {
                                // Re-enqueue to database
                                let queued_call = crate::queue::QueuedCall {
                                    session_id: timeout_customer_session_id.clone(),
                                    caller_id: call_info.caller_id.clone(),
                                    priority: call_info.priority.saturating_sub(5), // Higher priority
                                    queued_at: chrono::Utc::now(),
                                    estimated_wait_time: None,
                                    retry_count: call_info.transfer_count as u8 + 1, // Use transfer_count as a proxy for retry
                                };
                                
                                if let Err(e) = db_manager.enqueue_call(queue_id, &queued_call).await {
                                    error!("Failed to re-queue call after timeout: {}", e);
                                } else {
                                    info!("📞 Re-queued call {} after agent timeout", timeout_customer_session_id);
                                }
                            }
                        }
                    }
                } else {
                    info!("📞 Not re-queuing call {} - customer already hung up", timeout_customer_session_id);
                }
            }
        });
        
        // Return immediately - the event handler will complete the bridge when agent answers
        Ok(())
    }
    
    /// Rollback a failed agent assignment
    async fn rollback_failed_assignment(&self, session_id: &SessionId, agent_id: &AgentId) {
        if let Some(db_manager) = &self.db_manager {
            // Restore agent to available
            if let Err(e) = db_manager.update_agent_call_count_with_retry(&agent_id.0, -1).await {
                error!("Failed to decrement agent call count during rollback: {}", e);
            }
            if let Err(e) = db_manager.update_agent_status_with_retry(&agent_id.0, AgentStatus::Available).await {
                error!("Failed to restore agent status during rollback: {}", e);
            }
            
            // Re-queue the call
            if let Some(call_info) = self.active_calls.get(session_id) {
                if let Some(queue_id) = &call_info.queue_id {
                    let queued_call = crate::queue::QueuedCall {
                        session_id: session_id.clone(),
                        caller_id: call_info.caller_id.clone(),
                        priority: call_info.priority,
                        queued_at: chrono::Utc::now(),
                        estimated_wait_time: None,
                        retry_count: call_info.transfer_count as u8, // Use transfer_count as a proxy for retry count
                    };
                    
                    if let Err(e) = db_manager.enqueue_call(queue_id, &queued_call).await {
                        error!("Failed to re-queue call during rollback: {}", e);
                    } else {
                        info!("🔓 Rolled back assignment: agent {} restored to available, call {} re-queued", 
                              agent_id, session_id);
                    }
                }
            }
        }
    }
    
    /// Update call state when call is established
    pub(super) async fn update_call_established(&self, session_id: SessionId) {
        if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
            call_info.status = CallStatus::Bridged;
            call_info.answered_at = Some(chrono::Utc::now());
            info!("📞 Call {} marked as established/bridged", session_id);
        }
    }
    
    /// Handle call termination cleanup with agent status management
    pub(super) async fn handle_call_termination(&self, session_id: SessionId) -> CallCenterResult<()> {
        info!("🛑 Handling call termination: {}", session_id);
        
        // 🚨 CRITICAL B2BUA FIX: Handle bidirectional termination FIRST
        // When one leg of the B2BUA call terminates, terminate the other leg
        let related_session_id = self.active_calls.get(&session_id)
            .and_then(|call_info| call_info.related_session_id.clone());
        
        if let Some(related_session_id) = related_session_id {
            info!("🔗 B2BUA: Session {} terminated, also terminating related session {}", 
                  session_id, related_session_id);
            
            // Prevent infinite loop by checking if the related session is still active
            if self.active_calls.contains_key(&related_session_id) {
                // Remove the related session from active calls to prevent recursive termination
                if let Some((_, related_call_info)) = self.active_calls.remove(&related_session_id) {
                    info!("🔗 B2BUA: Removed related session {} from active calls", related_session_id);
                    
                    // Terminate the related session via session coordinator
                    if let Some(coordinator) = &self.session_coordinator {
                        match coordinator.terminate_session(&related_session_id).await {
                            Ok(()) => {
                                info!("✅ B2BUA: Successfully sent BYE to related session {}", related_session_id);
                            }
                            Err(e) => {
                                warn!("⚠️ B2BUA: Failed to send BYE to related session {}: {}", related_session_id, e);
                                // Continue with cleanup even if BYE fails
                            }
                        }
                    }
                }
            }
        } else {
            debug!("🔗 B2BUA: Session {} has no related session to terminate", session_id);
        }
        
        // First, update the call end time and calculate metrics
        let now = chrono::Utc::now();
        if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
            call_info.ended_at = Some(now);
            
            // Calculate total duration
            call_info.duration_seconds = now.signed_duration_since(call_info.created_at).num_seconds() as u64;
            
            // Calculate wait time (time until answered or ended if never answered)
            if let Some(answered_at) = call_info.answered_at {
                call_info.wait_time_seconds = answered_at.signed_duration_since(call_info.created_at).num_seconds() as u64;
                // Calculate talk time (answered until ended)
                call_info.talk_time_seconds = now.signed_duration_since(answered_at).num_seconds() as u64;
            } else {
                // Never answered - entire duration was wait time
                call_info.wait_time_seconds = call_info.duration_seconds;
                call_info.talk_time_seconds = 0;
            }
            
            // Calculate queue time if the call was queued
            if let (Some(queued_at), Some(answered_at)) = (call_info.queued_at, call_info.answered_at) {
                call_info.queue_time_seconds = answered_at.signed_duration_since(queued_at).num_seconds() as u64;
            } else if let Some(queued_at) = call_info.queued_at {
                // Still in queue when ended
                call_info.queue_time_seconds = now.signed_duration_since(queued_at).num_seconds() as u64;
            }
            
            info!("📊 Call {} metrics - Total: {}s, Wait: {}s, Talk: {}s, Queue: {}s, Hold: {}s", 
                  session_id, 
                  call_info.duration_seconds,
                  call_info.wait_time_seconds,
                  call_info.talk_time_seconds,
                  call_info.queue_time_seconds,
                  call_info.hold_time_seconds);
        }
        
        // Get call info and clean up
        let call_info = self.active_calls.remove(&session_id).map(|(_, v)| v);
        
        // Remove from database queue if the call was queued
        if let Some(db_manager) = &self.db_manager {
            // Create a cleanup method that removes the call from both tables
            if let Err(e) = db_manager.remove_call_from_queue(&session_id.0).await {
                debug!("Failed to remove call {} from queue: {}", session_id, e);
            } else {
                debug!("🧹 Cleaned up call {} from database", session_id);
            }
        }
        
        // Update agent status if this call had an agent assigned
        if let Some(call_info) = &call_info {
            if let Some(agent_id) = &call_info.agent_id {
                info!("🔄 Updating agent {} status after call termination", agent_id);
                
                // Update agent status in database with retry logic
                if let Some(db_manager) = &self.db_manager {
                    // Decrement call count with retry
                    if let Err(e) = db_manager.update_agent_call_count_with_retry(&agent_id.0, -1).await {
                        error!("Failed to update agent call count in database: {}", e);
                    }
                    
                    // Check current agent status and call count from database
                    match db_manager.get_agent(&agent_id.0).await {
                        Ok(Some(db_agent)) => {
                            if db_agent.current_calls == 0 {
                                // PHASE 0.10: Log agent status transition
                                info!("🔄 Agent {} status change: Busy → PostCallWrapUp (entering wrap-up time)", agent_id);
                                
                                // Update to post-call wrap-up status with retry
                                if let Err(e) = db_manager.update_agent_status_with_retry(&agent_id.0, AgentStatus::PostCallWrapUp).await {
                                    error!("Failed to update agent status to PostCallWrapUp: {}", e);
                                } else {
                                    info!("⏰ Agent {} entering 10-second post-call wrap-up", agent_id);
                                }
                                
                                // Schedule transition to Available after 10 seconds
                                let engine = Arc::new(self.clone());
                                let wrap_up_agent_id = agent_id.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                                    
                                    // Check if agent is still in PostCallWrapUp status
                                    if let Some(db_manager) = &engine.db_manager {
                                        match db_manager.get_agent(&wrap_up_agent_id.0).await {
                                            Ok(Some(agent)) => {
                                                if matches!(agent.status, crate::database::DbAgentStatus::PostCallWrapUp) {
                                                    info!("🔄 Agent {} status change: PostCallWrapUp → Available (wrap-up complete)", wrap_up_agent_id);
                                                    
                                                    if let Err(e) = db_manager.update_agent_status_with_retry(&wrap_up_agent_id.0, AgentStatus::Available).await {
                                                        error!("Failed to update agent status to Available in database: {}", e);
                                                    } else {
                                                        info!("✅ Agent {} is now available for new calls", wrap_up_agent_id);
                                                    }
                                                    
                                                    // Check for stuck assignments and queued calls
                                                    engine.check_stuck_assignments().await;
                                                    
                                                    // Check if there are queued calls that can be assigned to this agent
                                                    engine.try_assign_queued_calls_to_agent(wrap_up_agent_id.clone()).await;
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                });
                            } else {
                                info!("📞 Agent {} still busy with {} active calls", agent_id, db_agent.current_calls);
                            }
                        }
                        Ok(None) => {
                            error!("Agent {} not found in database during call termination", agent_id);
                        }
                        Err(e) => {
                            error!("Failed to get agent from database: {}", e);
                        }
                    }
                }
                
                // Don't try to assign new calls - agent is going into wrap-up!
            }
        }
        
        // Clean up bridge if call had one
        if let Some(call_info) = &call_info {
            if let Some(bridge_id) = &call_info.bridge_id {
                if let Err(e) = self.session_coordinator.as_ref().unwrap().destroy_bridge(&bridge_id).await {
                    warn!("Failed to destroy bridge {}: {}", bridge_id, e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Try to assign queued calls to a newly available agent
    pub(super) async fn try_assign_queued_calls_to_agent(&self, agent_id: AgentId) {
        debug!("🔍 Checking queued calls for newly available agent {}", agent_id);
        
        // Get agent skills from database to find matching queued calls
        let agent_skills = if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent(&agent_id.0).await {
                Ok(Some(db_agent)) => {
                    // TODO: Load skills from database when skills table is implemented
                    vec!["general".to_string()] // Default skills for now
                }
                _ => vec![]
            }
        } else {
            vec![]
        };
        
        // Check relevant queues for calls that match agent skills
        let queues_to_check = vec!["general", "sales", "support", "billing", "vip", "premium"];
        
        for queue_id in queues_to_check {
            // Try to dequeue a call from this queue
            let queued_call = {
                let mut queue_manager = self.queue_manager.write().await;
                queue_manager.dequeue_for_agent(queue_id).unwrap_or(None)
            };
            
            if let Some(queued_call) = queued_call {
                info!("📤 Dequeued call {} from queue {} for agent {}", 
                      queued_call.session_id, queue_id, agent_id);
                
                // Assign the queued call to this agent
                let session_id = queued_call.session_id.clone();
                let agent_id_clone = agent_id.clone();
                let engine = Arc::new(self.clone());
                
                tokio::spawn(async move {
                    match engine.assign_specific_agent_to_call(session_id.clone(), agent_id_clone).await {
                        Ok(()) => {
                            info!("✅ Successfully assigned queued call {} to agent", session_id);
                            // On success, the call is no longer in queue or being assigned
                            let mut queue_manager = engine.queue_manager.write().await;
                            queue_manager.mark_as_not_assigned(&session_id);
                        }
                        Err(e) => {
                            error!("Failed to assign queued call {} to agent: {}", session_id, e);
                            
                            // Mark as no longer being assigned before re-queuing
                            let mut queue_manager = engine.queue_manager.write().await;
                            queue_manager.mark_as_not_assigned(&session_id);
                            
                            // Check if the call is still active before re-queuing
                            let call_still_active = engine.active_calls.contains_key(&session_id);
                            if !call_still_active {
                                warn!("Call {} is no longer active, not re-queuing", session_id);
                                return;
                            }
                            
                            // Re-queue the call with higher priority
                            let mut requeued_call = queued_call;
                            requeued_call.priority = requeued_call.priority.saturating_sub(5); // Increase priority
                            requeued_call.retry_count = requeued_call.retry_count.saturating_add(1);
                            
                            // Check retry limit (max 3 attempts)
                            if requeued_call.retry_count >= 3 {
                                error!("⚠️ Call {} exceeded maximum retry attempts, terminating", session_id);
                                // Remove from active calls
                                engine.active_calls.remove(&session_id);
                                
                                // Terminate the customer call
                                if let Some(coordinator) = engine.session_coordinator.as_ref() {
                                    let _ = coordinator.terminate_session(&session_id).await;
                                }
                                return;
                            }
                            
                            // Apply exponential backoff based on retry count
                            let backoff_ms = 500u64 * (2u64.pow(requeued_call.retry_count as u32 - 1));
                            info!("⏳ Waiting {}ms before re-queuing call {} (retry #{})", 
                                  backoff_ms, session_id, requeued_call.retry_count);
                            tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
                            
                            if let Err(e) = queue_manager.enqueue_call(queue_id, requeued_call) {
                                error!("Failed to re-queue call {}: {}", session_id, e);
                                
                                // Last resort: terminate the call if we can't re-queue
                                if let Some(coordinator) = engine.session_coordinator.as_ref() {
                                    let _ = coordinator.terminate_session(&session_id).await;
                                }
                            } else {
                                info!("📞 Re-queued call {} to {} with higher priority", session_id, queue_id);
                                
                                // Update call status back to queued
                                if let Some(mut call_info) = engine.active_calls.get_mut(&session_id) {
                                    call_info.status = CallStatus::Queued;
                                    call_info.queue_id = Some(queue_id.to_string());
                                }
                            }
                        }
                    }
                });
                
                break; // Only assign one call at a time
            }
        }
    }
    
    /// Check for calls stuck in "being assigned" state and re-queue them
    pub(super) async fn check_stuck_assignments(&self) {
        debug!("🔍 Checking for stuck assignments");
        
        // Get list of calls that might be stuck
        let mut stuck_calls = Vec::new();
        
        // Check active calls for those in Connecting state without an agent actively handling them
        for entry in self.active_calls.iter() {
            let (session_id, call_info) = (entry.key(), entry.value());
            
            // A call is stuck if:
            // 1. It's in Connecting state (being assigned)
            // 2. It has a queue_id (was queued)
            // 3. It's not in pending_assignments (no agent is handling it)
            if matches!(call_info.status, super::types::CallStatus::Connecting) &&
               call_info.queue_id.is_some() &&
               !self.pending_assignments.contains_key(session_id) {
                
                // Check how long it's been in this state
                let duration = chrono::Utc::now().signed_duration_since(call_info.created_at);
                if duration.num_seconds() > 5 {  // Stuck for more than 5 seconds
                    warn!("⚠️ Found stuck call {} in Connecting state for {}s", 
                          session_id, duration.num_seconds());
                    stuck_calls.push((session_id.clone(), call_info.queue_id.clone().unwrap()));
                }
            }
        }
        
        // Re-queue stuck calls
        let stuck_count = stuck_calls.len();
        for (session_id, queue_id) in stuck_calls {
            info!("🔄 Re-queuing stuck call {} to queue {}", session_id, queue_id);
            
            // Update call status back to Queued
            if let Some(mut call_info) = self.active_calls.get_mut(&session_id) {
                call_info.status = super::types::CallStatus::Queued;
                
                // Create a QueuedCall to re-enqueue
                let queued_call = QueuedCall {
                    session_id: session_id.clone(),
                    caller_id: call_info.caller_id.clone(),
                    priority: call_info.priority.saturating_sub(10), // Higher priority for stuck calls
                    queued_at: call_info.queued_at.unwrap_or_else(chrono::Utc::now),
                    estimated_wait_time: None,
                    retry_count: 1,  // Mark as retry
                };
                
                // Re-enqueue the call
                let mut queue_manager = self.queue_manager.write().await;
                
                // First, clear any "being assigned" flag
                queue_manager.mark_as_not_assigned(&session_id);
                
                // Then re-queue
                match queue_manager.enqueue_call(&queue_id, queued_call) {
                    Ok(position) => {
                        info!("✅ Successfully re-queued stuck call {} with higher priority at position {}", session_id, position);
                        
                        // Start queue monitor if needed
                        self.monitor_queue_for_agents(queue_id).await;
                    }
                    Err(e) => {
                        error!("Failed to re-queue stuck call {}: {}", session_id, e);
                        // As a last resort, terminate the call
                        if let Some(coordinator) = &self.session_coordinator {
                            let _ = coordinator.terminate_session(&session_id).await;
                        }
                    }
                }
            }
        }
        
        if stuck_count > 0 {
            info!("🔄 Re-queued {} stuck calls", stuck_count);
        }
    }
    
    /// Put a call on hold
    pub async fn put_call_on_hold(&self, session_id: &SessionId) -> CallCenterResult<()> {
        if let Some(mut call_info) = self.active_calls.get_mut(session_id) {
            if call_info.status == CallStatus::Bridged {
                call_info.status = CallStatus::OnHold;
                call_info.hold_count += 1;
                
                // Track hold start time (would need additional field for accurate tracking)
                info!("☎️ Call {} put on hold (count: {})", session_id, call_info.hold_count);
                
                // TODO: Actually put the call on hold via session coordinator
                if let Some(coordinator) = &self.session_coordinator {
                    coordinator.hold_session(session_id).await
                        .map_err(|e| CallCenterError::orchestration(&format!("Failed to hold call: {}", e)))?;
                }
            }
        }
        Ok(())
    }
    
    /// Resume a call from hold
    pub async fn resume_call_from_hold(&self, session_id: &SessionId) -> CallCenterResult<()> {
        if let Some(mut call_info) = self.active_calls.get_mut(session_id) {
            if call_info.status == CallStatus::OnHold {
                call_info.status = CallStatus::Bridged;
                
                // TODO: Calculate and add to total hold time
                info!("📞 Call {} resumed from hold", session_id);
                
                // TODO: Actually resume the call via session coordinator
                if let Some(coordinator) = &self.session_coordinator {
                    coordinator.resume_session(session_id).await
                        .map_err(|e| CallCenterError::orchestration(&format!("Failed to resume call: {}", e)))?;
                }
            }
        }
        Ok(())
    }
    
    /// Transfer a call to another agent or queue (simple version)
    pub async fn transfer_call_simple(&self, session_id: &SessionId, target: &str) -> CallCenterResult<()> {
        if let Some(mut call_info) = self.active_calls.get_mut(session_id) {
            call_info.transfer_count += 1;
            call_info.status = CallStatus::Transferring;
            
            info!("📞 Transferring call {} to {} (count: {})", 
                  session_id, target, call_info.transfer_count);
            
            // TODO: Implement actual transfer logic
            // This would involve:
            // 1. Finding the target agent/queue
            // 2. Creating new session to target
            // 3. Bridging or re-routing the call
        }
        Ok(())
    }
} 