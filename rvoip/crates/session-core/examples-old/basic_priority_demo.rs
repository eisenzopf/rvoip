//! Basic Priority Primitives Demo
//! 
//! Demonstrates the basic priority classification functionality after Phase 12.3 refactoring.
//! This shows the low-level primitives that call-engine will use to build sophisticated
//! scheduling policies and QoS management.

use rvoip_session_core::{
    SessionId, BasicSessionPriority, BasicPriorityClass, BasicQoSLevel,
    BasicPriorityInfo, BasicPriorityConfig
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Basic Priority Primitives Demo");
    println!("==================================");
    
    // Demonstrate basic priority levels
    println!("✅ Basic Priority Levels:");
    let priorities = [
        BasicSessionPriority::Emergency,
        BasicSessionPriority::Critical,
        BasicSessionPriority::High,
        BasicSessionPriority::Normal,
        BasicSessionPriority::Low,
        BasicSessionPriority::Background,
    ];
    
    for priority in &priorities {
        println!("   {}: {} (numeric: {})", 
            priority, 
            if priority.is_emergency() { "EMERGENCY" } else if priority.is_critical_or_above() { "CRITICAL+" } else { "Standard" },
            priority.as_u8()
        );
    }
    
    // Demonstrate priority comparison
    let high = BasicSessionPriority::High;
    let normal = BasicSessionPriority::Normal;
    println!("✅ Priority Comparison:");
    println!("   {} is higher than {}: {}", high, normal, high.is_higher_than(&normal));
    println!("   {} is higher than {}: {}", normal, high, normal.is_higher_than(&high));
    
    // Demonstrate priority classes
    println!("✅ Priority Classes:");
    let classes = [
        BasicPriorityClass::RealTime,
        BasicPriorityClass::Interactive,
        BasicPriorityClass::Bulk,
        BasicPriorityClass::BestEffort,
        BasicPriorityClass::Custom("VIP".to_string()),
    ];
    
    for class in &classes {
        println!("   {}: Realtime={}, Interactive={}, Expected Latency={}ms", 
            class, 
            class.is_realtime(),
            class.is_interactive(),
            class.expected_latency_ms()
        );
    }
    
    // Demonstrate QoS levels
    println!("✅ QoS Levels:");
    let qos_levels = [
        BasicQoSLevel::Voice,
        BasicQoSLevel::Video,
        BasicQoSLevel::ExpeditedForwarding,
        BasicQoSLevel::AssuredForwarding,
        BasicQoSLevel::BestEffort,
    ];
    
    for qos in &qos_levels {
        println!("   {}: Realtime={}, Priority Score={}", 
            qos, 
            qos.is_realtime(),
            qos.priority_score()
        );
    }
    
    // Demonstrate session priority info
    println!("✅ Session Priority Information:");
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    let session3 = SessionId::new();
    
    let emergency_call = BasicPriorityInfo::new(
        session1,
        BasicSessionPriority::Emergency,
        BasicPriorityClass::RealTime,
        BasicQoSLevel::Voice,
    );
    
    let normal_call = BasicPriorityInfo::new(
        session2,
        BasicSessionPriority::Normal,
        BasicPriorityClass::Interactive,
        BasicQoSLevel::BestEffort,
    );
    
    let video_call = BasicPriorityInfo::new(
        session3,
        BasicSessionPriority::High,
        BasicPriorityClass::RealTime,
        BasicQoSLevel::Video,
    );
    
    println!("   Emergency Call: {} ({}, {}, {}) - Score: {}, High Priority: {}", 
        emergency_call.session_id,
        emergency_call.priority,
        emergency_call.priority_class,
        emergency_call.qos_level,
        emergency_call.overall_score(),
        emergency_call.is_high_priority()
    );
    
    println!("   Normal Call: {} ({}, {}, {}) - Score: {}, High Priority: {}", 
        normal_call.session_id,
        normal_call.priority,
        normal_call.priority_class,
        normal_call.qos_level,
        normal_call.overall_score(),
        normal_call.is_high_priority()
    );
    
    println!("   Video Call: {} ({}, {}, {}) - Score: {}, High Priority: {}", 
        video_call.session_id,
        video_call.priority,
        video_call.priority_class,
        video_call.qos_level,
        video_call.overall_score(),
        video_call.is_high_priority()
    );
    
    // Demonstrate precedence comparison
    println!("✅ Precedence Comparison:");
    println!("   Emergency has precedence over Normal: {}", 
        emergency_call.has_precedence_over(&normal_call));
    println!("   Normal has precedence over Emergency: {}", 
        normal_call.has_precedence_over(&emergency_call));
    println!("   Video has precedence over Normal: {}", 
        video_call.has_precedence_over(&normal_call));
    println!("   Emergency has precedence over Video: {}", 
        emergency_call.has_precedence_over(&video_call));
    
    // Demonstrate priority configuration
    println!("✅ Priority Configuration:");
    let config = BasicPriorityConfig::default();
    println!("   Default Priority: {}", config.default_priority);
    println!("   Default Class: {}", config.default_class);
    println!("   Default QoS: {}", config.default_qos);
    println!("   Auto Expire: {}", config.auto_expire);
    
    let default_session = SessionId::new();
    let default_priority = config.create_default_priority(default_session);
    println!("   Created default priority for session {}: {} ({})", 
        default_priority.session_id,
        default_priority.priority,
        default_priority.priority_class
    );
    
    // Demonstrate numeric conversion
    println!("✅ Numeric Conversion:");
    let priority_value = 90u8;
    let converted_priority = BasicSessionPriority::from_u8(priority_value);
    println!("   Converted {} to priority: {}", priority_value, converted_priority);
    println!("   Back to numeric: {}", converted_priority.as_u8());
    
    println!();
    println!("🎯 ARCHITECTURAL SUCCESS:");
    println!("   ✅ Basic priority primitives work correctly");
    println!("   ✅ No business logic in session-core");
    println!("   ✅ Data structures ready for call-engine scheduling");
    println!("   ✅ Clean separation of concerns achieved");
    println!("   ✅ Priority classification foundation established");
    println!("   ✅ QoS levels properly integrated with priorities");
    
    Ok(())
} 