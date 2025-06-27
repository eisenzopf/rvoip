//! Agent-related database operations

use anyhow::{Result, anyhow};
use tracing::{info, warn, debug};
use super::{DatabaseManager, DbAgent, DbAgentStatus, Transaction};
use chrono::{DateTime, Utc};
use super::value_helpers::*;
use crate::agent::{AgentId, AgentStatus};

impl DatabaseManager {
    /// Debug function to dump all database contents and verify Limbo compatibility
    pub async fn debug_dump_database(&self) -> Result<()> {
        info!("🔍 === DATABASE DEBUG DUMP ===");
        
        // Check if the agents table exists
        match self.query("SELECT name FROM sqlite_master WHERE type='table' AND name='agents'", ()).await {
            Ok(rows) => {
                if rows.is_empty() {
                    info!("🔍 ❌ agents table does not exist!");
                    return Ok(());
                } else {
                    info!("🔍 ✅ agents table exists");
                }
            }
            Err(e) => {
                info!("🔍 ❌ Error checking table existence: {}", e);
                return Ok(());
            }
        }
        
        // Get table schema
        match self.query("PRAGMA table_info(agents)", ()).await {
            Ok(rows) => {
                info!("🔍 agents table schema:");
                for row in rows {
                    if let (Ok(cid), Ok(name), Ok(type_), Ok(notnull), Ok(dflt_value), Ok(pk)) = (
                        row.get_value(0), row.get_value(1), row.get_value(2), 
                        row.get_value(3), row.get_value(4), row.get_value(5)
                    ) {
                        info!("🔍   Column: {:?} ({:?}), NOT NULL: {:?}, DEFAULT: {:?}, PK: {:?}", 
                              name, type_, notnull, dflt_value, pk);
                    }
                }
            }
            Err(e) => {
                info!("🔍 ❌ Error getting table schema: {}", e);
            }
        }
        
        // Count total rows
        match self.query("SELECT COUNT(*) FROM agents", ()).await {
            Ok(rows) => {
                if let Some(row) = rows.first() {
                    if let Ok(count) = row.get_value(0) {
                        info!("🔍 Total agents in database: {:?}", count);
                    }
                }
            }
            Err(e) => {
                info!("🔍 ❌ Error counting agents: {}", e);
            }
        }
        
        // Dump all agent records with full details
        match self.query("SELECT * FROM agents", ()).await {
            Ok(rows) => {
                info!("🔍 All agent records ({} rows):", rows.len());
                for (i, row) in rows.iter().enumerate() {
                    // Try to extract readable data from each column
                    let mut row_data = Vec::new();
                    for col_idx in 0..8 { // We expect 8 columns
                        match row.get_value(col_idx) {
                            Ok(value) => row_data.push(format!("{:?}", value)),
                            Err(_) => row_data.push("ERROR".to_string()),
                        }
                    }
                    info!("🔍   Row {}: [{}]", i + 1, row_data.join(", "));
                }
            }
            Err(e) => {
                info!("🔍 ❌ Error dumping agents: {}", e);
            }
        }
        
        info!("🔍 === END DATABASE DEBUG DUMP ===");
        Ok(())
    }

    /// Register or update an agent (simplified for Limbo compatibility)
    pub async fn upsert_agent(&self, agent_id: &str, username: &str, contact_uri: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        
        info!("🔍 upsert_agent called with agent_id='{}', username='{}', contact_uri='{:?}'", 
               agent_id, username, contact_uri);
        
        // DEBUG: Dump database contents BEFORE operation
        self.debug_dump_database().await?;
        
        // Since Limbo has "No indexing", we can't rely on UNIQUE constraints
        // Let's do a manual check-and-insert approach
        
        // First, check if agent already exists
        info!("🔍 Checking if agent {} already exists...", agent_id);
        let existing = self.query(
            "SELECT agent_id FROM agents WHERE agent_id = ?1",
            vec![agent_id.into()] as Vec<limbo::Value>
        ).await?;
        
        if existing.is_empty() {
            // Agent doesn't exist, insert new one
            info!("🔍 Agent {} not found, inserting new record", agent_id);
            let insert_result = self.execute(
                "INSERT INTO agents (agent_id, username, contact_uri, last_heartbeat, status, current_calls, max_calls, available_since)
                 VALUES (?1, ?2, ?3, ?4, 'AVAILABLE', 0, 1, ?5)",
                vec![
                    agent_id.into(),
                    username.into(), 
                    contact_uri.map(|s| s.into()).unwrap_or(limbo::Value::Null),
                    now.clone().into(),
                    now.clone().into(), // Set available_since timestamp  
                ] as Vec<limbo::Value>
            ).await;
            
            match insert_result {
                Ok(rows_affected) => {
                    info!("🔍 ✅ INSERT successful: {} rows affected", rows_affected);
                    
                    // Verify the insert by selecting the specific record we just created
                    info!("🔍 Verifying INSERT with targeted SELECT...");
                    let verification = self.query(
                        "SELECT agent_id, username, contact_uri, status, available_since FROM agents WHERE agent_id = ?1",
                        vec![agent_id.into()] as Vec<limbo::Value>
                    ).await;
                    
                    match verification {
                        Ok(rows) => {
                            if rows.is_empty() {
                                info!("🔍 ❌ VERIFICATION FAILED: Record not found after INSERT!");
                            } else if rows.len() > 1 {
                                info!("🔍 ⚠️ VERIFICATION WARNING: Multiple records found for agent_id {}", agent_id);
                            } else {
                                let row = &rows[0];
                                if let (Ok(db_agent_id), Ok(db_username), contact_uri_val, Ok(db_status), available_since_val) = (
                                    row.get_value(0), row.get_value(1), row.get_value(2), row.get_value(3), row.get_value(4)
                                ) {
                                    let db_agent_id = match db_agent_id {
                                        limbo::Value::Text(s) => s,
                                        _ => "INVALID".to_string(),
                                    };
                                    let db_username = match db_username {
                                        limbo::Value::Text(s) => s,
                                        _ => "INVALID".to_string(),
                                    };
                                    let db_contact_uri = match contact_uri_val {
                                        Ok(limbo::Value::Text(s)) => Some(s),
                                        _ => None,
                                    };
                                    let db_status = match db_status {
                                        limbo::Value::Text(s) => s,
                                        _ => "INVALID".to_string(),
                                    };
                                    let db_available_since = match available_since_val {
                                        Ok(limbo::Value::Text(s)) => Some(s),
                                        _ => None,
                                    };
                                    
                                    info!("🔍 ✅ VERIFICATION SUCCESS: Record found and verified:");
                                    info!("🔍   - agent_id: '{}' (expected: '{}')", db_agent_id, agent_id);
                                    info!("🔍   - username: '{}' (expected: '{}')", db_username, username);
                                    info!("🔍   - contact_uri: '{:?}' (expected: '{:?}')", db_contact_uri, contact_uri);
                                    info!("🔍   - status: '{}' (expected: 'AVAILABLE')", db_status);
                                    info!("🔍   - available_since: '{:?}' (expected: recent timestamp)", db_available_since);
                                    
                                    // Check for mismatches
                                    if db_agent_id != agent_id {
                                        info!("🔍 ❌ MISMATCH: agent_id doesn't match!");
                                    }
                                    if db_username != username {
                                        info!("🔍 ❌ MISMATCH: username doesn't match!");
                                    }
                                    if db_status != "AVAILABLE" {
                                        info!("🔍 ❌ MISMATCH: status is not AVAILABLE!");
                                    }
                                } else {
                                    info!("🔍 ❌ VERIFICATION FAILED: Could not parse record fields");
                                }
                            }
                        }
                        Err(e) => {
                            info!("🔍 ❌ VERIFICATION FAILED: SELECT error: {}", e);
                        }
                    }
                }
                Err(e) => {
                    info!("🔍 ❌ INSERT failed: {}", e);
                    return Err(e);
                }
            }
        } else {
            // Agent exists, update it
            info!("🔍 Agent {} found, updating existing record", agent_id);
            let update_result = self.execute(
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
            ).await;
            
            match update_result {
                Ok(rows_affected) => {
                    info!("🔍 ✅ UPDATE successful: {} rows affected", rows_affected);
                    
                    // Verify the update by selecting the specific record we just updated
                    info!("🔍 Verifying UPDATE with targeted SELECT...");
                    let verification = self.query(
                        "SELECT agent_id, username, contact_uri, status, available_since FROM agents WHERE agent_id = ?1",
                        vec![agent_id.into()] as Vec<limbo::Value>
                    ).await;
                    
                    match verification {
                        Ok(rows) => {
                            if rows.is_empty() {
                                info!("🔍 ❌ VERIFICATION FAILED: Record not found after UPDATE!");
                            } else {
                                let row = &rows[0];
                                if let (Ok(db_agent_id), Ok(db_username), contact_uri_val, Ok(db_status), available_since_val) = (
                                    row.get_value(0), row.get_value(1), row.get_value(2), row.get_value(3), row.get_value(4)
                                ) {
                                    info!("🔍 ✅ VERIFICATION SUCCESS: UPDATE verified with current values");
                                } else {
                                    info!("🔍 ❌ VERIFICATION FAILED: Could not parse updated record fields");
                                }
                            }
                        }
                        Err(e) => {
                            info!("🔍 ❌ VERIFICATION FAILED: SELECT error after UPDATE: {}", e);
                        }
                    }
                }
                Err(e) => {
                    info!("🔍 ❌ UPDATE failed: {}", e);
                    return Err(e);
                }
            }
        }
        
        // DEBUG: Dump database contents AFTER operation
        self.debug_dump_database().await?;
        
        info!("Agent {} processed in database with contact {:?}", agent_id, contact_uri);
        Ok(())
    }
    
    /// Update agent status (with availability timestamp for fair round robin)
    pub async fn update_agent_status(&self, agent_id: &str, status: AgentStatus) -> Result<()> {
        info!("🔧 update_agent_status called: agent_id='{}', status='{:?}'", agent_id, status);
        
        let status_str = match status {
            AgentStatus::Available => "AVAILABLE",
            AgentStatus::Busy(_) => "BUSY",
            AgentStatus::PostCallWrapUp => "POSTCALLWRAPUP",
            AgentStatus::Offline => "OFFLINE",
        };
        
        // If transitioning to AVAILABLE, update the available_since timestamp for fairness
        if matches!(status, AgentStatus::Available) {
            let now = chrono::Utc::now().to_rfc3339();
            info!("🔧 Updating agent {} to AVAILABLE with NEW timestamp: {}", agent_id, now);
            
            let rows_updated = self.execute(
                "UPDATE agents SET status = ?1, available_since = ?2 WHERE agent_id = ?3",
                vec![status_str.into(), now.clone().into(), agent_id.into()] as Vec<limbo::Value>
            ).await?;
            
            info!("🔧 Agent {} status updated to {:?} with available_since timestamp {} (rows affected: {})", 
                   agent_id, status, now, rows_updated);
                   
            // Verify the UPDATE by selecting the specific record we just updated
            info!("🔍 Verifying UPDATE with targeted SELECT...");
            let verification = self.query(
                "SELECT agent_id, status, available_since FROM agents WHERE agent_id = ?1",
                vec![agent_id.into()] as Vec<limbo::Value>
            ).await;
            
            match verification {
                Ok(rows) => {
                    if rows.is_empty() {
                        info!("🔍 ❌ VERIFICATION FAILED: Agent {} not found after UPDATE!", agent_id);
                    } else {
                        let row = &rows[0];
                        if let (Ok(db_agent_id), Ok(db_status), available_since_val) = (
                            row.get_value(0), row.get_value(1), row.get_value(2)
                        ) {
                            let db_agent_id = match db_agent_id {
                                limbo::Value::Text(s) => s,
                                _ => "INVALID".to_string(),
                            };
                            let db_status = match db_status {
                                limbo::Value::Text(s) => s,
                                _ => "INVALID".to_string(),
                            };
                            let db_available_since = match available_since_val {
                                Ok(limbo::Value::Text(s)) => Some(s),
                                _ => None,
                            };
                            
                            info!("🔍 ✅ UPDATE VERIFICATION SUCCESS:");
                            info!("🔍   - agent_id: '{}' (expected: '{}')", db_agent_id, agent_id);
                            info!("🔍   - status: '{}' (expected: 'AVAILABLE')", db_status);
                            info!("🔍   - available_since: '{:?}' (expected: '{}')", db_available_since, now);
                            
                            // Check for mismatches
                            if db_agent_id != agent_id {
                                info!("🔍 ❌ MISMATCH: agent_id doesn't match!");
                            }
                            if db_status != "AVAILABLE" {
                                info!("🔍 ❌ MISMATCH: status is not AVAILABLE!");
                            }
                            if let Some(db_timestamp) = &db_available_since {
                                if db_timestamp != &now {
                                    info!("🔍 ❌ TIMESTAMP MISMATCH: available_since '{}' != expected '{}'", db_timestamp, now);
                                } else {
                                    info!("🔍 ✅ TIMESTAMP MATCH: available_since correctly updated to '{}'", now);
                                }
                            } else {
                                info!("🔍 ❌ TIMESTAMP MISSING: available_since is NULL!");
                            }
                        } else {
                            info!("🔍 ❌ VERIFICATION FAILED: Could not parse updated record fields");
                        }
                    }
                }
                Err(e) => {
                    info!("🔍 ❌ VERIFICATION FAILED: SELECT error after UPDATE: {}", e);
                }
            }
        } else {
            info!("🔧 Updating agent {} to {} and clearing available_since timestamp", agent_id, status_str);
            
            // For non-available states, clear the available_since timestamp
            let rows_updated = self.execute(
                "UPDATE agents SET status = ?1, available_since = NULL WHERE agent_id = ?2",
                vec![status_str.into(), agent_id.into()] as Vec<limbo::Value>
            ).await?;
            
            info!("🔧 Agent {} status updated to {:?} (rows affected: {})", agent_id, status, rows_updated);
            
            // Verify the UPDATE by selecting the specific record we just updated
            info!("🔍 Verifying UPDATE with targeted SELECT...");
            let verification = self.query(
                "SELECT agent_id, status, available_since FROM agents WHERE agent_id = ?1",
                vec![agent_id.into()] as Vec<limbo::Value>
            ).await;
            
            match verification {
                Ok(rows) => {
                    if rows.is_empty() {
                        info!("🔍 ❌ VERIFICATION FAILED: Agent {} not found after UPDATE!", agent_id);
                    } else {
                        let row = &rows[0];
                        if let (Ok(db_agent_id), Ok(db_status), available_since_val) = (
                            row.get_value(0), row.get_value(1), row.get_value(2)
                        ) {
                            let db_agent_id = match db_agent_id {
                                limbo::Value::Text(s) => s,
                                _ => "INVALID".to_string(),
                            };
                            let db_status = match db_status {
                                limbo::Value::Text(s) => s,
                                _ => "INVALID".to_string(),
                            };
                            let db_available_since = match available_since_val {
                                Ok(limbo::Value::Text(s)) => Some(s),
                                Ok(limbo::Value::Null) => None,
                                _ => None,
                            };
                            
                            info!("🔍 ✅ UPDATE VERIFICATION SUCCESS:");
                            info!("🔍   - agent_id: '{}' (expected: '{}')", db_agent_id, agent_id);
                            info!("🔍   - status: '{}' (expected: '{}')", db_status, status_str);
                            info!("🔍   - available_since: '{:?}' (expected: NULL)", db_available_since);
                            
                            // Check for mismatches
                            if db_agent_id != agent_id {
                                info!("🔍 ❌ MISMATCH: agent_id doesn't match!");
                            }
                            if db_status != status_str {
                                info!("🔍 ❌ MISMATCH: status '{}' != expected '{}'!", db_status, status_str);
                            }
                            if db_available_since.is_some() {
                                info!("🔍 ❌ TIMESTAMP MISMATCH: available_since should be NULL but is '{:?}'", db_available_since);
                            } else {
                                info!("🔍 ✅ TIMESTAMP CORRECTLY CLEARED: available_since is NULL");
                            }
                        } else {
                            info!("🔍 ❌ VERIFICATION FAILED: Could not parse updated record fields");
                        }
                    }
                }
                Err(e) => {
                    info!("🔍 ❌ VERIFICATION FAILED: SELECT error after UPDATE: {}", e);
                }
            }
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
            info!("🔄 ROUND ROBIN: Only {} agent(s) available, no rotation needed", agents.len());
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