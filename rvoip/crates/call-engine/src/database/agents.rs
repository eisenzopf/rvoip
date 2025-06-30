//! # Agent Database Operations
//!
//! This module provides comprehensive database operations for managing call center
//! agents, including registration, status management, availability tracking, and
//! performance monitoring. It implements sophisticated agent selection algorithms
//! and maintains real-time agent state for optimal call routing.
//!
//! ## Overview
//!
//! Agent management is at the heart of call center operations. This module provides
//! robust database operations for agent lifecycle management, from initial registration
//! through ongoing status updates, call assignment, and performance tracking. It
//! includes advanced features like round-robin routing, last-agent exclusion for
//! fairness, and atomic reservation systems.
//!
//! ## Key Features
//!
//! - **Agent Registration**: Complete agent profile management and registration
//! - **Status Management**: Real-time agent status tracking and updates
//! - **Availability Tracking**: Sophisticated availability monitoring and selection
//! - **Round Robin Routing**: Fair distribution with last-agent exclusion
//! - **Atomic Reservations**: Transaction-safe agent reservation system
//! - **Performance Monitoring**: Agent statistics and performance metrics
//! - **Heartbeat Management**: Agent connectivity and health monitoring
//! - **Cleanup Operations**: Automated cleanup of stale agent connections
//!
//! ## Agent Status States
//!
//! The system tracks agents through several states:
//!
//! - **AVAILABLE**: Agent is online and ready to receive calls
//! - **BUSY**: Agent is currently handling one or more calls
//! - **POSTCALLWRAPUP**: Agent is completing post-call tasks
//! - **OFFLINE**: Agent is not available for calls
//! - **RESERVED**: Agent is temporarily reserved for assignment
//!
//! ## Database Schema
//!
//! ### agents Table
//! - `agent_id`: Unique identifier for the agent
//! - `username`: Agent's username for authentication
//! - `contact_uri`: SIP URI for agent contact
//! - `status`: Current agent status (see states above)
//! - `current_calls`: Number of calls currently assigned
//! - `max_calls`: Maximum concurrent calls the agent can handle
//! - `last_heartbeat`: Last heartbeat timestamp
//! - `available_since`: Timestamp when agent became available
//!
//! ## Examples
//!
//! ### Agent Registration and Management
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! use rvoip_call_engine::agent::AgentStatus;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Register a new agent
//! db.upsert_agent(
//!     "agent-001",
//!     "alice_smith", 
//!     Some("sip:alice@call-center.com")
//! ).await?;
//! 
//! println!("✅ Agent registered successfully");
//! 
//! // Update agent status
//! db.update_agent_status("agent-001", AgentStatus::Available).await?;
//! println!("🟢 Agent marked as available");
//! 
//! // Update heartbeat to show agent is online
//! db.update_agent_heartbeat("agent-001").await?;
//! println!("💓 Agent heartbeat updated");
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Availability and Selection
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Get available agents for call assignment
//! let available_agents = db.get_available_agents().await?;
//! 
//! println!("👥 Available Agents ({}):", available_agents.len());
//! for agent in &available_agents {
//!     println!("  {} ({}): {}/{} calls", 
//!              agent.agent_id, 
//!              agent.username,
//!              agent.current_calls, 
//!              agent.max_calls);
//!     
//!     if let Some(since) = &agent.available_since {
//!         println!("    Available since: {}", since);
//!     }
//! }
//! 
//! // Get available agents excluding the last assigned agent (for fairness)
//! let fair_agents = db.get_available_agents_excluding_last(Some("agent-001")).await?;
//! println!("🔄 Fair rotation agents: {}", fair_agents.len());
//! # Ok(())
//! # }
//! ```
//!
//! ### Atomic Agent Reservation
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Atomically reserve an agent for call assignment
//! if db.reserve_agent("agent-001").await? {
//!     println!("🔒 Agent successfully reserved");
//!     
//!     // Simulate call assignment process
//!     // ... assign call to agent ...
//!     
//!     // If assignment succeeds, agent stays reserved/busy
//!     // If assignment fails, release the reservation
//!     // db.release_agent_reservation("agent-001").await?;
//! } else {
//!     println!("❌ Could not reserve agent (not available or at capacity)");
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
//! // Update agent call count when call is assigned
//! db.update_agent_call_count("agent-001", 1).await?;
//! println!("📞 Agent call count incremented");
//! 
//! // When call ends, decrement call count
//! db.update_agent_call_count("agent-001", -1).await?;
//! println!("📞 Agent call count decremented");
//! 
//! // Get specific agent details
//! if let Some(agent) = db.get_agent("agent-001").await? {
//!     println!("📋 Agent Details:");
//!     println!("  Status: {:?}", agent.status);
//!     println!("  Current calls: {}", agent.current_calls);
//!     println!("  Max calls: {}", agent.max_calls);
//!     println!("  Contact: {:?}", agent.contact_uri);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Performance Monitoring
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Get comprehensive agent statistics
//! let stats = db.get_agent_stats().await?;
//! 
//! println!("📊 Agent Statistics:");
//! println!("┌─────────────────────────────────────┐");
//! println!("│ Total Agents: {:>18} │", stats.total_agents);
//! println!("│ Available: {:>21} │", stats.available_agents);
//! println!("│ Busy: {:>26} │", stats.busy_agents);
//! println!("│ Post-Call Wrap-Up: {:>12} │", stats.post_call_wrap_up_agents);
//! println!("│ Offline: {:>23} │", stats.offline_agents);
//! println!("│ Reserved: {:>22} │", stats.reserved_agents);
//! println!("└─────────────────────────────────────┘");
//! 
//! // Calculate utilization metrics
//! let online_agents = stats.available_agents + stats.busy_agents + stats.reserved_agents;
//! let utilization = if online_agents > 0 {
//!     (stats.busy_agents as f64 / online_agents as f64) * 100.0
//! } else {
//!     0.0
//! };
//! 
//! println!("📈 Utilization: {:.1}%", utilization);
//! 
//! // Alerts
//! if stats.available_agents == 0 && stats.busy_agents > 0 {
//!     println!("🚨 No agents available - all busy!");
//! }
//! 
//! if utilization > 90.0 {
//!     println!("⚠️ High utilization - consider adding agents");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### System Maintenance
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! use tokio::time::{interval, Duration};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Cleanup stale agents (in a real system, this would run periodically)
//! let cleaned_count = db.cleanup_stale_agents().await?;
//! if cleaned_count > 0 {
//!     println!("🧹 Cleaned up {} stale agents", cleaned_count);
//! }
//! 
//! // Count total agents in system
//! let total_count = db.count_total_agents().await?;
//! println!("👥 Total agents in system: {}", total_count);
//! 
//! // List all agents for administrative overview
//! let all_agents = db.list_agents().await?;
//! println!("📋 All Agents:");
//! for agent in all_agents {
//!     let status_icon = match agent.status {
//!         rvoip_call_engine::database::DbAgentStatus::Available => "🟢",
//!         rvoip_call_engine::database::DbAgentStatus::Busy => "🔴", 
//!         rvoip_call_engine::database::DbAgentStatus::PostCallWrapUp => "🟡",
//!         rvoip_call_engine::database::DbAgentStatus::Offline => "⚫",
//!         rvoip_call_engine::database::DbAgentStatus::Reserved => "🔒",
//!     };
//!     
//!     println!("  {} {} ({}): {}/{} calls", 
//!              status_icon, agent.agent_id, agent.username,
//!              agent.current_calls, agent.max_calls);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Round Robin Fairness Algorithm
//!
//! The agent selection system implements sophisticated fairness algorithms:
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Simulating multiple call assignments with fairness
//! let mut last_assigned: Option<String> = None;
//! 
//! for call_num in 1..=5 {
//!     println!("📞 Assigning call #{}", call_num);
//!     
//!     // Get agents with last-agent exclusion for fairness
//!     let agents = db.get_available_agents_excluding_last(
//!         last_assigned.as_deref()
//!     ).await?;
//!     
//!     if let Some(selected_agent) = agents.first() {
//!         println!("  ✅ Selected: {} (fair rotation)", selected_agent.agent_id);
//!         last_assigned = Some(selected_agent.agent_id.clone());
//!         
//!         // In real system: reserve agent and assign call
//!         // db.reserve_agent(&selected_agent.agent_id).await?;
//!     } else {
//!         println!("  ❌ No agents available");
//!         break;
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance Considerations
//!
//! ### Optimized Queries
//! 
//! The module uses optimized queries for high-performance operations:
//!
//! ```rust
//! # use rvoip_call_engine::database::DatabaseManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Efficient availability check with capacity filtering
//! let agents = db.get_available_agents().await?;
//! 
//! // This query efficiently finds agents where:
//! // - status = 'AVAILABLE'
//! // - current_calls < max_calls  
//! // - Ordered by available_since for fairness
//! 
//! println!("Found {} available agents efficiently", agents.len());
//! # Ok(())
//! # }
//! ```
//!
//! ### Transaction Safety
//!
//! Agent reservation uses atomic operations to prevent race conditions:
//!
//! ```rust
//! # use rvoip_call_engine::database::DatabaseManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Atomic reservation prevents double-assignment
//! if db.reserve_agent("agent-001").await? {
//!     println!("🔒 Agent atomically reserved");
//!     
//!     // The reservation changed status from AVAILABLE to RESERVED
//!     // Only one concurrent request can succeed
//!     
//! } else {
//!     println!("❌ Agent reservation failed - not available");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Error Handling Best Practices
//!
//! ```rust
//! # use rvoip_call_engine::database::DatabaseManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Robust error handling for agent operations
//! match db.get_agent("agent-001").await {
//!     Ok(Some(agent)) => {
//!         println!("✅ Found agent: {}", agent.agent_id);
//!     }
//!     Ok(None) => {
//!         println!("ℹ️ Agent not found - may need to register");
//!     }
//!     Err(e) => {
//!         eprintln!("❌ Database error: {}", e);
//!         // Implement appropriate error recovery
//!     }
//! }
//! 
//! // Graceful handling of status updates
//! if let Err(e) = db.update_agent_heartbeat("agent-001").await {
//!     eprintln!("⚠️ Failed to update heartbeat: {}", e);
//!     // Agent may be disconnected - handle accordingly
//! }
//! # Ok(())
//! # }
//! ```

use anyhow::{Result, anyhow};
use tracing::{info, warn, debug};
use super::{DatabaseManager, DbAgent, DbAgentStatus, Transaction};
use chrono::{DateTime, Utc};
use super::value_helpers::*;
use crate::agent::{AgentId, AgentStatus};

impl DatabaseManager {
    /// Lightweight debug function (Limbo-optimized) 
    pub async fn debug_dump_database(&self) -> Result<()> {
        // LIMBO OPTIMIZATION: Skip heavy debug operations to prevent database overload
        debug!("🔍 Database debug: Lightweight check only (Limbo mode)");
        Ok(())
    }

    /// Register or update an agent (Limbo-optimized for stability)
    pub async fn upsert_agent(&self, agent_id: &str, username: &str, contact_uri: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        info!("🔍 upsert_agent: {} -> {}", agent_id, username);
        
        // LIMBO OPTIMIZATION: Simple operations only, no verification queries
        let existing = self.query(
            "SELECT agent_id FROM agents WHERE agent_id = ?1",
            vec![agent_id.into()] as Vec<limbo::Value>
        ).await?;
        
        if existing.is_empty() {
            // Insert new agent
            self.execute(
                "INSERT INTO agents (agent_id, username, contact_uri, last_heartbeat, status, current_calls, max_calls, available_since)
                 VALUES (?1, ?2, ?3, ?4, 'AVAILABLE', 0, 1, ?5)",
                vec![
                    agent_id.into(),
                    username.into(), 
                    contact_uri.map(|s| s.into()).unwrap_or(limbo::Value::Null),
                    now.clone().into(),
                    now.into(),
                ] as Vec<limbo::Value>
            ).await?;
            info!("✅ Agent {} created", agent_id);
        } else {
            // Update existing agent
            self.execute(
                "UPDATE agents 
                 SET username = ?1, contact_uri = ?2, last_heartbeat = ?3, status = 'AVAILABLE', available_since = ?4
                 WHERE agent_id = ?5",
                vec![
                    username.into(),
                    contact_uri.map(|s| s.into()).unwrap_or(limbo::Value::Null),
                    now.clone().into(),
                    now.into(),
                    agent_id.into(),
                ] as Vec<limbo::Value>
            ).await?;
            info!("✅ Agent {} updated", agent_id);
        }
        
        Ok(())
    }
    
    /// Update agent status (Limbo-optimized)
    pub async fn update_agent_status(&self, agent_id: &str, status: AgentStatus) -> Result<()> {
        let status_str = match status {
            AgentStatus::Available => "AVAILABLE",
            AgentStatus::Busy(_) => "BUSY",
            AgentStatus::PostCallWrapUp => "POSTCALLWRAPUP",
            AgentStatus::Offline => "OFFLINE",
        };
        
        // LIMBO OPTIMIZATION: Simple update without verification
        if matches!(status, AgentStatus::Available) {
            let now = chrono::Utc::now().to_rfc3339();
            self.execute(
                "UPDATE agents SET status = ?1, available_since = ?2 WHERE agent_id = ?3",
                vec![status_str.into(), now.into(), agent_id.into()] as Vec<limbo::Value>
            ).await?;
        } else {
            self.execute(
                "UPDATE agents SET status = ?1, available_since = NULL WHERE agent_id = ?2",
                vec![status_str.into(), agent_id.into()] as Vec<limbo::Value>
            ).await?;
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
    
    /// Get available agents for assignment (with round robin + last agent exclusion for fairness)
    pub async fn get_available_agents(&self) -> Result<Vec<DbAgent>> {
        debug!("🔍 Getting available agents with round robin fairness...");
        
        let rows = self.query(
            "SELECT agent_id, username, contact_uri, status, current_calls, max_calls, available_since
             FROM agents 
             WHERE status = 'AVAILABLE' 
             AND current_calls < max_calls
             ORDER BY available_since ASC",  // Get all available agents
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
                
                info!("🔍 Found available agent: {} (since: {:?})", agent_id, available_since_str);
            }
        }

        // ROUND ROBIN WITH LAST AGENT EXCLUSION
        // Sort agents to implement fair round robin
        if agents.len() > 1 {
            info!("🔄 ROUND ROBIN: Implementing fair distribution among {} agents", agents.len());
            
            // Sort by available_since timestamp (oldest first)
            agents.sort_by(|a, b| {
                match (&a.available_since, &b.available_since) {
                    (Some(a_time), Some(b_time)) => a_time.cmp(b_time),
                    (Some(_), None) => std::cmp::Ordering::Less,    // Agents with timestamps come first
                    (None, Some(_)) => std::cmp::Ordering::Greater, // Agents without timestamps come last
                    (None, None) => std::cmp::Ordering::Equal,      // Equal if both have no timestamp
                }
            });
            
            info!("🔄 AGENTS SORTED BY AVAILABILITY TIME:");
            for (idx, agent) in agents.iter().enumerate() {
                info!("🔄   {}. {} (available since: {:?})", 
                      idx + 1, agent.agent_id, agent.available_since);
            }
        } else {
            debug!("🔄 ROUND ROBIN: Only {} agent(s) available, no rotation needed", agents.len());
        }

        Ok(agents)
    }
    
    /// Get available agents with last agent exclusion (NEW FUNCTION)
    pub async fn get_available_agents_excluding_last(&self, last_agent_id: Option<&str>) -> Result<Vec<DbAgent>> {
        info!("🚫 Getting available agents EXCLUDING last agent: {:?}", last_agent_id);
        
        let mut all_agents = self.get_available_agents().await?;
        
        if let Some(exclude_id) = last_agent_id {
            if all_agents.len() > 1 {
                // Remove the last agent from the front of the list and put them at the end
                if let Some(pos) = all_agents.iter().position(|agent| agent.agent_id == exclude_id) {
                    let excluded_agent = all_agents.remove(pos);
                    all_agents.push(excluded_agent); // Put at end of list
                    
                    info!("🚫 EXCLUSION: Moved agent '{}' to end of list for fairness", exclude_id);
                    info!("🚫 NEW ORDER:");
                    for (idx, agent) in all_agents.iter().enumerate() {
                        info!("🚫   {}. {} (available since: {:?})", 
                              idx + 1, agent.agent_id, agent.available_since);
                    }
                } else {
                    info!("🚫 EXCLUSION: Agent '{}' not found in available list", exclude_id);
                }
            } else {
                info!("🚫 EXCLUSION: Only 1 agent available, cannot exclude");
            }
        } else {
            info!("🚫 EXCLUSION: No last agent to exclude, using normal order");
        }
        
        Ok(all_agents)
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