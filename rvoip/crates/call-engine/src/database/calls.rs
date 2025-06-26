//! Active call-related database operations

use anyhow::{Result, anyhow};
use tracing::{info, warn, debug};
use super::{DatabaseManager, DbActiveCall, Transaction};
use chrono::{DateTime, Utc};
use super::value_helpers::*;

impl DatabaseManager {
    /// Add a new active call
    pub async fn add_active_call(
        &self,
        call_id: &str,
        agent_id: &str,
        session_id: &str,
        customer_dialog_id: Option<&str>,
        agent_dialog_id: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        
        self.execute(
            "INSERT INTO active_calls (call_id, agent_id, session_id, customer_dialog_id, agent_dialog_id, assigned_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            vec![
                call_id.into(),
                agent_id.into(),
                session_id.into(),
                customer_dialog_id.map(|s| s.into()).unwrap_or(limbo::Value::Null),
                agent_dialog_id.map(|s| s.into()).unwrap_or(limbo::Value::Null),
                now.into(),
            ]
        ).await?;
        
        info!("Active call {} created for agent {}", call_id, agent_id);
        Ok(())
    }
    
    /// Update call dialogs
    pub async fn update_call_dialogs(
        &self,
        call_id: &str,
        customer_dialog_id: Option<&str>,
        agent_dialog_id: Option<&str>,
    ) -> Result<()> {
        self.execute(
            "UPDATE active_calls 
             SET customer_dialog_id = ?2, agent_dialog_id = ?3
             WHERE call_id = ?1",
            vec![
                call_id.into(),
                customer_dialog_id.map(|s| s.into()).unwrap_or(limbo::Value::Null),
                agent_dialog_id.map(|s| s.into()).unwrap_or(limbo::Value::Null),
            ]
        ).await?;
        
        Ok(())
    }
    
    /// Mark call as answered
    pub async fn mark_call_answered(&self, call_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        
        self.execute(
            "UPDATE active_calls SET answered_at = ?1 WHERE call_id = ?2",
            vec![now.into(), call_id.into()] as Vec<limbo::Value>
        ).await?;
        
        info!("Call {} marked as answered", call_id);
        Ok(())
    }
    
    /// Get active call by ID
    pub async fn get_active_call(&self, call_id: &str) -> Result<Option<DbActiveCall>> {
        let params: Vec<limbo::Value> = vec![call_id.into()];
        let row = self.query_row(
            "SELECT * FROM active_calls WHERE call_id = ?1",
            params
        ).await?;
        
        if let Some(row) = row {
            Self::parse_active_call_row(&row)
        } else {
            Ok(None)
        }
    }
    
    /// Get active calls for an agent
    pub async fn get_agent_active_calls(&self, agent_id: &str) -> Result<Vec<DbActiveCall>> {
        let params: Vec<limbo::Value> = vec![agent_id.into()];
        let rows = self.query(
            "SELECT * FROM active_calls WHERE agent_id = ?1",
            params
        ).await?;
        
        let mut calls = Vec::new();
        for row in rows {
            if let Some(call) = Self::parse_active_call_row(&row)? {
                calls.push(call);
            }
        }
        
        Ok(calls)
    }
    
    /// Remove active call
    pub async fn remove_active_call(&self, call_id: &str) -> Result<()> {
        self.execute(
            "DELETE FROM active_calls WHERE call_id = ?1",
            vec![call_id.into()] as Vec<limbo::Value>
        ).await?;
        
        Ok(())
    }
    
    /// Update agent call count after removing call
    pub async fn update_agent_after_call_removal(&self, agent_id: &str) -> Result<()> {
        self.execute(
            "UPDATE agents 
             SET current_calls = MAX(0, current_calls - 1),
                 status = CASE 
                     WHEN current_calls - 1 = 0 THEN 'AVAILABLE'
                     ELSE status
                 END
             WHERE agent_id = ?1",
            vec![agent_id.into()] as Vec<limbo::Value>
        ).await?;
        
        Ok(())
    }
    
    /// Remove active call and update agent (atomic)
    pub async fn remove_active_call_with_agent_update(&self, call_id: &str) -> Result<()> {
        let call_id = call_id.to_string();
        
        self.transaction(|tx| {
            Box::pin(async move {
                // Get the agent ID first
                let rows = tx.query(
                    "SELECT agent_id FROM active_calls WHERE call_id = ?1",
                    vec![call_id.clone().into()] as Vec<limbo::Value>
                ).await?;
                
                if let Some(row) = rows.into_iter().next() {
                    let agent_id = value_to_string(&row.get_value(0)?)?;
                    
                    // Remove the call
                    tx.execute(
                        "DELETE FROM active_calls WHERE call_id = ?1",
                        vec![call_id.into()] as Vec<limbo::Value>
                    ).await?;
                    
                    // Update agent
                    tx.execute(
                        "UPDATE agents 
                         SET current_calls = MAX(0, current_calls - 1),
                             status = CASE 
                                 WHEN current_calls <= 1 THEN 'AVAILABLE'
                                 ELSE status
                             END
                         WHERE agent_id = ?1",
                        vec![agent_id.into()] as Vec<limbo::Value>
                    ).await?;
                }
                
                Ok(())
            })
        }).await
    }
    
    /// Get active calls count
    pub async fn get_active_calls_count(&self) -> Result<usize> {
        let row = self.query_row(
            "SELECT COUNT(*) FROM active_calls",
            ()
        ).await?;
        
        if let Some(row) = row {
            let count = value_to_i64(&row.get_value(0)?)?;
            Ok(count as usize)
        } else {
            Ok(0)
        }
    }
    
    /// Get dialog mapping by customer dialog ID
    pub async fn get_dialog_mapping(&self, dialog_id: &str) -> Result<Option<(Option<String>, String)>> {
        let params: Vec<limbo::Value> = vec![dialog_id.into()];
        let row = self.query_row(
            "SELECT agent_dialog_id, call_id FROM active_calls WHERE customer_dialog_id = ?1",
            params
        ).await?;
        
        if let Some(row) = row {
            let agent_dialog = value_to_optional_string(&row.get_value(0)?);
            let call_id = value_to_string(&row.get_value(1)?)?;
            Ok(Some((agent_dialog, call_id)))
        } else {
            // Try reverse lookup
            let params: Vec<limbo::Value> = vec![dialog_id.into()];
            let row = self.query_row(
                "SELECT customer_dialog_id, call_id FROM active_calls WHERE agent_dialog_id = ?1",
                params
            ).await?;
            
            if let Some(row) = row {
                let customer_dialog = value_to_optional_string(&row.get_value(0)?);
                let call_id = value_to_string(&row.get_value(1)?)?;
                Ok(Some((customer_dialog, call_id)))
            } else {
                Ok(None)
            }
        }
    }
    
    /// Parse active call row
    fn parse_active_call_row(row: &limbo::Row) -> Result<Option<DbActiveCall>> {
        let assigned_str = value_to_string(&row.get_value(5)?)?;
        let assigned_at = DateTime::parse_from_rfc3339(&assigned_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse assigned_at: {}", e))?
            .with_timezone(&Utc);
        
        let answered_str = value_to_optional_string(&row.get_value(6)?);
        let answered_at = answered_str
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.with_timezone(&Utc));
        
        Ok(Some(DbActiveCall {
            call_id: value_to_string(&row.get_value(0)?)?,
            agent_id: value_to_string(&row.get_value(1)?)?,
            session_id: value_to_string(&row.get_value(2)?)?,
            customer_dialog_id: value_to_optional_string(&row.get_value(3)?),
            agent_dialog_id: value_to_optional_string(&row.get_value(4)?),
            assigned_at,
            answered_at,
        }))
    }
    
    /// Remove a call from queue and active calls tables when it ends
    pub async fn remove_call_from_queue(&self, session_id: &str) -> Result<()> {
        // Delete from call_queue table using session_id
        let queue_result = self.execute(
            "DELETE FROM call_queue WHERE session_id = ?1",
            vec![session_id.into()] as Vec<limbo::Value>
        ).await;
        
        // Delete from active_calls table as well
        let active_result = self.execute(
            "DELETE FROM active_calls WHERE session_id = ?1", 
            vec![session_id.into()] as Vec<limbo::Value>
        ).await;
        
        // Log results
        if let Err(e) = queue_result {
            debug!("Call {} was not in call_queue: {}", session_id, e);
        }
        
        if let Err(e) = active_result {
            debug!("Call {} was not in active_calls: {}", session_id, e);
        }
        
        Ok(())
    }
}

/// Active call statistics
#[derive(Debug, Clone)]
pub struct CallStats {
    pub total_active_calls: usize,
    pub answered_calls: usize,
    pub unanswered_calls: usize,
    pub average_answer_time: Option<f64>,
}

impl DatabaseManager {
    /// Get call statistics
    pub async fn get_call_stats(&self) -> Result<(usize, usize, Option<f64>)> {
        let row = self.query_row(
            "SELECT 
                COUNT(*) as total,
                COUNT(answered_at) as answered,
                AVG(CAST((julianday(answered_at) - julianday(assigned_at)) * 86400 AS REAL)) as avg_time_to_answer
             FROM active_calls",
            ()
        ).await?;
        
        if let Some(row) = row {
            let total = value_to_i64(&row.get_value(0)?)? as usize;
            let answered = value_to_i64(&row.get_value(1)?)? as usize;
            let avg_time = value_to_optional_f64(&row.get_value(2)?);
            Ok((total, answered, avg_time))
        } else {
            Ok((0, 0, None))
        }
    }
} 