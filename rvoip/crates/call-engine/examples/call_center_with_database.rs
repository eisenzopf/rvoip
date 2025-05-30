use anyhow::Result;
use rvoip_call_engine::prelude::*;
use rvoip_call_engine::database::{
    CallCenterDatabase,
    agent_store::{AgentStore, CreateAgentRequest, AgentStatus},
};
use std::sync::Arc;
use tracing::{info, error};
use tokio;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();
    
    info!("🚀 Starting Call Center with Limbo Database Example (Phase 1)");
    
    // Initialize the database
    let db = match CallCenterDatabase::new_in_memory().await {
        Ok(db) => {
            info!("✅ Database initialized successfully (Phase 1 stub)");
            db
        }
        Err(e) => {
            error!("❌ Failed to initialize database: {}", e);
            return Err(e);
        }
    };
    
    // Initialize configuration
    let config = CallCenterConfig::default();
    info!("⚙️ Using default call center configuration");
    
    // Create the call center engine
    let call_center = CallCenterEngine::new(config, db.clone()).await?;
    info!("🎯 Call center engine created successfully");
    
    // Start the call center
    call_center.start().await?;
    info!("🚀 Call center engine started");
    
    // Create agent store for demonstration
    let agent_store = AgentStore::new(db.clone());
    
    // Phase 1 Demo: Create sample agents (using stubs)
    info!("👥 Creating sample agents...");
    
    let sales_agent = CreateAgentRequest {
        sip_uri: "sip:alice@call-center.local".to_string(),
        display_name: "Alice (Sales)".to_string(),
        max_concurrent_calls: 2,
        department: Some("sales".to_string()),
        extension: Some("1001".to_string()),
        phone_number: Some("+1-555-0101".to_string()),
    };
    
    let support_agent = CreateAgentRequest {
        sip_uri: "sip:bob@call-center.local".to_string(),
        display_name: "Bob (Support)".to_string(),
        max_concurrent_calls: 1,
        department: Some("support".to_string()),
        extension: Some("1002".to_string()),
        phone_number: Some("+1-555-0102".to_string()),
    };
    
    let manager_agent = CreateAgentRequest {
        sip_uri: "sip:charlie@call-center.local".to_string(),
        display_name: "Charlie (Manager)".to_string(),
        max_concurrent_calls: 3,
        department: Some("management".to_string()),
        extension: Some("1003".to_string()),
        phone_number: Some("+1-555-0103".to_string()),
    };
    
    // Create agents using the agent store (Phase 1 stubs)
    let agent1 = agent_store.create_agent(sales_agent).await?;
    info!("✅ Created agent: {} ({})", agent1.display_name, agent1.id);
    
    let agent2 = agent_store.create_agent(support_agent).await?;
    info!("✅ Created agent: {} ({})", agent2.display_name, agent2.id);
    
    let agent3 = agent_store.create_agent(manager_agent).await?;
    info!("✅ Created agent: {} ({})", agent3.display_name, agent3.id);
    
    // Phase 1 Demo: Show call center statistics
    info!("📊 Call center statistics:");
    let stats = call_center.get_statistics();
    info!("  Active calls: {}", stats.active_calls);
    info!("  Active bridges: {}", stats.active_bridges);
    info!("  Total calls handled: {}", stats.total_calls_handled);
    
    // Phase 1 Demo: Simulate an incoming call (stub)
    info!("📞 Simulating incoming call...");
    let dummy_request = Request {
        method: Method::Invite,
        uri: "sip:+15551234567@call-center.local".parse().unwrap(),
        version: rvoip_sip_core::Version::sip_2_0(),
        headers: vec![],
        body: bytes::Bytes::new(),
    };
    
    let session_id = call_center.handle_incoming_call(dummy_request).await?;
    info!("📋 Created session for incoming call: {}", session_id);
    
    info!("🎉 Call Center Example completed successfully!");
    info!("🚧 Note: This is Phase 1 with stubbed functionality");
    info!("🔮 Phase 2 will implement full session-core integration");
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_database_integration() -> Result<()> {
        let db = CallCenterDatabase::new_in_memory().await?;
        let agent_store = AgentStore::new(db.clone());
        
        // Create test agent
        let request = CreateAgentRequest {
            sip_uri: "sip:test@example.com".to_string(),
            display_name: "Test Agent".to_string(),
            max_concurrent_calls: Some(1),
            department: None,
            extension: None,
            phone_number: None,
            skills: Some(vec![("test_skill".to_string(), 3)]),
        };
        
        let agent = agent_store.create_agent(request).await?;
        assert_eq!(agent.display_name, "Test Agent");
        assert_eq!(agent.status, AgentStatus::Offline);
        
        // Update status
        let updated = agent_store.update_agent_status(&agent.id, AgentStatus::Available).await?;
        assert!(updated);
        
        // Find by URI
        let found = agent_store.get_agent_by_sip_uri("sip:test@example.com").await?;
        assert!(found.is_some());
        
        // Check skills
        let skills = agent_store.get_agent_skills(&agent.id).await?;
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].skill_name, "test_skill");
        assert_eq!(skills[0].skill_level, 3);
        
        Ok(())
    }
} 