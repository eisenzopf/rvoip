pub mod schema;
pub mod agent_store;
pub mod call_records;
pub mod queue_store;
pub mod routing_store;

use std::sync::Arc;
use anyhow::Result;
use limbo::{Builder, Database, Connection};
use tokio::sync::RwLock;
use tracing::{info, debug, warn, error};

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
        debug!("📋 Creating call center database schema");
        
        let conn = self.connection.read().await;
        
        // Create all tables using the correct Limbo execute() method for DDL
        schema::create_agents_table(&*conn).await?;
        schema::create_call_records_table(&*conn).await?;
        schema::create_call_queues_table(&*conn).await?;
        schema::create_routing_policies_table(&*conn).await?;
        schema::create_agent_skills_table(&*conn).await?;
        schema::create_call_metrics_table(&*conn).await?;
        
        // Create indexes for performance
        schema::create_indexes(&*conn).await?;
        
        debug!("✅ Database schema created successfully");
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