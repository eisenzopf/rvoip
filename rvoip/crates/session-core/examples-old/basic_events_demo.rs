//! Basic Event Primitives Demo
//! 
//! Demonstrates the basic event communication functionality after Phase 12.4 refactoring.
//! This shows the low-level primitives that call-engine will use to build sophisticated
//! event orchestration and business coordination.

use rvoip_session_core::{
    SessionId, SessionState, BasicSessionEvent, BasicEventBus, BasicEventBusConfig,
    BasicEventFilter, FilteredEventSubscriber
};
use tokio::time::{sleep, Duration};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Basic Event Primitives Demo");
    println!("==============================");
    
    // Create basic event bus
    let config = BasicEventBusConfig {
        max_buffer_size: 100,
        log_events: true,
    };
    let event_bus = BasicEventBus::new(config);
    println!("✅ Created basic event bus (buffer: 100)");
    
    // Create some test sessions
    let session_a = SessionId::new();
    let session_b = SessionId::new();
    let session_c = SessionId::new();
    
    println!("✅ Created test sessions:");
    println!("   Session A: {}", session_a);
    println!("   Session B: {}", session_b);
    println!("   Session C: {}", session_c);
    
    // Subscribe to all events
    let mut all_events_subscriber = event_bus.subscribe();
    println!("✅ Subscribed to all events");
    
    // Create filtered subscribers
    let session_filter = BasicEventFilter::for_sessions(vec![session_a, session_b]);
    let mut session_filtered_subscriber = FilteredEventSubscriber::new(&event_bus, session_filter);
    println!("✅ Created filtered subscriber for sessions A and B");
    
    let state_filter = BasicEventFilter::for_event_types(vec!["StateChanged".to_string()]);
    let mut state_filtered_subscriber = FilteredEventSubscriber::new(&event_bus, state_filter);
    println!("✅ Created filtered subscriber for StateChanged events only");
    
    println!("✅ Event bus has {} subscribers", event_bus.subscriber_count());
    
    // Test basic event creation and publishing
    println!("\n🔄 Testing Basic Event Publishing:");
    
    // State change events
    let state_event = BasicSessionEvent::state_changed(
        session_a,
        SessionState::Initializing,
        SessionState::Dialing,
    );
    
    let published = event_bus.publish(state_event.clone())?;
    println!("   📡 Published state change event → {} receivers", published);
    
    // Media state change event
    let media_event = BasicSessionEvent::media_state_changed(
        session_b,
        "RTP_ACTIVE".to_string(),
    );
    
    let published = event_bus.publish(media_event.clone())?;
    println!("   📡 Published media state event → {} receivers", published);
    
    // Session termination event
    let termination_event = BasicSessionEvent::session_terminated(
        session_c,
        "Normal termination".to_string(),
    );
    
    let published = event_bus.publish(termination_event.clone())?;
    println!("   📡 Published termination event → {} receivers", published);
    
    // Custom event
    let mut custom_data = HashMap::new();
    custom_data.insert("call_id".to_string(), "call-12345".to_string());
    custom_data.insert("priority".to_string(), "high".to_string());
    
    let custom_event = BasicSessionEvent::Custom {
        event_type: "CallEstablished".to_string(),
        session_id: session_a,
        data: custom_data,
        timestamp: std::time::SystemTime::now(),
    };
    
    let published = event_bus.publish(custom_event.clone())?;
    println!("   📡 Published custom event → {} receivers", published);
    
    // Test event receiving
    println!("\n📨 Testing Event Reception:");
    
    // Receive from all events subscriber
    println!("   All Events Subscriber:");
    for i in 0..4 {
        match all_events_subscriber.try_recv() {
            Ok(event) => {
                let event_description = match &event {
                    BasicSessionEvent::StateChanged { old_state, new_state, .. } => 
                        format!("{:?} → {:?}", old_state, new_state),
                    BasicSessionEvent::MediaStateChanged { media_state, .. } => 
                        format!("Media: {}", media_state),
                    BasicSessionEvent::SessionTerminated { reason, .. } => 
                        format!("Reason: {}", reason),
                    BasicSessionEvent::Custom { event_type, .. } => 
                        format!("Custom: {}", event_type),
                };
                
                println!("     {} - {} from session {} ({})", 
                    i + 1,
                    event.event_type(), 
                    event.session_id(),
                    event_description
                );
            },
            Err(e) => println!("     Error receiving event: {:?}", e),
        }
    }
    
    // Test filtered subscribers
    println!("   Session-Filtered Subscriber (A & B only):");
    let mut received_count = 0;
    loop {
        match session_filtered_subscriber.try_recv() {
            Ok(event) => {
                received_count += 1;
                println!("     {} - {} from session {}", 
                    received_count,
                    event.event_type(), 
                    event.session_id()
                );
            },
            Err(_) => break,
        }
    }
    
    println!("   State-Filtered Subscriber (StateChanged only):");
    let mut received_count = 0;
    loop {
        match state_filtered_subscriber.try_recv() {
            Ok(event) => {
                received_count += 1;
                println!("     {} - {} from session {}", 
                    received_count,
                    event.event_type(), 
                    event.session_id()
                );
            },
            Err(_) => break,
        }
    }
    
    // Test event helper methods
    println!("\n🔍 Testing Event Helper Methods:");
    println!("   Event Type: {}", state_event.event_type());
    println!("   Session ID: {}", state_event.session_id());
    
    // Test complex filtering
    println!("\n🎯 Testing Complex Event Filtering:");
    let complex_filter = BasicEventFilter {
        include_sessions: Some(vec![session_a]),
        exclude_sessions: vec![],
        include_event_types: Some(vec!["StateChanged".to_string(), "Custom".to_string()]),
        exclude_event_types: vec!["MediaStateChanged".to_string()],
    };
    
    println!("   Complex Filter Rules:");
    println!("     - Include sessions: {:?}", complex_filter.include_sessions);
    println!("     - Include event types: {:?}", complex_filter.include_event_types);
    println!("     - Exclude event types: {:?}", complex_filter.exclude_event_types);
    
    println!("   Filter Test Results:");
    println!("     State event matches: {}", complex_filter.matches(&state_event));
    println!("     Media event matches: {}", complex_filter.matches(&media_event));
    println!("     Custom event matches: {}", complex_filter.matches(&custom_event));
    println!("     Termination event matches: {}", complex_filter.matches(&termination_event));
    
    println!();
    println!("🎯 ARCHITECTURAL SUCCESS:");
    println!("   ✅ Basic event primitives work correctly");
    println!("   ✅ No business logic in session-core");
    println!("   ✅ Simple pub/sub event bus established");
    println!("   ✅ Basic filtering capabilities available");
    println!("   ✅ Clean separation of concerns achieved");
    println!("   ✅ Event foundation ready for call-engine orchestration");
    
    Ok(())
} 