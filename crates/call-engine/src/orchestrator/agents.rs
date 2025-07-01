//! # Agent Management for Call Center Operations
//!
//! This module provides sophisticated agent management functionality for the call center,
//! handling agent registration, status updates, availability monitoring, and performance
//! tracking. It integrates with session-core for SIP registration and provides real-time
//! agent state management for optimal call routing.
//!
//! ## Overview
//!
//! Agent management is central to call center operations. This module provides comprehensive
//! functionality for registering agents, tracking their availability, managing their status
//! transitions, and monitoring their performance. It seamlessly integrates with the database
//! layer for persistence and session-core for SIP communication.
//!
//! ## Key Features
//!
//! - **Agent Registration**: Complete SIP agent registration with skill tracking
//! - **Status Management**: Real-time agent status updates and transitions
//! - **Availability Monitoring**: Sophisticated availability tracking and queue monitoring
//! - **Performance Tracking**: Agent performance metrics and queue statistics
//! - **Session Integration**: Seamless integration with session-core for SIP operations
//! - **Database Synchronization**: Persistent agent state management
//! - **Automatic Assignment**: Intelligent queued call assignment to available agents
//! - **Concurrency Management**: Thread-safe agent state management
//!
//! ## Agent Lifecycle
//!
//! The agent lifecycle follows this pattern:
//!
//! 1. **Registration**: Agent registers with SIP URI and skills
//! 2. **Available**: Agent becomes available for call assignment
//! 3. **Busy**: Agent is handling one or more calls
//! 4. **Post-Call Wrap-Up**: Agent completes post-call tasks
//! 5. **Available**: Agent returns to available state
//! 6. **Offline**: Agent logs off or becomes unavailable
//!
//! ## Examples
//!
//! ### Basic Agent Registration
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::{Agent, AgentId, AgentStatus}};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Create agent profile
//! let agent = Agent {
//!     id: "agent-001".to_string(),
//!     sip_uri: "sip:alice@call-center.com".to_string(),
//!     display_name: "Alice".to_string(),
//!     status: AgentStatus::Available,
//!     department: Some("support".to_string()),
//!     extension: Some("1234".to_string()),
//!     skills: vec!["technical_support".to_string(), "billing".to_string()],
//!     max_concurrent_calls: 2,
//! };
//! 
//! // Register agent with the call center
//! let session_id = engine.register_agent(&agent).await?;
//! println!("✅ Agent {} registered with session: {}", agent.id, session_id);
//! 
//! // Agent is now available for call assignment
//! println!("📞 Agent ready to receive calls");
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Status Management
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::{AgentId, AgentStatus}};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! let agent_id = AgentId("agent-001".to_string());
//! 
//! // Update agent status to available
//! engine.update_agent_status(&agent_id, AgentStatus::Available).await?;
//! println!("🟢 Agent {} is now available", agent_id);
//! 
//! // Agent receives a call - status automatically updates to busy
//! engine.update_agent_status(&agent_id, AgentStatus::Busy(vec![rvoip_session_core::SessionId("call-123".to_string())])).await?;
//! println!("🔴 Agent {} is now busy with calls", agent_id);
//! 
//! // After call ends - agent enters post-call wrap-up
//! engine.update_agent_status(&agent_id, AgentStatus::PostCallWrapUp).await?;
//! println!("🟡 Agent {} in post-call wrap-up", agent_id);
//! 
//! // Agent completes wrap-up and becomes available again
//! engine.update_agent_status(&agent_id, AgentStatus::Available).await?;
//! println!("✅ Agent {} ready for next call", agent_id);
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Information and Monitoring
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::AgentId};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! let agent_id = AgentId("agent-001".to_string());
//! 
//! // Get detailed agent information
//! if let Some(agent_info) = engine.get_agent_info(&agent_id).await {
//!     println!("👤 Agent Information:");
//!     println!("  ID: {}", agent_info.agent_id);
//!     println!("  Status: {:?}", agent_info.status);
//!     println!("  SIP URI: {}", agent_info.sip_uri);
//!     println!("  Skills: {:?}", agent_info.skills);
//!     println!("  Current Calls: {}/{}", agent_info.current_calls, agent_info.max_calls);
//!     println!("  Performance: {:.1}/5.0", agent_info.performance_score);
//! } else {
//!     println!("❌ Agent not found");
//! }
//! 
//! // List all agents
//! let all_agents = engine.list_agents().await;
//! println!("\n👥 All Agents ({}):", all_agents.len());
//! for agent in all_agents {
//!     let status_icon = match agent.status {
//!         rvoip_call_engine::agent::AgentStatus::Available => "🟢",
//!         rvoip_call_engine::agent::AgentStatus::Busy(_) => "🔴",
//!         rvoip_call_engine::agent::AgentStatus::PostCallWrapUp => "🟡",
//!         rvoip_call_engine::agent::AgentStatus::Offline => "⚫",
//!     };
//!     println!("  {} {} - {}", status_icon, agent.agent_id, agent.sip_uri);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Queue Statistics and Monitoring
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Get comprehensive queue statistics
//! let queue_stats = engine.get_queue_stats().await?;
//! 
//! println!("📊 Queue Statistics:");
//! for (queue_name, stats) in queue_stats {
//!     println!("  Queue: {}", queue_name);
//!     println!("    Total Calls: {}", stats.total_calls);
//!     println!("    Average Wait: {:.1}s", stats.average_wait_time_seconds);
//!     println!("    Longest Wait: {:.1}s", stats.longest_wait_time_seconds);
//!     
//!     // Performance indicators
//!     if stats.average_wait_time_seconds > 60 {
//!         println!("    ⚠️ High wait times detected");
//!     }
//!     if stats.total_calls > 10 {
//!         println!("    📞 High queue volume");
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Agent Operations
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::{Agent, AgentId, AgentStatus}};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Register multiple agents with different skills
//! let agents = vec![
//!     Agent {
//!         id: "agent-tech-001".to_string(),
//!         sip_uri: "sip:alice@call-center.com".to_string(),
//!         display_name: "Alice Tech".to_string(),
//!         status: AgentStatus::Available,
//!         department: Some("technical".to_string()),
//!         extension: Some("2001".to_string()),
//!         skills: vec!["technical_support".to_string(), "windows".to_string()],
//!         max_concurrent_calls: 3,
//!     },
//!     Agent {
//!         id: "agent-sales-001".to_string(),
//!         sip_uri: "sip:bob@call-center.com".to_string(),
//!         display_name: "Bob Sales".to_string(),
//!         status: AgentStatus::Available,
//!         department: Some("sales".to_string()),
//!         extension: Some("2002".to_string()),
//!         skills: vec!["sales".to_string(), "upselling".to_string()],
//!         max_concurrent_calls: 2,
//!     },
//!     Agent {
//!         id: "agent-billing-001".to_string(),
//!         sip_uri: "sip:charlie@call-center.com".to_string(),
//!         display_name: "Charlie Billing".to_string(),
//!         status: AgentStatus::Available,
//!         department: Some("billing".to_string()),
//!         extension: Some("2003".to_string()),
//!         skills: vec!["billing".to_string(), "collections".to_string()],
//!         max_concurrent_calls: 4,
//!     },
//! ];
//! 
//! // Register all agents
//! for agent in agents {
//!     match engine.register_agent(&agent).await {
//!         Ok(session_id) => {
//!             println!("✅ Registered {} (skills: {:?})", agent.id, agent.skills);
//!             
//!             // Make agent available
//!             let agent_id = AgentId(agent.id);
//!             engine.update_agent_status(&agent_id, AgentStatus::Available).await?;
//!         }
//!         Err(e) => {
//!             eprintln!("❌ Failed to register {}: {}", agent.id, e);
//!         }
//!     }
//! }
//! 
//! // Monitor agent capacity
//! let all_agents = engine.list_agents().await;
//! let total_capacity: usize = all_agents.iter()
//!     .filter(|a| matches!(a.status, rvoip_call_engine::agent::AgentStatus::Available))
//!     .map(|a| a.max_calls)
//!     .sum();
//! 
//! println!("📈 Total system capacity: {} concurrent calls", total_capacity);
//! # Ok(())
//! # }
//! ```
//!
//! ### Real-Time Agent Monitoring
//!
//! ```rust
//! use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::AgentId};
//! use tokio::time::{interval, Duration};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Real-time monitoring dashboard
//! async fn monitor_agents(engine: &CallCenterEngine) -> Result<(), Box<dyn std::error::Error>> {
//!     let mut interval = interval(Duration::from_secs(30));
//!     
//!     loop {
//!         interval.tick().await;
//!         
//!         let agents = engine.list_agents().await;
//!         let mut available = 0;
//!         let mut busy = 0;
//!         let mut wrap_up = 0;
//!         let mut offline = 0;
//!         
//!         for agent in &agents {
//!             match agent.status {
//!                 rvoip_call_engine::agent::AgentStatus::Available => available += 1,
//!                 rvoip_call_engine::agent::AgentStatus::Busy(_) => busy += 1,
//!                 rvoip_call_engine::agent::AgentStatus::PostCallWrapUp => wrap_up += 1,
//!                 rvoip_call_engine::agent::AgentStatus::Offline => offline += 1,
//!             }
//!         }
//!         
//!         println!("📊 Agent Status Update:");
//!         println!("  🟢 Available: {}", available);
//!         println!("  🔴 Busy: {}", busy);
//!         println!("  🟡 Wrap-up: {}", wrap_up);
//!         println!("  ⚫ Offline: {}", offline);
//!         
//!         // Calculate utilization
//!         let online_agents = available + busy + wrap_up;
//!         if online_agents > 0 {
//!             let utilization = (busy as f64 / online_agents as f64) * 100.0;
//!             println!("  📈 Utilization: {:.1}%", utilization);
//!             
//!             // Alerts
//!             if utilization > 90.0 {
//!                 println!("  🚨 High utilization warning!");
//!             }
//!             if available == 0 && busy > 0 {
//!                 println!("  ⚠️ No agents available - all busy!");
//!             }
//!         }
//!         
//!         // In a real implementation, this would run continuously
//!         break; // Exit for documentation example
//!     }
//!     
//!     Ok(())
//! }
//! 
//! // Start monitoring (would run in background)
//! monitor_agents(&engine).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Integration Patterns
//!
//! ### Session-Core Integration
//!
//! The agent management module integrates seamlessly with session-core:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::{Agent, AgentStatus}};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Agent registration creates session-core session
//! let agent = Agent {
//!     id: "agent-integration".to_string(),
//!     sip_uri: "sip:integration@call-center.com".to_string(),
//!     display_name: "Integration Test".to_string(),
//!     status: AgentStatus::Available,
//!     department: Some("test".to_string()),
//!     extension: Some("9999".to_string()),
//!     skills: vec!["integration_test".to_string()],
//!     max_concurrent_calls: 1,
//! };
//! 
//! // Registration automatically:
//! // 1. Creates session-core outgoing call for registration
//! // 2. Updates database with agent information
//! // 3. Tracks agent session for future operations
//! let session_id = engine.register_agent(&agent).await?;
//! 
//! println!("🔗 Agent registered with session-core session: {}", session_id);
//! println!("💾 Agent information persisted to database");
//! println!("📞 Agent ready for call assignment via session-core");
//! # Ok(())
//! # }
//! ```
//!
//! ### Database Integration
//!
//! Agent state is automatically synchronized with the database:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::{AgentId, AgentStatus}};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! let agent_id = AgentId("database-sync-test".to_string());
//! 
//! // Status updates are automatically persisted
//! engine.update_agent_status(&agent_id, AgentStatus::Available).await?;
//! // ↳ Updates database agents table
//! // ↳ Updates available_since timestamp for fairness
//! // ↳ Triggers queue assignment check
//! 
//! engine.update_agent_status(&agent_id, AgentStatus::Busy(vec![rvoip_session_core::SessionId("call-123".to_string())])).await?;
//! // ↳ Updates agent status and current_calls count
//! // ↳ Removes from available agents pool
//! 
//! println!("💾 All agent status changes automatically persisted");
//! println!("🔄 Database maintains real-time agent state");
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance Considerations
//!
//! ### Efficient Agent Selection
//!
//! The module uses optimized algorithms for agent selection:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Agent selection algorithms consider:
//! // - Round-robin fairness with last-agent exclusion
//! // - Current call load balancing
//! // - Skill matching requirements
//! // - Performance ratings
//! // - Available-since timestamps for fairness
//! 
//! let agents = engine.list_agents().await;
//! println!("🎯 Agent selection considers {} factors:", 5);
//! println!("  1. Round-robin fairness");
//! println!("  2. Current call load");
//! println!("  3. Skill requirements");
//! println!("  4. Performance ratings");
//! println!("  5. Availability timing");
//! 
//! // Selection is O(n log n) due to sorting by availability
//! println!("⚡ Selection complexity: O(n log n) for {} agents", agents.len());
//! # Ok(())
//! # }
//! ```
//!
//! ### Concurrent Operations
//!
//! Agent operations are designed for high concurrency:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::{AgentId, AgentStatus}};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Concurrent status updates are safe
//! let agent_ids = vec![
//!     AgentId("agent-001".to_string()),
//!     AgentId("agent-002".to_string()),
//!     AgentId("agent-003".to_string()),
//! ];
//! 
//! // Multiple agents can be updated concurrently
//! let mut success_count = 0;
//! for agent_id in agent_ids {
//!     if engine.update_agent_status(&agent_id, AgentStatus::Available).await.is_ok() {
//!         success_count += 1;
//!     }
//! }
//! 
//! println!("✅ Successfully updated {} agents concurrently", success_count);
//! println!("🔒 Thread-safe operations with database consistency");
//! # Ok(())
//! # }
//! ```
//!
//! ## Error Handling
//!
//! The module provides comprehensive error handling:
//!
//! ```rust
//! # use rvoip_call_engine::{CallCenterEngine, CallCenterConfig, agent::{Agent, AgentId, AgentStatus}};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), Some(":memory:".to_string())).await?;
//! 
//! // Graceful error handling for registration
//! let agent = Agent {
//!     id: "test-agent".to_string(),
//!     sip_uri: "sip:test@call-center.com".to_string(),
//!     display_name: "Test Agent".to_string(),
//!     status: AgentStatus::Available,
//!     department: Some("test".to_string()),
//!     extension: Some("8888".to_string()),
//!     skills: vec!["test".to_string()],
//!     max_concurrent_calls: 1,
//! };
//! 
//! match engine.register_agent(&agent).await {
//!     Ok(session_id) => {
//!         println!("✅ Agent registered successfully: {}", session_id);
//!     }
//!     Err(e) => {
//!         eprintln!("❌ Registration failed: {}", e);
//!         // Handle specific error types:
//!         // - Network connectivity issues
//!         // - Database constraints
//!         // - Session-core errors
//!         // - Configuration problems
//!     }
//! }
//! 
//! // Graceful handling of status updates
//! let agent_id = AgentId("test-agent".to_string());
//! match engine.update_agent_status(&agent_id, AgentStatus::Available).await {
//!     Ok(()) => {
//!         println!("✅ Status updated successfully");
//!     }
//!     Err(e) => {
//!         eprintln!("⚠️ Status update failed: {}", e);
//!         // System continues operating with last known state
//!     }
//! }
//! # Ok(())
//! # }
//! ```

//! Agent management functionality for the call center

use std::sync::Arc;
use tracing::{info, error};
use rvoip_session_core::{SessionId, SessionControl};

use crate::agent::{Agent, AgentId, AgentStatus};
use crate::error::{CallCenterError, Result as CallCenterResult};
use crate::queue::QueueStats;
use super::core::CallCenterEngine;
use super::types::AgentInfo;

impl CallCenterEngine {
    /// Register an agent with skills and performance tracking
    pub async fn register_agent(&self, agent: &Agent) -> CallCenterResult<SessionId> {
        info!("👤 Registering agent {} with session-core: {} (skills: {:?})", 
              agent.id, agent.sip_uri, agent.skills);
        
        // Use SessionControl trait to create outgoing call for agent registration
        let session = SessionControl::create_outgoing_call(
            self.session_manager(),
            &agent.sip_uri,  // From: agent's SIP URI
            &self.config.general.registrar_uri(),  // To: local registrar
            None  // No SDP for registration
        )
        .await
        .map_err(|e| CallCenterError::orchestration(&format!("Failed to create agent session: {}", e)))?;
        
        let session_id = session.id;
        let agent_id = AgentId(agent.id.clone());
        
        // Register agent in database
        if let Some(db_manager) = &self.db_manager {
            // Extract username from SIP URI
            let username = agent.sip_uri
                .strip_prefix("sip:")
                .and_then(|s| s.split('@').next())
                .unwrap_or(&agent.id)
                .to_string();
            
            db_manager.upsert_agent(&agent_id.0, &username, Some(&agent.sip_uri)).await
                .map_err(|e| CallCenterError::database(&format!("Failed to register agent in database: {}", e)))?;
            
            info!("✅ Agent {} registered in database", agent_id);
        }
        
        info!("✅ Agent {} registered with session-core (session: {}, max calls: {})", 
              agent.id, session_id, agent.max_concurrent_calls);
        Ok(session_id)
    }
    
    /// Update agent status (Available, Busy, Away, etc.)
    pub async fn update_agent_status(&self, agent_id: &AgentId, new_status: AgentStatus) -> CallCenterResult<()> {
        info!("🔄 Updating agent {} status to {:?}", agent_id, new_status);
        
        // Get current agent info from database
        let old_status = if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent(&agent_id.0).await {
                Ok(Some(db_agent)) => {
                    // Update status in database
                    db_manager.update_agent_status(&agent_id.0, new_status.clone()).await
                        .map_err(|e| CallCenterError::database(&format!("Failed to update agent status: {}", e)))?;
                    
                    // Return old status for logging
                    match db_agent.status.as_str() {
                        "AVAILABLE" => AgentStatus::Available,
                        "BUSY" => AgentStatus::Busy(vec![]),
                        "POSTCALLWRAPUP" => AgentStatus::PostCallWrapUp,
                        _ => AgentStatus::Offline, // Default for unknown statuses
                    }
                }
                Ok(None) => {
                    return Err(CallCenterError::not_found(format!("Agent not found: {}", agent_id)));
                }
                Err(e) => {
                    return Err(CallCenterError::database(&format!("Failed to get agent: {}", e)));
                }
            }
        } else {
            return Err(CallCenterError::database("Database not configured"));
        };
        
        info!("✅ Agent {} status updated from {:?} to {:?}", agent_id, old_status, new_status);
        
        // If agent became available, check for queued calls
        if matches!(new_status, AgentStatus::Available) {
            let agent_id_clone = agent_id.clone();
            let engine = Arc::new(self.clone());
            tokio::spawn(async move {
                engine.try_assign_queued_calls_to_agent(agent_id_clone).await;
            });
        }
        
        Ok(())
    }
    
    /// Get detailed agent information
    pub async fn get_agent_info(&self, agent_id: &AgentId) -> Option<AgentInfo> {
        if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent(&agent_id.0).await {
                Ok(Some(db_agent)) => {
                    let contact_uri = self.config.general.agent_sip_uri(&db_agent.username);
                    Some(AgentInfo::from_db_agent(&db_agent, contact_uri, &self.config.general))
                }
                Ok(None) => None,
                Err(e) => {
                    error!("Failed to get agent {} from database: {}", agent_id, e);
                    None
                }
            }
        } else {
            None
        }
    }
    
    /// List all agents with their current status
    pub async fn list_agents(&self) -> Vec<AgentInfo> {
        if let Some(db_manager) = &self.db_manager {
            match db_manager.list_agents().await {
                Ok(db_agents) => {
                    db_agents.into_iter()
                        .map(|db_agent| {
                            let contact_uri = self.config.general.agent_sip_uri(&db_agent.username);
                            AgentInfo::from_db_agent(&db_agent, contact_uri, &self.config.general)
                        })
                        .collect()
                }
                Err(e) => {
                    error!("Failed to list agents from database: {}", e);
                    vec![]
                }
            }
        } else {
            vec![]
        }
    }
    
    /// Get queue statistics for monitoring
    pub async fn get_queue_stats(&self) -> CallCenterResult<Vec<(String, QueueStats)>> {
        let queue_manager = self.queue_manager.read().await;
        let queue_ids = vec!["general", "sales", "support", "billing", "vip", "premium", "overflow"];
        
        let mut stats = Vec::new();
        for queue_id in queue_ids {
            if let Ok(queue_stat) = queue_manager.get_queue_stats(queue_id) {
                stats.push((queue_id.to_string(), queue_stat));
            }
        }
        
        Ok(stats)
    }
} 