use std::collections::HashMap;
use std::sync::Arc;
use std::str::FromStr;
use tracing::{info, debug, warn};

use rvoip_session_core::SessionId;

use crate::error::{CallCenterError, Result};

/// Agent registry for managing call center agents
pub struct AgentRegistry {
    /// Active agent sessions (agent_id -> session_id)
    active_sessions: HashMap<String, SessionId>,
    
    /// Current agent status tracking
    agent_status: HashMap<String, AgentStatus>,
}

/// Agent information
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub sip_uri: String,
    pub display_name: String,
    pub skills: Vec<String>,
    pub max_concurrent_calls: u32,
    pub status: AgentStatus,
    pub department: Option<String>,
    pub extension: Option<String>,
}

/// Agent status enumeration
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    /// Agent is available for calls
    Available,
    
    /// Agent is busy with calls
    Busy(Vec<SessionId>),
    
    /// Agent is in post-call wrap-up time
    PostCallWrapUp,
    
    /// Agent is offline
    Offline,
}

impl FromStr for AgentStatus {
    type Err = String;
    
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "available" | "Available" | "AVAILABLE" => Ok(AgentStatus::Available),
            "offline" | "Offline" | "OFFLINE" => Ok(AgentStatus::Offline),
            "postcallwrapup" | "PostCallWrapUp" | "POSTCALLWRAPUP" | "post_call_wrap_up" => {
                Ok(AgentStatus::PostCallWrapUp)
            },
            s if s.starts_with("busy") || s.starts_with("Busy") || s.starts_with("BUSY") => {
                Ok(AgentStatus::Busy(Vec::new()))
            },
            _ => Err(format!("Unknown agent status: {}", s))
        }
    }
}

impl ToString for AgentStatus {
    fn to_string(&self) -> String {
        match self {
            AgentStatus::Available => "available".to_string(),
            AgentStatus::Busy(calls) => format!("busy({})", calls.len()),
            AgentStatus::PostCallWrapUp => "postcallwrapup".to_string(),
            AgentStatus::Offline => "offline".to_string(),
        }
    }
}

impl AgentRegistry {
    /// Create a new agent registry
    pub fn new() -> Self {
        Self {
            active_sessions: HashMap::new(),
            agent_status: HashMap::new(),
        }
    }
    
    /// Register a new agent
    pub async fn register_agent(&mut self, agent: Agent) -> Result<String> {
        info!("👤 Registering agent: {} ({})", agent.display_name, agent.sip_uri);
        
        let agent_id = agent.id.clone();
        self.agent_status.insert(agent_id.clone(), agent.status.clone());
        
        info!("✅ Agent registered: {}", agent_id);
        Ok(agent_id)
    }
    
    /// Update agent status
    pub fn update_agent_status(&mut self, agent_id: &str, status: AgentStatus) -> Result<()> {
        info!("🔄 Agent {} status: {:?}", agent_id, status);
        
        if self.agent_status.contains_key(agent_id) {
            self.agent_status.insert(agent_id.to_string(), status);
            Ok(())
        } else {
            Err(CallCenterError::not_found(format!("Agent not found: {}", agent_id)))
        }
    }
    
    /// Set agent session (when agent logs in)
    pub fn set_agent_session(&mut self, agent_id: String, session_id: SessionId) -> Result<()> {
        info!("🔗 Agent {} session: {}", agent_id, session_id);
        
        if self.agent_status.contains_key(&agent_id) {
            self.active_sessions.insert(agent_id.clone(), session_id);
            self.update_agent_status(&agent_id, AgentStatus::Available)?;
            Ok(())
        } else {
            Err(CallCenterError::not_found(format!("Agent not found: {}", agent_id)))
        }
    }
    
    /// Remove agent session (when agent logs out)
    pub fn remove_agent_session(&mut self, agent_id: &str) -> Result<()> {
        info!("🔌 Agent {} logged out", agent_id);
        
        if self.active_sessions.remove(agent_id).is_some() {
            self.update_agent_status(agent_id, AgentStatus::Offline)?;
            Ok(())
        } else {
            Err(CallCenterError::not_found(format!("No active session for agent: {}", agent_id)))
        }
    }
    
    /// Get agent by ID
    pub async fn get_agent(&self, agent_id: &str) -> Result<Option<Agent>> {
        // TODO: Load from database when integrated
        warn!("🚧 get_agent not yet implemented - returning None");
        Ok(None)
    }
    
    /// Get agent status
    pub fn get_agent_status(&self, agent_id: &str) -> Option<&AgentStatus> {
        self.agent_status.get(agent_id)
    }
    
    /// Get agent session
    pub fn get_agent_session(&self, agent_id: &str) -> Option<&SessionId> {
        self.active_sessions.get(agent_id)
    }
    
    /// Find available agents (excludes agents in post-call wrap-up)
    pub fn find_available_agents(&self) -> Vec<String> {
        self.agent_status.iter()
            .filter(|(_, status)| matches!(status, AgentStatus::Available))
            .map(|(id, _)| id.clone())
            .collect()
    }
    
    /// Find agents with specific skills
    pub async fn find_agents_with_skills(&self, required_skills: &[String]) -> Result<Vec<String>> {
        // TODO: Query database for agents with required skills
        warn!("🚧 find_agents_with_skills not yet implemented");
        Ok(Vec::new())
    }
    
    /// Get all agent statistics
    pub fn get_statistics(&self) -> AgentStats {
        let total = self.agent_status.len();
        let available = self.agent_status.values()
            .filter(|a| matches!(a, AgentStatus::Available))
            .count();
        let busy = self.agent_status.values()
            .filter(|a| matches!(a, AgentStatus::Busy(_)))
            .count();
        let post_call_wrap_up = self.agent_status.values()
            .filter(|a| matches!(a, AgentStatus::PostCallWrapUp))
            .count();
        let offline = self.agent_status.values()
            .filter(|a| matches!(a, AgentStatus::Offline))
            .count();
        
        AgentStats { total, available, busy, post_call_wrap_up, offline }
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Agent statistics
#[derive(Debug, Clone)]
pub struct AgentStats {
    pub total: usize,
    pub available: usize,
    pub busy: usize,
    pub post_call_wrap_up: usize,
    pub offline: usize,
} 