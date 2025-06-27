# Session-Core Cookbook

A practical guide with recipes for common VoIP scenarios using the session-core API.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Making Outgoing Calls](#making-outgoing-calls)
3. [Handling Incoming Calls](#handling-incoming-calls)
4. [Media Control](#media-control)
5. [Call Features](#call-features)
6. [Bridge Management](#bridge-management)
7. [Advanced Patterns](#advanced-patterns)
8. [Error Handling](#error-handling)
9. [Performance & Monitoring](#performance--monitoring)

## Getting Started

### Basic Setup

```rust
use rvoip_session_core::api::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("rvoip=debug")
        .init();
    
    // Build a session coordinator
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:user@192.168.1.100:5060")
        .with_rtp_port_range(10000, 20000)  // RTP port range
        .with_handler(Arc::new(AutoAnswerHandler))
        .build()
        .await?;
    
    // Start the coordinator
    SessionControl::start(&coordinator).await?;
    
    // Your app logic here...
    
    // Clean shutdown
    SessionControl::stop(&coordinator).await?;
    Ok(())
}
```

## Making Outgoing Calls

### Recipe 1: Simple Outgoing Call

```rust
async fn make_simple_call(coordinator: &Arc<SessionCoordinator>) -> Result<()> {
    // Create call with auto-generated SDP
    let session = SessionControl::create_outgoing_call(
        coordinator,
        "sip:alice@example.com",     // from
        "sip:bob@192.168.1.100",     // to
        None                         // SDP will be auto-generated
    ).await?;
    
    println!("Call created: {}", session.id());
    
    // Wait for answer (with timeout)
    SessionControl::wait_for_answer(
        coordinator, 
        session.id(), 
        Duration::from_secs(30)
    ).await?;
    
    println!("Call answered!");
    
    // Call is now active, media flows automatically
    Ok(())
}
```

### Recipe 2: Call with Prepared Media

```rust
async fn make_call_with_prepared_media(coordinator: &Arc<SessionCoordinator>) -> Result<()> {
    // Prepare call (allocates RTP port and generates SDP)
    let prepared = SessionControl::prepare_outgoing_call(
        coordinator,
        "sip:alice@example.com",
        "sip:bob@192.168.1.100"
    ).await?;
    
    println!("Allocated RTP port: {}", prepared.local_rtp_port);
    println!("Generated SDP:\n{}", prepared.sdp_offer);
    
    // Initiate the call with prepared resources
    let session = SessionControl::initiate_prepared_call(
        coordinator,
        &prepared
    ).await?;
    
    // Wait for answer
    SessionControl::wait_for_answer(
        coordinator,
        &session.id,
        Duration::from_secs(30)
    ).await?;
    
    Ok(())
}
```

## Handling Incoming Calls

### Recipe 3: Simple Auto-Accept Handler

```rust
#[derive(Debug)]
struct SimpleAcceptHandler;

#[async_trait::async_trait]
impl CallHandler for SimpleAcceptHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        println!("Incoming call from: {}", call.from);
        
        // Accept all calls
        CallDecision::Accept(None)  // SDP answer will be auto-generated
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call {} ended: {}", call.id(), reason);
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        println!("Call {} established", call.id());
        
        // Media flow is automatically set up by session-core
        // No need to manually establish media flow
    }
}
```

### Recipe 4: Conditional Accept Handler

```rust
#[derive(Debug)]
struct ConditionalHandler {
    allowed_domains: Vec<String>,
}

#[async_trait::async_trait]
impl CallHandler for ConditionalHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Extract domain from caller
        let caller_domain = call.from.split('@').nth(1).unwrap_or("");
        
        if self.allowed_domains.iter().any(|d| caller_domain.contains(d)) {
            println!("Accepting call from trusted domain: {}", caller_domain);
            CallDecision::Accept(None)
        } else {
            println!("Rejecting call from untrusted domain: {}", caller_domain);
            CallDecision::Reject("Untrusted domain".to_string())
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call ended: {}", reason);
    }
}
```

### Recipe 5: Deferred Decision with Database Lookup

```rust
#[derive(Debug)]
struct DatabaseHandler {
    pending_calls: Arc<Mutex<Vec<IncomingCall>>>,
}

#[async_trait::async_trait]
impl CallHandler for DatabaseHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        println!("Deferring call from {} for database lookup", call.from);
        
        // Store for async processing
        self.pending_calls.lock().unwrap().push(call);
        
        CallDecision::Defer
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        // Update call records
        update_call_record(&call.id, reason).await;
    }
}

// Process deferred calls in background
async fn process_pending_calls(
    coordinator: Arc<SessionCoordinator>,
    handler: Arc<DatabaseHandler>
) {
    loop {
        // Get pending calls
        let calls = {
            let mut pending = handler.pending_calls.lock().unwrap();
            pending.drain(..).collect::<Vec<_>>()
        };
        
        for call in calls {
            // Database lookup
            match lookup_caller_in_database(&call.from).await {
                Ok(caller_info) if caller_info.is_authorized => {
                    // Generate SDP answer if needed
                    let sdp_answer = if let Some(offer) = &call.sdp {
                        Some(MediaControl::generate_sdp_answer(
                            &coordinator,
                            &call.id,
                            offer
                        ).await?)
                    } else {
                        None
                    };
                    
                    // Accept the call
                    SessionControl::accept_incoming_call(
                        &coordinator,
                        &call,
                        sdp_answer
                    ).await?;
                    
                    println!("Accepted call from authorized user: {}", call.from);
                }
                _ => {
                    // Reject unauthorized
                    SessionControl::reject_incoming_call(
                        &coordinator,
                        &call,
                        "Not authorized"
                    ).await?;
                    
                    println!("Rejected unauthorized call from: {}", call.from);
                }
            }
        }
        
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

## Media Control

### Recipe 6: Manual Media Control

```rust
async fn control_media_during_call(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId
) -> Result<()> {
    // Get current media info
    let media_info = MediaControl::get_media_info(coordinator, session_id).await?;
    
    if let Some(info) = media_info {
        println!("Local RTP port: {:?}", info.local_rtp_port);
        println!("Remote RTP port: {:?}", info.remote_rtp_port);
        println!("Codec: {:?}", info.codec);
    }
    
    // Mute audio (stop transmission)
    MediaControl::stop_audio_transmission(coordinator, session_id).await?;
    println!("Audio muted");
    
    // Wait a bit
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    // Unmute audio (resume transmission)
    MediaControl::start_audio_transmission(coordinator, session_id).await?;
    println!("Audio unmuted");
    
    Ok(())
}
```

### Recipe 7: Quality Monitoring

```rust
async fn monitor_call_quality(
    coordinator: Arc<SessionCoordinator>,
    session_id: SessionId
) -> Result<()> {
    // Start automatic monitoring every 5 seconds
    MediaControl::start_statistics_monitoring(
        &coordinator,
        &session_id,
        Duration::from_secs(5)
    ).await?;
    
    // Also do manual checks
    let mut poor_quality_count = 0;
    
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        
        let stats = MediaControl::get_media_statistics(&coordinator, &session_id).await?;
        
        if let Some(stats) = stats {
            if let Some(quality) = stats.quality_metrics {
                let mos = quality.mos_score.unwrap_or(0.0);
                
                println!("Call Quality Report:");
                println!("  MOS Score: {:.1} ({})", mos, match mos {
                    x if x >= 4.0 => "Excellent",
                    x if x >= 3.5 => "Good",
                    x if x >= 3.0 => "Fair",
                    x if x >= 2.5 => "Poor",
                    _ => "Bad"
                });
                println!("  Packet Loss: {:.1}%", quality.packet_loss_percent);
                println!("  Jitter: {:.1}ms", quality.jitter_ms);
                println!("  RTT: {:.0}ms", quality.round_trip_time_ms);
                
                if mos < 3.0 {
                    poor_quality_count += 1;
                    if poor_quality_count >= 3 {
                        println!("⚠️  Sustained poor quality detected!");
                        // Take action: notify user, switch codec, etc.
                    }
                } else {
                    poor_quality_count = 0;
                }
            }
        }
        
        // Check if call is still active
        if let Ok(Some(session)) = SessionControl::get_session(&coordinator, &session_id).await {
            if session.state().is_final() {
                break;
            }
        } else {
            break;
        }
    }
    
    Ok(())
}
```

## Call Features

### Recipe 8: Call Hold/Resume

```rust
async fn demonstrate_hold_resume(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId
) -> Result<()> {
    // Put call on hold
    SessionControl::hold_session(coordinator, session_id).await?;
    println!("Call is now on hold");
    
    // Do something else...
    tokio::time::sleep(Duration::from_secs(5)).await;
    
    // Resume the call
    SessionControl::resume_session(coordinator, session_id).await?;
    println!("Call resumed");
    
    Ok(())
}
```

### Recipe 9: DTMF Tones

```rust
async fn send_dtmf_menu_selection(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId
) -> Result<()> {
    // Wait for menu prompt
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Press 1 for English
    SessionControl::send_dtmf(coordinator, session_id, "1").await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Press 2 for Sales
    SessionControl::send_dtmf(coordinator, session_id, "2").await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Enter account number
    SessionControl::send_dtmf(coordinator, session_id, "1234567890#").await?;
    
    println!("DTMF sequence sent");
    Ok(())
}
```

### Recipe 10: Call Transfer

```rust
async fn transfer_call_example(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId
) -> Result<()> {
    // Blind transfer to another extension
    SessionControl::transfer_session(
        coordinator,
        session_id,
        "sip:support@example.com"
    ).await?;
    
    println!("Call transferred to support");
    
    // The original call will be terminated after successful transfer
    Ok(())
}
```

## Bridge Management

### Recipe 11: Simple Two-Party Bridge

```rust
async fn bridge_two_calls(
    coordinator: Arc<SessionCoordinator>
) -> Result<()> {
    // Create first call
    let call1 = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:bridge@server.com",
        "sip:alice@example.com",
        None
    ).await?;
    
    // Create second call
    let call2 = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:bridge@server.com",
        "sip:bob@example.com",
        None
    ).await?;
    
    // Wait for both to answer
    SessionControl::wait_for_answer(&coordinator, &call1.id, Duration::from_secs(30)).await?;
    SessionControl::wait_for_answer(&coordinator, &call2.id, Duration::from_secs(30)).await?;
    
    // Bridge them together
    let bridge_id = coordinator.bridge_sessions(&call1.id, &call2.id).await?;
    println!("Created bridge: {}", bridge_id);
    
    // Monitor bridge events
    let mut events = coordinator.subscribe_to_bridge_events().await;
    
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                BridgeEvent::ParticipantAdded { bridge_id, session_id } => {
                    println!("Session {} joined bridge {}", session_id, bridge_id);
                }
                BridgeEvent::ParticipantRemoved { bridge_id, session_id, reason } => {
                    println!("Session {} left bridge {}: {}", session_id, bridge_id, reason);
                }
                BridgeEvent::BridgeDestroyed { bridge_id } => {
                    println!("Bridge {} destroyed", bridge_id);
                    break;
                }
            }
        }
    });
    
    Ok(())
}
```

### Recipe 12: Call Center Agent Bridge

```rust
async fn connect_customer_to_agent(
    coordinator: Arc<SessionCoordinator>,
    customer_session_id: SessionId,
    agent_uri: &str
) -> Result<BridgeId> {
    // Call the agent
    let agent_session = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:callcenter@server.com",
        agent_uri,
        None
    ).await?;
    
    // Wait for agent to answer
    match SessionControl::wait_for_answer(
        &coordinator,
        &agent_session.id,
        Duration::from_secs(20)
    ).await {
        Ok(_) => {
            // Bridge customer and agent
            let bridge_id = coordinator.bridge_sessions(
                &customer_session_id,
                &agent_session.id
            ).await?;
            
            println!("Customer connected to agent");
            Ok(bridge_id)
        }
        Err(_) => {
            // Agent didn't answer, try next agent
            SessionControl::terminate_session(&coordinator, &agent_session.id).await?;
            Err(anyhow::anyhow!("Agent unavailable"))
        }
    }
}
```

## Advanced Patterns

### Recipe 13: Composite Handler Chain

```rust
use std::sync::Arc;

async fn create_advanced_call_handler() -> Arc<CompositeHandler> {
    let composite = CompositeHandler::new()
        // First: Check blacklist
        .add_handler(Arc::new(BlacklistHandler {
            blocked_numbers: vec![
                "sip:spam@example.com".to_string(),
                "sip:*@blocked-domain.com".to_string(),
            ],
        }))
        // Then: Check business hours
        .add_handler(Arc::new(BusinessHoursHandler {
            start_hour: 9,
            end_hour: 17,
            timezone: "America/New_York".to_string(),
        }))
        // Then: Route by destination
        .add_handler(Arc::new({
            let mut router = RoutingHandler::new();
            router.add_route("sip:support@*", "sip:queue@support.internal");
            router.add_route("sip:sales@*", "sip:queue@sales.internal");
            router.add_route("sip:+1800*", "sip:tollfree@gateway.internal");
            router.set_default_action(CallDecision::Forward("sip:operator@default.internal".to_string()));
            router
        }))
        // Finally: Queue any remaining
        .add_handler(Arc::new(QueueHandler::new(100)));
    
    Arc::new(composite)
}

// Custom blacklist handler
#[derive(Debug)]
struct BlacklistHandler {
    blocked_numbers: Vec<String>,
}

#[async_trait::async_trait]
impl CallHandler for BlacklistHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        for blocked in &self.blocked_numbers {
            if blocked.contains('*') {
                // Wildcard matching
                let pattern = blocked.replace("*", "");
                if call.from.contains(&pattern) {
                    return CallDecision::Reject("Blocked number".to_string());
                }
            } else if call.from == *blocked {
                return CallDecision::Reject("Blocked number".to_string());
            }
        }
        
        // Not blocked, let next handler decide
        CallDecision::Defer
    }
    
    async fn on_call_ended(&self, _call: CallSession, _reason: &str) {}
}
```

### Recipe 14: Failover Pattern

```rust
async fn call_with_failover(
    coordinator: Arc<SessionCoordinator>,
    destinations: Vec<&str>
) -> Result<CallSession> {
    let mut last_error = None;
    
    for (idx, dest) in destinations.iter().enumerate() {
        println!("Trying destination {} of {}: {}", idx + 1, destinations.len(), dest);
        
        match SessionControl::create_outgoing_call(
            &coordinator,
            "sip:system@local",
            dest,
            None
        ).await {
            Ok(session) => {
                // Try to wait for answer with short timeout
                match SessionControl::wait_for_answer(
                    &coordinator,
                    &session.id,
                    Duration::from_secs(15)
                ).await {
                    Ok(_) => {
                        println!("Successfully connected to {}", dest);
                        return Ok(session);
                    }
                    Err(e) => {
                        println!("Destination {} didn't answer: {}", dest, e);
                        SessionControl::terminate_session(&coordinator, &session.id).await?;
                        last_error = Some(e);
                    }
                }
            }
            Err(e) => {
                println!("Failed to call {}: {}", dest, e);
                last_error = Some(e);
            }
        }
        
        // Brief delay before next attempt
        if idx < destinations.len() - 1 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("All destinations failed")))
}
```

## Error Handling

### Recipe 15: Comprehensive Error Handling

```rust
use rvoip_session_core::errors::SessionError;

async fn handle_call_with_retry(
    coordinator: Arc<SessionCoordinator>,
    destination: &str,
    max_retries: u32
) -> Result<CallSession> {
    let mut retries = 0;
    
    loop {
        match SessionControl::create_outgoing_call(
            &coordinator,
            "sip:app@local",
            destination,
            None
        ).await {
            Ok(session) => return Ok(session),
            Err(e) => {
                // Pattern match on specific errors
                match e.downcast_ref::<SessionError>() {
                    Some(SessionError::ResourceExhausted) => {
                        if retries < max_retries {
                            println!("No resources available, waiting before retry...");
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            retries += 1;
                            continue;
                        }
                    }
                    Some(SessionError::InvalidUri(uri)) => {
                        // Can't retry invalid URI
                        return Err(anyhow::anyhow!("Invalid SIP URI: {}", uri));
                    }
                    Some(SessionError::TransportError(msg)) => {
                        if retries < max_retries && msg.contains("timeout") {
                            println!("Network timeout, retrying...");
                            retries += 1;
                            continue;
                        }
                    }
                    _ => {}
                }
                
                // Unrecoverable error or max retries reached
                return Err(e);
            }
        }
    }
}
```

## Performance & Monitoring

### Recipe 16: Call Statistics Dashboard

```rust
async fn run_statistics_monitor(
    coordinator: Arc<SessionCoordinator>
) {
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    
    loop {
        interval.tick().await;
        
        // Get overall statistics
        match SessionControl::get_stats(&coordinator).await {
            Ok(stats) => {
                println!("\n📊 Session Statistics:");
                println!("  Active Sessions: {}", stats.active_sessions);
                println!("  Total Sessions: {}", stats.total_sessions);
                println!("  Failed Sessions: {}", stats.failed_sessions);
                
                if let Some(avg_duration) = stats.average_duration {
                    println!("  Average Duration: {:?}", avg_duration);
                }
            }
            Err(e) => eprintln!("Failed to get statistics: {}", e),
        }
        
        // Get active session details
        match SessionControl::list_active_sessions(&coordinator).await {
            Ok(sessions) => {
                for session_id in sessions {
                    if let Ok(Some(stats)) = MediaControl::get_media_statistics(
                        &coordinator,
                        &session_id
                    ).await {
                        if let Some(quality) = stats.quality_metrics {
                            println!("\n  Session {}: MOS={:.1}, Loss={:.1}%, Jitter={:.1}ms",
                                session_id,
                                quality.mos_score.unwrap_or(0.0),
                                quality.packet_loss_percent,
                                quality.jitter_ms
                            );
                        }
                    }
                }
            }
            Err(e) => eprintln!("Failed to list sessions: {}", e),
        }
    }
}
```

### Recipe 17: Load Testing Pattern

```rust
async fn load_test_concurrent_calls(
    coordinator: Arc<SessionCoordinator>,
    target_uri: &str,
    concurrent_calls: usize
) -> Result<()> {
    println!("Starting load test with {} concurrent calls", concurrent_calls);
    
    let start_time = Instant::now();
    let mut handles = Vec::new();
    
    // Spawn concurrent call tasks
    for i in 0..concurrent_calls {
        let coord = coordinator.clone();
        let uri = target_uri.to_string();
        
        let handle = tokio::spawn(async move {
            let call_start = Instant::now();
            
            match SessionControl::create_outgoing_call(
                &coord,
                &format!("sip:test{}@loadtest.local", i),
                &uri,
                None
            ).await {
                Ok(session) => {
                    // Wait for answer
                    if let Ok(_) = SessionControl::wait_for_answer(
                        &coord,
                        &session.id,
                        Duration::from_secs(30)
                    ).await {
                        // Hold call for some time
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        
                        // Terminate
                        let _ = SessionControl::terminate_session(&coord, &session.id).await;
                        
                        let duration = call_start.elapsed();
                        Ok((i, duration))
                    } else {
                        Err((i, "Failed to answer"))
                    }
                }
                Err(e) => Err((i, e.to_string().as_str())),
            }
        });
        
        handles.push(handle);
        
        // Stagger call initiation
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Wait for all calls to complete
    let results = futures::future::join_all(handles).await;
    
    // Analyze results
    let mut successful = 0;
    let mut total_duration = Duration::ZERO;
    
    for result in results {
        match result {
            Ok(Ok((idx, duration))) => {
                successful += 1;
                total_duration += duration;
                println!("Call {} succeeded, duration: {:?}", idx, duration);
            }
            Ok(Err((idx, error))) => {
                println!("Call {} failed: {}", idx, error);
            }
            Err(e) => {
                println!("Task panicked: {}", e);
            }
        }
    }
    
    let elapsed = start_time.elapsed();
    
    println!("\n📈 Load Test Results:");
    println!("  Total Time: {:?}", elapsed);
    println!("  Successful Calls: {}/{}", successful, concurrent_calls);
    println!("  Success Rate: {:.1}%", (successful as f64 / concurrent_calls as f64) * 100.0);
    
    if successful > 0 {
        println!("  Average Call Duration: {:?}", total_duration / successful as u32);
    }
    
    Ok(())
}
```

## Best Practices

### ✅ DO:

1. **Use the Public API** - Always use `SessionControl` and `MediaControl` traits
2. **Handle Errors Gracefully** - Network operations can and will fail
3. **Monitor Call Quality** - Use statistics API for production monitoring
4. **Clean Up Resources** - Always terminate sessions when done
5. **Use Appropriate Timeouts** - Don't wait forever for responses
6. **Log Important Events** - But avoid logging sensitive data
7. **Test Edge Cases** - Network failures, timeouts, busy responses

### ❌ DON'T:

1. **Access Internal Fields** - Never use `coordinator.dialog_manager` directly
2. **Block in Handlers** - Use `CallDecision::Defer` for async operations
3. **Ignore Errors** - Always handle potential failures
4. **Leak Resources** - Always clean up sessions and bridges
5. **Hardcode Values** - Use configuration for ports, timeouts, etc.

### Example: Proper Resource Cleanup

```rust
// Use a guard pattern for automatic cleanup
struct CallGuard {
    coordinator: Arc<SessionCoordinator>,
    session_id: SessionId,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        let coordinator = self.coordinator.clone();
        let session_id = self.session_id.clone();
        
        // Spawn cleanup task
        tokio::spawn(async move {
            if let Err(e) = SessionControl::terminate_session(&coordinator, &session_id).await {
                eprintln!("Failed to terminate session on drop: {}", e);
            }
        });
    }
}

async fn make_guarded_call(coordinator: Arc<SessionCoordinator>) -> Result<()> {
    let session = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:test@local",
        "sip:echo@example.com",
        None
    ).await?;
    
    let _guard = CallGuard {
        coordinator: coordinator.clone(),
        session_id: session.id().clone(),
    };
    
    // Do work with the call...
    // Session will be terminated when guard is dropped
    
    Ok(())
}
```

## Troubleshooting

### Common Issues and Solutions

1. **"No available RTP ports"**
   ```rust
   // Increase port range
   let coordinator = SessionManagerBuilder::new()
       .with_rtp_port_range(10000, 60000)
       .build()
       .await?;
   ```

2. **"Failed to bind SIP port"**
   ```rust
   // Use a different port or check if already in use
   let coordinator = SessionManagerBuilder::new()
       .with_sip_port(5061)  // Try alternate port
       .build()
       .await?;
   ```

3. **Poor Call Quality**
   ```rust
   // Enable quality monitoring and adapt
   async fn monitor_and_adapt(coordinator: &Arc<SessionCoordinator>, session_id: &SessionId) {
       let stats = MediaControl::get_media_statistics(coordinator, session_id).await?;
       
       if let Some(quality) = stats.and_then(|s| s.quality_metrics) {
           if quality.packet_loss_percent > 5.0 {
               // Consider switching to more resilient codec
               // or adjusting jitter buffer
           }
       }
   }
   ```

## Further Reading

- [API Documentation](src/api/mod.rs) - Complete API reference
- [Examples](examples/) - Full working examples
- [API Guide](API_GUIDE.md) - Comprehensive developer guide 