//! # Async Agent Database Operations (Future State with sqlx)
//! 
//! This module demonstrates how agent database operations will work with sqlx.
//! All operations are naturally async and Send-safe, eliminating the current issues.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{SqlitePool, Row};
use tracing::{info, debug};
use crate::agent::AgentStatus;

/// Async database manager using sqlx - no Send trait issues!
#[derive(Clone)]
pub struct AsyncAgentDatabase {
    pool: SqlitePool,
}

/// Agent record that derives from database rows automatically
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct AsyncDbAgent {
    pub agent_id: String,
    pub username: String,
    pub status: String,
    pub max_calls: i32,
    pub current_calls: i32,
    pub contact_uri: Option<String>,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub available_since: Option<DateTime<Utc>>,
}

impl AsyncAgentDatabase {
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(database_url).await?;
        Ok(Self { pool })
    }

    /// Update agent status - completely Send-safe!
    pub async fn update_agent_status(&self, agent_id: &str, status: AgentStatus) -> Result<()> {
        let status_str = match status {
            AgentStatus::Available => "AVAILABLE",
            AgentStatus::Busy(_) => "BUSY",
            AgentStatus::PostCallWrapUp => "POSTCALLWRAPUP",
            AgentStatus::Offline => "OFFLINE",
        };

        if matches!(status, AgentStatus::Available) {
            let now = Utc::now();
            sqlx::query!(
                "UPDATE agents SET status = $1, available_since = $2 WHERE agent_id = $3",
                status_str,
                now,
                agent_id
            )
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query!(
                "UPDATE agents SET status = $1, available_since = NULL WHERE agent_id = $2",
                status_str,
                agent_id
            )
            .execute(&self.pool)
            .await?;
        }
        
        debug!("Agent {} status updated to {}", agent_id, status_str);
        Ok(())
    }

    /// Get available agents - type-safe and async
    pub async fn get_available_agents(&self) -> Result<Vec<AsyncDbAgent>> {
        let agents = sqlx::query_as!(
            AsyncDbAgent,
            "SELECT agent_id, username, contact_uri, status, current_calls, max_calls, 
                    last_heartbeat, available_since
             FROM agents 
             WHERE status = 'AVAILABLE' AND current_calls < max_calls
             ORDER BY available_since ASC"
        )
        .fetch_all(&self.pool)
        .await?;

        info!("Found {} available agents", agents.len());
        Ok(agents)
    }

    /// Reserve agent atomically - proper async transactions
    pub async fn reserve_agent(&self, agent_id: &str) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        
        let result = sqlx::query!(
            "UPDATE agents 
             SET status = 'RESERVED' 
             WHERE agent_id = $1 AND status = 'AVAILABLE'",
            agent_id
        )
        .execute(&mut *tx)
        .await?;
        
        let success = result.rows_affected() > 0;
        
        if success {
            tx.commit().await?;
            debug!("Agent {} reserved successfully", agent_id);
        } else {
            tx.rollback().await?;
        }
        
        Ok(success)
    }

    /// Update agent call count - no parameter conversion issues
    pub async fn update_agent_call_count(&self, agent_id: &str, delta: i32) -> Result<()> {
        sqlx::query!(
            "UPDATE agents 
             SET current_calls = MAX(0, current_calls + $1)
             WHERE agent_id = $2",
            delta,
            agent_id
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }

    /// Complex query with joins - compile-time checked
    pub async fn get_agent_with_stats(&self, agent_id: &str) -> Result<Option<AgentWithStats>> {
        let result = sqlx::query_as!(
            AgentWithStats,
            "SELECT 
                a.agent_id,
                a.username,
                a.status,
                a.current_calls,
                a.max_calls,
                COUNT(ac.call_id) as total_calls_today,
                AVG(cr.duration_seconds) as avg_call_duration
             FROM agents a
             LEFT JOIN active_calls ac ON a.agent_id = ac.agent_id
             LEFT JOIN call_records cr ON a.agent_id = cr.agent_id 
                 AND date(cr.start_time) = date('now')
             WHERE a.agent_id = $1
             GROUP BY a.agent_id",
            agent_id
        )
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(result)
    }

    /// Bulk operations are efficient and type-safe
    pub async fn update_multiple_agents_status(&self, agent_ids: &[String], status: &str) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        let mut total_updated = 0;
        
        for agent_id in agent_ids {
            let result = sqlx::query!(
                "UPDATE agents SET status = $1 WHERE agent_id = $2",
                status,
                agent_id
            )
            .execute(&mut *tx)
            .await?;
            
            total_updated += result.rows_affected();
        }
        
        tx.commit().await?;
        info!("Updated {} agents to status {}", total_updated, status);
        Ok(total_updated)
    }
}

#[derive(sqlx::FromRow, Debug)]
pub struct AgentWithStats {
    pub agent_id: String,
    pub username: String,
    pub status: String,
    pub current_calls: i32,
    pub max_calls: i32,
    pub total_calls_today: i64,
    pub avg_call_duration: Option<f64>,
}

// Usage in orchestrator - no more Send issues!
pub async fn example_orchestrator_usage(db: AsyncAgentDatabase, agent_id: String) {
    // This code can be spawned without any Send trait issues
    tokio::spawn(async move {
        // All operations are naturally Send + Sync
        if let Ok(success) = db.reserve_agent(&agent_id).await {
            if success {
                info!("Agent {} reserved for call", agent_id);
                
                // Update call count
                db.update_agent_call_count(&agent_id, 1).await.ok();
                
                // Update status
                db.update_agent_status(&agent_id, AgentStatus::Busy(vec![])).await.ok();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_agent_operations() {
        let db = AsyncAgentDatabase::new("sqlite::memory:").await.unwrap();
        
        // All operations work seamlessly in async tests
        let agents = db.get_available_agents().await.unwrap();
        assert!(agents.is_empty());
        
        // No more fighting with trait objects or Send bounds
        let success = db.reserve_agent("agent-001").await.unwrap();
        assert!(!success); // Agent doesn't exist yet
    }
} 