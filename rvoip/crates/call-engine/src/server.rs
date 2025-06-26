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
    
    /// Optional handle to the queue processor task
    queue_processor_handle: Option<JoinHandle<()>>,
}

impl CallCenterServer {
    /// Create a new CallCenterServer with the given configuration and database path
    pub async fn new(
        config: CallCenterConfig, 
        db_path: Option<String>
    ) -> Result<Self> {
        info!("🚀 Creating CallCenterEngine with session-core CallHandler integration");
        
        // Create the engine
        let engine = CallCenterEngine::new(config.clone(), db_path).await?;
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
            queue_processor_handle: None,
        })
    }
    
    /// Create a new CallCenterServer with an in-memory database
    pub async fn new_in_memory(config: CallCenterConfig) -> Result<Self> {
        Self::new(config, None).await
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
        
        // Start queue processor
        let engine = self.engine.clone();
        let queue_handle = tokio::spawn(async move {
            Self::queue_processor_loop(engine).await;
        });
        
        self.queue_processor_handle = Some(queue_handle);
        info!("✅ Started queue processor for automatic call distribution");
        
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
        
        // Cancel queue processor task
        if let Some(handle) = self.queue_processor_handle.take() {
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
            
            // PHASE 0.10: Enhanced queue monitoring with detailed stats
            info!("📊 === Call Center Status Update ===");
            
            // Get current queue stats
            match supervisor_api.get_all_queue_stats().await {
                Ok(queue_stats) => {
                    let total_queued: usize = queue_stats.iter().map(|(_, s)| s.total_calls).sum();
                    info!("📥 Total calls in all queues: {}", total_queued);
                    
                    for (queue_id, stats) in queue_stats {
                        if stats.total_calls > 0 || queue_id == "general" || queue_id == "support" {
                            info!("  📋 Queue '{}': {} waiting, avg wait: {}s", 
                                  queue_id, stats.total_calls, 
                                  stats.average_wait_time_seconds);
                        }
                    }
                }
                Err(e) => error!("Failed to get queue stats: {}", e),
            }
            
            // Get agent status with detailed breakdown
            let agents = supervisor_api.list_agents().await;
            let available = agents.iter().filter(|a| matches!(a.status, AgentStatus::Available)).count();
            let busy = agents.iter().filter(|a| matches!(a.status, AgentStatus::Busy(..))).count();
            let offline = agents.iter().filter(|a| matches!(a.status, AgentStatus::Offline)).count();
            
            info!("👥 Agent Status Summary:");
            info!("  ✅ Available: {}", available);
            info!("  🔴 Busy: {}", busy);
            info!("  ⚫ Offline: {}", offline);
            info!("  📋 Total: {}", agents.len());
            
            // PHASE 0.10: Show individual agent status for debugging
            if agents.len() > 0 && agents.len() <= 5 {  // Only show individual status for small teams
                info!("👤 Individual Agent Status:");
                for agent in &agents {
                    let status_str = match &agent.status {
                        AgentStatus::Available => "Available ✅".to_string(),
                        AgentStatus::Busy(calls) => format!("Busy ({} calls) 🔴", calls.len()),
                        AgentStatus::Offline => "Offline ⚫".to_string(),
                    };
                    info!("  - {} ({}): {}", agent.sip_uri, agent.agent_id, status_str);
                }
            }
            
            // Get overall stats
            let stats = supervisor_api.get_stats().await;
            info!("📞 Active bridges: {}", stats.active_bridges);
            info!("================================");
        }
    }
    
    /// Internal queue processor loop - assigns waiting calls to available agents
    async fn queue_processor_loop(engine: Arc<CallCenterEngine>) {
        info!("🔄 Starting queue processor for automatic call distribution");
        
        let mut interval = interval(Duration::from_millis(100)); // Check every 100ms
        
        loop {
            interval.tick().await;
            
            // Process all queues
            if let Err(e) = engine.process_all_queues().await {
                error!("Error processing queues: {}", e);
            }
        }
    }
    
    /// Helper to create test agents (for examples/testing)
    pub async fn create_test_agents(&self, agents: Vec<(&str, &str, &str)>) -> Result<()> {
        for (username, name, department) in agents {
            let agent = Agent {
                id: username.to_string(),  // Use just the username as ID
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
        // Create queues that match the names expected by the routing logic
        let queues = vec![
            ("general", "General Support"),
            ("support", "Technical Support"),
            ("sales", "Sales"),
            ("billing", "Billing"),
            ("vip", "VIP Support"),
            ("premium", "Premium Support"),
        ];
        
        for (queue_id, queue_name) in queues {
            self.admin_api.create_queue(queue_id).await
                .map_err(|e| CallCenterError::Configuration(
                    format!("Failed to create {} queue: {}", queue_name, e)
                ))?;
            info!("✅ Created queue: {} ({})", queue_id, queue_name);
        }
        
        info!("✅ Default queues created");
        
        Ok(())
    }
}

/// Builder for CallCenterServer with fluent API
pub struct CallCenterServerBuilder {
    config: Option<CallCenterConfig>,
    db_path: Option<String>,
}

impl CallCenterServerBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            config: None,
            db_path: None,
        }
    }
    
    /// Set the configuration
    pub fn with_config(mut self, config: CallCenterConfig) -> Self {
        self.config = Some(config);
        self
    }
    
    /// Set the database path
    pub fn with_database_path(mut self, path: String) -> Self {
        self.db_path = Some(path);
        self
    }
    
    /// Use an in-memory database
    pub fn with_in_memory_database(mut self) -> Self {
        self.db_path = None;
        self
    }
    
    /// Build the server
    pub async fn build(self) -> Result<CallCenterServer> {
        let config = self.config.ok_or_else(|| CallCenterError::Configuration(
            "Configuration not provided".to_string()
        ))?;
        
        CallCenterServer::new(config, self.db_path).await
    }
}

impl Default for CallCenterServerBuilder {
    fn default() -> Self {
        Self::new()
    }
} 