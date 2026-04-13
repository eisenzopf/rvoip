use std::collections::HashMap;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use tracing::{debug, info, warn};

use super::CallCenterDatabase;

/// Agent information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub sip_uri: String,
    pub display_name: String,
    pub status: AgentStatus,
    pub max_concurrent_calls: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub department: Option<String>,
    pub extension: Option<String>,
    pub phone_number: Option<String>,
}

/// Agent status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentStatus {
    Available,
    Busy { active_calls: u32 },
    Away { reason: String },
    Offline,
    Break { duration_minutes: u32 },
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentStatus::Available => write!(f, "available"),
            AgentStatus::Busy { active_calls } => write!(f, "busy({})", active_calls),
            AgentStatus::Away { reason } => write!(f, "away({})", reason),
            AgentStatus::Offline => write!(f, "offline"),
            AgentStatus::Break { duration_minutes } => write!(f, "break({})", duration_minutes),
        }
    }
}

impl std::str::FromStr for AgentStatus {
    type Err = String;
    
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "available" => Ok(AgentStatus::Available),
            "offline" => Ok(AgentStatus::Offline),
            _ if s.starts_with("busy(") => {
                // Parse busy status - simplified for now
                Ok(AgentStatus::Busy { active_calls: 1 })
            }
            _ if s.starts_with("away(") => {
                // Parse away status - simplified for now
                Ok(AgentStatus::Away { reason: "generic".to_string() })
            }
            _ if s.starts_with("break(") => {
                // Parse break status - simplified for now
                Ok(AgentStatus::Break { duration_minutes: 15 })
            }
            _ => Err(format!("Unknown agent status: {}", s)),
        }
    }
}

/// Agent skill information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    pub agent_id: String,
    pub skill_name: String,
    pub skill_level: u32,
    pub created_at: DateTime<Utc>,
}

/// Request to create a new agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgentRequest {
    pub sip_uri: String,
    pub display_name: String,
    pub max_concurrent_calls: u32,
    pub department: Option<String>,
    pub extension: Option<String>,
    pub phone_number: Option<String>,
}

/// Agent store for database operations
pub struct AgentStore {
    db: CallCenterDatabase,
}

impl AgentStore {
    /// Create a new agent store
    pub fn new(db: CallCenterDatabase) -> Self {
        Self { db }
    }
    
    /// Create a new agent
    pub async fn create_agent(&self, request: CreateAgentRequest) -> Result<Agent> {
        info!("👤 Creating new agent: {}", request.display_name);
        
        let agent_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        
        let agent = Agent {
            id: agent_id.clone(),
            sip_uri: request.sip_uri.clone(),
            display_name: request.display_name.clone(),
            status: AgentStatus::Offline,
            max_concurrent_calls: request.max_concurrent_calls,
            created_at: now,
            updated_at: now,
            last_seen_at: None,
            department: request.department.clone(),
            extension: request.extension.clone(),
            phone_number: request.phone_number.clone(),
        };
        
        let conn = self.db.connection().await;
        
        // Prepare the INSERT statement
        let mut stmt = conn.prepare(
            r#"
            INSERT INTO agents (
                id, sip_uri, display_name, status, max_concurrent_calls,
                created_at, updated_at, department, extension, phone_number
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#
        ).await?;
        
        // Execute with parameters
        stmt.execute([
            agent.id.as_str(),
            agent.sip_uri.as_str(),
            agent.display_name.as_str(),
            &agent.status.to_string(),
            &agent.max_concurrent_calls.to_string(),
            &agent.created_at.to_rfc3339(),
            &agent.updated_at.to_rfc3339(),
            agent.department.as_deref().unwrap_or(""),
            agent.extension.as_deref().unwrap_or(""),
            agent.phone_number.as_deref().unwrap_or(""),
        ]).await?;
        
        info!("✅ Agent created successfully: {} ({})", agent.display_name, agent.id);
        Ok(agent)
    }
    
    /// Get agent by ID
    pub async fn get_agent_by_id(&self, agent_id: &str) -> Result<Option<Agent>> {
        debug!("🔍 Looking up agent: {}", agent_id);
        
        let conn = self.db.connection().await;
        
        let mut stmt = conn.prepare(
            "SELECT id, sip_uri, display_name, status, max_concurrent_calls, created_at, updated_at, last_seen_at, department, extension, phone_number FROM agents WHERE id = ?1"
        ).await?;
        
        let mut rows = stmt.query([agent_id]).await?;
        
        if let Some(row) = rows.next().await? {
            let agent = self.parse_agent_from_row(&row)?;
            Ok(Some(agent))
        } else {
            Ok(None)
        }
    }
    
    /// Get agent by SIP URI
    pub async fn get_agent_by_sip_uri(&self, sip_uri: &str) -> Result<Option<Agent>> {
        debug!("🔍 Looking up agent by SIP URI: {}", sip_uri);
        
        let conn = self.db.connection().await;
        
        let mut stmt = conn.prepare(
            "SELECT id, sip_uri, display_name, status, max_concurrent_calls, created_at, updated_at, last_seen_at, department, extension, phone_number FROM agents WHERE sip_uri = ?1"
        ).await?;
        
        let mut rows = stmt.query([sip_uri]).await?;
        
        if let Some(row) = rows.next().await? {
            let agent = self.parse_agent_from_row(&row)?;
            Ok(Some(agent))
        } else {
            Ok(None)
        }
    }
    
    /// Update agent
    pub async fn update_agent(&self, agent: &Agent) -> Result<bool> {
        info!("📝 Updating agent: {}", agent.id);
        
        let conn = self.db.connection().await;
        let now = Utc::now();
        
        let mut stmt = conn.prepare(
            r#"
            UPDATE agents 
            SET sip_uri = ?1, display_name = ?2, status = ?3, max_concurrent_calls = ?4,
                updated_at = ?5, last_seen_at = ?6, department = ?7, extension = ?8, phone_number = ?9
            WHERE id = ?10
            "#
        ).await?;
        
        stmt.execute([
            agent.sip_uri.as_str(),
            agent.display_name.as_str(),
            &agent.status.to_string(),
            &agent.max_concurrent_calls.to_string(),
            &now.to_rfc3339(),
            &agent.last_seen_at.map(|dt| dt.to_rfc3339()).unwrap_or_default(),
            agent.department.as_deref().unwrap_or(""),
            agent.extension.as_deref().unwrap_or(""),
            agent.phone_number.as_deref().unwrap_or(""),
            agent.id.as_str(),
        ]).await?;
        
        Ok(true)
    }
    
    /// Delete agent
    pub async fn delete_agent(&self, agent_id: &str) -> Result<bool> {
        info!("🗑️ Deleting agent: {}", agent_id);
        
        let conn = self.db.connection().await;
        
        // First delete agent skills
        let mut stmt = conn.prepare("DELETE FROM agent_skills WHERE agent_id = ?1").await?;
        stmt.execute([agent_id]).await?;
        
        // Then delete the agent
        let mut stmt = conn.prepare("DELETE FROM agents WHERE id = ?1").await?;
        stmt.execute([agent_id]).await?;
        
        Ok(true)
    }
    
    /// Get agent skills
    pub async fn get_agent_skills(&self, agent_id: &str) -> Result<Vec<AgentSkill>> {
        debug!("🔍 Getting skills for agent: {}", agent_id);
        
        let conn = self.db.connection().await;
        
        let mut stmt = conn.prepare(
            "SELECT agent_id, skill_name, skill_level, created_at FROM agent_skills WHERE agent_id = ?1 ORDER BY skill_name"
        ).await?;
        
        let mut rows = stmt.query([agent_id]).await?;
        let mut skills = Vec::new();
        
        while let Some(row) = rows.next().await? {
            let skill = AgentSkill {
                agent_id: self.get_string_value(&row, 0)?,
                skill_name: self.get_string_value(&row, 1)?,
                skill_level: self.get_string_value(&row, 2)?.parse::<u32>().unwrap_or(1),
                created_at: DateTime::parse_from_rfc3339(&self.get_string_value(&row, 3)?)?.with_timezone(&Utc),
            };
            skills.push(skill);
        }
        
        Ok(skills)
    }
    
    /// Get all agents
    pub async fn get_all_agents(&self) -> Result<Vec<Agent>> {
        debug!("📋 Getting all agents");
        
        let conn = self.db.connection().await;
        
        let mut stmt = conn.prepare(
            "SELECT id, sip_uri, display_name, status, max_concurrent_calls, created_at, updated_at, last_seen_at, department, extension, phone_number FROM agents ORDER BY created_at DESC"
        ).await?;
        
        let mut rows = stmt.query(()).await?;
        let mut agents = Vec::new();
        
        while let Some(row) = rows.next().await? {
            let agent = self.parse_agent_from_row(&row)?;
            agents.push(agent);
        }
        
        Ok(agents)
    }
    
    /// Add skill to agent
    pub async fn add_skill(&self, agent_id: &str, skill_name: &str, skill_level: u32) -> Result<()> {
        debug!("🎯 Adding skill to agent {}: {} (level {})", agent_id, skill_name, skill_level);
        
        let conn = self.db.connection().await;
        let now = Utc::now();
        
        // First, try to delete any existing skill entry (for update behavior)
        let mut delete_stmt = conn.prepare(
            "DELETE FROM agent_skills WHERE agent_id = ?1 AND skill_name = ?2"
        ).await?;
        
        let _ = delete_stmt.execute([agent_id, skill_name]).await;
        
        // Then insert the new skill
        let mut insert_stmt = conn.prepare(
            r#"
            INSERT INTO agent_skills (agent_id, skill_name, skill_level, created_at)
            VALUES (?1, ?2, ?3, ?4)
            "#
        ).await?;
        
        insert_stmt.execute([
            agent_id,
            skill_name,
            &skill_level.to_string(),
            &now.to_rfc3339(),
        ]).await?;
        
        Ok(())
    }
    
    /// Update agent status
    pub async fn update_agent_status(&self, agent_id: &str, status: AgentStatus) -> Result<bool> {
        info!("📱 Updating agent status: {} -> {}", agent_id, status);
        
        let conn = self.db.connection().await;
        let now = Utc::now();
        
        let mut stmt = conn.prepare(
            r#"
            UPDATE agents 
            SET status = ?1, updated_at = ?2, last_seen_at = ?3
            WHERE id = ?4
            "#
        ).await?;
        
        stmt.execute([
            &status.to_string(),
            &now.to_rfc3339(),
            &now.to_rfc3339(),
            agent_id,
        ]).await?;
        
        Ok(true)
    }
    
    /// Update agent status by SIP URI
    pub async fn update_agent_status_by_sip_uri(&self, sip_uri: &str, status: &str, now: &DateTime<Utc>) -> Result<bool> {
        info!("📱 Updating agent status by SIP URI: {} -> {}", sip_uri, status);
        
        let conn = self.db.connection().await;
        
        let mut stmt = conn.prepare(
            r#"
            UPDATE agents 
            SET status = ?1, updated_at = ?2, last_seen_at = ?3
            WHERE sip_uri = ?4
            "#
        ).await?;
        
        stmt.execute([
            status,
            &now.to_rfc3339(),
            &now.to_rfc3339(),
            sip_uri,
        ]).await?;
        
        Ok(true)
    }
    
    /// Helper method to parse agent from row
    fn parse_agent_from_row(&self, row: &limbo::Row) -> Result<Agent> {
        Ok(Agent {
            id: self.get_string_value(row, 0)?,
            sip_uri: self.get_string_value(row, 1)?,
            display_name: self.get_string_value(row, 2)?,
            status: self.get_string_value(row, 3)?.parse().map_err(|e| anyhow::anyhow!("Invalid status: {}", e))?,
            max_concurrent_calls: self.get_string_value(row, 4)?.parse::<u32>().unwrap_or(1),
            created_at: DateTime::parse_from_rfc3339(&self.get_string_value(row, 5)?)?.with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&self.get_string_value(row, 6)?)?.with_timezone(&Utc),
            last_seen_at: {
                let val = self.get_string_value(row, 7)?;
                if val.is_empty() {
                    None
                } else {
                    Some(DateTime::parse_from_rfc3339(&val)?.with_timezone(&Utc))
                }
            },
            department: {
                let val = self.get_string_value(row, 8)?;
                if val.is_empty() { None } else { Some(val) }
            },
            extension: {
                let val = self.get_string_value(row, 9)?;
                if val.is_empty() { None } else { Some(val) }
            },
            phone_number: {
                let val = self.get_string_value(row, 10)?;
                if val.is_empty() { None } else { Some(val) }
            },
        })
    }
    
    /// Helper method to get string value from row
    fn get_string_value(&self, row: &limbo::Row, index: usize) -> Result<String> {
        match row.get_value(index) {
            Ok(value) => {
                // Convert the Limbo Value to string
                match value {
                    limbo::Value::Text(s) => Ok(s),
                    limbo::Value::Integer(i) => Ok(i.to_string()),
                    limbo::Value::Real(f) => Ok(f.to_string()),
                    limbo::Value::Null => Ok(String::new()),
                    limbo::Value::Blob(b) => Ok(String::from_utf8_lossy(&b).to_string()),
                }
            }
            Err(e) => Err(anyhow::anyhow!("Failed to get value at index {}: {}", index, e)),
        }
    }
    
    /// Get available agents with optional skill requirements
    pub async fn get_available_agents(&self, required_skills: Option<&[String]>) -> Result<Vec<Agent>> {
        debug!("🔍 Finding available agents with skills: {:?}", required_skills);
        
        let conn = self.db.connection().await;
        
        // For now, just get all available agents - skill filtering can be added later
        let mut stmt = conn.prepare(
            "SELECT id, sip_uri, display_name, status, max_concurrent_calls, created_at, updated_at, last_seen_at, department, extension, phone_number FROM agents WHERE status = ?1 ORDER BY last_seen_at ASC"
        ).await?;
        
        let mut rows = stmt.query(["available"]).await?;
        let mut agents = Vec::new();
        
        while let Some(row) = rows.next().await? {
            let agent = self.parse_agent_from_row(&row)?;
            agents.push(agent);
        }
        
        debug!("Found {} available agents", agents.len());
        Ok(agents)
    }
    
    /// List all agents with pagination
    pub async fn list_agents(&self, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<Agent>> {
        debug!("📋 Listing agents (limit: {:?}, offset: {:?})", limit, offset);
        
        let limit = limit.unwrap_or(100);
        let offset = offset.unwrap_or(0);
        
        let conn = self.db.connection().await;
        
        // Use different queries based on offset to work around Limbo's OFFSET parsing
        let mut agents = Vec::new();
        
        if offset == 0 {
            // Simple LIMIT query when no offset needed - use static SQL to avoid parameter issues
            let mut stmt = conn.prepare(
                "SELECT id, sip_uri, display_name, status, max_concurrent_calls, created_at, updated_at, last_seen_at, department, extension, phone_number FROM agents ORDER BY created_at DESC LIMIT 100"
            ).await?;
            
            let mut rows = stmt.query(&[] as &[&str; 0]).await?;
            
            while let Some(row) = rows.next().await? {
                let agent = self.parse_agent_from_row(&row)?;
                agents.push(agent);
            }
        } else {
            // For now, just return all agents and handle pagination in memory
            // This is a workaround for Limbo's OFFSET parsing issue
            let mut stmt = conn.prepare(
                "SELECT id, sip_uri, display_name, status, max_concurrent_calls, created_at, updated_at, last_seen_at, department, extension, phone_number FROM agents ORDER BY created_at DESC"
            ).await?;
            
            let mut rows = stmt.query(&[] as &[&str; 0]).await?;
            let mut all_agents = Vec::new();
            
            while let Some(row) = rows.next().await? {
                let agent = self.parse_agent_from_row(&row)?;
                all_agents.push(agent);
            }
            
            // Apply pagination in memory
            let start = offset as usize;
            let end = start + limit as usize;
            agents = all_agents.into_iter().skip(start).take(limit as usize).collect();
        }
        
        Ok(agents)
    }
} 