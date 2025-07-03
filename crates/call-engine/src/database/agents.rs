//! Agent database operations (sqlx-based)
//!
//! All agent database functionality is now implemented in the main database module
//! using sqlx for async-first operations.

pub use super::{DatabaseManager, DbAgent, DbAgentStatus, AgentStats};

// Re-export the agent-specific methods as a trait for convenience
use anyhow::Result;
use crate::agent::AgentStatus;

impl DatabaseManager {
    /// Convenience methods for agent operations
    /// (All actual implementations are in the main mod.rs file)
    
    pub async fn get_agent(&self, agent_id: &str) -> Result<Option<DbAgent>> {
        let agents = sqlx::query_as!(
            DbAgent,
            "SELECT agent_id, username, contact_uri, last_heartbeat, status, current_calls, max_calls, available_since
             FROM agents WHERE agent_id = ?1",
            agent_id
        )
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(agents)
    }
    
    pub async fn list_agents(&self) -> Result<Vec<DbAgent>> {
        let agents = sqlx::query_as!(
            DbAgent,
            "SELECT agent_id, username, contact_uri, last_heartbeat, status, current_calls, max_calls, available_since
             FROM agents ORDER BY agent_id"
        )
        .fetch_all(&self.pool)
        .await?;
        
        Ok(agents)
    }
    
    pub async fn mark_agent_offline(&self, agent_id: &str) -> Result<()> {
        sqlx::query!(
            "UPDATE agents SET status = 'OFFLINE', current_calls = 0 WHERE agent_id = ?1",
            agent_id
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    pub async fn update_agent_heartbeat(&self, agent_id: &str) -> Result<()> {
        let now = chrono::Utc::now();
        
        sqlx::query!(
            "UPDATE agents SET last_heartbeat = ?1 WHERE agent_id = ?2",
            now,
            agent_id
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
} 