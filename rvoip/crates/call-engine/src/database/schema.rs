//! Database schema definitions for the call center
//!
//! This module contains all the SQL schema definitions for the call center database.
//! It includes tables for agents, call records, queues, routing policies, and metrics.

use anyhow::Result;
use limbo::Connection;
use tracing::{debug, info, error, warn};
use super::DatabaseManager;

/// **CENTRALIZED DATABASE SCHEMA INITIALIZATION**
/// This is the single source of truth for all database schema creation
pub async fn initialize_call_center_schema(db_manager: &DatabaseManager) -> Result<()> {
    info!("🗄️ Initializing centralized call center database schema");
    
    // Create agents table with correct schema
    db_manager.execute(
        "CREATE TABLE IF NOT EXISTS agents (
            agent_id TEXT PRIMARY KEY,
            username TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'OFFLINE',
            max_calls INTEGER DEFAULT 1,
            current_calls INTEGER DEFAULT 0,
            contact_uri TEXT,
            last_heartbeat DATETIME,
            CHECK (current_calls >= 0),
            CHECK (current_calls <= max_calls),
            CHECK (status IN ('OFFLINE', 'AVAILABLE', 'BUSY', 'RESERVED', 'POSTCALLWRAPUP'))
        )",
        ()
    ).await?;
    
    // Create queues table
    db_manager.execute(
        "CREATE TABLE IF NOT EXISTS queues (
            queue_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            capacity INTEGER DEFAULT 100,
            overflow_queue TEXT,
            priority_boost INTEGER DEFAULT 5,
            CHECK (capacity > 0),
            CHECK (priority_boost >= 0)
        )",
        ()
    ).await?;
    
    // Create call_queue table
    db_manager.execute(
        "CREATE TABLE IF NOT EXISTS call_queue (
            call_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            queue_id TEXT NOT NULL,
            customer_info TEXT,
            priority INTEGER DEFAULT 0,
            enqueued_at DATETIME DEFAULT (datetime('now')),
            attempts INTEGER DEFAULT 0,
            last_attempt DATETIME,
            expires_at DATETIME DEFAULT (datetime('now', '+60 minutes')),
            FOREIGN KEY (queue_id) REFERENCES queues(queue_id)
        )",
        ()
    ).await?;
    
    // Create active_calls table
    db_manager.execute(
        "CREATE TABLE IF NOT EXISTS active_calls (
            call_id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            customer_dialog_id TEXT,
            agent_dialog_id TEXT,
            assigned_at DATETIME DEFAULT (datetime('now')),
            answered_at DATETIME,
            FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
        )",
        ()
    ).await?;
    
    // Create call records table (for historical data)
    db_manager.execute(
        "CREATE TABLE IF NOT EXISTS call_records (
            id TEXT PRIMARY KEY,
            session_id TEXT UNIQUE,
            caller_uri TEXT NOT NULL,
            called_uri TEXT,
            direction TEXT NOT NULL,
            status TEXT NOT NULL,
            start_time TEXT NOT NULL,
            answer_time TEXT,
            end_time TEXT,
            duration_seconds INTEGER,
            assigned_agent_id TEXT,
            queue_id TEXT,
            disconnect_reason TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (assigned_agent_id) REFERENCES agents(agent_id)
        )",
        ()
    ).await?;
    
    // Create agent skills table
    db_manager.execute(
        "CREATE TABLE IF NOT EXISTS agent_skills (
            agent_id TEXT NOT NULL,
            skill_name TEXT NOT NULL,
            skill_level INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            PRIMARY KEY (agent_id, skill_name),
            FOREIGN KEY (agent_id) REFERENCES agents(agent_id) ON DELETE CASCADE
        )",
        ()
    ).await?;
    
    // Create indexes for performance
    create_performance_indexes(db_manager).await?;
    
    // Insert default queues
    create_default_queues(db_manager).await?;
    
    info!("✅ Centralized database schema initialized successfully");
    Ok(())
}

/// Create performance indexes
async fn create_performance_indexes(db_manager: &DatabaseManager) -> Result<()> {
    debug!("📋 Creating performance indexes");
    
    // Agents indexes
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status)",
        ()
    ).await?;
    
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_status_calls 
         ON agents(status, current_calls)",
        ()
    ).await?;
    
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_username ON agents(username)",
        ()
    ).await?;
    
    // Call queue indexes
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_queue_priority 
         ON call_queue(queue_id, priority DESC, enqueued_at)",
        ()
    ).await?;
    
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_queue_expires 
         ON call_queue(expires_at)",
        ()
    ).await?;
    
    // Active calls indexes
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_active_calls_agent 
         ON active_calls(agent_id)",
        ()
    ).await?;
    
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_active_calls_session 
         ON active_calls(session_id)",
        ()
    ).await?;
    
    // Call records indexes
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_records_session_id ON call_records(session_id)",
        ()
    ).await?;
    
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_records_agent_id ON call_records(assigned_agent_id)",
        ()
    ).await?;
    
    db_manager.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_records_start_time ON call_records(start_time)",
        ()
    ).await?;
    
    debug!("✅ Performance indexes created");
    Ok(())
}

/// Create default queues
async fn create_default_queues(db_manager: &DatabaseManager) -> Result<()> {
    debug!("📋 Creating default queues");
    
    let default_queues = vec![
        ("general", "General Support", 100, 0),
        ("support", "Technical Support", 50, 5),
        ("sales", "Sales", 50, 10),
        ("billing", "Billing", 30, 5),
        ("vip", "VIP Support", 20, 20),
        ("premium", "Premium Support", 30, 15),
        ("overflow", "Overflow", 200, 0),
    ];
    
    for (id, name, capacity, priority_boost) in default_queues {
        // Check if queue already exists
        let check_params: Vec<limbo::Value> = vec![id.into()];
        let exists = db_manager.query_row(
            "SELECT 1 FROM queues WHERE queue_id = ?1",
            check_params
        ).await?.is_some();
        
        // Only insert if it doesn't exist
        if !exists {
            let params: Vec<limbo::Value> = vec![
                id.into(),
                name.into(),
                (capacity as i64).into(),
                (priority_boost as i64).into(),
            ];
            
            db_manager.execute(
                "INSERT INTO queues (queue_id, name, capacity, priority_boost) 
                 VALUES (?1, ?2, ?3, ?4)",
                params
            ).await?;
        }
    }
    
    debug!("✅ Default queues created");
    Ok(())
}

// ============================================================================
// DEPRECATED: Legacy functions - kept for compatibility but should not be used
// ============================================================================

/// DEPRECATED: Use initialize_call_center_schema() instead
pub async fn create_agents_table(conn: &Connection) -> Result<()> {
    warn!("create_agents_table() is deprecated. Use initialize_call_center_schema() instead.");
    
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agents (
            agent_id TEXT PRIMARY KEY,
            username TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'OFFLINE',
            max_calls INTEGER DEFAULT 1,
            current_calls INTEGER DEFAULT 0,
            contact_uri TEXT,
            last_heartbeat DATETIME,
            CHECK (current_calls >= 0),
            CHECK (current_calls <= max_calls),
            CHECK (status IN ('OFFLINE', 'AVAILABLE', 'BUSY', 'RESERVED'))
        )
        "#,
        (),
    ).await?;
    
    Ok(())
}

/// DEPRECATED: Use initialize_call_center_schema() instead
pub async fn create_agent_skills_table(conn: &Connection) -> Result<()> {
    warn!("create_agent_skills_table() is deprecated. Use initialize_call_center_schema() instead.");
    
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agent_skills (
            agent_id TEXT NOT NULL,
            skill_name TEXT NOT NULL,
            skill_level INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            PRIMARY KEY (agent_id, skill_name),
            FOREIGN KEY (agent_id) REFERENCES agents(agent_id) ON DELETE CASCADE
        )
        "#,
        (),
    ).await?;
    
    Ok(())
}

/// DEPRECATED: Use initialize_call_center_schema() instead
pub async fn create_call_records_table(conn: &Connection) -> Result<()> {
    warn!("create_call_records_table() is deprecated. Use initialize_call_center_schema() instead.");
    
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS call_records (
            id TEXT PRIMARY KEY,
            session_id TEXT UNIQUE,
            caller_uri TEXT NOT NULL,
            called_uri TEXT,
            direction TEXT NOT NULL,
            status TEXT NOT NULL,
            start_time TEXT NOT NULL,
            answer_time TEXT,
            end_time TEXT,
            duration_seconds INTEGER,
            assigned_agent_id TEXT,
            queue_id TEXT,
            disconnect_reason TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (assigned_agent_id) REFERENCES agents(agent_id)
        )
        "#,
        (),
    ).await?;
    
    Ok(())
}

/// DEPRECATED: Use initialize_call_center_schema() instead
pub async fn create_call_queues_table(conn: &Connection) -> Result<()> {
    warn!("create_call_queues_table() is deprecated. Use initialize_call_center_schema() instead.");
    
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS call_queues (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            description TEXT,
            max_wait_time_seconds INTEGER DEFAULT 300,
            max_queue_size INTEGER DEFAULT 100,
            priority INTEGER NOT NULL DEFAULT 1,
            routing_strategy TEXT NOT NULL DEFAULT 'round_robin',
            overflow_queue_id TEXT,
            overflow_action TEXT DEFAULT 'voicemail',
            is_active BOOLEAN NOT NULL DEFAULT true,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (overflow_queue_id) REFERENCES call_queues(id)
        )
        "#,
        (),
    ).await?;
    
    Ok(())
}

/// DEPRECATED: Use initialize_call_center_schema() instead
pub async fn create_routing_policies_table(conn: &Connection) -> Result<()> {
    warn!("create_routing_policies_table() is deprecated. Use initialize_call_center_schema() instead.");
    
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS routing_policies (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            policy_type TEXT NOT NULL,
            conditions TEXT NOT NULL,
            actions TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 1,
            is_active BOOLEAN NOT NULL DEFAULT true,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
        (),
    ).await?;
    
    Ok(())
}

/// DEPRECATED: Use initialize_call_center_schema() instead
pub async fn create_call_metrics_table(conn: &Connection) -> Result<()> {
    warn!("create_call_metrics_table() is deprecated. Use initialize_call_center_schema() instead.");
    
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS call_metrics (
            id TEXT PRIMARY KEY,
            call_record_id TEXT NOT NULL,
            metric_name TEXT NOT NULL,
            metric_value REAL NOT NULL,
            metric_unit TEXT,
            timestamp TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (call_record_id) REFERENCES call_records(id) ON DELETE CASCADE
        )
        "#,
        (),
    ).await?;
    
    Ok(())
}

/// DEPRECATED: Use create_performance_indexes() instead
pub async fn create_indexes(conn: &Connection) -> Result<()> {
    warn!("create_indexes() is deprecated. Use initialize_call_center_schema() instead.");
    
    // Just call the basic ones to avoid breaking existing code
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_username ON agents(username)",
        (),
    ).await?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status)",
        (),
    ).await?;
    
    Ok(())
}

impl DatabaseManager {
    /// **NEW CENTRALIZED SCHEMA INITIALIZATION**
    /// This replaces the old initialize_schema() method
    pub async fn initialize_schema(&self) -> Result<()> {
        // Use the centralized schema initialization
        initialize_call_center_schema(self).await
    }
    
    /// Clean up test data
    pub async fn cleanup(&self) -> Result<()> {
        self.execute("DROP TABLE IF EXISTS active_calls", ()).await?;
        self.execute("DROP TABLE IF EXISTS call_queue", ()).await?;
        self.execute("DROP TABLE IF EXISTS agent_skills", ()).await?;
        self.execute("DROP TABLE IF EXISTS call_records", ()).await?;
        self.execute("DROP TABLE IF EXISTS agents", ()).await?;
        self.execute("DROP TABLE IF EXISTS queues", ()).await?;
        Ok(())
    }
} 