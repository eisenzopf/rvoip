//! Supervisor API for call center oversight
//!
//! This module provides APIs for supervisors to monitor and manage
//! call center operations in real-time.

use std::sync::Arc;
use chrono::{DateTime, Utc};
use rvoip_session_core::{SessionId, BridgeId};

use crate::{
    agent::AgentId,
    error::Result as CallCenterResult,
    orchestrator::{CallCenterEngine, types::{AgentInfo, CallInfo, OrchestratorStats}},
    queue::QueueStats,
};

/// Supervisor API for call center oversight
/// 
/// This provides comprehensive monitoring and management capabilities:
/// - Real-time agent and call monitoring
/// - Queue statistics and management
/// - Call listening and coaching features
/// - Performance metrics and reporting
#[derive(Clone)]
pub struct SupervisorApi {
    engine: Arc<CallCenterEngine>,
}

impl SupervisorApi {
    /// Create a new supervisor API instance
    pub fn new(engine: Arc<CallCenterEngine>) -> Self {
        Self { engine }
    }
    
    /// Get real-time orchestrator statistics
    /// 
    /// Returns comprehensive statistics including:
    /// - Active calls count
    /// - Available/busy agents
    /// - Queue depths
    /// - Routing performance metrics
    pub async fn get_stats(&self) -> OrchestratorStats {
        self.engine.get_stats().await
    }
    
    /// List all agents with their current status
    /// 
    /// Returns detailed information for each agent including:
    /// - Current status (available, busy, away)
    /// - Active calls count
    /// - Skills and performance score
    /// - Last call timestamp
    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        self.engine.list_agents().await
    }
    
    /// Get detailed information about a specific agent
    pub async fn get_agent_details(&self, agent_id: &AgentId) -> Option<AgentInfo> {
        self.engine.get_agent_info(agent_id).await
    }
    
    /// List all active calls
    /// 
    /// Returns information about all calls currently in the system
    pub async fn list_active_calls(&self) -> Vec<CallInfo> {
        self.engine.active_calls()
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get detailed information about a specific call
    pub async fn get_call_details(&self, session_id: &SessionId) -> Option<CallInfo> {
        self.engine.active_calls()
            .get(session_id)
            .map(|entry| entry.clone())
    }
    
    /// Monitor calls assigned to a specific agent
    pub async fn monitor_agent_calls(&self, agent_id: &AgentId) -> Vec<CallInfo> {
        self.engine.active_calls()
            .iter()
            .filter(|entry| entry.value().agent_id.as_ref() == Some(agent_id))
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get queue statistics for all queues
    pub async fn get_all_queue_stats(&self) -> CallCenterResult<Vec<(String, QueueStats)>> {
        self.engine.get_queue_stats().await
    }
    
    /// Get calls in a specific queue
    pub async fn get_queued_calls(&self, queue_id: &str) -> Vec<CallInfo> {
        self.engine.active_calls()
            .iter()
            .filter(|entry| {
                let call = entry.value();
                call.queue_id.as_ref().map(|q| q == queue_id).unwrap_or(false) &&
                matches!(call.status, crate::orchestrator::types::CallStatus::Queued)
            })
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// List all active bridges (connected calls)
    pub async fn list_active_bridges(&self) -> Vec<rvoip_session_core::BridgeInfo> {
        self.engine.list_active_bridges().await
    }
    
    /// Force assign a queued call to a specific agent
    /// 
    /// This allows supervisors to manually route calls when needed
    pub async fn force_assign_call(
        &self, 
        session_id: SessionId, 
        agent_id: AgentId
    ) -> CallCenterResult<()> {
        self.engine.assign_agent_to_call(session_id, agent_id).await
    }
    
    /// Get performance metrics for a specific time period
    pub async fn get_performance_metrics(
        &self,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> PerformanceMetrics {
        let stats = self.engine.routing_stats().read().await;
        
        // In a real implementation, this would query historical data
        PerformanceMetrics {
            total_calls: (stats.calls_routed_directly + stats.calls_queued) as usize,
            calls_answered: stats.calls_routed_directly as usize,
            calls_queued: stats.calls_queued as usize,
            calls_abandoned: stats.calls_rejected as usize,
            average_wait_time_ms: stats.average_routing_time_ms,
            average_handle_time_ms: 180000, // 3 minutes placeholder
            service_level_percentage: 85.0, // Placeholder
            start_time,
            end_time,
        }
    }
    
    /// Listen to a live call (supervisor monitoring)
    /// 
    /// Returns the bridge ID that can be used to join the call in listen-only mode
    /// Note: Actual implementation would require additional session-core support
    pub async fn listen_to_call(&self, session_id: &SessionId) -> CallCenterResult<Option<BridgeId>> {
        Ok(self.engine.active_calls()
            .get(session_id)
            .and_then(|entry| entry.bridge_id.clone()))
    }
    
    /// Send a message to an agent during a call (coaching)
    /// 
    /// Note: This is a placeholder - actual implementation would require
    /// whisper/coaching support in the media layer
    pub async fn coach_agent(
        &self,
        agent_id: &AgentId,
        message: &str,
    ) -> CallCenterResult<()> {
        tracing::info!("Supervisor coaching message to {}: {}", agent_id, message);
        // TODO: Implement actual whisper/coaching functionality
        Ok(())
    }
}

/// Performance metrics for a time period
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    pub total_calls: usize,
    pub calls_answered: usize,
    pub calls_queued: usize,
    pub calls_abandoned: usize,
    pub average_wait_time_ms: u64,
    pub average_handle_time_ms: u64,
    pub service_level_percentage: f32,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

impl Default for SupervisorApi {
    fn default() -> Self {
        panic!("SupervisorApi requires an engine instance")
    }
} 