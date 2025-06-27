//! Agent-related database operations

use anyhow::{Result, anyhow};
use tracing::{info, warn, debug};
use super::{DatabaseManager, DbAgent, DbAgentStatus, Transaction};
use chrono::{DateTime, Utc};
use super::value_helpers::*;
use crate::agent::{AgentId, AgentStatus};

impl DatabaseManager {
    /// Register or update an agent (simplified for Limbo compatibility)
    pub async fn upsert_agent(&self, agent_id: &str, username: &str, contact_uri: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        
        debug!("🔍 upsert_agent called with agent_id='{}', username='{}', contact_uri='{:?}'", 
               agent_id, username, contact_uri);
        
        // Try to update existing agent first (with availability timestamp)
        let updated_rows = self.execute(
            "UPDATE agents 
             SET username = ?1, 
                 contact_uri = ?2, 
                 last_heartbeat = ?3,
                 status = 'AVAILABLE',
                 available_since = ?4
             WHERE agent_id = ?5",
            vec![
                username.into(),
                contact_uri.map(|s| s.into()).unwrap_or(limbo::Value::Null),
                now.clone().into(),
                now.clone().into(), // Set available_since timestamp
                agent_id.into(),
            ] as Vec<limbo::Value>
        ).await?;
        
        if updated_rows == 0 {
            // Agent doesn't exist, try to insert (with availability timestamp)
            let _result = self.execute(
                "INSERT INTO agents (agent_id, username, contact_uri, last_heartbeat, status, current_calls, max_calls, available_since)
                 VALUES (?1, ?2, ?3, ?4, 'AVAILABLE', 0, 1, ?5)",
                vec![
                    agent_id.into(),
                    username.into(), 
                    contact_uri.map(|s| s.into()).unwrap_or(limbo::Value::Null),
                    now.clone().into(),
                    now.into(), // Set available_since timestamp  
                ] as Vec<limbo::Value>
            ).await; // Ignore errors in case of duplicates
            
            debug!("🔍 Attempted to insert new agent {} as AVAILABLE with timestamp", agent_id);
        } else {
            debug!("🔍 Updated existing agent {} to AVAILABLE with timestamp: {} rows affected", agent_id, updated_rows);
        }
        
        info!("Agent {} processed in database with contact {:?}", agent_id, contact_uri);
        Ok(())
    }
    
    /// Update agent status (with availability timestamp for fair round robin)
    pub async fn update_agent_status(&self, agent_id: &str, status: AgentStatus) -> Result<()> {
        let status_str = match status {
            AgentStatus::Available => "AVAILABLE",
            AgentStatus::Busy(_) => "BUSY",
            AgentStatus::PostCallWrapUp => "POSTCALLWRAPUP",
            AgentStatus::Offline => "OFFLINE",
        };
        
        // If transitioning to AVAILABLE, update the available_since timestamp for fairness
        if matches!(status, AgentStatus::Available) {
            let now = chrono::Utc::now().to_rfc3339();
            self.execute(
                "UPDATE agents SET status = ?1, available_since = ?2 WHERE agent_id = ?3",
                vec![status_str.into(), now.into(), agent_id.into()] as Vec<limbo::Value>
            ).await?;
            debug!("Agent {} status updated to {:?} with available_since timestamp", agent_id, status);
        } else {
            // For non-available states, clear the available_since timestamp
            self.execute(
                "UPDATE agents SET status = ?1, available_since = NULL WHERE agent_id = ?2",
                vec![status_str.into(), agent_id.into()] as Vec<limbo::Value>
            ).await?;
            debug!("Agent {} status updated to {:?}", agent_id, status);
        }
        
        Ok(())
    }
    
    /// Update agent call count
    pub async fn update_agent_call_count(&self, agent_id: &str, delta: i32) -> Result<()> {
        self.execute(
            "UPDATE agents 
             SET current_calls = MAX(0, current_calls + ?1)
             WHERE agent_id = ?2",
            vec![(delta as i64).into(), agent_id.into()] as Vec<limbo::Value>
        ).await?;
        
        Ok(())
    }
    
    /// Get available agents for assignment (ordered by fairness - longest available time first)
    pub async fn get_available_agents(&self) -> Result<Vec<DbAgent>> {
        debug!("🔍 Getting available agents with fairness ordering...");
        
        let rows = self.query(
            "SELECT agent_id, username, contact_uri, status, current_calls, max_calls, available_since
             FROM agents 
             WHERE status = 'AVAILABLE' 
             AND current_calls < max_calls
             AND available_since IS NOT NULL
             ORDER BY available_since ASC",  // Oldest available_since timestamp first = fairest
            vec![] as Vec<limbo::Value>
        ).await?;

        let mut agents = Vec::new();
        for row in rows {
            if let (
                Ok(limbo::Value::Text(agent_id)),
                Ok(limbo::Value::Text(username)), 
                contact_uri,
                Ok(limbo::Value::Text(status)),
                Ok(current_calls),
                Ok(max_calls),
                available_since
            ) = (
                row.get_value(0), row.get_value(1), row.get_value(2), 
                row.get_value(3), row.get_value(4), row.get_value(5), row.get_value(6)
            ) {
                let contact_uri = match contact_uri {
                    Ok(limbo::Value::Text(uri)) => Some(uri.clone()),
                    _ => None,
                };
                
                let available_since_str = match available_since {
                    Ok(limbo::Value::Text(ts)) => Some(ts.clone()),
                    _ => None,
                };
                
                let current_calls = match current_calls {
                    limbo::Value::Integer(n) => n as i32,
                    _ => 0,
                };
                
                let max_calls = match max_calls {
                    limbo::Value::Integer(n) => n as i32,
                    _ => 1,
                };

                let db_status = DbAgentStatus::from_str(&status).unwrap_or(DbAgentStatus::Offline);

                agents.push(DbAgent {
                    agent_id: agent_id.clone(),
                    username: username.clone(),
                    contact_uri,
                    status: db_status,
                    current_calls,
                    max_calls,
                    last_heartbeat: None, // Simplified for now
                    available_since: available_since_str.clone(),
                });
                
                info!("🔍 FAIRNESS: Agent {} (available since: {:?}) added to list", 
                       agent_id, available_since_str);
            }
        }

        info!("🔍 FAIRNESS: Found {} available agents in order:", agents.len());
        for (idx, agent) in agents.iter().enumerate() {
            info!("🔍 FAIRNESS: {}. {} (since: {:?})", 
                  idx + 1, agent.agent_id, agent.available_since);
        }

        Ok(agents)
    }
    
    /// Get a specific agent
    pub async fn get_agent(&self, agent_id: &str) -> Result<Option<DbAgent>> {
        let params: Vec<limbo::Value> = vec![agent_id.into()];
        let row = self.query_row(
            "SELECT id, agent_id, username, contact_uri, last_heartbeat, status, current_calls, max_calls FROM agents WHERE agent_id = ?1",
            params
        ).await?;
        
        match row {
            Some(row) => Ok(Some(self.row_to_agent(&row)?)),
            None => Ok(None),
        }
    }
    
    /// Get all agents
    pub async fn list_agents(&self) -> Result<Vec<DbAgent>> {
        let rows = self.query("SELECT id, agent_id, username, contact_uri, last_heartbeat, status, current_calls, max_calls FROM agents ORDER BY agent_id", ()).await?;
        
        let mut agents = Vec::new();
        for row in rows {
            agents.push(self.row_to_agent(&row)?);
        }
        
        Ok(agents)
    }
    
    /// Mark agent as offline
    pub async fn mark_agent_offline(&self, agent_id: &str) -> Result<()> {
        self.execute(
            "UPDATE agents SET status = 'OFFLINE', current_calls = 0 WHERE agent_id = ?1",
            vec![agent_id.into()] as Vec<limbo::Value>
        ).await?;
        
        info!("Agent {} marked offline", agent_id);
        Ok(())
    }
    
    /// Reserve an agent for assignment (atomic operation)
    pub async fn reserve_agent(&self, agent_id: &str) -> Result<bool> {
        let agent_id = agent_id.to_string();
        let result = self.transaction(|tx| {
            let agent_id = agent_id.clone();
            Box::pin(async move {
                // Try to reserve the agent
                let rows = tx.execute(
                    "UPDATE agents 
                     SET status = 'RESERVED' 
                     WHERE agent_id = ?1 AND status = 'AVAILABLE'",
                    vec![agent_id.into()] as Vec<limbo::Value>
                ).await?;
                
                Ok(rows > 0)
            })
        }).await?;
        
        if result {
            debug!("Agent {} reserved successfully", agent_id);
        }
        
        Ok(result)
    }
    
    /// Release a reserved agent
    pub async fn release_agent_reservation(&self, agent_id: &str) -> Result<()> {
        self.execute(
            "UPDATE agents SET status = 'AVAILABLE' WHERE agent_id = ?1 AND status = 'RESERVED'",
            vec![agent_id.into()] as Vec<limbo::Value>
        ).await?;
        
        debug!("Agent {} reservation released", agent_id);
        Ok(())
    }
    
    /// Update agent heartbeat
    pub async fn update_agent_heartbeat(&self, agent_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        
        self.execute(
            "UPDATE agents SET last_heartbeat = ?1 WHERE agent_id = ?2",
            vec![
                now.into(),
                agent_id.into(),
            ] as Vec<limbo::Value>
        ).await?;
        
        Ok(())
    }
    
    /// Clean up stale agents (offline if no heartbeat for 5 minutes)
    pub async fn cleanup_stale_agents(&self) -> Result<usize> {
        let cutoff = Utc::now().to_rfc3339();
        
        let rows = self.execute(
            "UPDATE agents 
             SET status = 'OFFLINE', current_calls = 0 
             WHERE last_heartbeat < datetime('now', '-5 minutes') 
             AND status != 'OFFLINE'",
            ()
        ).await?;
        
        if rows > 0 {
            info!("Cleaned up {} stale agents", rows);
        }
        
        Ok(rows)
    }
    
    /// Convert database row to agent struct
    /// Column order: id, agent_id, username, contact_uri, last_heartbeat, status, current_calls, max_calls
    fn row_to_agent(&self, row: &limbo::Row) -> Result<DbAgent> {
        let status_str = value_to_string(&row.get_value(5)?)?; // status is at index 5
        let status = DbAgentStatus::from_str(&status_str)
            .ok_or_else(|| anyhow!("Invalid agent status: {}", status_str))?;
        
        let last_heartbeat_str = value_to_optional_string(&row.get_value(4)?); // last_heartbeat is at index 4
        let last_heartbeat = last_heartbeat_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        
        Ok(DbAgent {
            agent_id: value_to_string(&row.get_value(1)?)?,      // agent_id at index 1
            username: value_to_string(&row.get_value(2)?)?,      // username at index 2
            status,
            max_calls: value_to_i32(&row.get_value(7)?)?,        // max_calls at index 7
            current_calls: value_to_i32(&row.get_value(6)?)?,    // current_calls at index 6
            contact_uri: value_to_optional_string(&row.get_value(3)?), // contact_uri at index 3
            last_heartbeat,
            available_since: None, // Not included in standard queries, only in get_available_agents
        })
    }
    
    /// Count total number of agents in the system
    pub async fn count_total_agents(&self) -> Result<usize> {
        let query = "SELECT COUNT(*) as count FROM agents";
        let rows = self.query(query, ()).await?;
        
        if let Some(row) = rows.into_iter().next() {
            let count = value_to_i64(&row.get_value(0)?)?;
            Ok(count as usize)
        } else {
            Ok(0)
        }
    }
}

/// Agent statistics
#[derive(Debug, Clone)]
pub struct AgentStats {
    pub total_agents: usize,
    pub available_agents: usize,
    pub busy_agents: usize,
    pub post_call_wrap_up_agents: usize,
    pub offline_agents: usize,
    pub reserved_agents: usize,
}

impl DatabaseManager {
    /// Get agent statistics
    pub async fn get_agent_stats(&self) -> Result<AgentStats> {
        let rows = self.query(
            "SELECT status, COUNT(*) as count FROM agents GROUP BY status",
            ()
        ).await?;
        
        let mut stats = AgentStats {
            total_agents: 0,
            available_agents: 0,
            busy_agents: 0,
            post_call_wrap_up_agents: 0,
            offline_agents: 0,
            reserved_agents: 0,
        };
        
        for row in rows {
            let status: String = value_to_string(&row.get_value(0)?)?;
            let count: i64 = value_to_i64(&row.get_value(1)?)?;
            let count = count as usize;
            
            stats.total_agents += count;
            
            match status.as_str() {
                "AVAILABLE" => stats.available_agents = count,
                "BUSY" => stats.busy_agents = count,
                "POSTCALLWRAPUP" => stats.post_call_wrap_up_agents = count,
                "OFFLINE" => stats.offline_agents = count,
                "RESERVED" => stats.reserved_agents = count,
                _ => warn!("Unknown agent status in database: {}", status),
            }
        }
        
        Ok(stats)
    }
} 