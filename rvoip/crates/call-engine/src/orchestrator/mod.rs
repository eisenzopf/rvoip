//! # Call Center Orchestration Module
//!
//! This module provides comprehensive orchestration functionality for call center operations,
//! coordinating between agents, queues, routing, bridge management, and session-core integration.
//! It serves as the central coordination layer that brings together all call center components
//! to deliver enterprise-grade call handling capabilities.
//!
//! ## Overview
//!
//! The orchestrator module is the heart of the call center system, providing sophisticated
//! coordination between multiple subsystems to deliver seamless call center operations. It
//! handles everything from incoming call processing through agent assignment, bridge management,
//! and comprehensive monitoring. The module is designed for high-performance, concurrent
//! operation while maintaining data consistency and providing robust error recovery.
//!
//! ## Module Organization
//!
//! The orchestrator is organized into specialized modules, each handling specific aspects
//! of call center operations:
//!
//! ### Core Components
//!
//! - **[`core`]**: Main CallCenterEngine with configuration and coordination
//! - **[`handler`]**: CallHandler implementation for session-core integration
//! - **[`types`]**: Shared type definitions and data structures
//!
//! ### Call Management
//!
//! - **[`calls`]**: Call handling logic with B2BUA operations and routing
//! - **[`lifecycle`]**: Call lifecycle management and state transitions
//! - **[`routing`]**: Intelligent call routing and decision algorithms
//!
//! ### Agent and Queue Management
//!
//! - **[`agents`]**: Agent management, registration, and status tracking
//! - **[`bridge`]**: Bridge management policies and configuration
//! - **[`bridge_operations`]**: Actual bridge operations via session-core
//!
//! ### Utility Components
//!
//! - **[`uri_builder`]**: SIP URI generation and management utilities
//!
//! ## Key Features
//!
//! - **Integrated Orchestration**: Seamless coordination between all call center components
//! - **B2BUA Operations**: Complete Back-to-Back User Agent functionality
//! - **Agent Management**: Comprehensive agent lifecycle and status management
//! - **Queue Management**: Advanced queue processing with fair distribution
//! - **Bridge Operations**: Multi-participant bridge creation and management
//! - **Event Processing**: Real-time event handling and state management
//! - **Performance Monitoring**: Comprehensive metrics and performance tracking
//! - **Error Recovery**: Robust error handling with automatic recovery
//! - **Scalability**: High-performance concurrent operations
//! - **Configurability**: Flexible configuration for different deployment scenarios
//!
//! ## Examples
//!
//! ### Basic Orchestrator Usage
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterConfig, orchestrator::{CallCenterEngine, CallCenterCallHandler}};
//! use std::sync::Arc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create and configure call center engine
//! let engine = Arc::new(CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! // Create call handler for session-core integration
//! let call_handler = CallCenterCallHandler {
//!     engine: Arc::downgrade(&engine),
//! };
//! 
//! println!("🎛️ Call center orchestrator initialized");
//! println!("📞 Ready to handle incoming calls");
//! println!("👥 Agent management active");
//! println!("📋 Queue processing enabled");
//! println!("🌉 Bridge operations available");
//! 
//! // The orchestrator is now ready for full call center operations
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent and Queue Coordination
//!
//! ```rust
//! use rvoip_call_engine::{
//!     CallCenterEngine, CallCenterConfig,
//!     orchestrator::types::{AgentInfo, CallInfo},
//!     agent::{Agent, AgentId, AgentStatus}
//! };
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Register agents
//! let agent = Agent {
//!     id: "agent-001".to_string(),
//!     sip_uri: "sip:agent001@call-center.local".to_string(),
//!     display_name: "Agent 001".to_string(),
//!     skills: vec!["english".to_string(), "sales".to_string()],
//!     max_concurrent_calls: 2,
//!     status: AgentStatus::Available,
//!     department: Some("sales".to_string()),
//!     extension: Some("1001".to_string()),
//!     // Note: performance_rating field does not exist in Agent struct
//! };
//! 
//! let session_id = engine.register_agent(&agent).await?;
//! println!("✅ Agent registered with session: {}", session_id);
//! 
//! // Update agent status for availability
//! let agent_id = AgentId("agent-001".to_string());
//! engine.update_agent_status(&agent_id, AgentStatus::Available).await?;
//! println!("🟢 Agent marked as available");
//! 
//! // Get queue statistics
//! let queue_stats = engine.get_queue_stats().await?;
//! println!("📊 Monitoring {} queues", queue_stats.len());
//! 
//! // The orchestrator coordinates all these operations seamlessly
//! # Ok(())
//! # }
//! ```
//!
//! ### Call Processing Integration
//!
//! ```rust
//! use rvoip_call_engine::{
//!     CallCenterEngine, CallCenterConfig,
//!     orchestrator::{CallCenterCallHandler, types::CallStatus}
//! };
//! use rvoip_session_core::{IncomingCall, SessionId, CallHandler};
//! use std::{sync::Arc, collections::HashMap};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = Arc::new(CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?);
//! 
//! let call_handler = CallCenterCallHandler {
//!     engine: Arc::downgrade(&engine),
//! };
//! 
//! // Simulate incoming call processing
//! let incoming_call = IncomingCall {
//!     id: SessionId("customer-call".to_string()),
//!     from: "sip:customer@external.com".to_string(),
//!     to: "sip:support@call-center.com".to_string(),
//!     sdp: Some("v=0\r\no=- 123456 IN IP4 192.168.1.100\r\n...".to_string()),
//!     headers: HashMap::new(),
//!     received_at: std::time::Instant::now(),
//! };
//! 
//! // Process through orchestrator
//! let decision = call_handler.on_incoming_call(incoming_call).await;
//! println!("📞 Call processed with decision: {:?}", decision);
//! 
//! // The orchestrator handles:
//! // 1. Call analysis and customer classification
//! // 2. Routing decision based on availability and skills
//! // 3. Queue placement with appropriate priority
//! // 4. Agent assignment when available
//! // 5. Bridge creation and management
//! # Ok(())
//! # }
//! ```
//!
//! ### Bridge Operations
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Create conference bridge
//! let participants = vec![
//!     SessionId("agent-001".to_string()),
//!     SessionId("customer-123".to_string()),
//!     SessionId("supervisor-456".to_string()),
//! ];
//! 
//! let bridge_id = engine.create_conference(&participants).await?;
//! println!("🎤 Conference created: {}", bridge_id);
//! 
//! // Get bridge information
//! let bridge_info = engine.get_bridge_info(&bridge_id).await?;
//! println!("📊 Bridge participants: {}", bridge_info.participant_count);
//! 
//! // List all active bridges
//! let active_bridges = engine.list_active_bridges().await;
//! println!("🌉 Total active bridges: {}", active_bridges.len());
//! 
//! // The orchestrator manages bridge lifecycle automatically
//! # Ok(())
//! # }
//! ```
//!
//! ### Comprehensive System Monitoring
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Get comprehensive system status
//! println!("📊 Call Center System Status:");
//! 
//! // Agent status overview
//! let agents = engine.list_agents().await;
//! let available_agents = agents.iter()
//!     .filter(|a| matches!(a.status, rvoip_call_engine::agent::AgentStatus::Available))
//!     .count();
//! 
//! println!("👥 Agents: {} total, {} available", agents.len(), available_agents);
//! 
//! // Queue status overview
//! let queue_stats = engine.get_queue_stats().await?;
//! let total_queued: usize = queue_stats.iter()
//!     .map(|(_, stats)| stats.total_calls)
//!     .sum();
//! 
//! println!("📋 Queues: {} active, {} calls waiting", queue_stats.len(), total_queued);
//! 
//! // Bridge status overview
//! let active_bridges = engine.list_active_bridges().await;
//! println!("🌉 Bridges: {} active conferences", active_bridges.len());
//! 
//! // System health indicators
//! if available_agents == 0 && total_queued > 0 {
//!     println!("🚨 Alert: No agents available with calls waiting");
//! } else if total_queued > available_agents * 5 {
//!     println!("⚠️ Warning: High queue load detected");
//! } else {
//!     println!("✅ System operating normally");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Integration Patterns
//!
//! ### Session-Core Integration
//!
//! The orchestrator integrates seamlessly with session-core for SIP operations:
//!
//! ```rust
//! # use rvoip_call_engine::CallCenterEngine;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Integration architecture:
//! println!("🔗 Session-Core Integration Architecture:");
//! 
//! println!("  📡 Event Flow:");
//! println!("     Session-Core SIP Stack");
//! println!("     ↓ (SIP events)");
//! println!("     CallCenterCallHandler");
//! println!("     ↓ (processed events)");
//! println!("     CallCenterEngine");
//! println!("     ↓ (business logic)");
//! println!("     Database & Queue Management");
//! 
//! println!("  🔄 Response Flow:");
//! println!("     CallCenterEngine");
//! println!("     ↓ (API calls)");
//! println!("     Session-Core APIs");
//! println!("     ↓ (SIP messages)");
//! println!("     Network/Agents");
//! 
//! // This integration enables complete SIP call center functionality
//! # Ok(())
//! # }
//! ```
//!
//! ### Database Integration
//!
//! The orchestrator maintains consistency with database operations:
//!
//! ```rust
//! # use rvoip_call_engine::CallCenterEngine;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Database integration patterns:
//! println!("💾 Database Integration:");
//! 
//! println!("  🔄 Real-time Synchronization:");
//! println!("     ↳ Agent status changes → Database updates");
//! println!("     ↳ Call state changes → Call records");
//! println!("     ↳ Queue operations → Queue persistence");
//! println!("     ↳ Metrics collection → Performance data");
//! 
//! println!("  🛡️ Consistency Guarantees:");
//! println!("     ↳ Atomic operations for critical updates");
//! println!("     ↳ Transaction support for complex operations");
//! println!("     ↳ Rollback capability for failed operations");
//! println!("     ↳ Eventual consistency for non-critical data");
//! 
//! println!("  📊 Performance Optimization:");
//! println!("     ↳ Connection pooling for scalability");
//! println!("     ↳ Async operations for non-blocking access");
//! println!("     ↳ Batch operations for efficiency");
//! println!("     ↳ Caching for frequently accessed data");
//! 
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance and Scalability
//!
//! ### High-Performance Architecture
//!
//! The orchestrator is designed for high-performance operation:
//!
//! ```rust
//! # use rvoip_call_engine::CallCenterEngine;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! println!("⚡ Performance Characteristics:");
//! 
//! println!("  🚀 Concurrency:");
//! println!("     ↳ Async/await throughout for non-blocking operations");
//! println!("     ↳ Concurrent call processing");
//! println!("     ↳ Parallel agent assignment");
//! println!("     ↳ Independent queue processing");
//! 
//! println!("  💾 Memory Efficiency:");
//! println!("     ↳ Efficient data structures (DashMap, Arc)");
//! println!("     ↳ Minimal allocations per operation");
//! println!("     ↳ Lazy initialization of resources");
//! println!("     ↳ Automatic cleanup of completed operations");
//! 
//! println!("  📊 Scalability:");
//! println!("     ↳ Linear scaling with call volume");
//! println!("     ↳ Horizontal scaling support");
//! println!("     ↳ Load balancing across instances");
//! println!("     ↳ Resource-aware operation limits");
//! 
//! // The orchestrator supports enterprise-scale deployments
//! # Ok(())
//! # }
//! ```
//!
//! ### Error Handling and Recovery
//!
//! Comprehensive error handling ensures system reliability:
//!
//! ```rust
//! # use rvoip_call_engine::CallCenterEngine;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! println!("🛡️ Error Handling Strategy:");
//! 
//! println!("  🔧 Recovery Mechanisms:");
//! println!("     ↳ Automatic retry with exponential backoff");
//! println!("     ↳ Graceful degradation on component failures");
//! println!("     ↳ Rollback capability for failed operations");
//! println!("     ↳ Circuit breaker for external dependencies");
//! 
//! println!("  📊 Monitoring and Alerting:");
//! println!("     ↳ Comprehensive error logging");
//! println!("     ↳ Metrics collection for error rates");
//! println!("     ↳ Alerting for critical failures");
//! println!("     ↳ Health checks for system components");
//! 
//! println!("  🔄 Operational Continuity:");
//! println!("     ↳ Continue operation with partial failures");
//! println!("     ↳ Fallback to simplified operations");
//! println!("     ↳ Automatic recovery when possible");
//! println!("     ↳ Manual intervention alerts when needed");
//! 
//! # Ok(())
//! # }
//! ```

pub mod core;
pub mod types;
pub mod handler;
pub mod routing;
pub mod calls;
pub mod agents;
pub mod bridge_operations;
pub mod bridge;
pub mod lifecycle;
pub mod uri_builder;

// Export the main call center engine
pub use core::CallCenterEngine;

// Export types
pub use types::{
    CallInfo, AgentInfo, CustomerType, CallStatus, 
    RoutingDecision, RoutingStats, OrchestratorStats
};

// Export handler for advanced use cases
pub use handler::CallCenterCallHandler;

// Export other managers
pub use bridge::BridgeManager;
pub use lifecycle::CallLifecycleManager;

// Export URI builder
pub use uri_builder::SipUriBuilder; 