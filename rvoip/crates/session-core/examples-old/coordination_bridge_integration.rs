//! Integration Example: Session Coordination with Bridge Infrastructure
//! 
//! This example demonstrates how the new session coordination patterns work
//! together with the existing bridge infrastructure to manage complex call scenarios.

use std::sync::Arc;
use rvoip_session_core::{
    SessionId, SessionConfig, 
    SessionGroupManager, GroupConfig, GroupType,
    SessionBridge, BridgeConfig, BridgeId,
    SessionDependencyTracker, DependencyType, DependencyConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize components
    let group_manager = SessionGroupManager::new(Default::default());
    let dependency_tracker = SessionDependencyTracker::new(DependencyConfig::default());
    
    println!("🚀 Session Coordination + Bridge Integration Demo");
    
    // === SCENARIO 1: Conference Call ===
    println!("\n📞 === CONFERENCE CALL SCENARIO ===");
    
    // 1. Create session group for conference coordination
    let conference_config = GroupConfig {
        max_sessions: Some(10),
        min_sessions: 2,
        synchronize_states: true,
        propagate_events: true,
        ..Default::default()
    };
    
    let (conference_group_id, bridge_id) = group_manager.create_bridge_group(
        conference_config,
        |bridge_config| Arc::new(SessionBridge::new(bridge_config))
    ).await?;
    
    println!("✅ Created conference group {} with media bridge {}", conference_group_id, bridge_id);
    
    // 2. Add participants with coordinated session and media management
    let participant1 = SessionId::new();
    let participant2 = SessionId::new();
    let participant3 = SessionId::new();
    
    // Simulate bridge manager that adds sessions to actual media bridge
    let bridge_manager = |bridge_id: BridgeId, session_id: SessionId| {
        println!("  🎵 Adding session {} to media bridge {}", session_id, bridge_id);
        // In real implementation: bridge.add_session(session_id).await
        Ok(())
    };
    
    group_manager.add_session_with_bridge(&conference_group_id, participant1, "moderator".to_string(), bridge_manager).await?;
    group_manager.add_session_with_bridge(&conference_group_id, participant2, "participant".to_string(), bridge_manager).await?;
    group_manager.add_session_with_bridge(&conference_group_id, participant3, "participant".to_string(), bridge_manager).await?;
    
    println!("✅ Added 3 participants to conference (1 moderator, 2 participants)");
    
    // === SCENARIO 2: Attended Transfer with Dependencies ===
    println!("\n🔄 === ATTENDED TRANSFER SCENARIO ===");
    
    // 1. Create transfer group 
    let transfer_config = GroupConfig {
        max_sessions: Some(3),
        min_sessions: 2,
        ..Default::default()
    };
    
    let transfer_group_id = group_manager.create_group(GroupType::Transfer, transfer_config).await?;
    
    // 2. Set up dependency relationships
    let original_session = SessionId::new();
    let consultation_session = SessionId::new();
    let target_session = SessionId::new();
    
    // Add sessions to transfer group
    group_manager.add_session_to_group(&transfer_group_id, original_session, "original".to_string()).await?;
    group_manager.add_session_to_group(&transfer_group_id, consultation_session, "consultation".to_string()).await?;
    group_manager.add_session_to_group(&transfer_group_id, target_session, "target".to_string()).await?;
    
    // Create dependency relationships
    let consultation_dep = dependency_tracker.create_dependency(
        consultation_session,
        original_session,
        DependencyType::Consultation,
    ).await?;
    
    let transfer_dep = dependency_tracker.create_dependency(
        target_session,
        original_session,
        DependencyType::Transfer,
    ).await?;
    
    println!("✅ Created transfer group with consultation and target dependencies");
    
    // === SCENARIO 3: Hunt Group with Sequential Processing ===
    println!("\n🎯 === HUNT GROUP SCENARIO ===");
    
    let hunt_config = GroupConfig {
        max_sessions: Some(5),
        min_sessions: 1,
        ..Default::default()
    };
    
    let hunt_group_id = group_manager.create_group(GroupType::Hunt, hunt_config).await?;
    
    // Add hunt group members with priority ordering via dependencies
    let hunt_member1 = SessionId::new();
    let hunt_member2 = SessionId::new();
    let hunt_member3 = SessionId::new();
    
    group_manager.add_session_to_group(&hunt_group_id, hunt_member1, "primary".to_string()).await?;
    group_manager.add_session_to_group(&hunt_group_id, hunt_member2, "secondary".to_string()).await?;
    group_manager.add_session_to_group(&hunt_group_id, hunt_member3, "tertiary".to_string()).await?;
    
    // Create sequential dependencies for hunt order
    dependency_tracker.create_dependency(hunt_member2, hunt_member1, DependencyType::Sequential).await?;
    dependency_tracker.create_dependency(hunt_member3, hunt_member2, DependencyType::Sequential).await?;
    
    println!("✅ Created hunt group with sequential processing order");
    
    // === SCENARIO 4: Complex Multi-Bridge Conference ===
    println!("\n🌟 === MULTI-BRIDGE CONFERENCE SCENARIO ===");
    
    // Create multiple conference groups that can be bridged together
    let regional_conference_config = GroupConfig {
        max_sessions: Some(5),
        min_sessions: 1,
        ..Default::default()
    };
    
    let (europe_group, europe_bridge) = group_manager.create_bridge_group(
        regional_conference_config.clone(),
        |config| Arc::new(SessionBridge::new(config))
    ).await?;
    
    let (americas_group, americas_bridge) = group_manager.create_bridge_group(
        regional_conference_config,
        |config| Arc::new(SessionBridge::new(config))
    ).await?;
    
    // Create bridge dependency between regional conferences
    dependency_tracker.create_dependency(
        SessionId::new(), // Placeholder for Americas group representative
        SessionId::new(), // Placeholder for Europe group representative  
        DependencyType::Bridge,
    ).await?;
    
    println!("✅ Created multi-region conference with bridge dependencies");
    
    // === METRICS AND STATUS ===
    println!("\n📊 === FINAL STATUS ===");
    
    let group_metrics = group_manager.get_metrics().await;
    let dependency_metrics = dependency_tracker.get_metrics().await;
    let active_groups = group_manager.get_active_group_count().await;
    let bridge_groups = group_manager.get_bridge_groups().await;
    
    println!("📈 Session Coordination Metrics:");
    println!("  - Active groups: {}", active_groups);
    println!("  - Total groups created: {}", group_metrics.total_groups_created);
    println!("  - Groups with media bridges: {}", bridge_groups.len());
    println!("  - Active dependencies: {}", dependency_metrics.active_dependencies);
    println!("  - Total dependencies created: {}", dependency_metrics.total_dependencies_created);
    
    println!("\n🎯 Integration Benefits Demonstrated:");
    println!("  ✅ Session coordination layer manages call relationships");
    println!("  ✅ Media bridge layer handles audio routing"); 
    println!("  ✅ Coordinated session and media management");
    println!("  ✅ Flexible patterns: conference, transfer, hunt, multi-bridge");
    println!("  ✅ Dependency tracking for complex call flows");
    
    Ok(())
} 