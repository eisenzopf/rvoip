//! # Active Call Database Operations
//!
//! This module provides comprehensive database operations for managing active calls
//! within the call center system. It handles the lifecycle of calls from assignment
//! to agents through completion, including dialog management, status tracking, and
//! atomic operations to ensure data consistency.
//!
//! ## Overview
//!
//! Active call operations are critical for maintaining real-time state consistency
//! between the call center engine and the database. This module provides atomic
//! operations for call assignment, status updates, and cleanup to ensure data
//! integrity during high-volume call processing.
//!
//! ## Key Features
//!
//! - **Call Lifecycle Management**: Complete active call lifecycle from assignment to completion
//! - **Agent Assignment**: Atomic operations for assigning calls to agents
//! - **Dialog Tracking**: Management of customer and agent dialog identifiers
//! - **Status Updates**: Real-time call status tracking and updates
//! - **Atomic Operations**: Transaction-safe operations to prevent data corruption
//! - **Performance Monitoring**: Call statistics and performance metrics
//! - **Cleanup Operations**: Safe removal of completed calls with agent state updates
//!
//! ## Database Schema
//!
//! The active calls operations work with the following key tables:
//!
//! ### active_calls Table
//! - `call_id`: Unique identifier for the call
//! - `agent_id`: Assigned agent identifier
//! - `session_id`: Session management identifier
//! - `customer_dialog_id`: Customer-side dialog identifier
//! - `agent_dialog_id`: Agent-side dialog identifier
//! - `assigned_at`: Timestamp when call was assigned
//! - `answered_at`: Timestamp when call was answered
//!
//! ### Integration Points
//! - Agent status updates for availability tracking
//! - Queue management for call routing
//! - Session coordination for call state management
//!
//! ## Examples
//!
//! ### Basic Call Assignment
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! use chrono::Utc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Add a new active call when agent is assigned
//! db.add_active_call(
//!     "call-12345",
//!     "agent-001", 
//!     "session-abc123",
//!     Some("customer-dialog-456"),
//!     Some("agent-dialog-789")
//! ).await?;
//! 
//! println!("✅ Active call created and assigned to agent");
//! 
//! // Mark call as answered when agent picks up
//! db.mark_call_answered("call-12345").await?;
//! println!("📞 Call marked as answered");
//! # Ok(())
//! # }
//! ```
//!
//! ### Call Status Tracking
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Check if a specific call is active
//! if let Some(call) = db.get_active_call("call-12345").await? {
//!     println!("📋 Active Call Details:");
//!     println!("  Call ID: {}", call.call_id);
//!     println!("  Agent: {}", call.agent_id);
//!     println!("  Session: {}", call.session_id);
//!     println!("  Assigned: {}", call.assigned_at);
//!     
//!     if let Some(answered) = call.answered_at {
//!         let duration = answered.signed_duration_since(call.assigned_at);
//!         println!("  Answer time: {}s", duration.num_seconds());
//!     } else {
//!         println!("  Status: Ringing (not yet answered)");
//!     }
//! } else {
//!     println!("❌ Call not found or no longer active");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Call Management
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Get all active calls for a specific agent
//! let agent_calls = db.get_agent_active_calls("agent-001").await?;
//! 
//! println!("📞 Agent's Active Calls:");
//! if agent_calls.is_empty() {
//!     println!("  No active calls");
//! } else {
//!     for call in agent_calls {
//!         let status = if call.answered_at.is_some() {
//!             "Connected"
//!         } else {
//!             "Ringing"
//!         };
//!         
//!         println!("  Call {}: {} ({})", call.call_id, status, call.session_id);
//!     }
//! }
//! 
//! // Get system-wide call statistics
//! let (total, answered, avg_answer_time) = db.get_call_stats().await?;
//! println!("\n📊 System Statistics:");
//! println!("  Total active calls: {}", total);
//! println!("  Answered calls: {}", answered);
//! if let Some(avg_time) = avg_answer_time {
//!     println!("  Average answer time: {:.1}s", avg_time);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Atomic Call Completion
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // When a call ends, atomically remove it and update agent status
//! // This ensures the agent's availability is properly updated
//! db.remove_active_call_with_agent_update("call-12345").await?;
//! 
//! println!("✅ Call completed and agent status updated atomically");
//! 
//! // Alternative: Remove call from queue when call ends
//! db.remove_call_from_queue("session-abc123").await?;
//! println!("📋 Call removed from queue system");
//! # Ok(())
//! # }
//! ```
//!
//! ### Dialog Management
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Update dialog identifiers as they become available
//! db.update_call_dialogs(
//!     "call-12345",
//!     Some("customer-dialog-new-456"),
//!     Some("agent-dialog-new-789")
//! ).await?;
//! 
//! println!("🔄 Call dialog IDs updated");
//! 
//! // Look up call information by dialog ID
//! if let Some((agent_dialog, call_id)) = db.get_dialog_mapping("customer-dialog-456").await? {
//!     println!("🔍 Dialog Mapping:");
//!     println!("  Call ID: {}", call_id);
//!     if let Some(agent_dialog) = agent_dialog {
//!         println!("  Agent Dialog: {}", agent_dialog);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Performance Monitoring
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Monitor system call volume
//! let active_count = db.get_active_calls_count().await?;
//! println!("📈 Current active calls: {}", active_count);
//! 
//! // Get detailed call statistics
//! let (total_calls, answered_calls, avg_answer_time) = db.get_call_stats().await?;
//! 
//! // Calculate performance metrics
//! let answer_rate = if total_calls > 0 {
//!     (answered_calls as f64 / total_calls as f64) * 100.0
//! } else {
//!     0.0
//! };
//! 
//! println!("📊 Performance Metrics:");
//! println!("  Answer rate: {:.1}%", answer_rate);
//! 
//! if let Some(avg_time) = avg_answer_time {
//!     println!("  Average answer time: {:.1}s", avg_time);
//!     
//!     // Alert on slow answer times
//!     if avg_time > 30.0 {
//!         println!("⚠️ Answer times above target (30s)");
//!     }
//! }
//! 
//! // Performance alerts
//! if answer_rate < 90.0 {
//!     println!("🚨 Low answer rate detected!");
//! }
//! 
//! if active_count > 100 {
//!     println!("📞 High call volume alert!");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Transaction Safety
//!
//! Many operations in this module use database transactions to ensure atomicity.
//! The `remove_active_call_with_agent_update` function is a prime example of
//! atomic operations that prevent data inconsistency:
//!
//! ```rust
//! # use rvoip_call_engine::database::DatabaseManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // This operation ensures that:
//! // 1. The call is removed from active_calls
//! // 2. The agent's call count is decremented
//! // 3. The agent's status is updated if they become available
//! // All within a single atomic transaction
//! db.remove_active_call_with_agent_update("call-12345").await?;
//! 
//! println!("✅ Atomic operation completed successfully");
//! # Ok(())
//! # }
//! ```
//!
//! ## Error Handling
//!
//! All database operations return `Result` types and should be handled appropriately:
//!
//! ```rust
//! # use rvoip_call_engine::database::DatabaseManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! match db.get_active_call("call-12345").await {
//!     Ok(Some(call)) => {
//!         println!("✅ Found active call: {}", call.call_id);
//!     }
//!     Ok(None) => {
//!         println!("ℹ️ Call not found - may have ended");
//!     }
//!     Err(e) => {
//!         eprintln!("❌ Database error: {}", e);
//!         // Handle error appropriately
//!     }
//! }
//! # Ok(())
//! # }
//! ```

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
            "SELECT call_id, agent_id, session_id, customer_dialog_id, agent_dialog_id, assigned_at, answered_at FROM active_calls WHERE call_id = ?1",
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
            "SELECT call_id, agent_id, session_id, customer_dialog_id, agent_dialog_id, assigned_at, answered_at FROM active_calls WHERE agent_id = ?1",
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