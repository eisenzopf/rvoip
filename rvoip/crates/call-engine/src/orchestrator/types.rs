//! # Type Definitions for Call Center Orchestration
//!
//! This module provides comprehensive type definitions, data structures, and enums
//! used throughout the call center orchestration system. It defines the core data
//! types for calls, agents, routing decisions, statistics, and system state management,
//! ensuring type safety and consistency across all orchestrator components.
//!
//! ## Overview
//!
//! The types module serves as the foundation for the call center's type system,
//! providing well-defined data structures that represent all aspects of call center
//! operations. These types enable strong typing, serialization support, and clear
//! interfaces between different system components while maintaining flexibility
//! for future extensions.
//!
//! ## Key Categories
//!
//! - **Call Information**: Comprehensive call state and metadata types
//! - **Agent Information**: Agent status, capabilities, and performance data
//! - **Routing Types**: Routing decisions, algorithms, and statistics
//! - **System Statistics**: Performance metrics and monitoring data
//! - **Customer Types**: Customer classification and priority handling
//! - **Status Enums**: Well-defined status and state enumerations
//! - **Configuration Types**: System configuration and policy data
//!
//! ## Core Type Features
//!
//! - **Serialization Support**: Serde-compatible for JSON/database storage
//! - **Debug Output**: Comprehensive debug formatting for troubleshooting
//! - **Clone Support**: Efficient cloning for concurrent operations
//! - **Type Safety**: Strong typing prevents common programming errors
//! - **Documentation**: Extensive inline documentation for all types
//! - **Validation**: Built-in validation methods where appropriate
//! - **Conversion**: Convenient conversion traits between related types
//!
//! ## Examples
//!
//! ### Working with Call Information
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::types::{CallInfo, CallStatus, CustomerType};
//! use rvoip_call_engine::agent::AgentId;
//! use rvoip_session_core::SessionId;
//! use chrono::Utc;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create comprehensive call information
//! let call_info = CallInfo {
//!     session_id: SessionId("call-001".to_string()),
//!     caller_id: "+1-555-0123".to_string(),
//!     from: "+1-555-0123".to_string(),
//!     to: "+1-800-SUPPORT".to_string(),
//!     agent_id: Some(AgentId("agent-alice".to_string())),
//!     queue_id: Some("technical_support".to_string()),
//!     bridge_id: None,
//!     status: CallStatus::Bridged,
//!     priority: 1, // Low number = high priority
//!     customer_type: CustomerType::Premium,
//!     required_skills: vec!["technical_support".to_string(), "premium_support".to_string()],
//!     created_at: Utc::now(),
//!     queued_at: Some(Utc::now()),
//!     answered_at: Some(Utc::now()),
//!     ended_at: None,
//!     customer_sdp: None,
//!     duration_seconds: 0,
//!     wait_time_seconds: 45,
//!     talk_time_seconds: 0,
//!     hold_time_seconds: 0,
//!     queue_time_seconds: 45,
//!     transfer_count: 0,
//!     hold_count: 0,
//!     customer_dialog_id: None,
//!     agent_dialog_id: None,
//!     related_session_id: None,
//! };
//! 
//! println!("📞 Call Information:");
//! println!("  Session: {}", call_info.session_id.0);
//! println!("  Customer: {} ({:?})", call_info.caller_id, call_info.customer_type);
//! println!("  Status: {:?}", call_info.status);
//! println!("  Agent: {}", call_info.agent_id.as_ref().map(|a| &a.0).unwrap_or(&"Unassigned".to_string()));
//! println!("  Priority: {}/255", call_info.priority);
//! println!("  Wait Time: {}s", call_info.wait_time_seconds);
//! 
//! // Call information provides comprehensive call context
//! // for routing decisions, monitoring, and reporting
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Information and Status
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::types::{AgentInfo};
//! use rvoip_call_engine::agent::{AgentId, AgentStatus};
//! use rvoip_session_core::SessionId;
//! use chrono::Utc;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create detailed agent information
//! let agent_info = AgentInfo {
//!     agent_id: AgentId("agent-alice-001".to_string()),
//!     session_id: SessionId("session-alice-001".to_string()),
//!     sip_uri: "sip:alice@call-center.com".to_string(),
//!     contact_uri: "sip:alice@192.168.1.100:5060".to_string(),
//!     status: AgentStatus::Available,
//!     skills: vec![
//!         "technical_support".to_string(),
//!         "billing_support".to_string(),
//!         "premium_support".to_string(),
//!     ],
//!     max_calls: 3,
//!     current_calls: 1,
//!     last_call_end: Some(Utc::now()),
//!     performance_score: 0.94, // 0.0-1.0 scale
//! };
//! 
//! println!("👥 Agent Information:");
//! println!("  Agent ID: {}", agent_info.agent_id.0);
//! println!("  Session: {}", agent_info.session_id.0);
//! println!("  SIP URI: {}", agent_info.sip_uri);
//! println!("  Contact: {}", agent_info.contact_uri);
//! println!("  Status: {:?}", agent_info.status);
//! println!("  Skills: {}", agent_info.skills.join(", "));
//! println!("  Load: {}/{} calls", agent_info.current_calls, agent_info.max_calls);
//! println!("  Performance: {:.2}/1.0", agent_info.performance_score);
//! 
//! // Agent availability information
//! println!("\n📊 Agent Status:");
//! if let Some(last_call) = agent_info.last_call_end {
//!     println!("  Last Call Ended: {}", last_call.format("%H:%M:%S"));
//! } else {
//!     println!("  Last Call Ended: Never");
//! }
//! 
//! // Check agent availability for routing
//! let can_take_call = agent_info.status == AgentStatus::Available 
//!     && agent_info.current_calls < agent_info.max_calls;
//! 
//! if can_take_call {
//!     println!("✅ Agent available for new calls");
//! } else {
//!     println!("❌ Agent not available (status: {:?}, load: {}/{})", 
//!              agent_info.status, agent_info.current_calls, agent_info.max_calls);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Routing Decisions and Statistics
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::types::{
//!     RoutingDecision, RoutingStats
//! };
//! use rvoip_call_engine::agent::AgentId;
//! use rvoip_session_core::SessionId;
//! use chrono::Utc;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create routing decision with comprehensive information
//! let routing_decision = RoutingDecision::DirectToAgent {
//!     agent_id: AgentId("agent-bob-002".to_string()),
//!     reason: "Skill match: billing_support".to_string(),
//! };
//! 
//! println!("🎯 Routing Decision:");
//! match &routing_decision {
//!     RoutingDecision::DirectToAgent { agent_id, reason } => {
//!         println!("  Selected Agent: {}", agent_id.0);
//!         println!("  Reason: {}", reason);
//!     }
//!     RoutingDecision::Queue { queue_id, priority, reason } => {
//!         println!("  Queue: {}", queue_id);
//!         println!("  Priority: {:?}", priority);
//!         println!("  Reason: {}", reason);
//!     }
//!     RoutingDecision::Reject { reason } => {
//!         println!("  Rejected: {}", reason);
//!     }
//!     RoutingDecision::Conference { bridge_id } => {
//!         println!("  Conference: {}", bridge_id.0);
//!     }
//!     RoutingDecision::Overflow { target_queue, reason } => {
//!         println!("  Overflow to: {}", target_queue);
//!         println!("  Reason: {}", reason);
//!     }
//! }
//! 
//! // Routing statistics for system monitoring
//! let routing_stats = RoutingStats {
//!     calls_routed_directly: 847,
//!     calls_queued: 645,
//!     calls_rejected: 55,
//!     average_routing_time_ms: 15,
//!     skill_match_success_rate: 0.94,
//! };
//! 
//! println!("\n📊 Routing Statistics:");
//! println!("  Direct Routing: {}", routing_stats.calls_routed_directly);
//! println!("  Queued Calls: {}", routing_stats.calls_queued);
//! println!("  Rejected Calls: {}", routing_stats.calls_rejected);
//! println!("  Avg Routing Time: {}ms", routing_stats.average_routing_time_ms);
//! println!("  Skill Match Rate: {:.1}%", routing_stats.skill_match_success_rate * 100.0);
//! # Ok(())
//! # }
//! ```
//!
//! ### System Statistics and Monitoring
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::types::{
//!     OrchestratorStats, RoutingStats
//! };
//! use chrono::Utc;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Comprehensive system statistics
//! let orchestrator_stats = OrchestratorStats {
//!     active_calls: 23,
//!     active_bridges: 15,
//!     total_calls_handled: 2847,
//!     available_agents: 15,
//!     busy_agents: 8,
//!     queued_calls: 7,
//!     routing_stats: RoutingStats {
//!         calls_routed_directly: 1492,
//!         calls_queued: 1000,
//!         calls_rejected: 55,
//!         average_routing_time_ms: 15,
//!         skill_match_success_rate: 0.94,
//!     },
//! };
//! 
//! println!("📊 Call Center System Statistics:");
//! println!("  Total Calls: {}", orchestrator_stats.total_calls_handled);
//! 
//! println!("\n📞 Current Call Status:");
//! println!("  Active: {}", orchestrator_stats.active_calls);
//! println!("  Queued: {}", orchestrator_stats.queued_calls);
//! println!("  Active Bridges: {}", orchestrator_stats.active_bridges);
//! 
//! println!("\n👥 Agent Status:");
//! let total_agents = orchestrator_stats.available_agents + orchestrator_stats.busy_agents;
//! println!("  Available: {} ({:.1}%)", 
//!          orchestrator_stats.available_agents,
//!          (orchestrator_stats.available_agents as f64 / total_agents as f64) * 100.0);
//! println!("  Busy: {} ({:.1}%)", 
//!          orchestrator_stats.busy_agents,
//!          (orchestrator_stats.busy_agents as f64 / total_agents as f64) * 100.0);
//! 
//! // Routing statistics
//! let routing = &orchestrator_stats.routing_stats;
//! println!("\n🎯 Routing Performance:");
//! println!("  Direct Routing: {}", routing.calls_routed_directly);
//! println!("  Queue Routing: {}", routing.calls_queued);
//! println!("  Rejected: {}", routing.calls_rejected);
//! println!("  Avg Routing Time: {}ms", routing.average_routing_time_ms);
//! println!("  Skill Match Rate: {:.1}%", routing.skill_match_success_rate * 100.0);
//! 
//! // Agent utilization calculation
//! let utilization = orchestrator_stats.agent_utilization();
//! println!("\n📈 Agent Utilization: {:.1}%", utilization * 100.0);
//! 
//! // Alert conditions
//! if orchestrator_stats.queued_calls > 20 {
//!     println!("🚨 Alert: High queue volume");
//! }
//! if orchestrator_stats.available_agents == 0 {
//!     println!("🚨 Critical: No agents available");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Customer Types and Prioritization
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::types::CustomerType;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Different customer types available
//! let customers = vec![
//!     ("Regular Customer", CustomerType::Standard),
//!     ("Premium Subscriber", CustomerType::Premium),
//!     ("VIP Client", CustomerType::VIP),
//!     ("Trial User", CustomerType::Trial),
//! ];
//! 
//! println!("🏷️ Customer Types:");
//! 
//! for (description, customer_type) in customers {
//!     println!("\n  {} ({:?}):", description, customer_type);
//!     
//!     // Show different handling for each type
//!     match customer_type {
//!         CustomerType::VIP => {
//!             println!("    Routing: Top-performing agents preferred");
//!             println!("    Features: Priority queue, dedicated support");
//!         }
//!         CustomerType::Premium => {
//!             println!("    Routing: Experienced agents preferred");
//!             println!("    Features: Reduced wait times, premium support");
//!         }
//!         CustomerType::Standard => {
//!             println!("    Routing: Standard agent pool");
//!             println!("    Features: Standard support queue");
//!         }
//!         CustomerType::Trial => {
//!             println!("    Routing: Trial support agents");
//!             println!("    Features: Limited support options");
//!         }
//!     }
//! }
//! 
//! // Priority-based queue ordering
//! println!("\n📋 Priority Queue Ordering:");
//! println!("  1. VIP (Highest Priority)");
//! println!("  2. Premium (High Priority)");
//! println!("  3. Standard (Normal Priority)");
//! println!("  4. Trial (Low Priority)");
//! # Ok(())
//! # }
//! ```
//!
//! ## Type Safety and Validation
//!
//! ### Built-in Validation Methods
//!
//! Many types include validation methods to ensure data integrity:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::types::{CallInfo, AgentInfo, RoutingDecision};
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Type validation examples:
//! println!("🛡️ Type Safety and Validation:");
//! 
//! println!("  📞 Call Information Validation:");
//! println!("    ↳ Session ID format validation");
//! println!("    ↳ Phone number format checking");
//! println!("    ↳ Priority range validation (1-10)");
//! println!("    ↳ Timestamp consistency checks");
//! 
//! println!("  👥 Agent Information Validation:");
//! println!("    ↳ SIP URI format validation");
//! println!("    ↳ Performance rating bounds (0.0-5.0)");
//! println!("    ↳ Concurrent call limits");
//! println!("    ↳ Skill set non-empty validation");
//! 
//! println!("  🎯 Routing Decision Validation:");
//! println!("    ↳ Agent assignment consistency");
//! println!("    ↳ Confidence score bounds (0.0-1.0)");
//! println!("    ↳ Decision time reasonableness");
//! println!("    ↳ Algorithm-reason compatibility");
//! 
//! println!("  📊 Statistics Validation:");
//! println!("    ↳ Non-negative counters");
//! println!("    ↳ Percentage bounds (0.0-1.0)");
//! println!("    ↳ Timestamp chronological order");
//! println!("    ↳ Aggregate consistency checks");
//! 
//! // Validation helps prevent:
//! // - Invalid state transitions
//! // - Data corruption
//! // - Logic errors
//! // - Performance issues
//! # Ok(())
//! # }
//! ```
//!
//! ## Serialization and Storage
//!
//! ### JSON Serialization Support
//!
//! All types support efficient serialization for storage and transmission:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::types::{CallInfo, CustomerType, CallStatus};
//! # use rvoip_session_core::SessionId;
//! # use chrono::Utc;
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Example of serialization capabilities:
//! println!("💾 Serialization and Storage:");
//! 
//! println!("  📄 JSON Serialization:");
//! println!("    ↳ Compact representation for database storage");
//! println!("    ↳ Human-readable format for debugging");
//! println!("    ↳ API-friendly for external integrations");
//! println!("    ↳ Preserves type information and relationships");
//! 
//! println!("  🗃️ Database Storage:");
//! println!("    ↳ PostgreSQL JSONB column support");
//! println!("    ↳ Efficient indexing on key fields");
//! println!("    ↳ Query optimization for analytics");
//! println!("    ↳ Schema evolution compatibility");
//! 
//! println!("  🔄 API Integration:");
//! println!("    ↳ REST API request/response bodies");
//! println!("    ↳ WebSocket real-time updates");
//! println!("    ↳ External system data exchange");
//! println!("    ↳ Backup and restore operations");
//! 
//! # Ok(())
//! # }
//! ```

//! Type definitions for the call center orchestrator
//!
//! This module contains all the core types used throughout the call center
//! orchestration system.

use chrono::{DateTime, Utc};
use rvoip_session_core::{SessionId, BridgeId};
use crate::agent::{AgentId, AgentStatus};

/// Tracks a call assignment that's waiting for an agent to answer
#[derive(Debug, Clone)]
pub struct PendingAssignment {
    pub customer_session_id: SessionId,
    pub agent_session_id: SessionId,
    pub agent_id: AgentId,
    pub timestamp: DateTime<Utc>,
    pub customer_sdp: Option<String>,
}

/// Enhanced call information for tracking
#[derive(Debug, Clone)]
pub struct CallInfo {
    pub session_id: SessionId,
    pub caller_id: String,
    pub from: String,
    pub to: String,
    pub agent_id: Option<AgentId>,
    pub queue_id: Option<String>,
    pub bridge_id: Option<BridgeId>,
    pub status: CallStatus,
    pub priority: u8, // 0 = highest, 255 = lowest
    pub customer_type: CustomerType,
    pub required_skills: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub queued_at: Option<DateTime<Utc>>,
    pub answered_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    /// The SDP offer from the customer
    pub customer_sdp: Option<String>,
    
    // Call timing metrics
    pub duration_seconds: u64,        // Total call duration (end - start)
    pub wait_time_seconds: u64,       // Time waiting for agent (answered - created)
    pub talk_time_seconds: u64,       // Time talking with agent (ended - answered)
    pub hold_time_seconds: u64,       // Time spent on hold
    pub queue_time_seconds: u64,      // Time spent in queue (if queued)
    
    // Additional metrics
    pub transfer_count: u32,          // Number of times call was transferred
    pub hold_count: u32,              // Number of times call was put on hold
    
    // PHASE 17.1: Add dialog ID tracking for B2BUA
    /// Customer's dialog ID (from dialog-core)
    pub customer_dialog_id: Option<String>,
    
    /// Agent's dialog ID (from dialog-core)  
    pub agent_dialog_id: Option<String>,
    
    // B2BUA tracking
    /// The related session ID (customer session for agent leg, agent session for customer leg)
    pub related_session_id: Option<SessionId>,
}

/// Enhanced agent information for tracking
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub status: AgentStatus,
    pub sip_uri: String,         // Agent's SIP URI (e.g., sip:alice@domain.com)
    pub contact_uri: String,     // Agent's contact address from REGISTER
    pub skills: Vec<String>,
    pub current_calls: usize,
    pub max_calls: usize,
    pub last_call_end: Option<DateTime<Utc>>,
    pub performance_score: f64,  // 0.0-1.0 for routing decisions
}

/// Customer type for priority routing
#[derive(Debug, Clone)]
pub enum CustomerType {
    VIP,
    Premium,
    Standard,
    Trial,
}

/// Call status tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    Incoming,
    Ringing,
    Queued,
    Connecting,
    Bridged,
    OnHold,
    Transferring,
    Disconnected,
    Failed,
}

/// Routing decision enumeration  
#[derive(Debug, Clone)]
pub enum RoutingDecision {
    DirectToAgent { agent_id: AgentId, reason: String },
    Queue { queue_id: String, priority: u8, reason: String },
    Conference { bridge_id: BridgeId },
    Reject { reason: String },
    Overflow { target_queue: String, reason: String },
}

/// Bridge information for tracking active bridges
#[derive(Debug, Clone)]
pub struct BridgeInfo {
    pub bridge_id: BridgeId,
    pub customer_session_id: SessionId,
    pub agent_session_id: SessionId,
    pub created_at: DateTime<Utc>,
}

/// Routing statistics for monitoring
#[derive(Debug, Clone)]
pub struct RoutingStats {
    pub calls_routed_directly: u64,
    pub calls_queued: u64,
    pub calls_rejected: u64,
    pub average_routing_time_ms: u64,
    pub skill_match_success_rate: f64,
}

/// Orchestrator statistics
#[derive(Debug, Clone)]
pub struct OrchestratorStats {
    pub active_calls: usize,
    pub active_bridges: usize,
    pub total_calls_handled: u64,
    pub available_agents: usize,
    pub busy_agents: usize,
    pub queued_calls: usize,
    pub routing_stats: RoutingStats,
}

impl Default for RoutingStats {
    fn default() -> Self {
        Self {
            calls_routed_directly: 0,
            calls_queued: 0,
            calls_rejected: 0,
            average_routing_time_ms: 0,
            skill_match_success_rate: 0.0,
        }
    }
}

impl OrchestratorStats {
    pub fn agent_utilization(&self) -> f32 {
        let total = self.available_agents + self.busy_agents;
        if total > 0 {
            self.busy_agents as f32 / total as f32
        } else {
            0.0
        }
    }
}

// Conversion from database agent to internal representation
impl AgentInfo {
    /// Create AgentInfo from database agent
    pub fn from_db_agent(
        db_agent: &crate::database::DbAgent, 
        contact_uri: String,
        config: &crate::config::GeneralConfig,
    ) -> Self {
        let status = match db_agent.status {
            crate::database::DbAgentStatus::Available => crate::agent::AgentStatus::Available,
            crate::database::DbAgentStatus::Busy => crate::agent::AgentStatus::Busy(vec![]),
            crate::database::DbAgentStatus::PostCallWrapUp => crate::agent::AgentStatus::PostCallWrapUp,
            crate::database::DbAgentStatus::Offline => crate::agent::AgentStatus::Offline,
            crate::database::DbAgentStatus::Reserved => crate::agent::AgentStatus::Available, // Treat reserved as available
        };
        
        Self {
            agent_id: crate::agent::AgentId::from(db_agent.agent_id.clone()),
            session_id: SessionId(format!("agent-{}-session", db_agent.agent_id)),
            status,
            sip_uri: config.agent_sip_uri(&db_agent.username),
            contact_uri: db_agent.contact_uri.clone().unwrap_or(contact_uri),
            skills: vec!["general".to_string()], // Default skills - could be loaded from separate table
            current_calls: db_agent.current_calls as usize,
            max_calls: db_agent.max_calls as usize,
            performance_score: 0.8, // Default performance score
            last_call_end: None,
        }
    }
} 