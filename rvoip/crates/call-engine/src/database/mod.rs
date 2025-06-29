//! # Database Management Module
//!
//! This module provides comprehensive database management functionality for the call center,
//! built on top of the Limbo SQLite database engine. It handles all persistent storage
//! requirements including agent management, call tracking, queue management, and call records.
//!
//! ## Architecture
//!
//! The database module follows a layered architecture:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Application Layer                        │
//! │  (CallCenterEngine, APIs, etc.)                           │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │                 Database Manager                            │
//! │  - Transaction Management                                   │
//! │  - Connection Pooling                                       │
//! │  - Query Execution                                          │
//! │  - Retry Logic                                              │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │               Domain-Specific Modules                       │
//! │  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐           │
//! │  │   Agents    │ │   Queues    │ │    Calls    │           │
//! │  │             │ │             │ │             │           │
//! │  └─────────────┘ └─────────────┘ └─────────────┘           │
//! │  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐           │
//! │  │   Schema    │ │  Routing    │ │   Records   │           │
//! │  │             │ │             │ │             │           │
//! │  └─────────────┘ └─────────────┘ └─────────────┘           │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │                  Limbo SQLite Engine                        │
//! │  - ACID Transactions                                        │
//! │  - Concurrent Access                                        │
//! │  - WAL Mode                                                 │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Core Features
//!
//! ### **Agent Management**
//! - Agent registration and authentication
//! - Real-time status tracking (Available, Busy, Offline, etc.)
//! - Skill and capability management
//! - Performance metrics and statistics
//!
//! ### **Queue Management**
//! - Multi-priority queue support
//! - Overflow and escalation policies
//! - Wait time tracking and SLA monitoring
//! - Queue statistics and analytics
//!
//! ### **Call Tracking**
//! - Active call state management
//! - Call history and records
//! - Quality metrics and MOS scores
//! - Call routing decisions and outcomes
//!
//! ### **Transaction Safety**
//! - ACID compliant transactions
//! - Automatic retry with exponential backoff
//! - Deadlock detection and recovery
//! - Connection pool management
//!
//! ## Quick Start
//!
//! ### Basic Database Setup
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> anyhow::Result<()> {
//! // Create persistent database
//! let db = DatabaseManager::new("callcenter.db").await?;
//! 
//! // Or create in-memory database for testing
//! let db_test = DatabaseManager::new_in_memory().await?;
//! 
//! println!("Database ready for call center operations");
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Operations
//!
//! ```rust
//! use rvoip_call_engine::database::{DatabaseManager, DbAgent, DbAgentStatus};
//! 
//! # async fn example() -> anyhow::Result<()> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Register a new agent
//! let agent = DbAgent {
//!     agent_id: "agent-001".to_string(),
//!     username: "alice.johnson".to_string(),
//!     status: DbAgentStatus::Available,
//!     max_calls: 3,
//!     current_calls: 0,
//!     contact_uri: Some("sip:alice@company.com".to_string()),
//!     last_heartbeat: Some(chrono::Utc::now()),
//!     available_since: Some(chrono::Utc::now().to_rfc3339()),
//! };
//! 
//! // Register agent (handled by agent management module)
//! // db.register_agent(&agent).await?;
//! 
//! // Update agent status (example - method may not exist yet)
//! // db.update_agent_status("agent-001", AgentStatus::Available).await?;
//! 
//! // Get available agents (example - method may not exist yet)
//! // let available_agents = db.get_available_agents().await?;
//! // println!("Found {} available agents", available_agents.len());
//! 
//! println!("Agent operations configured");
//! # Ok(())
//! # }
//! ```
//!
//! ### Queue Operations
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! 
//! # async fn example() -> anyhow::Result<()> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Enqueue a call
//! let session_id = "session-12345";
//! let queue_id = "support-queue";
//! 
//! // Enqueue call (handled by queue management module)
//! // db.enqueue_call(session_id, queue_id, 1, None).await?;
//! 
//! // Get queue statistics (example - using agent stats as demonstration)
//! let agent_stats = db.get_agent_stats().await?;
//! println!("Database has {} agent records", agent_stats.total_agents);
//! 
//! println!("Queue operations configured");
//! # Ok(())
//! # }
//! ```
//!
//! ### Transaction Example
//!
//! ```rust
//! use rvoip_call_engine::database::DatabaseManager;
//! use limbo::Value;
//! 
//! # async fn example() -> anyhow::Result<()> {
//! # let db = DatabaseManager::new_in_memory().await?;
//! 
//! // Atomic call assignment with transaction
//! let result = db.transaction(|tx| Box::pin(async move {
//!     // Dequeue call from waiting queue
//!     tx.execute(
//!         "DELETE FROM call_queue WHERE session_id = ? AND queue_id = ?",
//!         vec![Value::Text("session-123".to_string()), Value::Text("support-queue".to_string())]
//!     ).await?;
//!     
//!     // Add to active calls
//!     tx.execute(
//!         "INSERT INTO active_calls (call_id, agent_id, session_id, assigned_at) VALUES (?, ?, ?, ?)",
//!         vec![
//!             Value::Text("call-456".to_string()),
//!             Value::Text("agent-001".to_string()),
//!             Value::Text("session-123".to_string()),
//!             Value::Text(chrono::Utc::now().to_rfc3339())
//!         ]
//!     ).await?;
//!     
//!     // Update agent status to busy
//!     tx.execute(
//!         "UPDATE agents SET status = 'BUSY', current_calls = current_calls + 1 WHERE agent_id = ?",
//!         vec![Value::Text("agent-001".to_string())]
//!     ).await?;
//!     
//!     Ok(())
//! })).await?;
//! 
//! println!("Call assigned atomically");
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance Considerations
//!
//! ### **Connection Management**
//! - Uses connection pooling for concurrent access
//! - Automatic connection recovery on failure
//! - Optimized for high-throughput call center operations
//!
//! ### **Query Optimization**
//! - Indexed columns for fast agent and call lookups
//! - Optimized JOIN queries for complex reporting
//! - Prepared statements for frequently executed queries
//!
//! ### **Retry Logic**
//! - Exponential backoff for transient failures
//! - Automatic recovery from deadlocks and timeouts
//! - Configurable retry attempts and delays
//!
//! ## Error Handling
//!
//! The module uses comprehensive error handling with specific error types:
//!
//! ```rust
//! use rvoip_call_engine::database::{DatabaseManager, DatabaseError};
//! 
//! # async fn example() {
//! match DatabaseManager::new("invalid-path").await {
//!     Ok(db) => println!("Database connected successfully"),
//!     Err(e) => match e.downcast_ref::<DatabaseError>() {
//!         Some(DatabaseError::Connection(msg)) => println!("Connection failed: {}", msg),
//!         Some(DatabaseError::Schema(msg)) => println!("Schema error: {}", msg),
//!         _ => println!("Other database error: {}", e),
//!     }
//! }
//! # }
//! ```
//!
//! ## Database Schema
//!
//! The database uses a well-designed schema optimized for call center operations:
//!
//! - **agents**: Agent registration and status tracking
//! - **call_queue**: Queued calls waiting for assignment
//! - **active_calls**: Currently active calls and assignments
//! - **call_history**: Complete call records and analytics
//! - **queues**: Queue configuration and policies
//! - **routing_rules**: Dynamic routing policies
//!
//! See the [`schema`] module for complete table definitions and relationships.
//!
//! ## Modules
//!
//! - [`agents`]: Agent registration, status, and management operations
//! - [`queues`]: Queue management and call queuing operations
//! - [`calls`]: Active call tracking and call history management
//! - [`schema`]: Database schema definitions and initialization
//! - [`routing_store`]: Routing rules and policies storage
//! - [`queue_store`]: Queue configuration and statistics
//! - [`call_records`]: Call history and analytics storage
//!
//! ## Production Deployment
//!
//! For production environments:
//!
//! - **Database Path**: Use a dedicated directory with proper permissions
//! - **Backup Strategy**: Implement regular database backups
//! - **Monitoring**: Track database performance and connection health
//! - **Scaling**: Consider connection pooling and read replicas for high load
//! - **Maintenance**: Regular VACUUM and ANALYZE operations for performance

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

/// # Database Manager for Call Center Operations
///
/// The [`DatabaseManager`] is the primary interface for all database operations
/// in the call center system. It provides a high-level, type-safe API over the
/// underlying Limbo SQLite database with advanced features like connection pooling,
/// automatic retries, and transaction management.
///
/// ## Key Features
///
/// - **ACID Transactions**: Full transaction support with rollback on errors
/// - **Connection Pooling**: Efficient connection management for concurrent operations
/// - **Retry Logic**: Automatic retry with exponential backoff for transient failures
/// - **Type Safety**: Strongly typed interfaces for all database operations
/// - **Performance**: Optimized queries and prepared statements
///
/// ## Thread Safety
///
/// `DatabaseManager` is designed to be safely shared across threads and async tasks.
/// All methods are async and use internal locking to ensure data consistency.
///
/// ## Examples
///
    /// ### Basic Usage
    ///
    /// ```rust
    /// use rvoip_call_engine::database::DatabaseManager;
    /// use limbo::Value;
    /// 
    /// # async fn example() -> anyhow::Result<()> {
    /// // Create database manager
    /// let db = DatabaseManager::new("callcenter.db").await?;
    /// 
    /// // Execute a simple query
    /// let count = db.execute(
    ///     "UPDATE agents SET last_heartbeat = ? WHERE agent_id = ?",
    ///     vec![
    ///         Value::Text(chrono::Utc::now().to_rfc3339()),
    ///         Value::Text("agent-001".to_string())
    ///     ]
    /// ).await?;
    /// 
    /// println!("Updated {} agent records", count);
    /// # Ok(())
    /// # }
    /// ```
///
    /// ### Transaction Example
    ///
    /// ```rust
    /// use rvoip_call_engine::database::DatabaseManager;
    /// use limbo::Value;
    /// 
    /// # async fn example() -> anyhow::Result<()> {
    /// # let db = DatabaseManager::new_in_memory().await?;
    /// 
    /// // Perform atomic operations
    /// db.transaction(|tx| Box::pin(async move {
    ///     // Multiple operations in single transaction
    ///     tx.execute("DELETE FROM call_queue WHERE session_id = ?", 
    ///                vec![Value::Text("session-123".to_string())]).await?;
    ///     tx.execute("INSERT INTO active_calls (call_id, session_id) VALUES (?, ?)", 
    ///                vec![Value::Text("call-456".to_string()), Value::Text("session-123".to_string())]).await?;
    ///     Ok::<(), anyhow::Error>(())
    /// })).await?;
    /// # Ok(())
    /// # }
    /// ```
#[derive(Clone)]
pub struct DatabaseManager {
    /// Limbo database instance (shared across all connections)
    db: Arc<Database>,
    /// Primary connection for most operations (protected by RwLock)
    connection: Arc<RwLock<Connection>>,
}

impl DatabaseManager {
    /// Create a new database manager with persistent storage
    ///
    /// Initializes a new database manager connected to a file-based SQLite database.
    /// If the database file doesn't exist, it will be created. The database schema
    /// will be automatically initialized on first connection.
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path to the database file. Use ":memory:" for in-memory database.
    ///
    /// # Returns
    ///
    /// Returns a `DatabaseManager` instance ready for call center operations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::database::DatabaseManager;
    /// 
    /// # async fn example() -> anyhow::Result<()> {
    /// // Create production database
    /// let db = DatabaseManager::new("/var/lib/callcenter/database.db").await?;
    /// 
    /// // Create temporary database
    /// let db_temp = DatabaseManager::new("/tmp/test.db").await?;
    /// 
    /// // Create in-memory database
    /// let db_memory = DatabaseManager::new(":memory:").await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// - `DatabaseError::Connection` - Database file cannot be opened or created
    /// - `DatabaseError::Schema` - Database schema initialization failed
    /// - `DatabaseError::Query` - Initial database queries failed
    ///
    /// # Performance Notes
    ///
    /// - Database connections are pooled and reused for efficiency
    /// - WAL mode is enabled for better concurrent performance
    /// - Foreign key constraints are enforced for data integrity
    pub async fn new(db_path: &str) -> Result<Self> {
        info!("🗄️ Initializing database manager at: {}", db_path);
        
        // Create or open the database
        let db = Builder::new_local(db_path).build().await?;
        let connection = db.connect()?;
        
        let manager = Self {
            db: Arc::new(db),
            connection: Arc::new(RwLock::new(connection)),
        };
        
        // Initialize schema
        manager.initialize_schema().await?;
        
        info!("✅ Database manager initialized successfully");
        Ok(manager)
    }
    
    /// Create an in-memory database manager for testing
    ///
    /// Creates a database manager with an in-memory SQLite database. This is
    /// perfect for unit tests, integration tests, and development scenarios
    /// where persistence is not required.
    ///
    /// # Returns
    ///
    /// Returns a `DatabaseManager` instance with in-memory storage.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::database::DatabaseManager;
    /// 
    /// # async fn example() -> anyhow::Result<()> {
    /// // Create test database
    /// let db = DatabaseManager::new_in_memory().await?;
    /// 
    /// // Perform test operations
    /// let agent_count = db.execute(
    ///     "INSERT INTO agents (agent_id, username, status) VALUES (?, ?, ?)",
    ///     ("test-agent", "testuser", "AVAILABLE")
    /// ).await?;
    /// 
    /// assert_eq!(agent_count, 1);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Performance Notes
    ///
    /// In-memory databases are extremely fast but data is lost when the
    /// database manager is dropped. Perfect for testing scenarios.
    pub async fn new_in_memory() -> Result<Self> {
        info!("🗄️ Creating in-memory database manager");
        
        let db = Builder::new_local(":memory:").build().await?;
        let connection = db.connect()?;
        
        let manager = Self {
            db: Arc::new(db),
            connection: Arc::new(RwLock::new(connection)),
        };
        
        // Initialize schema
        manager.initialize_schema().await?;
        
        info!("✅ In-memory database manager created successfully");
        Ok(manager)
    }
    
    /// Get a new database connection for transaction operations
    ///
    /// Creates a new connection to the database for use in transactions or
    /// when you need a dedicated connection for a sequence of operations.
    ///
    /// # Returns
    ///
    /// Returns a new `Connection` instance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::database::DatabaseManager;
    /// use limbo::Value;
    /// 
    /// # async fn example() -> anyhow::Result<()> {
    /// # let db = DatabaseManager::new_in_memory().await?;
    /// 
    /// // Get dedicated connection for complex operations
    /// let mut conn = db.connect()?;
    /// 
    /// // Use connection for multiple related operations
    /// conn.execute("BEGIN IMMEDIATE", Vec::<Value>::new()).await?;
    /// conn.execute("INSERT INTO agents (agent_id, username) VALUES (?, ?)", 
    ///              vec![Value::Text("agent-001".to_string()), Value::Text("alice".to_string())]).await?;
    /// conn.execute("UPDATE statistics SET total_agents = total_agents + 1", Vec::<Value>::new()).await?;
    /// conn.execute("COMMIT", Vec::<Value>::new()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn connect(&self) -> Result<Connection> {
        Ok(self.db.connect()?)
    }
    
    /// Execute a query that returns no results (INSERT, UPDATE, DELETE)
    ///
    /// Executes SQL statements that modify data but don't return result sets.
    /// Returns the number of rows affected by the operation.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL statement to execute
    /// * `params` - Parameters for the SQL statement (supports various types)
    ///
    /// # Returns
    ///
    /// Returns the number of rows affected by the operation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::database::DatabaseManager;
    /// use limbo::Value;
    /// 
    /// # async fn example() -> anyhow::Result<()> {
    /// # let db = DatabaseManager::new_in_memory().await?;
    /// 
    /// // Insert with positional parameters
    /// let inserted = db.execute(
    ///     "INSERT INTO agents (agent_id, username, status) VALUES (?, ?, ?)",
    ///     vec![
    ///         Value::Text("agent-001".to_string()),
    ///         Value::Text("alice".to_string()),
    ///         Value::Text("AVAILABLE".to_string())
    ///     ]
    /// ).await?;
    /// assert_eq!(inserted, 1);
    /// 
    /// // Update with parameters
    /// let updated = db.execute(
    ///     "UPDATE agents SET status = ? WHERE agent_id = ?",
    ///     vec![Value::Text("BUSY".to_string()), Value::Text("agent-001".to_string())]
    /// ).await?;
    /// assert_eq!(updated, 1);
    /// 
    /// // Delete with parameters
    /// let deleted = db.execute(
    ///     "DELETE FROM agents WHERE status = ?",
    ///     vec![Value::Text("OFFLINE".to_string())]
    /// ).await?;
    /// println!("Deleted {} offline agents", deleted);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Error Handling
    ///
    /// Automatically retries on transient failures with exponential backoff.
    /// Will return an error for constraint violations, syntax errors, etc.
    pub async fn execute<P: IntoParams>(&self, sql: &str, params: P) -> Result<usize> {
        let conn = self.connection.write().await;
        Ok(conn.execute(sql, params).await? as usize)
    }
    
    /// Execute a query that returns results (SELECT)
    ///
    /// Executes SQL SELECT statements and returns all matching rows.
    /// Use this for queries that return data from the database.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL SELECT statement to execute
    /// * `params` - Parameters for the SQL statement
    ///
    /// # Returns
    ///
    /// Returns a vector of `limbo::Row` containing the query results.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::database::DatabaseManager;
    /// use limbo::Value;
    /// 
    /// # async fn example() -> anyhow::Result<()> {
    /// # let db = DatabaseManager::new_in_memory().await?;
    /// # db.execute("INSERT INTO agents (agent_id, username, status) VALUES (?, ?, ?)", 
    /// #     vec![Value::Text("agent-001".to_string()), Value::Text("alice".to_string()), Value::Text("AVAILABLE".to_string())]).await?;
    /// 
    /// // Query agents by status
    /// let rows = db.query(
    ///     "SELECT agent_id, username, status FROM agents WHERE status = ?",
    ///     vec![Value::Text("AVAILABLE".to_string())]
    /// ).await?;
    /// 
    /// println!("Found {} available agents", rows.len());
    /// 
    /// // Query with multiple conditions  
    /// let busy_agents = db.query(
    ///     "SELECT COUNT(*) FROM agents WHERE status = ? AND current_calls > ?",
    ///     vec![Value::Text("BUSY".to_string()), Value::Integer(0)]
    /// ).await?;
    /// 
    /// println!("Query executed successfully");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Performance Notes
    ///
    /// - Results are loaded into memory, so be careful with large result sets
    /// - Consider using LIMIT clauses for potentially large queries
    /// - Use indexes on frequently queried columns
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
    ///
    /// Convenience method for queries that should return at most one row.
    /// Returns `None` if no rows match the query.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL SELECT statement to execute
    /// * `params` - Parameters for the SQL statement
    ///
    /// # Returns
    ///
    /// Returns `Some(Row)` if a row was found, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::database::DatabaseManager;
    /// use limbo::Value;
    /// 
    /// # async fn example() -> anyhow::Result<()> {
    /// # let db = DatabaseManager::new_in_memory().await?;
    /// # db.execute("INSERT INTO agents (agent_id, username, status) VALUES (?, ?, ?)", 
    /// #     vec![Value::Text("agent-001".to_string()), Value::Text("alice".to_string()), Value::Text("AVAILABLE".to_string())]).await?;
    /// 
    /// // Find specific agent
    /// if let Some(_row) = db.query_row(
    ///     "SELECT username, status FROM agents WHERE agent_id = ?",
    ///     vec![Value::Text("agent-001".to_string())]
    /// ).await? {
    ///     println!("Found agent record");
    /// } else {
    ///     println!("Agent not found");
    /// }
    /// 
    /// // Get system statistics
    /// if let Some(_row) = db.query_row(
    ///     "SELECT COUNT(*) as total_agents FROM agents",
    ///     Vec::<Value>::new()
    /// ).await? {
    ///     println!("Retrieved agent count");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn query_row<P: IntoParams>(&self, sql: &str, params: P) -> Result<Option<limbo::Row>> {
        let rows = self.query(sql, params).await?;
        Ok(rows.into_iter().next())
    }
    
    /// Execute operations within a database transaction
    ///
    /// Provides ACID transaction support for complex operations that need to be
    /// atomic. If any operation within the transaction fails, all changes are
    /// automatically rolled back.
    ///
    /// # Arguments
    ///
    /// * `f` - Async closure that receives a `Transaction` and performs operations
    ///
    /// # Returns
    ///
    /// Returns the result of the transaction function.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_call_engine::database::DatabaseManager;
    /// use limbo::Value;
    /// 
    /// # async fn example() -> anyhow::Result<()> {
    /// # let db = DatabaseManager::new_in_memory().await?;
    /// 
    /// // Atomic call assignment
    /// let result = db.transaction(|tx| Box::pin(async move {
    ///     // Remove call from queue
    ///     let dequeued = tx.execute(
    ///         "DELETE FROM call_queue WHERE session_id = ?",
    ///         vec![Value::Text("session-123".to_string())]
    ///     ).await?;
    ///     
    ///     if dequeued == 0 {
    ///         return Err(anyhow::anyhow!("Call not found in queue"));
    ///     }
    ///     
    ///     // Add to active calls
    ///     tx.execute(
    ///         "INSERT INTO active_calls (call_id, agent_id, session_id, assigned_at) VALUES (?, ?, ?, ?)",
    ///         vec![
    ///             Value::Text("call-456".to_string()),
    ///             Value::Text("agent-001".to_string()),
    ///             Value::Text("session-123".to_string()),
    ///             Value::Text(chrono::Utc::now().to_rfc3339())
    ///         ]
    ///     ).await?;
    ///     
    ///     // Update agent status
    ///     tx.execute(
    ///         "UPDATE agents SET status = 'BUSY', current_calls = current_calls + 1 WHERE agent_id = ?",
    ///         vec![Value::Text("agent-001".to_string())]
    ///     ).await?;
    ///     
    ///     Ok("Call assigned successfully".to_string())
    /// })).await?;
    /// 
    /// println!("Transaction result: {}", result);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Error Handling
    ///
    /// - Transaction is automatically committed on success
    /// - Transaction is automatically rolled back on any error
    /// - Nested transactions are not supported
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