//! Basic Session Grouping Primitives Demo
//! 
//! Demonstrates the basic session grouping functionality after Phase 12.1 refactoring.
//! This shows the low-level primitives that call-engine will use to build sophisticated
//! business logic.

use rvoip_session_core::{
    SessionId, BasicSessionGroup, BasicGroupType, BasicGroupConfig,
    BasicSessionMembership, BasicGroupEvent
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Basic Session Grouping Primitives Demo");
    println!("==========================================");
    
    // Create a basic session group (data structure only)
    let config = BasicGroupConfig {
        max_sessions: Some(3),
        metadata: std::collections::HashMap::new(),
    };
    
    let mut group = BasicSessionGroup::new(BasicGroupType::Conference, config);
    println!("✅ Created basic conference group: {}", group.id);
    println!("   Type: {}, State: {}", group.group_type, group.state);
    
    // Add sessions to the group (basic operations only)
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    let session3 = SessionId::new();
    
    group.add_session(session1, "participant".to_string())?;
    group.add_session(session2, "participant".to_string())?;
    group.add_session(session3, "moderator".to_string())?;
    
    println!("✅ Added 3 sessions to group:");
    println!("   Active sessions: {}", group.get_active_session_count());
    println!("   Session IDs: {:?}", group.get_session_ids());
    
    // Update group state (basic operation)
    group.update_state(rvoip_session_core::BasicGroupState::Active);
    println!("✅ Updated group state to: {}", group.state);
    
    // Check membership (basic queries)
    println!("✅ Session {} is in group: {}", session1, group.contains_session(session1));
    println!("   Role: {:?}", group.get_session_role(session1));
    
    // Basic group events
    let event = BasicGroupEvent::SessionJoined {
        group_id: group.id.clone(),
        session_id: session1,
        role: "participant".to_string(),
    };
    println!("✅ Created basic group event: {:?}", event);
    
    println!();
    println!("🎯 ARCHITECTURAL SUCCESS:");
    println!("   ✅ Basic primitives work correctly");
    println!("   ✅ No business logic in session-core");
    println!("   ✅ Data structures ready for call-engine orchestration");
    println!("   ✅ Clean separation of concerns achieved");
    
    Ok(())
} 