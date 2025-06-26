//! Queue-related database operations

use anyhow::{Result, anyhow};
use tracing::{info, warn, debug};
use super::{DatabaseManager, DbQueuedCall, DbQueue, Transaction};
use chrono::{DateTime, Utc};
use serde_json;
use crate::queue::QueuedCall;
use rvoip_session_core::SessionId;
use super::value_helpers::*;
use uuid::Uuid;

impl DatabaseManager {
    /// Enqueue a call
    pub async fn enqueue_call(&self, queue_id: &str, call: &QueuedCall) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let expires_at = (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
        
        // Generate a call ID since QueuedCall doesn't have one
        let call_id = Uuid::new_v4().to_string();
        let session_id = call.session_id.0.clone();
        let priority = call.priority as i64;
        
        // Create customer info as JSON with available fields
        let customer_info = serde_json::json!({
            "caller_id": call.caller_id,
            "queued_at": call.queued_at.to_rfc3339(),
            "retry_count": call.retry_count,
        }).to_string();
        
        self.execute(
            "INSERT INTO call_queue (call_id, session_id, queue_id, customer_info, priority, enqueued_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            vec![
                call_id.into(),
                session_id.into(),
                queue_id.into(),
                customer_info.into(),
                priority.into(),
                now.into(),
                expires_at.into(),
            ] as Vec<limbo::Value>
        ).await?;
        
        Ok(())
    }
    
    /// Dequeue a call for a specific agent (atomic operation)
    pub async fn dequeue_call_for_agent(&self, queue_id: &str, agent_id: &str) -> Result<Option<QueuedCall>> {
        let queue_id = queue_id.to_string();
        let agent_id = agent_id.to_string();
        
        self.transaction(|tx| {
            Box::pin(async move {
                // First, reserve the agent
                let reserved = tx.execute(
                    "UPDATE agents 
                     SET status = 'BUSY', current_calls = current_calls + 1
                     WHERE agent_id = ?1 
                     AND status = 'AVAILABLE' 
                     AND current_calls < max_calls",
                    vec![agent_id.clone().into()] as Vec<limbo::Value>
                ).await?;
                
                if reserved == 0 {
                    // Agent not available
                    return Ok(None);
                }
                
                // Find highest priority call in the queue
                let rows = tx.query(
                    "SELECT * FROM call_queue 
                     WHERE queue_id = ?1 
                     AND expires_at > datetime('now')
                     ORDER BY priority ASC, enqueued_at ASC 
                     LIMIT 1",
                    vec![queue_id.into()] as Vec<limbo::Value>
                ).await?;
                
                if let Some(row) = rows.into_iter().next() {
                    let db_call = Self::parse_db_queued_call_row(&row)?;
                    
                    if let Some(db_call) = db_call {
                        // Convert DbQueuedCall to QueuedCall
                        let call = Self::db_call_to_queued_call(&db_call)?;
                        
                        // Remove from queue and add to active calls
                        tx.execute(
                            "DELETE FROM call_queue WHERE call_id = ?1",
                            vec![db_call.call_id.clone().into()] as Vec<limbo::Value>
                        ).await?;
                        
                        let now = Utc::now().to_rfc3339();
                        tx.execute(
                            "INSERT INTO active_calls (call_id, agent_id, session_id, assigned_at)
                             VALUES (?1, ?2, ?3, ?4)",
                            vec![
                                db_call.call_id.into(),
                                agent_id.clone().into(),
                                call.session_id.0.clone().into(),
                                now.into(),
                            ] as Vec<limbo::Value>
                        ).await?;
                        
                        Ok(Some(call))
                    } else {
                        // Failed to parse call, unreserve agent
                        tx.execute(
                            "UPDATE agents 
                             SET status = 'AVAILABLE', current_calls = current_calls - 1
                             WHERE agent_id = ?1",
                            vec![agent_id.into()] as Vec<limbo::Value>
                        ).await?;
                        Ok(None)
                    }
                } else {
                    // No calls in queue, unreserve agent
                    tx.execute(
                        "UPDATE agents 
                         SET status = 'AVAILABLE', current_calls = current_calls - 1
                         WHERE agent_id = ?1",
                        vec![agent_id.into()] as Vec<limbo::Value>
                    ).await?;
                    Ok(None)
                }
            })
        }).await
    }
    
    /// Get queue depth
    pub async fn get_queue_depth(&self, queue_id: &str) -> Result<usize> {
        let params: Vec<limbo::Value> = vec![queue_id.into()];
        let row = self.query_row(
            "SELECT COUNT(*) FROM call_queue 
             WHERE queue_id = ?1 AND expires_at > datetime('now')",
            params
        ).await?;
        
        if let Some(row) = row {
            let count = value_to_i64(&row.get_value(0)?)?;
            Ok(count as usize)
        } else {
            Ok(0)
        }
    }
    
    /// Get available assignments (returns agent_id, call_id, session_id tuples)
    pub async fn get_available_assignments(&self, queue_id: &str) -> Result<Vec<(String, String, String)>> {
        let params: Vec<limbo::Value> = vec![queue_id.into()];
        let rows = self.query(
            "SELECT a.agent_id, c.call_id, c.session_id
             FROM agents a
             CROSS JOIN (
                 SELECT call_id, session_id 
                 FROM call_queue 
                 WHERE queue_id = ?1 AND expires_at > datetime('now')
                 ORDER BY priority ASC, enqueued_at ASC
             ) c
             WHERE a.status = 'AVAILABLE' 
             AND a.current_calls < a.max_calls
             AND NOT EXISTS (
                 SELECT 1 FROM active_calls ac WHERE ac.call_id = c.call_id
             )
             LIMIT 10",
            params
        ).await?;
        
        let mut assignments = Vec::new();
        for row in rows {
            let agent_id = value_to_string(&row.get_value(0)?)?;
            let call_id = value_to_string(&row.get_value(1)?)?;
            let session_id = value_to_string(&row.get_value(2)?)?;
            assignments.push((agent_id, call_id, session_id));
        }
        
        Ok(assignments)
    }
    
    /// Remove expired calls from queue
    pub async fn remove_expired_calls(&self, queue_id: &str) -> Result<usize> {
        let result = self.execute(
            "DELETE FROM call_queue 
             WHERE queue_id = ?1 AND expires_at <= datetime('now')",
            vec![queue_id.into()] as Vec<limbo::Value>
        ).await?;
        
        Ok(result)
    }
    
    /// Parse queued call row
    fn parse_db_queued_call_row(row: &limbo::Row) -> Result<Option<DbQueuedCall>> {
        let enqueued_str = value_to_string(&row.get_value(5)?)?;
        let enqueued_at = DateTime::parse_from_rfc3339(&enqueued_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse enqueued_at: {}", e))?
            .with_timezone(&Utc);
        
        let last_attempt_str = value_to_optional_string(&row.get_value(7)?);
        let last_attempt = last_attempt_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        
        let expires_str = value_to_string(&row.get_value(8)?)?;
        let expires_at = DateTime::parse_from_rfc3339(&expires_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse expires_at: {}", e))?
            .with_timezone(&Utc);
        
        Ok(Some(DbQueuedCall {
            call_id: value_to_string(&row.get_value(0)?)?,
            session_id: value_to_string(&row.get_value(1)?)?,
            queue_id: value_to_string(&row.get_value(2)?)?,
            customer_info: value_to_optional_string(&row.get_value(3)?),
            priority: value_to_i32(&row.get_value(4)?)?,
            enqueued_at,
            attempts: value_to_i32(&row.get_value(6)?)?,
            last_attempt,
            expires_at,
        }))
    }
    
    /// Convert DbQueuedCall to QueuedCall
    fn db_call_to_queued_call(db_call: &DbQueuedCall) -> Result<QueuedCall> {
        // Parse customer info JSON to get caller_id
        let customer_info: serde_json::Value = if let Some(info) = &db_call.customer_info {
            serde_json::from_str(info)?
        } else {
            serde_json::json!({})
        };
        
        let caller_id = customer_info.get("caller_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        
        let retry_count = customer_info.get("retry_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;
        
        Ok(QueuedCall {
            session_id: SessionId(db_call.session_id.clone()),
            caller_id,
            priority: db_call.priority as u8,
            queued_at: db_call.enqueued_at,
            estimated_wait_time: None,
            retry_count,
        })
    }
} 