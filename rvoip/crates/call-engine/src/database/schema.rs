//! Database schema definitions for the call center
//!
//! This module contains all the SQL schema definitions for the call center database.
//! It includes tables for agents, call records, queues, routing policies, and metrics.

use anyhow::Result;
use limbo::Connection;
use tracing::{debug, info};

/// Create the agents table
pub async fn create_agents_table(conn: &Connection) -> Result<()> {
    debug!("📋 Creating agents table");
    
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agents (
            id TEXT PRIMARY KEY,
            sip_uri TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'offline',
            max_concurrent_calls INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            last_seen_at TEXT,
            department TEXT,
            extension TEXT,
            phone_number TEXT
        )
        "#,
        (),
    ).await?;
    
    debug!("✅ Agents table created");
    Ok(())
}

/// Create the agent skills table
pub async fn create_agent_skills_table(conn: &Connection) -> Result<()> {
    debug!("📋 Creating agent_skills table");
    
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS agent_skills (
            agent_id TEXT NOT NULL,
            skill_name TEXT NOT NULL,
            skill_level INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            PRIMARY KEY (agent_id, skill_name),
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        )
        "#,
        (),
    ).await?;
    
    debug!("✅ Agent skills table created");
    Ok(())
}

/// Create the call records table
pub async fn create_call_records_table(conn: &Connection) -> Result<()> {
    debug!("📋 Creating call_records table");
    
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
            FOREIGN KEY (assigned_agent_id) REFERENCES agents(id)
        )
        "#,
        (),
    ).await?;
    
    debug!("✅ Call records table created");
    Ok(())
}

/// Create the call queues table
pub async fn create_call_queues_table(conn: &Connection) -> Result<()> {
    debug!("📋 Creating call_queues table");
    
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
    
    debug!("✅ Call queues table created");
    Ok(())
}

/// Create the routing policies table
pub async fn create_routing_policies_table(conn: &Connection) -> Result<()> {
    debug!("📋 Creating routing_policies table");
    
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
    
    debug!("✅ Routing policies table created");
    Ok(())
}

/// Create the call metrics table for analytics
pub async fn create_call_metrics_table(conn: &Connection) -> Result<()> {
    debug!("📋 Creating call_metrics table");
    
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
    
    debug!("✅ Call metrics table created");
    Ok(())
}

/// Create indexes for better query performance
pub async fn create_indexes(conn: &Connection) -> Result<()> {
    debug!("📋 Creating database indexes");
    
    // Indexes for agents table
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_sip_uri ON agents(sip_uri)",
        (),
    ).await?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status)",
        (),
    ).await?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agents_department ON agents(department)",
        (),
    ).await?;
    
    // Indexes for call records table
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_records_session_id ON call_records(session_id)",
        (),
    ).await?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_records_caller_uri ON call_records(caller_uri)",
        (),
    ).await?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_records_status ON call_records(status)",
        (),
    ).await?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_records_start_time ON call_records(start_time)",
        (),
    ).await?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_records_agent_id ON call_records(assigned_agent_id)",
        (),
    ).await?;
    
    // Indexes for agent skills table
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agent_skills_skill_name ON agent_skills(skill_name)",
        (),
    ).await?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_agent_skills_skill_level ON agent_skills(skill_level)",
        (),
    ).await?;
    
    // Indexes for call metrics table
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_metrics_call_record_id ON call_metrics(call_record_id)",
        (),
    ).await?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_call_metrics_timestamp ON call_metrics(timestamp)",
        (),
    ).await?;
    
    debug!("✅ Database indexes created");
    Ok(())
} 