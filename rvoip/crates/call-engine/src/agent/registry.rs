use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, debug, warn};

use rvoip_sip_core::Uri;
use rvoip_session_core::SessionId;

use crate::error::{CallCenterError, Result};
use crate::database::{CallCenterDatabase, agent_store::{Agent as DbAgent, AgentStore}};

/// Agent registry for managing call center agents
pub struct AgentRegistry {
    /// Database store for agent persistence
    agent_store: AgentStore,
    
    /// Active agent sessions (agent_id -> session_id)
    active_sessions: HashMap<String, SessionId>,
    
    /// Current agent status tracking
    agent_status: HashMap<String, AgentStatus>,
}

/// Agent information
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub sip_uri: Uri,
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
    Busy { active_calls: u32 },
    
    /// Agent is away
    Away { reason: String },
    
    /// Agent is offline
    Offline,
    
    /// Agent is in break
    Break { duration_minutes: u32 },
}

impl AgentRegistry {
    /// Create a new agent registry
    pub fn new(database: CallCenterDatabase) -> Self {
        let agent_store = AgentStore::new(database);
        
        Self {
            agent_store,
            active_sessions: HashMap::new(),
            agent_status: HashMap::new(),
        }
    }
    
    /// Register a new agent
    pub async fn register_agent(&mut self, agent: Agent) -> Result<String> {
        info!("👤 Registering agent: {} ({})", agent.display_name, agent.sip_uri);
        
        // TODO: Store agent in database using agent_store
        // TODO: Validate agent information
        // TODO: Check for conflicts
        
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
        // TODO: Load from database via agent_store
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
    
    /// Find available agents
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
        let mut available_count = 0;
        let mut busy_count = 0;
        let mut away_count = 0;
        let mut offline_count = 0;
        
        for status in self.agent_status.values() {
            match status {
                AgentStatus::Available => available_count += 1,
                AgentStatus::Busy { .. } => busy_count += 1,
                AgentStatus::Away { .. } => away_count += 1,
                AgentStatus::Offline => offline_count += 1,
                AgentStatus::Break { .. } => away_count += 1,
            }
        }
        
        AgentStats {
            total_agents: self.agent_status.len(),
            available_agents: available_count,
            busy_agents: busy_count,
            away_agents: away_count,
            offline_agents: offline_count,
            active_sessions: self.active_sessions.len(),
        }
    }
}

/// Agent statistics
#[derive(Debug, Clone)]
pub struct AgentStats {
    pub total_agents: usize,
    pub available_agents: usize,
    pub busy_agents: usize,
    pub away_agents: usize,
    pub offline_agents: usize,
    pub active_sessions: usize,
} 