//! Queue database operations (sqlx-based)

pub use super::{DatabaseManager, DbQueuedCall, DbQueue};

use anyhow::Result;
use chrono::{DateTime, Utc};

impl DatabaseManager {
    /// Get all queued calls for a specific queue
    pub async fn get_queued_calls(&self, queue_id: &str) -> Result<Vec<DbQueuedCall>> {
        let calls = sqlx::query_as!(
            DbQueuedCall,
            "SELECT call_id, session_id, queue_id, customer_info, priority, enqueued_at, attempts, last_attempt, expires_at
             FROM call_queue 
             WHERE queue_id = ?1 AND expires_at > datetime('now')
             ORDER BY priority DESC, enqueued_at ASC",
            queue_id
        )
        .fetch_all(&self.pool)
        .await?;
        
        Ok(calls)
    }
    
    /// Get queue configuration
    pub async fn get_queue(&self, queue_id: &str) -> Result<Option<DbQueue>> {
        let queue = sqlx::query_as!(
            DbQueue,
            "SELECT queue_id, name, description, max_wait_time, priority_routing FROM queues WHERE queue_id = ?1",
            queue_id
        )
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(queue)
    }
    
    /// Get all queues
    pub async fn list_queues(&self) -> Result<Vec<DbQueue>> {
        let queues = sqlx::query_as!(
            DbQueue,
            "SELECT queue_id, name, description, max_wait_time, priority_routing FROM queues ORDER BY name"
        )
        .fetch_all(&self.pool)
        .await?;
        
        Ok(queues)
    }
} 