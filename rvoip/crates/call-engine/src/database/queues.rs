//! # Queue Database Operations
//!
//! This module provides comprehensive database operations for managing call queues,
//! including call enqueueing, dequeuing, priority handling, and atomic agent assignment.
//! It implements sophisticated queue management algorithms with fairness, expiration
//! handling, and performance optimization for high-volume call processing.
//!
//! ## Overview
//!
//! Queue management is critical for effective call center operations. This module
//! provides robust database operations for managing calls waiting for agent assignment,
//! including priority-based ordering, automatic expiration, and atomic operations
//! that ensure consistent state between queues and agent assignments.
//!
//! ## Key Features
//!
//! - **Priority Queue Management**: Multi-level priority call queuing
//! - **Atomic Dequeuing**: Transaction-safe call assignment to agents
//! - **Expiration Handling**: Automatic cleanup of expired calls
//! - **Agent Reservation**: Atomic agent reservation with rollback capability
//! - **Fair Distribution**: First-in-first-out within priority levels
//! - **Performance Monitoring**: Queue depth and timing analytics
//! - **Bulk Operations**: Efficient assignment of multiple calls
//! - **Error Recovery**: Graceful handling of assignment failures
//!
//! ## Queue Processing Flow
//!
//! 1. **Enqueue**: Calls enter queues with priority and customer information
//! 2. **Priority Ordering**: Calls are ordered by priority, then by arrival time
//! 3. **Agent Matching**: Available agents are matched with waiting calls
//! 4. **Atomic Assignment**: Agent reservation and call assignment in single transaction
//! 5. **Cleanup**: Expired calls are automatically removed
//!
//! ## Database Schema
//!
//! ### call_queue Table
//! - `call_id`: Unique identifier for the queued call
//! - `session_id`: Session management identifier
//! - `queue_id`: Target queue identifier
//! - `customer_info`: JSON containing caller details
//! - `priority`: Call priority (lower numbers = higher priority)
//! - `enqueued_at`: Timestamp when call entered queue
//! - `expires_at`: Automatic expiration timestamp
//! - `attempts`: Number of assignment attempts
//! - `last_attempt`: Timestamp of last assignment attempt
//!
//! ## Examples
//!
//! ### Basic Queue Operations
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! use rvoip_call_engine::queue::QueuedCall;
//! use rvoip_session_core::SessionId;
//! use chrono::Utc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Create a queued call
//! let queued_call = QueuedCall {
//!     session_id: SessionId("session-12345".to_string()),
//!     caller_id: "+1-555-0123".to_string(),
//!     priority: 5, // Normal priority
//!     queued_at: Utc::now(),
//!     estimated_wait_time: None,
//!     retry_count: 0,
//! };
//! 
//! // Enqueue the call
//! db.enqueue_call("general", &queued_call).await?;
//! println!("📋 Call enqueued in 'general' queue");
//! 
//! // Check queue depth
//! let depth = db.get_queue_depth("general").await?;
//! println!("📊 Queue depth: {}", depth);
//! # Ok(())
//! # }
//! ```
//!
//! ### Priority Queue Management
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! use rvoip_call_engine::queue::QueuedCall;
//! use rvoip_session_core::SessionId;
//! use chrono::Utc;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Enqueue calls with different priorities
//! let calls = vec![
//!     ("session-normal", "+1-555-0001", 5),   // Normal priority
//!     ("session-vip", "+1-555-0002", 1),      // High priority (VIP)
//!     ("session-urgent", "+1-555-0003", 0),   // Urgent priority
//!     ("session-low", "+1-555-0004", 10),     // Low priority
//! ];
//! 
//! for (session_id, caller_id, priority) in calls {
//!     let call = QueuedCall {
//!         session_id: SessionId(session_id.to_string()),
//!         caller_id: caller_id.to_string(),
//!         priority,
//!         queued_at: Utc::now(),
//!         estimated_wait_time: None,
//!         retry_count: 0,
//!     };
//!     
//!     db.enqueue_call("support", &call).await?;
//!     println!("📋 Enqueued {} with priority {}", caller_id, priority);
//! }
//! 
//! println!("✅ All calls enqueued - will be processed by priority order");
//! # Ok(())
//! # }
//! ```
//!
//! ### Atomic Agent Assignment
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Atomically dequeue call for specific agent
//! match db.dequeue_call_for_agent("support", "agent-001").await? {
//!     Some(assigned_call) => {
//!         println!("✅ Call assigned to agent-001:");
//!         println!("  Session: {}", assigned_call.session_id.0);
//!         println!("  Caller: {}", assigned_call.caller_id);
//!         println!("  Priority: {}", assigned_call.priority);
//!         println!("  Queued at: {}", assigned_call.queued_at);
//!         
//!         // Call is now removed from queue and agent is marked busy
//!         let remaining_depth = db.get_queue_depth("support").await?;
//!         println!("📊 Remaining calls in queue: {}", remaining_depth);
//!     }
//!     None => {
//!         println!("ℹ️ No calls available for assignment or agent not available");
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Bulk Assignment Operations
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Get available assignments for bulk processing
//! let assignments = db.get_available_assignments("support").await?;
//! 
//! println!("📋 Available Assignments:");
//! for (agent_id, call_id, session_id) in assignments {
//!     println!("  Agent {}: Call {} (Session {})", 
//!              agent_id, call_id, session_id);
//!     
//!     // In a real system, you would process these assignments
//!     // For example, create SIP dialogs, establish media sessions, etc.
//! }
//! 
//! // This is useful for batch processing during high-volume periods
//! println!("💡 Bulk assignment can improve performance during peak times");
//! # Ok(())
//! # }
//! ```
//!
//! ### Queue Maintenance and Cleanup
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Remove expired calls from queue
//! let expired_count = db.remove_expired_calls("support").await?;
//! if expired_count > 0 {
//!     println!("🧹 Removed {} expired calls from queue", expired_count);
//! }
//! 
//! // Check current queue status
//! let current_depth = db.get_queue_depth("support").await?;
//! println!("📊 Current queue depth after cleanup: {}", current_depth);
//! 
//! // This maintenance should be run periodically to prevent
//! // accumulation of expired calls
//! println!("💡 Regular cleanup prevents queue bloat");
//! # Ok(())
//! # }
//! ```
//!
//! ### Queue Performance Monitoring
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Monitor multiple queues
//! let queue_names = vec!["general", "support", "sales", "vip"];
//! 
//! println!("📊 Queue Status Report:");
//! println!("┌─────────────┬───────────┐");
//! println!("│ Queue Name  │ Depth     │");
//! println!("├─────────────┼───────────┤");
//! 
//! let mut total_queued = 0;
//! for queue_name in queue_names {
//!     let depth = db.get_queue_depth(queue_name).await?;
//!     total_queued += depth;
//!     println!("│ {:11} │ {:>9} │", queue_name, depth);
//! }
//! 
//! println!("└─────────────┴───────────┘");
//! println!("Total calls in all queues: {}", total_queued);
//! 
//! // Alert on high queue volumes
//! if total_queued > 50 {
//!     println!("🚨 High queue volume - consider adding agents!");
//! } else if total_queued > 20 {
//!     println!("⚠️ Elevated queue volume - monitor closely");
//! } else {
//!     println!("✅ Queue volumes normal");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Queue Processing
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! use tokio::time::{interval, Duration};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Simulate a queue processing loop
//! async fn process_queue_assignments(
//!     db: &DatabaseManager,
//!     queue_id: &str
//! ) -> Result<usize, Box<dyn std::error::Error>> {
//!     let mut assignments_made = 0;
//!     
//!     // Get available assignments
//!     let assignments = db.get_available_assignments(queue_id).await?;
//!     
//!     for (agent_id, call_id, session_id) in assignments {
//!         // Try to assign this specific call to this specific agent
//!         if let Some(assigned_call) = db.dequeue_call_for_agent(queue_id, &agent_id).await? {
//!             println!("✅ Assigned call {} to agent {}", 
//!                      assigned_call.session_id.0, agent_id);
//!             assignments_made += 1;
//!         }
//!     }
//!     
//!     Ok(assignments_made)
//! }
//! 
//! // Process assignments for a specific queue
//! let assigned = process_queue_assignments(&db, "support").await?;
//! println!("📋 Made {} assignments in support queue", assigned);
//! 
//! // In a real system, this would run continuously:
//! // let mut interval = interval(Duration::from_secs(1));
//! // loop {
//! //     interval.tick().await;
//! //     process_queue_assignments(&db, "support").await?;
//! // }
//! # Ok(())
//! # }
//! ```
//!
//! ## Transaction Safety and Error Handling
//!
//! The queue operations use sophisticated transaction management:
//!
//! ```rust
//! # use rvoip_call_engine::database::DatabaseManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // The dequeue_call_for_agent operation is atomic and includes:
//! // 1. Agent availability verification
//! // 2. Agent status update to BUSY
//! // 3. Call removal from queue
//! // 4. Active call record creation
//! // 5. Automatic rollback on any failure
//! 
//! match db.dequeue_call_for_agent("support", "agent-001").await {
//!     Ok(Some(call)) => {
//!         println!("✅ Atomic assignment successful");
//!         // Agent is now BUSY, call is in active_calls table
//!     }
//!     Ok(None) => {
//!         println!("ℹ️ No assignment possible");
//!         // Agent remains AVAILABLE, no state changes
//!     }
//!     Err(e) => {
//!         eprintln!("❌ Assignment failed: {}", e);
//!         // All changes automatically rolled back
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance Optimization
//!
//! ### Efficient Query Patterns
//!
//! ```rust
//! # use rvoip_call_engine::database::DatabaseManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Queue depth queries are optimized for speed
//! let depth = db.get_queue_depth("support").await?;
//! 
//! // This query efficiently:
//! // - Filters by queue_id
//! // - Excludes expired calls
//! // - Returns count without loading call data
//! 
//! println!("📊 Queue depth retrieved efficiently: {}", depth);
//! # Ok(())
//! # }
//! ```
//!
//! ### Batch Processing
//!
//! ```rust
//! # use rvoip_call_engine::database::DatabaseManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Batch cleanup of expired calls across all queues
//! let queue_ids = vec!["general", "support", "sales", "vip"];
//! let mut total_cleaned = 0;
//! 
//! for queue_id in queue_ids {
//!     let cleaned = db.remove_expired_calls(queue_id).await?;
//!     total_cleaned += cleaned;
//! }
//! 
//! if total_cleaned > 0 {
//!     println!("🧹 Batch cleanup removed {} expired calls", total_cleaned);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Error Recovery Patterns
//!
//! ```rust
//! # use rvoip_call_engine::database::DatabaseManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Graceful handling of assignment failures
//! async fn safe_assignment(
//!     db: &DatabaseManager,
//!     queue_id: &str,
//!     agent_id: &str
//! ) -> Result<bool, Box<dyn std::error::Error>> {
//!     match db.dequeue_call_for_agent(queue_id, agent_id).await {
//!         Ok(Some(_call)) => {
//!             println!("✅ Assignment successful");
//!             Ok(true)
//!         }
//!         Ok(None) => {
//!             println!("ℹ️ No assignment made - normal condition");
//!             Ok(false)
//!         }
//!         Err(e) => {
//!             eprintln!("⚠️ Assignment error (recovered): {}", e);
//!             // Log error but don't propagate - system continues
//!             Ok(false)
//!         }
//!     }
//! }
//! 
//! // Attempt assignment with graceful error handling
//! let success = safe_assignment(&db, "support", "agent-001").await?;
//! println!("Assignment result: {}", if success { "Success" } else { "No assignment" });
//! # Ok(())
//! # }
//! ```

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
                // First, check agent status for debugging
                let agent_status_rows = tx.query(
                    "SELECT agent_id, status, current_calls, max_calls FROM agents WHERE agent_id = ?1",
                    vec![agent_id.clone().into()] as Vec<limbo::Value>
                ).await?;
                
                if let Some(status_row) = agent_status_rows.into_iter().next() {
                    let status = value_to_string(&status_row.get_value(1)?)?;
                    let current_calls = value_to_i32(&status_row.get_value(2)?)?;
                    let max_calls = value_to_i32(&status_row.get_value(3)?)?;
                    tracing::info!("📋 DB ASSIGNMENT: Agent {} has status='{}', current_calls={}, max_calls={}", 
                                  agent_id, status, current_calls, max_calls);
                }
                
                // LIMBO FIX: Use atomic UPDATE with WHERE conditions (no separate SELECT)
                // This avoids transaction isolation issues by doing everything in one operation
                tracing::info!("📋 DB ASSIGNMENT: Attempting atomic reservation for agent {} with WHERE conditions", agent_id);
                
                // Execute the atomic UPDATE (Limbo always reports 0 rows, so we ignore the return value)
                match tx.execute(
                    "UPDATE agents SET status = 'BUSY', current_calls = current_calls + 1 WHERE agent_id = ?1 AND (status = 'AVAILABLE' OR status = 'RESERVED') AND current_calls < max_calls",
                    vec![agent_id.clone().into()] as Vec<limbo::Value>
                ).await {
                    Ok(count) => {
                        tracing::info!("📋 DB ASSIGNMENT: Atomic UPDATE executed for agent {} (Limbo reported {} rows, will verify with SELECT)", agent_id, count);
                    }
                    Err(e) => {
                        tracing::error!("📋 DB ASSIGNMENT: Atomic UPDATE failed for agent {}: {:?}", agent_id, e);
                        return Ok(None);
                    }
                };
                
                // LIMBO QUIRK FIX: Verify the UPDATE worked by checking the agent's current status
                // Since Limbo reports 0 rows even on successful UPDATEs, we must confirm with SELECT
                let verify_rows = tx.query(
                    "SELECT status, current_calls FROM agents WHERE agent_id = ?1",
                    vec![agent_id.clone().into()] as Vec<limbo::Value>
                ).await?;
                
                if let Some(verify_row) = verify_rows.into_iter().next() {
                    let current_status = value_to_string(&verify_row.get_value(0)?)?;
                    let current_calls = value_to_i32(&verify_row.get_value(1)?)?;
                    
                    if current_status == "BUSY" {
                        tracing::info!("✅ DB ASSIGNMENT: CONFIRMED - Agent {} successfully reserved (status='{}', current_calls={})", 
                                     agent_id, current_status, current_calls);
                    } else {
                        tracing::warn!("📋 DB ASSIGNMENT: Agent {} still has status='{}' after UPDATE - reservation failed (conditions not met)", 
                                     agent_id, current_status);
                        return Ok(None);
                    }
                } else {
                    tracing::error!("📋 DB ASSIGNMENT: Cannot find agent {} to verify UPDATE result", agent_id);
                    return Ok(None);
                }
                
                // Agent successfully reserved - proceed to find calls in queue
                
                // Find highest priority call in the queue
                let rows = tx.query(
                    "SELECT call_id, session_id, queue_id, customer_info, priority, enqueued_at, attempts, last_attempt, expires_at FROM call_queue 
                     WHERE queue_id = ?1 
                     AND expires_at > datetime('now')
                     ORDER BY priority ASC, enqueued_at ASC 
                     LIMIT 1",
                    vec![queue_id.clone().into()] as Vec<limbo::Value>
                ).await?;
                
                tracing::info!("📋 DB ASSIGNMENT: Found {} calls in queue '{}' for agent {}", rows.len(), queue_id, agent_id);
                
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
                        tracing::warn!("📋 DB ASSIGNMENT: Failed to parse call, unreserving agent {}", agent_id);
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
                    tracing::warn!("📋 DB ASSIGNMENT: No calls found in queue '{}' for agent {}, unreserving agent", queue_id, agent_id);
                    let unreserved = tx.execute(
                        "UPDATE agents 
                         SET status = 'AVAILABLE', current_calls = current_calls - 1
                         WHERE agent_id = ?1",
                        vec![agent_id.clone().into()] as Vec<limbo::Value>
                    ).await?;
                    tracing::info!("📋 DB ASSIGNMENT: Unreserved agent {} (updated {} rows)", agent_id, unreserved);
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
        // Limbo doesn't support complex queries, so we'll do this in steps
        
        // Step 1: Get available agents
        let agent_rows = self.query(
            "SELECT agent_id FROM agents 
             WHERE status = 'AVAILABLE' 
             AND current_calls < max_calls
             LIMIT 10",
            ()
        ).await?;
        
        if agent_rows.is_empty() {
            return Ok(Vec::new());
        }
        
        // Step 2: Get queued calls
        let call_rows = self.query(
            "SELECT call_id, session_id 
             FROM call_queue 
             WHERE queue_id = ?1 AND expires_at > datetime('now')
             ORDER BY priority ASC, enqueued_at ASC
             LIMIT 10",
            vec![queue_id.into()] as Vec<limbo::Value>
        ).await?;
        
        if call_rows.is_empty() {
            return Ok(Vec::new());
        }
        
        // Step 3: Check which calls are not already active
        let mut assignments = Vec::new();
        
        for (agent_idx, agent_row) in agent_rows.iter().enumerate() {
            if agent_idx >= call_rows.len() {
                break; // No more calls to assign
            }
            
            let agent_id = value_to_string(&agent_row.get_value(0)?)?;
            let call_row = &call_rows[agent_idx];
            let call_id = value_to_string(&call_row.get_value(0)?)?;
            let session_id = value_to_string(&call_row.get_value(1)?)?;
            
            // Check if this call is already active
            let active_check = self.query(
                "SELECT 1 FROM active_calls WHERE call_id = ?1",
                vec![call_id.clone().into()] as Vec<limbo::Value>
            ).await?;
            
            if active_check.is_empty() {
                // Call is not active, can be assigned
                assignments.push((agent_id, call_id, session_id));
            }
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