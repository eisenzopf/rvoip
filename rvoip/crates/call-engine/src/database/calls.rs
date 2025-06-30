//! Call database operations (sqlx-based)

pub use super::{DatabaseManager, DbActiveCall};

use anyhow::Result;

impl DatabaseManager {
    /// Get all active calls
    pub async fn get_active_calls(&self) -> Result<Vec<DbActiveCall>> {
        let calls = sqlx::query_as!(
            DbActiveCall,
            "SELECT call_id, agent_id, session_id, customer_dialog_id, agent_dialog_id, assigned_at, answered_at
             FROM active_calls ORDER BY assigned_at DESC"
        )
        .fetch_all(&self.pool)
        .await?;
        
        Ok(calls)
    }
    
    /// Get active calls for a specific agent
    pub async fn get_active_calls_for_agent(&self, agent_id: &str) -> Result<Vec<DbActiveCall>> {
        let calls = sqlx::query_as!(
            DbActiveCall,
            "SELECT call_id, agent_id, session_id, customer_dialog_id, agent_dialog_id, assigned_at, answered_at
             FROM active_calls WHERE agent_id = ?1 ORDER BY assigned_at DESC",
            agent_id
        )
        .fetch_all(&self.pool)
        .await?;
        
        Ok(calls)
    }
    
    /// Get active call by session ID
    pub async fn get_active_call(&self, session_id: &str) -> Result<Option<DbActiveCall>> {
        let call = sqlx::query_as!(
            DbActiveCall,
            "SELECT call_id, agent_id, session_id, customer_dialog_id, agent_dialog_id, assigned_at, answered_at
             FROM active_calls WHERE session_id = ?1",
            session_id
        )
        .fetch_optional(&self.pool)
        .await?;
        
        Ok(call)
    }
} 