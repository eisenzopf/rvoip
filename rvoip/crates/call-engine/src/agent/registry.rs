use std::collections::HashMap;
use std::sync::Arc;
use std::str::FromStr;
use tracing::{info, debug, warn};

use rvoip_session_core::SessionId;

use crate::error::{CallCenterError, Result};
use crate::database::{
    CallCenterDatabase, 
    agent_store::{Agent as DbAgent, AgentStore, CreateAgentRequest, AgentSkill}
};

/// Agent registry for managing call center agents
pub struct AgentRegistry {
    /// Database for agent persistence
    database: CallCenterDatabase,
    
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
    Busy { active_calls: u32 },
    
    /// Agent is away
    Away { reason: String },
    
    /// Agent is offline
    Offline,
    
    /// Agent is in break
    Break { duration_minutes: u32 },
}

impl FromStr for AgentStatus {
    type Err = String;
    
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "available" | "Available" => Ok(AgentStatus::Available),
            "offline" | "Offline" => Ok(AgentStatus::Offline),
            s if s.starts_with("busy") || s.starts_with("Busy") => {
                // Try to parse active calls from string like "Busy(3)"
                Ok(AgentStatus::Busy { active_calls: 0 })
            },
            s if s.starts_with("away") || s.starts_with("Away") => {
                Ok(AgentStatus::Away { reason: "Unknown".to_string() })
            },
            s if s.starts_with("break") || s.starts_with("Break") => {
                Ok(AgentStatus::Break { duration_minutes: 15 })
            },
            _ => Err(format!("Unknown agent status: {}", s))
        }
    }
}

impl ToString for AgentStatus {
    fn to_string(&self) -> String {
        match self {
            AgentStatus::Available => "available".to_string(),
            AgentStatus::Busy { active_calls } => format!("busy({})", active_calls),
            AgentStatus::Away { reason } => format!("away:{}", reason),
            AgentStatus::Offline => "offline".to_string(),
            AgentStatus::Break { duration_minutes } => format!("break({})", duration_minutes),
        }
    }
}

impl From<crate::database::agent_store::AgentStatus> for AgentStatus {
    fn from(db_status: crate::database::agent_store::AgentStatus) -> Self {
        match db_status {
            crate::database::agent_store::AgentStatus::Available => AgentStatus::Available,
            crate::database::agent_store::AgentStatus::Busy { active_calls } => AgentStatus::Busy { active_calls },
            crate::database::agent_store::AgentStatus::Away { reason } => AgentStatus::Away { reason },
            crate::database::agent_store::AgentStatus::Offline => AgentStatus::Offline,
            crate::database::agent_store::AgentStatus::Break { duration_minutes } => AgentStatus::Break { duration_minutes },
        }
    }
}

impl From<AgentStatus> for crate::database::agent_store::AgentStatus {
    fn from(status: AgentStatus) -> Self {
        match status {
            AgentStatus::Available => crate::database::agent_store::AgentStatus::Available,
            AgentStatus::Busy { active_calls } => crate::database::agent_store::AgentStatus::Busy { active_calls },
            AgentStatus::Away { reason } => crate::database::agent_store::AgentStatus::Away { reason },
            AgentStatus::Offline => crate::database::agent_store::AgentStatus::Offline,
            AgentStatus::Break { duration_minutes } => crate::database::agent_store::AgentStatus::Break { duration_minutes },
        }
    }
}

impl AgentRegistry {
    /// Create a new agent registry
    pub fn new(database: CallCenterDatabase) -> Self {
        Self {
            database,
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
        let total = self.agent_status.len();
        let available = self.agent_status.values()
            .filter(|a| matches!(a, AgentStatus::Available))
            .count();
        let busy = self.agent_status.values()
            .filter(|a| matches!(a, AgentStatus::Busy { .. }))
            .count();
        let away = self.agent_status.values()
            .filter(|a| matches!(a, AgentStatus::Away { .. }))
            .count();
        
        AgentStats { total, available, busy, away }
    }
    
    /// Add a new agent to the system
    pub async fn add_agent(&mut self, agent: Agent) -> Result<()> {
        // Store in database
        let agent_store = AgentStore::new(self.database.clone());
        let create_req = CreateAgentRequest {
            sip_uri: agent.sip_uri.clone(),
            display_name: agent.display_name.clone(),
            max_concurrent_calls: agent.max_concurrent_calls,
            department: agent.department.clone(),
            extension: agent.extension.clone(),
            phone_number: None, // Not in our Agent struct
        };
        let db_agent = agent_store.create_agent(create_req).await
            .map_err(|e| CallCenterError::database(&format!("Failed to create agent: {}", e)))?;
        
        // Store skills separately
        for skill in &agent.skills {
            agent_store.add_skill(&db_agent.id, skill, 1).await
                .map_err(|e| CallCenterError::database(&format!("Failed to add skill: {}", e)))?;
        }
        
        // Add to in-memory registry
        self.register_agent(agent).await?;
        Ok(())
    }
    
    /// Update an existing agent
    pub async fn update_agent(&mut self, agent: Agent) -> Result<()> {
        // Update in database
        let agent_store = AgentStore::new(self.database.clone());
        
        // First get the existing agent to preserve the database fields
        let existing = agent_store.get_agent_by_id(&agent.id).await
            .map_err(|e| CallCenterError::database(&format!("Failed to get agent: {}", e)))?;
        
        if let Some(mut db_agent) = existing {
            // Update only the fields we manage
            db_agent.display_name = agent.display_name.clone();
            db_agent.sip_uri = agent.sip_uri.clone();
            db_agent.max_concurrent_calls = agent.max_concurrent_calls;
            db_agent.status = agent.status.clone().into();
            db_agent.department = agent.department.clone();
            db_agent.extension = agent.extension.clone();
            
            agent_store.update_agent(&db_agent).await
                .map_err(|e| CallCenterError::database(&format!("Failed to update agent: {}", e)))?;
            
            // Update skills separately
            // Note: We should clear and re-add skills, but the store doesn't have a clear method
            for skill in &agent.skills {
                agent_store.add_skill(&agent.id, skill, 1).await
                    .map_err(|e| CallCenterError::database(&format!("Failed to update skill: {}", e)))?;
            }
        } else {
            return Err(CallCenterError::not_found(format!("Agent {} not found", agent.id)));
        }
        
        // Update in-memory registry if exists
        if self.agent_status.contains_key(&agent.id) {
            self.agent_status.insert(agent.id.clone(), agent.status);
        }
        Ok(())
    }
    
    /// Remove an agent from the system
    pub async fn remove_agent(&mut self, agent_id: &str) -> Result<()> {
        // Remove from database
        let agent_store = AgentStore::new(self.database.clone());
        agent_store.delete_agent(agent_id).await
            .map_err(|e| CallCenterError::database(&format!("Failed to delete agent: {}", e)))?;
        
        // Remove from in-memory registry
        self.active_sessions.remove(agent_id);
        self.agent_status.remove(agent_id);
        Ok(())
    }
    
    /// List all agents in the system
    pub async fn list_agents(&self) -> Result<Vec<Agent>> {
        let agent_store = AgentStore::new(self.database.clone());
        let db_agents = agent_store.get_all_agents().await
            .map_err(|e| CallCenterError::database(&format!("Failed to list agents: {}", e)))?;
        
        let mut agents = Vec::new();
        for db_agent in db_agents {
            // Get skills for this agent
            let skills = agent_store.get_agent_skills(&db_agent.id).await
                .map_err(|e| CallCenterError::database(&format!("Failed to get skills: {}", e)))?
                .into_iter()
                .map(|s| s.skill_name)
                .collect();
            
            agents.push(Agent {
                id: db_agent.id,
                sip_uri: db_agent.sip_uri,
                display_name: db_agent.display_name,
                skills,
                max_concurrent_calls: db_agent.max_concurrent_calls,
                status: db_agent.status.into(),
                department: db_agent.department,
                extension: db_agent.extension,
            });
        }
        Ok(agents)
    }
    
    /// Update agent skills
    pub async fn update_agent_skills(&mut self, agent_id: &str, skills: Vec<AgentSkill>) -> Result<()> {
        let agent_store = AgentStore::new(self.database.clone());
        
        // Add new skills with their metadata
        // Note: In a real implementation, we'd need to clear existing skills first
        // but the AgentStore doesn't have a clear_skills method
        for skill in skills {
            agent_store.add_skill(agent_id, &skill.skill_name, skill.skill_level).await
                .map_err(|e| CallCenterError::database(&format!("Failed to add skill: {}", e)))?;
        }
        
        Ok(())
    }
}

/// Agent statistics
#[derive(Debug, Clone)]
pub struct AgentStats {
    pub total: usize,
    pub available: usize,
    pub busy: usize,
    pub away: usize,
} 