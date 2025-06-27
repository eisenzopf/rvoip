//! Database-backed queue management module

use anyhow::{Result, anyhow};
use limbo::{Builder, Connection, Database, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use tracing::{info, error, warn, debug};
use std::pin::Pin;
use std::future::Future;
use crate::agent::AgentStatus;
use uuid::Uuid;

pub mod agents;
pub mod queues;
pub mod calls;
pub mod schema;
pub mod routing_store;
pub mod queue_store;
pub mod call_records;

pub use agents::*;
pub use queues::*;
pub use calls::*;
pub use schema::*;

/// Database manager for call center operations
#[derive(Clone)]
pub struct DatabaseManager {
    db: Arc<Database>,
    connection: Arc<RwLock<Connection>>,
}

impl DatabaseManager {
    /// Create a new database manager
    pub async fn new(db_path: &str) -> Result<Self> {
        // Create or open the database
        let db = Builder::new_local(db_path).build().await?;
        let connection = db.connect()?;
        
        let manager = Self {
            db: Arc::new(db),
            connection: Arc::new(RwLock::new(connection)),
        };
        
        // Initialize schema
        manager.initialize_schema().await?;
        
        Ok(manager)
    }
    
    /// Create in-memory database for testing
    pub async fn new_in_memory() -> Result<Self> {
        let db = Builder::new_local(":memory:").build().await?;
        let connection = db.connect()?;
        
        let manager = Self {
            db: Arc::new(db),
            connection: Arc::new(RwLock::new(connection)),
        };
        
        // Initialize schema
        manager.initialize_schema().await?;
        
        Ok(manager)
    }
    
    /// Get a new connection for transactions
    pub fn connect(&self) -> Result<Connection> {
        Ok(self.db.connect()?)
    }
    
    /// Execute a query that returns no results with positional params
    pub async fn execute<P: IntoParams>(&self, sql: &str, params: P) -> Result<usize> {
        let conn = self.connection.write().await;
        Ok(conn.execute(sql, params).await? as usize)
    }
    
    /// Execute a query that returns results with positional params
    pub async fn query<P: IntoParams>(&self, sql: &str, params: P) -> Result<Vec<limbo::Row>> {
        let conn = self.connection.read().await;
        let mut stmt = conn.prepare(sql).await?;
        let mut rows = stmt.query(params).await?;
        
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(row);
        }
        
        Ok(results)
    }
    
    /// Execute a query that returns a single row
    pub async fn query_row<P: IntoParams>(&self, sql: &str, params: P) -> Result<Option<limbo::Row>> {
        let rows = self.query(sql, params).await?;
        Ok(rows.into_iter().next())
    }
    
    /// Begin a transaction
    pub async fn transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: for<'a> FnOnce(&'a mut Transaction) -> Pin<Box<dyn Future<Output = Result<R>> + Send + 'a>>,
        R: Send,
    {
        let mut conn = self.connect()?;
        
        // Start transaction
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        
        let mut tx = Transaction { conn };
        
        match f(&mut tx).await {
            Ok(result) => {
                // Commit on success
                tx.conn.execute("COMMIT", ()).await?;
                Ok(result)
            }
            Err(e) => {
                // Rollback on error
                let _ = tx.conn.execute("ROLLBACK", ()).await;
                Err(e)
            }
        }
    }
    
    /// Helper function to retry database operations with exponential backoff
    async fn retry_operation<F, T, Fut>(&self, operation_name: &str, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        info!("🔧 retry_operation started for '{}'", operation_name);
        let mut attempts = 0;
        let max_attempts = 3;
        let mut backoff_ms = 100;
        
        loop {
            attempts += 1;
            info!("🔧 retry_operation attempt {}/{} for '{}'", attempts, max_attempts, operation_name);
            
            match operation().await {
                Ok(result) => {
                    info!("🔧 retry_operation SUCCESS on attempt {}/{} for '{}'", attempts, max_attempts, operation_name);
                    return Ok(result);
                },
                Err(e) if attempts < max_attempts => {
                    warn!("Database operation '{}' failed (attempt {}/{}): {}", 
                          operation_name, attempts, max_attempts, e);
                    
                    // Check if it's a known recoverable error
                    let error_msg = e.to_string();
                    if error_msg.contains("current_page") || 
                       error_msg.contains("btree") ||
                       error_msg.contains("locked") ||
                       error_msg.contains("busy") {
                        // These are potentially recoverable errors
                        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                        backoff_ms *= 2; // Exponential backoff
                        continue;
                    }
                    
                    // Non-recoverable error
                    info!("🔧 retry_operation FAILED (non-recoverable) on attempt {}/{} for '{}': {}", attempts, max_attempts, operation_name, e);
                    return Err(e);
                }
                Err(e) => {
                    error!("Database operation '{}' failed after {} attempts: {}", 
                           operation_name, attempts, e);
                    info!("🔧 retry_operation FAILED (max attempts) after {}/{} for '{}': {}", attempts, max_attempts, operation_name, e);
                    return Err(e);
                }
            }
        }
    }
    
    /// Assign a call to an agent with simplified operations to avoid Limbo database bugs
    /// ASSUMES: Agent has already been reserved (marked as BUSY) in previous step
    pub async fn atomic_assign_call_to_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        customer_sdp: Option<String>,
    ) -> Result<()> {
        info!("🔄 Starting simplified call assignment for session {} to agent {}", session_id, agent_id);
        
        // Step 1: Remove call from queue (simple DELETE)
        let dequeue_query = "DELETE FROM call_queue WHERE session_id = ?";
        self.execute(dequeue_query, vec![limbo::Value::Text(session_id.to_string())]).await
            .map_err(|e| anyhow!("Failed to dequeue call {}: {}", session_id, e))?;
        
        info!("✅ Dequeued call {} from queue", session_id);
        
        // Step 2: Add to active calls (fixed column names to match schema)
        let now = chrono::Utc::now().to_rfc3339();
        let call_id = format!("call_{}", uuid::Uuid::new_v4());
        let add_active_query = "INSERT INTO active_calls 
            (call_id, agent_id, session_id, assigned_at) 
            VALUES (?, ?, ?, ?)";
        self.execute(add_active_query, vec![
            limbo::Value::Text(call_id.clone()),
            limbo::Value::Text(agent_id.to_string()),
            limbo::Value::Text(session_id.to_string()),
            limbo::Value::Text(now),
        ]).await
            .map_err(|e| anyhow!("Failed to add active call {}: {}", call_id, e))?;
        
        info!("✅ Added call {} to active calls for agent {}", call_id, agent_id);
        
        info!("✅ Successfully assigned call {} to agent {} (simplified approach)", session_id, agent_id);
        Ok(())
    }
    
    /// Update agent status with retry logic
    pub async fn update_agent_status_with_retry(&self, agent_id: &str, status: AgentStatus) -> Result<()> {
        info!("🔧 update_agent_status_with_retry called: agent_id='{}', status='{:?}'", agent_id, status);
        
        let operation = || async {
            info!("🔧 Retry operation calling update_agent_status for agent '{}'", agent_id);
            // Use simple method call since duplicates are removed
            self.update_agent_status(agent_id, status.clone()).await
        };
        
        let result = self.retry_operation("update_agent_status", operation).await;
        info!("🔧 update_agent_status_with_retry result for agent '{}': {:?}", agent_id, result);
        result
    }
    
    /// Update agent call count with retry logic
    pub async fn update_agent_call_count_with_retry(&self, agent_id: &str, delta: i32) -> Result<()> {
        let operation = || async {
            self.update_agent_call_count(agent_id, delta).await
        };
        
        self.retry_operation("update_agent_call_count", operation).await
    }
}

/// Transaction wrapper for atomic operations
pub struct Transaction {
    conn: Connection,
}

impl Transaction {
    /// Execute a query within the transaction
    pub async fn execute<P: IntoParams>(&mut self, sql: &str, params: P) -> Result<usize> {
        Ok(self.conn.execute(sql, params).await? as usize)
    }
    
    /// Query within the transaction
    pub async fn query<P: IntoParams>(&mut self, sql: &str, params: P) -> Result<Vec<limbo::Row>> {
        let mut stmt = self.conn.prepare(sql).await?;
        let mut rows = stmt.query(params).await?;
        
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(row);
        }
        
        Ok(results)
    }
    
    /// Get number of rows changed in last operation
    pub async fn changes(&self) -> usize {
        // TODO: Implement this when limbo supports it
        // For now, return a placeholder
        1
    }
}

/// Utility functions for Limbo value conversions
pub mod value_helpers {
    use limbo::Value;
    use anyhow::{Result, bail};
    
    /// Convert Value to String
    pub fn value_to_string(val: &Value) -> Result<String> {
        match val {
            Value::Text(s) => Ok(s.clone()),
            Value::Blob(b) => Ok(String::from_utf8_lossy(b).to_string()),
            Value::Integer(i) => Ok(i.to_string()),
            Value::Real(f) => Ok(f.to_string()),
            Value::Null => bail!("Cannot convert NULL to string"),
        }
    }
    
    /// Convert Value to optional String
    pub fn value_to_optional_string(val: &Value) -> Option<String> {
        match val {
            Value::Text(s) => Some(s.clone()),
            Value::Blob(b) => Some(String::from_utf8_lossy(b).to_string()),
            Value::Integer(i) => Some(i.to_string()),
            Value::Real(f) => Some(f.to_string()),
            Value::Null => None,
        }
    }
    
    /// Convert Value to i32
    pub fn value_to_i32(val: &Value) -> Result<i32> {
        match val {
            Value::Integer(i) => Ok(*i as i32),
            Value::Real(f) => Ok(*f as i32),
            _ => bail!("Cannot convert {:?} to i32", val),
        }
    }
    
    /// Convert Value to i64
    pub fn value_to_i64(val: &Value) -> Result<i64> {
        match val {
            Value::Integer(i) => Ok(*i),
            Value::Real(f) => Ok(*f as i64),
            _ => bail!("Cannot convert {:?} to i64", val),
        }
    }
    
    /// Convert Value to f64
    pub fn value_to_f64(val: &Value) -> Result<f64> {
        match val {
            Value::Real(f) => Ok(*f),
            Value::Integer(i) => Ok(*i as f64),
            _ => bail!("Cannot convert {:?} to f64", val),
        }
    }
    
    /// Convert Value to optional f64
    pub fn value_to_optional_f64(val: &Value) -> Option<f64> {
        match val {
            Value::Real(f) => Some(*f),
            Value::Integer(i) => Some(*i as f64),
            Value::Null => None,
            _ => None,
        }
    }
}

// Re-export IntoParams for convenience
pub use limbo::params::IntoParams;

/// Agent status enum for database
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DbAgentStatus {
    Offline,
    Available,
    Busy,  // Changed from Busy(Vec<SessionId>) to just Busy
    PostCallWrapUp,
    Reserved,
}

impl DbAgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DbAgentStatus::Offline => "OFFLINE",
            DbAgentStatus::Available => "AVAILABLE",
            DbAgentStatus::Busy => "BUSY",
            DbAgentStatus::PostCallWrapUp => "POSTCALLWRAPUP",
            DbAgentStatus::Reserved => "RESERVED",
        }
    }
    
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "OFFLINE" => Some(DbAgentStatus::Offline),
            "AVAILABLE" => Some(DbAgentStatus::Available),
            "BUSY" => Some(DbAgentStatus::Busy),
            "POSTCALLWRAPUP" => Some(DbAgentStatus::PostCallWrapUp),
            "RESERVED" => Some(DbAgentStatus::Reserved),
            _ => None,
        }
    }
}

/// Agent record from database
#[derive(Debug, Clone)]
pub struct DbAgent {
    pub agent_id: String,
    pub username: String,
    pub status: DbAgentStatus,
    pub max_calls: i32,
    pub current_calls: i32,
    pub contact_uri: Option<String>,
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub available_since: Option<String>, // For fair round robin ordering
}

/// Queued call record
#[derive(Debug, Clone)]
pub struct DbQueuedCall {
    pub call_id: String,
    pub session_id: String,
    pub queue_id: String,
    pub customer_info: Option<String>,
    pub priority: i32,
    pub enqueued_at: DateTime<Utc>,
    pub attempts: i32,
    pub last_attempt: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

/// Active call record
#[derive(Debug, Clone)]
pub struct DbActiveCall {
    pub call_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub customer_dialog_id: Option<String>,
    pub agent_dialog_id: Option<String>,
    pub assigned_at: DateTime<Utc>,
    pub answered_at: Option<DateTime<Utc>>,
}

/// Queue configuration
#[derive(Debug, Clone)]
pub struct DbQueue {
    pub queue_id: String,
    pub name: String,
    pub capacity: i32,
    pub overflow_queue: Option<String>,
    pub priority_boost: i32,
}

/// Call center database manager using Limbo
#[derive(Clone)]
pub struct CallCenterDatabase {
    /// Limbo database instance
    db: Arc<Database>,
    
    /// Database connection pool (simplified for now)
    connection: Arc<RwLock<Connection>>,
}

impl CallCenterDatabase {
    /// Create a new call center database
    pub async fn new(db_path: &str) -> Result<Self> {
        info!("🗄️ Initializing Limbo database at: {}", db_path);
        
        // Create database using Limbo's API
        let db = Builder::new_local(db_path).build().await?;
        let connection = db.connect()?;
        
        let database = Self {
            db: Arc::new(db),
            connection: Arc::new(RwLock::new(connection)),
        };
        
        // Initialize schema
        database.initialize_schema().await?;
        
        info!("✅ Call center database initialized successfully");
        Ok(database)
    }
    
    /// Create in-memory database for testing
    pub async fn new_in_memory() -> Result<Self> {
        info!("🗄️ Creating in-memory Limbo database");
        
        let db = Builder::new_local(":memory:").build().await?;
        let connection = db.connect()?;
        
        let database = Self {
            db: Arc::new(db),
            connection: Arc::new(RwLock::new(connection)),
        };
        
        database.initialize_schema().await?;
        
        info!("✅ In-memory database created successfully");
        Ok(database)
    }
    
    /// Initialize database schema
    async fn initialize_schema(&self) -> Result<()> {
        info!("📋 Creating call center database schema");
        
        // Use the new centralized schema initialization by creating a temporary DatabaseManager
        // and delegating to it (this ensures consistency)
        let temp_db_manager = DatabaseManager {
            db: self.db.clone(),
            connection: self.connection.clone(),
        };
        
        // Use the centralized schema initialization
        schema::initialize_call_center_schema(&temp_db_manager).await?;
        
        info!("✅ Database schema created successfully using centralized initialization");
        Ok(())
    }
    
    /// Get a database connection
    pub async fn connection(&self) -> tokio::sync::RwLockReadGuard<'_, Connection> {
        self.connection.read().await
    }
    
    /// Get a mutable database connection
    pub async fn connection_mut(&self) -> tokio::sync::RwLockWriteGuard<'_, Connection> {
        self.connection.write().await
    }
    
    /// Execute a health check query
    pub async fn health_check(&self) -> Result<bool> {
        let conn = self.connection().await;
        let result = conn.query("SELECT 1", ()).await;
        match result {
            Ok(_) => {
                debug!("💚 Database health check passed");
                Ok(true)
            }
            Err(e) => {
                error!("❌ Database health check failed: {}", e);
                Ok(false)
            }
        }
    }
}

/// Database error types
#[derive(thiserror::Error, Debug)]
pub enum DatabaseError {
    #[error("Database connection error: {0}")]
    Connection(String),
    
    #[error("Query execution error: {0}")]
    Query(String),
    
    #[error("Schema creation error: {0}")]
    Schema(String),
    
    #[error("Data validation error: {0}")]
    Validation(String),
} 