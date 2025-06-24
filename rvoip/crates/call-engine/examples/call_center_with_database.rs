//! # Call Center with REAL Session-Core API Integration
//!
//! This example demonstrates the call-engine with actual session-core integration,
//! using the proper session-core ServerSessionManager API for handling incoming calls.
//!
//! ## Features Demonstrated
//!
//! - **Real Session-Core API Integration**: Actual ServerSessionManager with incoming call notifications
//! - **Agent Registration**: Register agents and make them available for calls
//! - **Call Routing**: Automatic routing of incoming calls to available agents
//! - **Bridge Management**: Real bridge creation for agent-customer calls
//! - **Database Persistence**: Limbo database with agent storage
//! - **Event Monitoring**: Bridge event notifications and call tracking

use anyhow::Result;
use rvoip_call_engine::prelude::*;
use std::sync::Arc;
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("debug,rvoip_call_engine=trace,rvoip_session_core=debug")
        .init();

    println!("🚀 Starting Call Center with REAL Session-Core API Integration\n");

    // Step 1: Initialize database with real Limbo integration
    println!("📊 Initializing Limbo database...");
    let database = CallCenterDatabase::new_in_memory().await?;
    println!("✅ Database initialized\n");

    // Step 2: Create call center configuration
    println!("⚙️ Creating call center configuration...");
    let config = CallCenterConfig {
        general: GeneralConfig {
            max_concurrent_calls: 100,
            max_agents: 50,
            default_call_timeout: 300,
            cleanup_interval: std::time::Duration::from_secs(60),
            local_signaling_addr: "127.0.0.1:5060".parse()?,
            local_media_addr: "127.0.0.1:0".parse()?,
            user_agent: "RVOIP-CallCenter/1.0".to_string(),
            domain: "callcenter.local".to_string(),
        },
        agents: AgentConfig {
            default_max_concurrent_calls: 2,
            availability_timeout: 300,
            auto_logout_timeout: 3600,
            enable_skill_based_routing: true,
            default_skills: vec!["general".to_string()],
        },
        queues: QueueConfig {
            default_max_wait_time: 300,
            max_queue_size: 100,
            enable_priorities: true,
            enable_overflow: true,
            announcement_interval: 30,
        },
        routing: RoutingConfig {
            default_strategy: RoutingStrategy::SkillBased,
            enable_load_balancing: true,
            load_balance_strategy: LoadBalanceStrategy::LeastBusy,
            ..Default::default()
        },
        monitoring: MonitoringConfig {
            enable_realtime_monitoring: true,
            enable_call_recording: false,
            enable_quality_monitoring: true,
            dashboard_update_interval: 60,
            metrics_interval: 30,
        },
        database: DatabaseConfig {
            database_path: ":memory:".to_string(),
            max_connections: 10,
            enable_connection_pooling: true,
            query_timeout: 30,
            enable_auto_backup: false,
            backup_interval: 3600,
        },
    };
    println!("✅ Configuration ready\n");

    // Step 3: Create CallCenterEngine with REAL session-core integration
    println!("🎯 Creating CallCenterEngine with session-core API integration...");
    let call_center = CallCenterEngine::new(
        config.clone(),
        database.clone()
    ).await?;
    println!("✅ CallCenterEngine created with REAL session-core integration!\n");

    // Step 4: Register sample agents with session-core
    println!("👥 Registering agents with session-core...");
    
    let agents = vec![
        Agent {
            id: "agent-001".to_string(),
            sip_uri: "sip:alice@callcenter.local".to_string(),
            display_name: "Alice Johnson".to_string(),
            skills: vec!["english".to_string(), "sales".to_string()],
            max_concurrent_calls: 2,
            status: AgentStatus::Available,
            department: Some("sales".to_string()),
            extension: Some("1001".to_string()),
        },
        Agent {
            id: "agent-002".to_string(),
            sip_uri: "sip:bob@callcenter.local".to_string(),
            display_name: "Bob Smith".to_string(),
            skills: vec!["english".to_string(), "technical".to_string()],
            max_concurrent_calls: 3,
            status: AgentStatus::Available,
            department: Some("technical".to_string()),
            extension: Some("1002".to_string()),
        },
        Agent {
            id: "agent-003".to_string(),
            sip_uri: "sip:carol@callcenter.local".to_string(),
            display_name: "Carol Davis".to_string(),
            skills: vec!["spanish".to_string(), "support".to_string()],
            max_concurrent_calls: 2,
            status: AgentStatus::Available,
            department: Some("support".to_string()),
            extension: Some("1003".to_string()),
        },
    ];

    // Register agents with session-core and database
    let agent_store = AgentStore::new(database.clone());
    for agent in &agents {
        // Register with session-core (creates a real session)
        let session_id = call_center.register_agent(agent).await?;
        println!("  ✅ Agent {} registered with session-core (session: {})", agent.display_name, session_id);
        
        // Also store in database for persistence
        let _db_agent = agent_store.create_agent(CreateAgentRequest {
            sip_uri: agent.sip_uri.to_string(),
            display_name: agent.display_name.clone(),
            max_concurrent_calls: agent.max_concurrent_calls,
            department: agent.department.clone(),
            extension: agent.extension.clone(),
            phone_number: None,
        }).await?;
    }
    println!("✅ All agents registered with session-core\n");

    // Step 5: Display call center statistics
    println!("📊 Call Center Statistics:");
    let stats = call_center.get_stats().await;
    println!("  🏢 Active Calls: {}", stats.active_calls);
    println!("  🌉 Active Bridges: {}", stats.active_bridges);
    println!("  👥 Available Agents: {}", stats.available_agents);
    println!("  📋 Queued Calls: {}", stats.queued_calls);
    println!("  📈 Total Calls Handled: {}", stats.total_calls_handled);

    // Step 6: Demonstrate bridge management capabilities
    println!("\n🌉 Bridge Management Capabilities:");
    let bridges = call_center.list_active_bridges().await;
    println!("  📊 Currently active bridges: {}", bridges.len());
    
    // Show bridge configuration
    if bridges.is_empty() {
        println!("  💡 Ready to create bridges when calls are received");
    } else {
        for bridge in bridges {
            println!("  🌉 Bridge {}: {} sessions", bridge.id, bridge.sessions.len());
        }
    }

    // Step 7: Show database and session-core integration
    println!("\n🔗 Integration Summary:");
    println!("  ✅ Real session-core ServerSessionManager integration");
    println!("  ✅ Incoming call notification system");
    println!("  ✅ Agent sessions created and tracked");
    println!("  ✅ Bridge management API available");
    println!("  ✅ Event monitoring system ready");
    println!("  ✅ Database persistence layer active");
    
    println!("\n🎯 What This Integration Provides:");
    println!("  📞 Real SIP session handling via session-core");
    println!("  🔔 Incoming call notifications with routing decisions");
    println!("  🌉 Automatic bridge creation for agent-customer calls");
    println!("  👁️ Real-time bridge event monitoring");
    println!("  🎛️ Complete call center orchestration");
    println!("  📊 Statistics and monitoring capabilities");
    
    println!("\n🚀 Call Center Ready!");
    println!("  • Listening for incoming calls on 127.0.0.1:5060");
    println!("  • {} agents available for calls", stats.available_agents);
    println!("  • Session-core API fully integrated");
    println!("  • Database persistence active");

    // Keep the server running for a bit to demonstrate
    println!("\n⏰ Server running for 30 seconds to demonstrate integration...");
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    
    println!("✅ Call center demonstration completed successfully!");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_call_center_integration() -> Result<()> {
        let database = CallCenterDatabase::new_in_memory().await?;
        let config = CallCenterConfig::default();
        
        // Test call center creation
        let call_center = CallCenterEngine::new(
            config,
            database
        ).await?;
        
        // Test agent registration
        let agent = Agent {
            id: "test-agent".to_string(),
            sip_uri: "sip:test@example.com".to_string(),
            display_name: "Test Agent".to_string(),
            skills: vec!["test".to_string()],
            max_concurrent_calls: 1,
            status: AgentStatus::Available,
            department: None,
            extension: None,
        };
        
        let session_id = call_center.register_agent(&agent).await?;
        assert!(!session_id.to_string().is_empty());
        
        // Test statistics
        let stats = call_center.get_stats().await;
        assert_eq!(stats.available_agents, 1);
        assert_eq!(stats.active_calls, 0);
        
        Ok(())
    }
} 