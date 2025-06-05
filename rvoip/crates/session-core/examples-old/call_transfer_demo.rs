//! # Call Transfer Demonstration with Zero-Copy Events
//!
//! This example demonstrates the call transfer functionality implemented in session-core,
//! including both blind and attended transfers using the REFER method with the high-performance
//! zero-copy event system.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, error};

use rvoip_session_core::{
    SessionManager, SessionConfig, SessionId, SessionState,
    TransferType, SessionDirection, EventBus, SessionEvent,
    events::EventFilters,
};

// Mock transaction manager for the example
struct MockTransactionManager;

impl MockTransactionManager {
    fn new() -> Self {
        Self
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    
    info!("�� Call Transfer Demo with Zero-Copy Events Starting");
    
    // Create high-performance zero-copy event bus
    let event_bus = EventBus::new(1000).await?;
    info!("✅ Zero-copy event system initialized");
    
    // For this demo, we'll show the API structure and event handling
    demo_zero_copy_event_system(&event_bus).await?;
    demo_transfer_api_structure().await?;
    demo_transfer_event_handling(&event_bus).await?;
    
    // Shutdown the event system
    event_bus.shutdown().await?;
    info!("✅ Call Transfer Demo with Zero-Copy Events Completed Successfully");
    Ok(())
}

/// Demonstrate the zero-copy event system capabilities
async fn demo_zero_copy_event_system(event_bus: &EventBus) -> Result<(), Box<dyn std::error::Error>> {
    info!("🚀 === ZERO-COPY EVENT SYSTEM DEMO ===");
    
    let session_id = SessionId::new();
    
    // Create a subscriber for transfer events only
    let mut transfer_subscriber = event_bus.subscribe().await?;
    
    // Publish some transfer events to demonstrate zero-copy performance
    let transfer_events = vec![
        SessionEvent::TransferInitiated {
            session_id: session_id.clone(),
            transfer_id: "transfer-001".to_string(),
            transfer_type: "Blind".to_string(),
            target_uri: "sip:alice@example.com".to_string(),
        },
        SessionEvent::TransferProgress {
            session_id: session_id.clone(),
            transfer_id: "transfer-001".to_string(),
            status: "100 Trying".to_string(),
        },
        SessionEvent::TransferProgress {
            session_id: session_id.clone(),
            transfer_id: "transfer-001".to_string(),
            status: "180 Ringing".to_string(),
        },
        SessionEvent::TransferCompleted {
            session_id: session_id.clone(),
            transfer_id: "transfer-001".to_string(),
            final_status: "200 OK".to_string(),
        },
    ];
    
    info!("📤 Publishing batch of {} transfer events using zero-copy system", transfer_events.len());
    
    // Batch publish for optimal performance
    event_bus.publish_batch(transfer_events).await?;
    
    // Receive and process events
    info!("📥 Receiving events with zero-copy performance:");
    for i in 0..4 {
        match transfer_subscriber.receive_timeout(Duration::from_millis(100)).await {
            Ok(event) => {
                let priority = event.priority();
                info!("   Event {}: {:?} (Priority: {:?})", i + 1, 
                      match event.as_ref() {
                          SessionEvent::TransferInitiated { transfer_type, target_uri, .. } => 
                              format!("Transfer Initiated: {} -> {}", transfer_type, target_uri),
                          SessionEvent::TransferProgress { status, .. } => 
                              format!("Transfer Progress: {}", status),
                          SessionEvent::TransferCompleted { final_status, .. } => 
                              format!("Transfer Completed: {}", final_status),
                          _ => "Other Event".to_string(),
                      }, priority);
            },
            Err(e) => error!("Failed to receive event: {}", e),
        }
    }
    
    info!("✅ Zero-copy event system demonstration completed");
    Ok(())
}

/// Demonstrate the call transfer API structure
async fn demo_transfer_api_structure() -> Result<(), Box<dyn std::error::Error>> {
    info!("🔄 === CALL TRANSFER API DEMONSTRATION ===");
    
    // In a real application, you would:
    // 1. Create a real transaction manager with transport
    // 2. Create sessions from actual SIP dialogs
    // 3. Process real REFER requests and responses
    
    // Show the structure of transfer operations
    info!("📋 Transfer Operations Available:");
    info!("   ✅ initiate_transfer() - Start a blind, attended, or consultative transfer");
    info!("   ✅ send_refer_request() - Build and send REFER SIP request");
    info!("   ✅ handle_refer_request() - Process incoming REFER requests");
    info!("   ✅ send_refer_accepted() - Send 202 Accepted response");
    info!("   ✅ create_consultation_call() - Create consultation session for attended transfer");
    info!("   ✅ complete_attended_transfer() - Connect transferor and transferee");
    info!("   ✅ handle_transfer_notify() - Process NOTIFY for transfer progress");
    info!("   ✅ send_transfer_notify() - Send transfer progress notifications");
    info!("   ✅ cancel_transfer() - Cancel an ongoing transfer");
    info!("   ✅ get_sessions_with_transfers() - Get sessions with active transfers");
    
    info!("📋 Transfer Types Supported:");
    info!("   • Blind Transfer - Direct transfer without consultation");
    info!("   • Attended Transfer - Transfer after speaking with target");
    info!("   • Consultative Transfer - Transfer with confirmation");
    
    info!("📋 SIP Methods Implemented:");
    info!("   • REFER - RFC 3515 call transfer method");
    info!("   • NOTIFY - Transfer progress notifications");
    info!("   • Replaces parameter - For attended transfers");
    
    info!("📋 Zero-Copy Event Features:");
    info!("   • High-performance sharded event distribution");
    info!("   • Priority-based event processing");
    info!("   • Batch publishing for optimal throughput");
    info!("   • Filtered subscriptions for specific event types");
    info!("   • Configurable timeouts and backpressure handling");
    
    // Demonstrate transfer type usage
    demonstrate_transfer_types().await?;
    
    Ok(())
}

/// Demonstrate transfer event handling with zero-copy system
async fn demo_transfer_event_handling(event_bus: &EventBus) -> Result<(), Box<dyn std::error::Error>> {
    info!("📡 === TRANSFER EVENT HANDLING WITH ZERO-COPY ===");
    
    let session_id = SessionId::new();
    
    // Create filtered subscribers for different event types
    let mut all_events_subscriber = event_bus.subscribe().await?;
    let mut transfer_only_subscriber = event_bus.subscribe().await?;
    
    // Publish a mix of events
    let mixed_events = vec![
        SessionEvent::Created { session_id: session_id.clone() },
        SessionEvent::TransferInitiated {
            session_id: session_id.clone(),
            transfer_id: "demo-transfer".to_string(),
            transfer_type: "Attended".to_string(),
            target_uri: "sip:support@example.com".to_string(),
        },
        SessionEvent::StateChanged {
            session_id: session_id.clone(),
            old_state: SessionState::Connected,
            new_state: SessionState::Transferring,
        },
        SessionEvent::ConsultationCallCreated {
            original_session_id: session_id.clone(),
            consultation_session_id: SessionId::new(),
            transfer_id: "demo-transfer".to_string(),
        },
        SessionEvent::TransferCompleted {
            session_id: session_id.clone(),
            transfer_id: "demo-transfer".to_string(),
            final_status: "200 OK".to_string(),
        },
    ];
    
    info!("📤 Publishing mixed events to demonstrate filtering");
    event_bus.publish_batch(mixed_events).await?;
    
    // Process all events
    info!("📥 All events subscriber:");
    for i in 0..5 {
        if let Ok(event) = all_events_subscriber.receive_timeout(Duration::from_millis(50)).await {
            let is_transfer = EventFilters::transfers_only()(&event);
            info!("   Event {}: {} (Transfer: {})", i + 1, 
                  event_type_name(&event), is_transfer);
        }
    }
    
    // Process only transfer events (client-side filtering)
    info!("📥 Transfer-only events (filtered):");
    let mut transfer_count = 0;
    for _ in 0..5 {
        if let Ok(event) = transfer_only_subscriber.receive_timeout(Duration::from_millis(50)).await {
            if EventFilters::transfers_only()(&event) {
                transfer_count += 1;
                info!("   Transfer Event {}: {}", transfer_count, event_type_name(&event));
            }
        }
    }
    
    info!("✅ Event handling demonstration completed");
    Ok(())
}

/// Get a human-readable name for an event type
fn event_type_name(event: &SessionEvent) -> &'static str {
    match event {
        SessionEvent::Created { .. } => "Session Created",
        SessionEvent::StateChanged { .. } => "State Changed",
        SessionEvent::TransferInitiated { .. } => "Transfer Initiated",
        SessionEvent::TransferProgress { .. } => "Transfer Progress",
        SessionEvent::TransferCompleted { .. } => "Transfer Completed",
        SessionEvent::TransferFailed { .. } => "Transfer Failed",
        SessionEvent::ConsultationCallCreated { .. } => "Consultation Call Created",
        SessionEvent::ConsultationCallCompleted { .. } => "Consultation Call Completed",
        _ => "Other Event",
    }
}

/// Demonstrate different transfer types
async fn demonstrate_transfer_types() -> Result<(), Box<dyn std::error::Error>> {
    info!("🔄 === TRANSFER TYPE DEMONSTRATIONS ===");
    
    // Blind Transfer Example
    info!("📤 === BLIND TRANSFER ===");
    info!("   1. Alice is talking to Bob");
    info!("   2. Alice decides to transfer Bob to Carol");
    info!("   3. Alice calls: session_manager.initiate_transfer(session_id, \"sip:carol@example.com\", TransferType::Blind)");
    info!("   4. REFER request sent to Bob with Refer-To: <sip:carol@example.com>");
    info!("   5. Bob receives 202 Accepted, then calls Carol");
    info!("   6. Alice's call with Bob ends, Bob talks to Carol");
    info!("   7. Zero-copy events: TransferInitiated → TransferProgress → TransferCompleted");
    
    sleep(Duration::from_millis(100)).await;
    
    // Attended Transfer Example
    info!("📞 === ATTENDED TRANSFER ===");
    info!("   1. Alice is talking to Bob");
    info!("   2. Alice creates consultation call to Carol");
    info!("   3. Alice calls: session_manager.create_consultation_call(alice_bob_session, \"sip:carol@example.com\")");
    info!("   4. Alice talks to Carol to explain the transfer");
    info!("   5. Alice calls: session_manager.initiate_transfer(session_id, target, TransferType::Attended)");
    info!("   6. REFER request sent with Replaces parameter referencing Alice-Carol call");
    info!("   7. Bob calls Carol with Replaces header, replacing Alice");
    info!("   8. Bob and Carol are now connected, Alice is out of the call");
    info!("   9. Zero-copy events: ConsultationCallCreated → TransferInitiated → TransferCompleted");
    
    sleep(Duration::from_millis(100)).await;
    
    // Consultative Transfer Example
    info!("🔄 === CONSULTATIVE TRANSFER ===");
    info!("   1. Alice is talking to Bob");
    info!("   2. Alice wants to transfer but get confirmation first");
    info!("   3. Alice calls: session_manager.initiate_transfer(session_id, target, TransferType::Consultative)");
    info!("   4. REFER request sent with consultation semantics");
    info!("   5. Target is contacted and asked to confirm acceptance");
    info!("   6. Transfer proceeds only after confirmation");
    info!("   7. Zero-copy events: TransferInitiated → TransferProgress → (TransferCompleted | TransferFailed)");
    
    sleep(Duration::from_millis(100)).await;
    
    // Progress Tracking
    info!("📈 === TRANSFER PROGRESS TRACKING ===");
    info!("   • 100 Trying - Transfer attempt started (Low Priority)");
    info!("   • 180 Ringing - Target is ringing (Low Priority)");
    info!("   • 200 OK - Transfer completed successfully (Normal Priority)");
    info!("   • 4xx/5xx/6xx - Transfer failed with specific reason (High Priority)");
    info!("   • Updates sent via NOTIFY with message/sipfrag body");
    info!("   • Zero-copy system ensures minimal latency for critical events");
    
    sleep(Duration::from_millis(100)).await;
    
    info!("✅ Transfer type demonstrations completed");
    
    Ok(())
}

/// Helper function showing how to handle incoming REFER requests
#[allow(dead_code)]
async fn demonstrate_refer_handling() -> Result<(), Box<dyn std::error::Error>> {
    info!("📥 === INCOMING REFER REQUEST HANDLING ===");
    
    info!("   When receiving a REFER request:");
    info!("   1. Extract Refer-To header to get target URI");
    info!("   2. Check for Replaces parameter to determine transfer type");
    info!("   3. Create transfer context and validate request");
    info!("   4. Send 202 Accepted response");
    info!("   5. Initiate outbound call to target");
    info!("   6. Send NOTIFY messages with progress updates");
    info!("   7. Complete or fail the transfer based on result");
    
    Ok(())
}

/// Example of working with transfer events
#[allow(dead_code)]
async fn demonstrate_transfer_events() -> Result<(), Box<dyn std::error::Error>> {
    info!("📡 === TRANSFER EVENT HANDLING ===");
    
    info!("   Available transfer events:");
    info!("   • TransferInitiated {{ session_id, transfer_id, transfer_type, target_uri }}");
    info!("   • TransferAccepted {{ session_id, transfer_id }}");
    info!("   • TransferProgress {{ session_id, transfer_id, status }}");
    info!("   • TransferCompleted {{ session_id, transfer_id, final_status }}");
    info!("   • TransferFailed {{ session_id, transfer_id, reason }}");
    info!("   • ConsultationCallCreated {{ original_session_id, consultation_session_id, transfer_id }}");
    info!("   • ConsultationCallCompleted {{ original_session_id, consultation_session_id, transfer_id, success }}");
    
    info!("   To handle events:");
    info!("   let mut event_rx = event_bus.subscribe();");
    info!("   while let Ok(event) = event_rx.recv().await {{");
    info!("       match event {{");
    info!("           SessionEvent::TransferInitiated {{ .. }} => {{ /* handle */ }},");
    info!("           SessionEvent::TransferCompleted {{ .. }} => {{ /* handle */ }},");
    info!("           _ => {{}}");
    info!("       }}");
    info!("   }}");
    
    Ok(())
} 