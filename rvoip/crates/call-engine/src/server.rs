//! Call Center Server Manager
//!
//! Provides a high-level server struct that manages the lifecycle of the call center engine
//! and provides a clean API for server operations.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, interval};
use tokio::task::JoinHandle;
use tracing::{info, error};

use crate::{
    prelude::*,
    api::{AdminApi, SupervisorApi, CallCenterClient},
    config::CallCenterConfig,
    database::CallCenterDatabase,
    orchestrator::CallCenterEngine,
    error::CallCenterError,
    agent::{AgentStatus, Agent},
};

/// A complete call center server that manages engine lifecycle and provides APIs
pub struct CallCenterServer {
    /// The core call center engine
    engine: Arc<CallCenterEngine>,
    
    /// Admin API for system administration
    admin_api: AdminApi,
    
    /// Supervisor API for monitoring and management
    supervisor_api: SupervisorApi,
    
    /// Server configuration
    config: CallCenterConfig,
    
    /// Optional handle to the monitoring task
    monitor_handle: Option<JoinHandle<()>>,
}

impl CallCenterServer {
    /// Create a new CallCenterServer with the given configuration and database
    pub async fn new(
        config: CallCenterConfig, 
        database: CallCenterDatabase
    ) -> Result<Self> {
        info!("🚀 Creating CallCenterEngine with session-core CallHandler integration");
        
        // Create the engine
        let engine = CallCenterEngine::new(config.clone(), database).await?;
        info!("✅ Call center engine initialized with session-core integration");
        
        // Create APIs
        let admin_api = AdminApi::new(engine.clone());
        let supervisor_api = SupervisorApi::new(engine.clone());
        
        Ok(Self {
            engine,
            admin_api,
            supervisor_api,
            config,
            monitor_handle: None,
        })
    }
    
    /// Create a new CallCenterServer with an in-memory database
    pub async fn new_in_memory(config: CallCenterConfig) -> Result<Self> {
        let database = CallCenterDatabase::new_in_memory().await
            .map_err(|e| CallCenterError::Configuration(
                format!("Failed to create in-memory database: {}", e)
            ))?;
        Self::new(config, database).await
    }
    
    /// Start the server and begin accepting calls
    pub async fn start(&mut self) -> Result<()> {
        info!("✅ Call center engine started on {}", self.config.general.local_signaling_addr);
        
        // Start event monitoring
        self.engine.clone().start_event_monitoring().await?;
        info!("✅ Started monitoring for REGISTER and other events");
        
        // Start periodic monitoring
        let supervisor_api = self.supervisor_api.clone();
        let handle = tokio::spawn(async move {
            Self::monitor_loop(supervisor_api).await;
        });
        
        self.monitor_handle = Some(handle);
        
        Ok(())
    }
    
    /// Stop the server gracefully
    pub async fn stop(&mut self) -> Result<()> {
        info!("🛑 Stopping call center server...");
        
        // Cancel monitoring task
        if let Some(handle) = self.monitor_handle.take() {
            handle.abort();
            let _ = handle.await;
        }
        
        // TODO: Add graceful shutdown for engine
        // - Stop accepting new calls
        // - Wait for existing calls to complete
        // - Clean up resources
        
        info!("✅ Call center server stopped");
        Ok(())
    }
    
    /// Run the server indefinitely
    pub async fn run(&self) -> Result<()> {
        info!("📞 Call center server is running");
        
        // Display configuration
        self.display_info();
        
        // Keep the server running
        loop {
            sleep(Duration::from_secs(60)).await;
            
            // Periodically display stats
            let stats = self.supervisor_api.get_stats().await;
            info!("📊 Stats - Active Calls: {}, Queued: {}, Agents Available: {}", 
                  stats.active_calls, stats.queued_calls, stats.available_agents);
        }
    }
    
    /// Get a reference to the admin API
    pub fn admin_api(&self) -> &AdminApi {
        &self.admin_api
    }
    
    /// Get a reference to the supervisor API  
    pub fn supervisor_api(&self) -> &SupervisorApi {
        &self.supervisor_api
    }
    
    /// Get a reference to the engine (for advanced usage)
    pub fn engine(&self) -> &Arc<CallCenterEngine> {
        &self.engine
    }
    
    /// Create a new client API for agent applications
    pub fn create_client(&self, agent_id: String) -> CallCenterClient {
        CallCenterClient::new(self.engine.clone())
    }
    
    /// Display server information
    fn display_info(&self) {
        println!("\n📞 CALL CENTER IS READY!");
        println!("=======================");
        println!("\n🔧 Configuration:");
        println!("  - SIP Address: {}", self.config.general.local_signaling_addr);
        println!("  - Domain: {}", self.config.general.domain);
        println!("\n📋 How to Test:");
        println!("  1. Configure agent SIP phones to register");
        println!("  2. Point them to this server ({})", self.config.general.local_signaling_addr);
        println!("  3. Once registered, they'll show as 'available'");
        println!("  4. Make test calls to configured queues");
        println!("  5. Calls will be routed to available agents");
        println!("\n🛑 Press Ctrl+C to stop the server\n");
    }
    
    /// Internal monitoring loop
    async fn monitor_loop(supervisor_api: SupervisorApi) {
        info!("👀 Starting event monitor");
        
        let mut interval = interval(Duration::from_secs(10));
        
        loop {
            interval.tick().await;
            
            // Get current queue stats
            match supervisor_api.get_all_queue_stats().await {
                Ok(queue_stats) => {
                    for (queue_id, stats) in queue_stats {
                        if stats.total_calls > 0 {
                            info!("📊 Queue '{}' - Waiting: {}, Avg Wait: {}s", 
                                  queue_id, stats.total_calls, stats.average_wait_time_seconds);
                        }
                    }
                }
                Err(e) => error!("Failed to get queue stats: {}", e),
            }
            
            // Get agent status
            let agents = supervisor_api.list_agents().await;
            let available = agents.iter().filter(|a| matches!(a.status, AgentStatus::Available)).count();
            let busy = agents.iter().filter(|a| matches!(a.status, AgentStatus::Busy { .. })).count();
            
            if available > 0 || busy > 0 {
                info!("👥 Agents - Available: {}, Busy: {}", available, busy);
            }
        }
    }
    
    /// Helper to create test agents (for examples/testing)
    pub async fn create_test_agents(&self, agents: Vec<(&str, &str, &str)>) -> Result<()> {
        for (username, name, department) in agents {
            let agent = Agent {
                id: crate::agent::AgentId::from(format!("agent_{}", username)),
                sip_uri: format!("sip:{}@{}", username, self.config.general.domain),
                display_name: name.to_string(),
                skills: vec!["english".to_string(), department.to_string()],
                max_concurrent_calls: 1,
                status: AgentStatus::Offline,
                department: Some(department.to_string()),
                extension: None,
            };

            self.admin_api.add_agent(agent.clone()).await
                .map_err(|e| CallCenterError::Configuration(
                    format!("Failed to add agent {}: {}", name, e)
                ))?;
            info!("Created agent: {} ({})", name, agent.sip_uri);
        }
        
        Ok(())
    }
    
    /// Helper to create default queues (for examples/testing)
    pub async fn create_default_queues(&self) -> Result<()> {
        self.admin_api.create_queue("support_queue").await
            .map_err(|e| CallCenterError::Configuration(
                format!("Failed to create support queue: {}", e)
            ))?;
        self.admin_api.create_queue("sales_queue").await
            .map_err(|e| CallCenterError::Configuration(
                format!("Failed to create sales queue: {}", e)
            ))?;
        info!("✅ Default queues created");
        
        Ok(())
    }
}

/// Builder for CallCenterServer with fluent API
pub struct CallCenterServerBuilder {
    config: Option<CallCenterConfig>,
    database: Option<CallCenterDatabase>,
}

impl CallCenterServerBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            config: None,
            database: None,
        }
    }
    
    /// Set the configuration
    pub fn with_config(mut self, config: CallCenterConfig) -> Self {
        self.config = Some(config);
        self
    }
    
    /// Set the database
    pub fn with_database(mut self, database: CallCenterDatabase) -> Self {
        self.database = Some(database);
        self
    }
    
    /// Use an in-memory database
    pub async fn with_in_memory_database(mut self) -> Result<Self> {
        self.database = Some(CallCenterDatabase::new_in_memory().await
            .map_err(|e| CallCenterError::Configuration(
                format!("Failed to create in-memory database: {}", e)
            ))?);
        Ok(self)
    }
    
    /// Build the server
    pub async fn build(self) -> Result<CallCenterServer> {
        let config = self.config.ok_or_else(|| CallCenterError::Configuration(
            "Configuration not provided".to_string()
        ))?;
        
        let database = self.database.ok_or_else(|| CallCenterError::Configuration(
            "Database not provided".to_string()
        ))?;
        
        CallCenterServer::new(config, database).await
    }
}

impl Default for CallCenterServerBuilder {
    fn default() -> Self {
        Self::new()
    }
} 