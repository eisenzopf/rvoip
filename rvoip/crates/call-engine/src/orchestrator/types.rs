//! Type definitions for the call center orchestrator
//!
//! This module contains all the core types used throughout the call center
//! orchestration system.

use chrono::{DateTime, Utc};
use rvoip_session_core::{SessionId, BridgeId};
use crate::agent::{AgentId, AgentStatus};

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
    /// The SDP offer from the customer
    pub customer_sdp: Option<String>,
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

/// Call status enumeration
#[derive(Debug, Clone)]
pub enum CallStatus {
    Incoming,
    Routing,     // Being processed by routing engine
    Queued,
    Ringing,     // Ringing at agent
    Connecting,
    Bridged,
    OnHold,
    Transferring,
    Ended,
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