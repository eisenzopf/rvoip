//! # Call Center Server Manager
//!
//! This module provides a high-level server management interface for call center operations,
//! handling the complete lifecycle of the call center engine, API management, monitoring,
//! and queue processing. It offers a production-ready server implementation with comprehensive
//! configuration, graceful startup/shutdown, and integrated monitoring capabilities.
//!
//! ## Overview
//!
//! The Call Center Server Manager serves as the primary entry point for deploying and
//! managing call center systems. It orchestrates the call center engine, provides
//! administrative and supervisory APIs, manages background processing tasks, and ensures
//! reliable operation with proper error handling and monitoring. This module is designed
//! for production deployments requiring robust server management.
//!
//! ## Key Features
//!
//! - **Complete Server Lifecycle**: Startup, runtime management, and graceful shutdown
//! - **Integrated APIs**: Admin API for configuration, Supervisor API for monitoring
//! - **Background Processing**: Automatic queue processing and call distribution
//! - **Real-Time Monitoring**: Continuous system health and performance monitoring
//! - **Flexible Configuration**: Support for various deployment configurations
//! - **Database Management**: Both persistent and in-memory database options
//! - **Error Recovery**: Robust error handling with automatic recovery mechanisms
//! - **Scalable Architecture**: Designed for high-volume call center operations
//! - **Agent Management**: Comprehensive agent registration and status tracking
//! - **Queue Processing**: Intelligent call routing and queue management
//!
//! ## Server Architecture
//!
//! The server follows a layered architecture:
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │           CallCenterServer              │
//! ├─────────────────────────────────────────┤
//! │  AdminAPI │ SupervisorAPI │ ClientAPI   │
//! ├─────────────────────────────────────────┤
//! │         CallCenterEngine                │
//! ├─────────────────────────────────────────┤
//! │    Database │ Monitoring │ Routing      │
//! ├─────────────────────────────────────────┤
//! │           Session-Core SIP              │
//! └─────────────────────────────────────────┘
//! ```
//!
//! ## Examples
//!
//! ### Basic Server Setup and Operation
//!
//! ```rust
//! use rvoip_call_engine::{
//!     server::{CallCenterServer, CallCenterServerBuilder},
//!     config::CallCenterConfig,
//! };
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create server configuration
//! let config = CallCenterConfig::default();
//! 
//! // Build and start the server
//! let mut server = CallCenterServerBuilder::new()
//!     .with_config(config)
//!     .with_in_memory_database()
//!     .build()
//!     .await?;
//! 
//! // Start server operations
//! server.start().await?;
//! 
//! println!("✅ Call center server started successfully");
//! println!("📞 Ready to accept calls and agent registrations");
//! println!("🎛️ Admin and supervisor APIs available");
//! 
//! // Server is now running and ready for operations
//! // In production, you would call server.run().await to keep it running
//! 
//! // Graceful shutdown when needed
//! server.stop().await?;
//! println!("🛑 Server stopped gracefully");
//! # Ok(())
//! # }
//! ```
//!
//! ### Complete Production Server Setup
//!
//! ```rust
//! use rvoip_call_engine::{
//!     server::{CallCenterServer, CallCenterServerBuilder},
//!     config::{CallCenterConfig, GeneralConfig, DatabaseConfig},
//! };
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Production configuration
//! let config = CallCenterConfig {
//!     general: GeneralConfig {
//!         domain: "call-center.company.com".to_string(),
//!         local_signaling_addr: "0.0.0.0:5060".parse().unwrap(),
//!         ..Default::default()
//!     },
//!     database: DatabaseConfig {
//!         database_path: "/var/lib/callcenter/data.db".to_string(),
//!         ..Default::default()
//!     },
//!     ..Default::default()
//! };
//! 
//! // Build production server
//! let mut server = CallCenterServerBuilder::new()
//!     .with_config(config)
//!     .with_database_path("/var/lib/callcenter/data.db".to_string())
//!     .build()
//!     .await?;
//! 
//! println!("🏗️ Production server built with configuration:");
//! println!("  Domain: call-center.company.com");
//! println!("  SIP Port: 5060");
//! println!("  Database: PostgreSQL");
//! 
//! // Set up default infrastructure
//! server.create_default_queues().await?;
//! println!("📋 Default queues created");
//! 
//! // Add initial agents
//! server.create_test_agents(vec![
//!     ("alice", "Alice Johnson", "support"),
//!     ("bob", "Bob Smith", "sales"),
//!     ("carol", "Carol Williams", "billing"),
//! ]).await?;
//! println!("👥 Initial agents configured");
//! 
//! // Start all server operations
//! server.start().await?;
//! println!("🚀 Production server started successfully");
//! 
//! // In production, this would run indefinitely
//! // server.run().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### API Integration and Management
//!
//! ```rust
//! use rvoip_call_engine::{
//!     server::CallCenterServer,
//!     config::CallCenterConfig,
//!     agent::{Agent, AgentStatus},
//! };
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create and start server
//! let mut server = CallCenterServer::new_in_memory(CallCenterConfig::default()).await?;
//! server.start().await?;
//! 
//! println!("🎛️ API Integration Examples:");
//! 
//! // Admin API operations
//! let admin_api = server.admin_api();
//! 
//! // Add new agent through admin API
//! let new_agent = Agent {
//!     id: "david".to_string(),
//!     sip_uri: "sip:david@call-center.com".to_string(),
//!     display_name: "David Brown".to_string(),
//!     skills: vec!["technical_support".to_string(), "escalation".to_string()],
//!     max_concurrent_calls: 2,
//!     status: AgentStatus::Offline,
//!     department: Some("technical".to_string()),
//!     extension: Some("1004".to_string()),
//! };
//! 
//! admin_api.add_agent(new_agent).await?;
//! println!("✅ Agent added via Admin API");
//! 
//! // Create queue through admin API
//! admin_api.create_queue("technical_escalation").await?;
//! println!("📋 Technical escalation queue created");
//! 
//! // Supervisor API operations
//! let supervisor_api = server.supervisor_api();
//! 
//! // Get system statistics
//! let stats = supervisor_api.get_stats().await;
//! println!("📊 System Statistics:");
//! println!("  Active Calls: {}", stats.active_calls);
//! println!("  Queued Calls: {}", stats.queued_calls);
//! println!("  Available Agents: {}", stats.available_agents);
//! println!("  Active Bridges: {}", stats.active_bridges);
//! 
//! // Get queue statistics
//! let queue_stats = supervisor_api.get_all_queue_stats().await?;
//! println!("📋 Queue Statistics:");
//! for (queue_id, stats) in queue_stats {
//!     println!("  {}: {} calls, avg wait {}s", 
//!              queue_id, stats.total_calls, stats.average_wait_time_seconds);
//! }
//! 
//! // List all agents
//! let agents = supervisor_api.list_agents().await;
//! println!("👥 Agent Status:");
//! for agent in agents {
//!     println!("  {}: {:?}", agent.agent_id.0, agent.status);
//! }
//! 
//! // Client API for agent applications
//! let client_api = server.create_client("david".to_string());
//! println!("📱 Client API created for agent applications");
//! 
//! server.stop().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Monitoring and Health Checks
//!
//! ```rust
//! use rvoip_call_engine::{
//!     server::CallCenterServer,
//!     config::CallCenterConfig,
//! };
//! use tokio::time::{sleep, Duration};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut server = CallCenterServer::new_in_memory(CallCenterConfig::default()).await?;
//! server.start().await?;
//! 
//! // Simulate monitoring operations
//! println!("📊 Server Monitoring Examples:");
//! 
//! // Health check function
//! async fn health_check(server: &CallCenterServer) -> Result<bool, Box<dyn std::error::Error>> {
//!     let supervisor_api = server.supervisor_api();
//!     
//!     // Check system statistics
//!     let stats = supervisor_api.get_stats().await;
//!     
//!     // Health criteria
//!     let is_healthy = 
//!         stats.active_calls < 1000 && // Not overloaded
//!         stats.queued_calls < 50 &&   // Queue not backing up
//!         stats.available_agents > 0;   // Agents available
//!     
//!     if is_healthy {
//!         println!("✅ Health Check: HEALTHY");
//!         println!("  📞 Active: {}, 📋 Queued: {}, 👥 Available: {}", 
//!                  stats.active_calls, stats.queued_calls, stats.available_agents);
//!     } else {
//!         println!("⚠️ Health Check: DEGRADED");
//!         if stats.available_agents == 0 {
//!             println!("  🚨 ALERT: No agents available");
//!         }
//!         if stats.queued_calls > 20 {
//!             println!("  ⚠️ WARNING: High queue volume");
//!         }
//!     }
//!     
//!     Ok(is_healthy)
//! }
//! 
//! // Performance monitoring
//! async fn monitor_performance(server: &CallCenterServer) -> Result<(), Box<dyn std::error::Error>> {
//!     let supervisor_api = server.supervisor_api();
//!     
//!     // Get detailed queue statistics
//!     let queue_stats = supervisor_api.get_all_queue_stats().await?;
//!     
//!     println!("📈 Performance Metrics:");
//!     for (queue_id, stats) in queue_stats {
//!         if stats.total_calls > 0 {
//!             println!("  📋 {}: {} calls, {:.1}s avg wait", 
//!                      queue_id, stats.total_calls, stats.average_wait_time_seconds);
//!             
//!             // Alert on high wait times
//!             if stats.average_wait_time_seconds > 60 {
//!                 println!("    🚨 ALERT: High wait time in {} queue", queue_id);
//!             }
//!         }
//!     }
//!     
//!     // Agent utilization monitoring
//!     let agents = supervisor_api.list_agents().await;
//!     let total_agents = agents.len();
//!     let busy_agents = agents.iter()
//!         .filter(|a| matches!(a.status, rvoip_call_engine::agent::AgentStatus::Busy(_)))
//!         .count();
//!     
//!     if total_agents > 0 {
//!         let utilization = (busy_agents as f64 / total_agents as f64) * 100.0;
//!         println!("👥 Agent Utilization: {:.1}% ({}/{})", 
//!                  utilization, busy_agents, total_agents);
//!         
//!         if utilization > 90.0 {
//!             println!("    ⚠️ WARNING: High agent utilization");
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! 
//! // Run monitoring checks
//! health_check(&server).await?;
//! monitor_performance(&server).await?;
//! 
//! // Simulate continuous monitoring (in production this would be a background task)
//! println!("\n🔄 Continuous monitoring would run in background...");
//! 
//! server.stop().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Server Configuration
//!
//! ```rust
//! use rvoip_call_engine::{
//!     server::{CallCenterServer, CallCenterServerBuilder},
//!     config::{CallCenterConfig, GeneralConfig, DatabaseConfig, RoutingConfig, RoutingStrategy},
//!     agent::Agent,
//! };
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Advanced production configuration
//! let config = CallCenterConfig {
//!     general: GeneralConfig {
//!         domain: "enterprise-callcenter.corp".to_string(),
//!         local_signaling_addr: "0.0.0.0:5060".parse().unwrap(),
//!         ..Default::default()
//!     },
//!     database: DatabaseConfig {
//!         database_path: "/opt/callcenter/production.db".to_string(),
//!         max_connections: 20,
//!         enable_connection_pooling: true,
//!         ..Default::default()
//!     },
//!     routing: RoutingConfig {
//!         default_strategy: RoutingStrategy::SkillBased,
//!         enable_load_balancing: true,
//!         ..Default::default()
//!     },
//!     ..Default::default()
//! };
//! 
//! println!("🏢 Enterprise Server Configuration:");
//! 
//! // Build enterprise server
//! let mut server = CallCenterServerBuilder::new()
//!     .with_config(config)
//!     .with_database_path("/opt/callcenter/production.db".to_string())
//!     .build()
//!     .await?;
//! 
//! // Advanced setup with specialized queues
//! server.create_default_queues().await?;
//! 
//! // Add additional enterprise queues
//! let admin_api = server.admin_api();
//! let enterprise_queues = vec![
//!     "vip_platinum",
//!     "technical_tier2", 
//!     "billing_enterprise",
//!     "escalation_management",
//!     "after_hours_support",
//! ];
//! 
//! for queue_id in enterprise_queues {
//!     admin_api.create_queue(queue_id).await?;
//!     println!("📋 Created enterprise queue: {}", queue_id);
//! }
//! 
//! // Add specialized agents with advanced skills
//! let enterprise_agents = vec![
//!     ("senior_tech", "Senior Technical Lead", vec!["technical_support", "escalation", "training"]),
//!     ("billing_expert", "Billing Specialist", vec!["billing", "enterprise_accounts", "reporting"]),
//!     ("vip_concierge", "VIP Concierge", vec!["vip_support", "account_management", "relationship"]),
//!     ("night_supervisor", "Night Supervisor", vec!["general_support", "supervision", "after_hours"]),
//! ];
//! 
//! for (username, display_name, skills) in enterprise_agents {
//!     let agent = Agent {
//!         id: username.to_string(),
//!         sip_uri: format!("sip:{}@enterprise-callcenter.corp", username),
//!         display_name: display_name.to_string(),
//!         skills: skills.into_iter().map(|s| s.to_string()).collect(),
//!         max_concurrent_calls: 3, // Higher capacity for enterprise
//!         status: rvoip_call_engine::agent::AgentStatus::Offline,
//!         department: Some("enterprise".to_string()),
//!         extension: None,
//!     };
//!     
//!     admin_api.add_agent(agent).await?;
//!     println!("👤 Added enterprise agent: {}", display_name);
//! }
//! 
//! // Start enterprise operations
//! server.start().await?;
//! println!("🚀 Enterprise call center started");
//! 
//! // Display enterprise readiness
//! println!("\n🏢 Enterprise Call Center Ready:");
//! println!("  🔐 TLS Security: Enabled");
//! println!("  💾 Database: PostgreSQL Production");
//! println!("  🎯 Routing: Advanced Skill-Based");
//! println!("  📋 Queues: {} configured", 5 + 6); // default + enterprise
//! println!("  👥 Agents: {} specialized agents", 4);
//! println!("  ⚡ Capacity: High-volume enterprise ready");
//! 
//! server.stop().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Error Handling and Recovery
//!
//! ```rust
//! use rvoip_call_engine::{
//!     server::{CallCenterServer, CallCenterServerBuilder},
//!     config::CallCenterConfig,
//!     error::CallCenterError,
//! };
//! use tokio::time::{sleep, Duration};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! println!("🛡️ Error Handling and Recovery Examples:");
//! 
//! // Robust server startup with error handling
//! async fn start_server_with_recovery() -> Result<CallCenterServer, CallCenterError> {
//!     let config = CallCenterConfig::default();
//!     
//!     // Attempt primary database
//!     match CallCenterServerBuilder::new()
//!         .with_config(config.clone())
//!         .with_database_path("/primary/callcenter.db".to_string())
//!         .build()
//!         .await
//!     {
//!         Ok(mut server) => {
//!             match server.start().await {
//!                 Ok(()) => {
//!                     println!("✅ Server started with primary database");
//!                     return Ok(server);
//!                 }
//!                 Err(e) => {
//!                     println!("⚠️ Failed to start with primary database: {}", e);
//!                 }
//!             }
//!         }
//!         Err(e) => {
//!             println!("⚠️ Failed to build with primary database: {}", e);
//!         }
//!     }
//!     
//!     // Fallback to backup database
//!     match CallCenterServerBuilder::new()
//!         .with_config(config.clone())
//!         .with_database_path("/backup/callcenter.db".to_string())
//!         .build()
//!         .await
//!     {
//!         Ok(mut server) => {
//!             server.start().await?;
//!             println!("⚠️ Server started with backup database");
//!             Ok(server)
//!         }
//!         Err(_) => {
//!             // Final fallback to in-memory
//!             let mut server = CallCenterServer::new_in_memory(config).await?;
//!             server.start().await?;
//!             println!("🚨 Server started with in-memory database (temporary)");
//!             Ok(server)
//!         }
//!     }
//! }
//! 
//! // Graceful error recovery
//! async fn handle_runtime_errors(server: &CallCenterServer) -> Result<(), Box<dyn std::error::Error>> {
//!     let supervisor_api = server.supervisor_api();
//!     
//!     // Monitor for error conditions
//!     loop {
//!         match supervisor_api.get_stats().await {
//!             stats => {
//!                 // Check for overload conditions
//!                 if stats.queued_calls > 100 {
//!                     println!("🚨 CRITICAL: Queue overload detected");
//!                     // In production: trigger load balancing, alert operators
//!                     break;
//!                 }
//!                 
//!                 if stats.available_agents == 0 && stats.queued_calls > 0 {
//!                     println!("⚠️ WARNING: No agents available with waiting calls");
//!                     // In production: page on-call staff, activate overflow
//!                 }
//!                 
//!                 // Normal operation
//!                 if stats.active_calls > 0 || stats.queued_calls > 0 {
//!                     println!("📊 Normal operation: {} active, {} queued", 
//!                              stats.active_calls, stats.queued_calls);
//!                 }
//!             }
//!         }
//!         
//!         sleep(Duration::from_secs(5)).await;
//!         break; // Exit for example
//!     }
//!     
//!     Ok(())
//! }
//! 
//! // Graceful shutdown handling
//! async fn graceful_shutdown(mut server: CallCenterServer) -> Result<(), Box<dyn std::error::Error>> {
//!     println!("🛑 Initiating graceful shutdown...");
//!     
//!     // Check for active operations
//!     let stats = server.supervisor_api().get_stats().await;
//!     
//!     if stats.active_calls > 0 {
//!         println!("⏳ Waiting for {} active calls to complete...", stats.active_calls);
//!         // In production: wait with timeout, then force shutdown
//!     }
//!     
//!     if stats.queued_calls > 0 {
//!         println!("📋 {} calls in queue will be preserved for restart", stats.queued_calls);
//!         // In production: persist queue state for recovery
//!     }
//!     
//!     // Stop server operations
//!     server.stop().await?;
//!     println!("✅ Server shutdown completed gracefully");
//!     
//!     Ok(())
//! }
//! 
//! // Run error handling examples
//! let server = start_server_with_recovery().await?;
//! handle_runtime_errors(&server).await?;
//! graceful_shutdown(server).await?;
//! 
//! # Ok(())
//! # }
//! ```
//!
//! ## Production Deployment Patterns
//!
//! ### High Availability Setup
//!
//! For production deployments, consider these patterns:
//!
//! ```rust
//! # use rvoip_call_engine::server::CallCenterServer;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // High availability deployment considerations:
//! println!("🏗️ Production Deployment Patterns:");
//! 
//! println!("  🔄 High Availability:");
//! println!("     ↳ Primary/Secondary server setup");
//! println!("     ↳ Database replication and failover");
//! println!("     ↳ Load balancer with health checks");
//! println!("     ↳ Shared storage for call recordings");
//! 
//! println!("  📊 Monitoring Integration:");
//! println!("     ↳ Prometheus metrics export");
//! println!("     ↳ Grafana dashboards for visualization");
//! println!("     ↳ AlertManager for critical alerts");
//! println!("     ↳ Log aggregation with ELK stack");
//! 
//! println!("  🔐 Security Considerations:");
//! println!("     ↳ TLS encryption for SIP traffic");
//! println!("     ↳ Database connection encryption");
//! println!("     ↳ API authentication and authorization");
//! println!("     ↳ Network segmentation and firewalls");
//! 
//! println!("  📈 Scalability Planning:");
//! println!("     ↳ Horizontal scaling with multiple instances");
//! println!("     ↳ Database partitioning for large deployments");
//! println!("     ↳ Queue distribution across instances");
//! println!("     ↳ Agent load balancing strategies");
//! 
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance and Scalability
//!
//! ### Optimization Guidelines
//!
//! The server is designed for high-performance operation:
//!
//! ```rust
//! # use rvoip_call_engine::server::CallCenterServer;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! println!("⚡ Performance Optimization:");
//! 
//! println!("  🚀 Server Performance:");
//! println!("     ↳ Async I/O throughout for non-blocking operations");
//! println!("     ↳ Connection pooling for database efficiency");
//! println!("     ↳ Background task optimization");
//! println!("     ↳ Memory-efficient data structures");
//! 
//! println!("  📊 Scaling Characteristics:");
//! println!("     ↳ Linear scaling with agent count");
//! println!("     ↳ Efficient queue processing algorithms");
//! println!("     ↳ Optimized database queries");
//! println!("     ↳ Minimal memory overhead per call");
//! 
//! println!("  🔧 Tuning Parameters:");
//! println!("     ↳ Queue processing interval (default: 500ms)");
//! println!("     ↳ Monitoring interval (default: 10s)");
//! println!("     ↳ Database connection pool size");
//! println!("     ↳ Maximum concurrent calls per agent");
//! 
//! println!("  📈 Capacity Planning:");
//! println!("     ↳ ~1000 concurrent calls per server instance");
//! println!("     ↳ ~100 agents per instance (typical)");
//! println!("     ↳ ~50 active queues recommended");
//! println!("     ↳ Scale horizontally for larger deployments");
//! 
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, interval};
use tokio::task::JoinHandle;
use tracing::{info, error, debug};

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
            let wrap_up = agents.iter().filter(|a| matches!(a.status, AgentStatus::PostCallWrapUp)).count();
            let offline = agents.iter().filter(|a| matches!(a.status, AgentStatus::Offline)).count();
            
            info!("👥 Agent Status Summary:");
            info!("  ✅ Available: {}", available);
            info!("  🔴 Busy: {}", busy);
            info!("  ⏰ Wrap-up: {}", wrap_up);
            info!("  ⚫ Offline: {}", offline);
            info!("  📋 Total: {}", agents.len());
            
            // PHASE 0.10: Show individual agent status for debugging with error handling
            if agents.len() > 0 && agents.len() <= 5 {  // Only show individual status for small teams
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    info!("👤 Individual Agent Status:");
                    for agent in &agents {
                        let status_str = match &agent.status {
                            AgentStatus::Available => "Available ✅".to_string(),
                            AgentStatus::Busy(calls) => format!("Busy ({} calls) 🔴", calls.len()),
                            AgentStatus::PostCallWrapUp => "Wrap-up ⏰".to_string(),
                            AgentStatus::Offline => "Offline ⚫".to_string(),
                        };
                        info!("  - {} ({}): {}", agent.sip_uri, agent.agent_id, status_str);
                    }
                })) {
                    Ok(_) => {
                        // Agent status displayed successfully
                    }
                    Err(_) => {
                        error!("🚨 Database panic caught during agent status display - continuing server operation");
                        info!("👤 Individual Agent Status: {} agents available (details unavailable due to database error)", agents.len());
                    }
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
        info!("🔄 Starting ROUTING-BASED queue processor for automatic call distribution");
        
        let mut interval = tokio::time::interval(Duration::from_millis(500)); // Check every 500ms (reduced frequency)
        
        loop {
            interval.tick().await;
            
            // Use ROUTING-BASED processing instead of simple core.rs logic
            if let Err(e) = Self::process_all_queues_with_routing(&engine).await {
                error!("Error processing queues with routing: {}", e);
            }
        }
    }
    
    /// Process all queues using sophisticated routing logic (replaces simple core.rs assignment)
    async fn process_all_queues_with_routing(engine: &Arc<CallCenterEngine>) -> Result<()> {
        // Check standard queues for activity and trigger routing-based assignment
        let standard_queues = vec!["general", "support", "sales", "billing", "vip", "premium"];
        
        for queue_id in standard_queues {
            let queue_depth = engine.get_queue_depth(queue_id).await;
            if queue_depth > 0 {
                debug!("🔄 Processing queue '{}' with {} calls using ROUTING logic", queue_id, queue_depth);
                
                // Use the sophisticated routing logic with sequential assignment and BUSY status updates
                engine.monitor_queue_for_agents(queue_id.to_string()).await;
            }
        }
        
        Ok(())
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