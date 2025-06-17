# Session-Core Cookbook

A practical guide with recipes for common VoIP scenarios using the session-core API.

## Table of Contents

1. [Getting Started](#getting-started)
2. [Making Outgoing Calls](#making-outgoing-calls)
3. [Handling Incoming Calls](#handling-incoming-calls)
4. [Media Control](#media-control)
5. [Call Features](#call-features)
6. [Advanced Patterns](#advanced-patterns)
7. [Error Handling](#error-handling)
8. [Performance & Monitoring](#performance--monitoring)

## Getting Started

### Basic Setup

```rust
use rvoip_session_core::api::*;  // Single import for everything
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt().init();
    
    // Build a session coordinator
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:user@192.168.1.100:5060")
        .with_media_ports(10000, 20000)  // RTP port range
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
    // Prepare the call (allocates media resources)
    let (sdp_offer, rtp_port) = SessionControl::prepare_outgoing_call(
        coordinator, 
        "unique-call-id"
    ).await?;
    
    // Create the call with SDP
    let session = SessionControl::create_outgoing_call(
        coordinator,
        "sip:alice@example.com",     // to
        "sip:bob@192.168.1.100",     // from
        Some(sdp_offer)
    ).await?;
    
    info!("Call created: {}", session.id());
    
    // Wait for answer
    let answered_session = SessionControl::wait_for_answer(
        coordinator, 
        session.id(), 
        Duration::from_secs(30)
    ).await?;
    
    // Handle the call...
    Ok(())
}
```

### Recipe 2: Call with Custom Headers

```rust
async fn make_call_with_headers(coordinator: &Arc<SessionCoordinator>) -> Result<()> {
    // Use the builder pattern for more control
    let mut builder = CallBuilder::new()
        .to("sip:alice@example.com")
        .from("sip:bob@192.168.1.100")
        .with_header("X-Custom-ID", "12345")
        .with_header("X-Department", "Sales");
    
    // Add SDP if needed
    let (sdp_offer, _) = SessionControl::prepare_outgoing_call(
        coordinator, 
        "call-123"
    ).await?;
    builder = builder.with_sdp(sdp_offer);
    
    let session = SessionControl::create_outgoing_call_with_builder(
        coordinator,
        builder
    ).await?;
    
    Ok(())
}
```

## Handling Incoming Calls

### Recipe 3: Auto-Accept Pattern (CallHandler)

```rust
#[derive(Debug)]
struct AutoAcceptHandler;

#[async_trait::async_trait]
impl CallHandler for AutoAcceptHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("Incoming call from {}", call.from);
        
        // Auto-accept with SDP answer
        if let Some(offer) = &call.sdp {
            // In real app, generate answer based on offer
            let answer = generate_compatible_answer(offer);
            CallDecision::Accept(Some(answer))
        } else {
            CallDecision::Accept(None)
        }
    }
    
    async fn on_call_established(&self, session: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("Call established: {}", session.id());
        
        // Set up media flow
        if let Some(sdp) = remote_sdp {
            if let Ok(info) = parse_sdp_connection(&sdp) {
                let remote_addr = format!("{}:{}", info.ip, info.port);
                // Media flow setup would go here
            }
        }
    }
}
```

### Recipe 4: Deferred Decision Pattern

```rust
#[derive(Debug)]
struct DeferredHandler {
    pending_calls: Arc<Mutex<Vec<IncomingCall>>>,
}

#[async_trait::async_trait]
impl CallHandler for DeferredHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        info!("Deferring decision for call from {}", call.from);
        
        // Store for later processing
        self.pending_calls.lock().await.push(call);
        
        // Defer the decision
        CallDecision::Defer
    }
}

// Later, in your business logic:
async fn process_pending_calls(
    coordinator: &Arc<SessionCoordinator>,
    handler: &DeferredHandler
) -> Result<()> {
    let calls = handler.pending_calls.lock().await.drain(..).collect::<Vec<_>>();
    
    for call in calls {
        // Check business rules (database, permissions, etc.)
        if should_accept_call(&call).await? {
            // Generate SDP answer
            let answer = if let Some(offer) = &call.sdp {
                Some(MediaControl::generate_sdp_answer(coordinator, &call.id, offer).await?)
            } else {
                None
            };
            
            // Accept the call
            SessionControl::accept_incoming_call(coordinator, &call, answer).await?;
        } else {
            // Reject the call
            SessionControl::reject_incoming_call(coordinator, &call, "Forbidden").await?;
        }
    }
    
    Ok(())
}
```

## Media Control

### Recipe 5: Setting Up Media Flow

```rust
async fn setup_media_flow(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    remote_sdp: &str
) -> Result<()> {
    // Parse the remote SDP
    let sdp_info = parse_sdp_connection(remote_sdp)?;
    info!("Remote endpoint: {}:{}", sdp_info.ip, sdp_info.port);
    info!("Codecs: {:?}", sdp_info.codecs);
    
    // Establish media flow
    let remote_addr = format!("{}:{}", sdp_info.ip, sdp_info.port);
    MediaControl::establish_media_flow(coordinator, session_id, &remote_addr).await?;
    
    // Start monitoring
    MediaControl::start_statistics_monitoring(
        coordinator, 
        session_id, 
        Duration::from_secs(5)
    ).await?;
    
    Ok(())
}
```

### Recipe 6: Media Control During Call

```rust
async fn control_media(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId
) -> Result<()> {
    // Mute audio
    SessionControl::set_audio_muted(coordinator, session_id, true).await?;
    
    // Get current media info
    let media_info = SessionControl::get_media_info(coordinator, session_id).await?;
    if let Some(info) = media_info {
        info!("Local SDP: {:?}", info.local_sdp);
        info!("Remote SDP: {:?}", info.remote_sdp);
        info!("RTP port: {}", info.rtp_port);
    }
    
    // Monitor quality
    let stats = MediaControl::get_media_statistics(coordinator, session_id).await?;
    if let Some(stats) = stats {
        if let Some(quality) = &stats.quality_metrics {
            info!("Packet loss: {:.1}%", quality.packet_loss_percent);
            info!("Jitter: {:.1}ms", quality.jitter_ms);
            info!("MOS score: {:.1}", quality.mos_score.unwrap_or(0.0));
        }
    }
    
    Ok(())
}
```

## Call Features

### Recipe 7: Call Hold/Resume

```rust
async fn toggle_hold(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    hold: bool
) -> Result<()> {
    if hold {
        SessionControl::hold_session(coordinator, session_id).await?;
        info!("Call {} is now on hold", session_id);
    } else {
        SessionControl::resume_session(coordinator, session_id).await?;
        info!("Call {} resumed", session_id);
    }
    Ok(())
}
```

### Recipe 8: Call Transfer

```rust
async fn transfer_call(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    transfer_to: &str
) -> Result<()> {
    // Blind transfer
    SessionControl::transfer_session(
        coordinator, 
        session_id, 
        transfer_to,
        false  // blind transfer
    ).await?;
    
    info!("Call {} transferred to {}", session_id, transfer_to);
    Ok(())
}
```

### Recipe 9: DTMF Tones

```rust
async fn send_dtmf_sequence(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    digits: &str
) -> Result<()> {
    for digit in digits.chars() {
        if digit.is_numeric() || matches!(digit, '*' | '#') {
            SessionControl::send_dtmf(coordinator, session_id, digit).await?;
            // Small delay between digits
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    Ok(())
}
```

## Advanced Patterns

### Recipe 10: Conference Calls

```rust
async fn create_conference(
    coordinator: &Arc<SessionCoordinator>,
    participant_uris: Vec<&str>
) -> Result<Vec<SessionId>> {
    let mut sessions = Vec::new();
    
    // Create calls to all participants
    for uri in participant_uris {
        let (sdp_offer, _) = SessionControl::prepare_outgoing_call(
            coordinator, 
            &format!("conf-{}", uuid::Uuid::new_v4())
        ).await?;
        
        let session = SessionControl::create_outgoing_call(
            coordinator,
            uri,
            "sip:conference@server.com",
            Some(sdp_offer)
        ).await?;
        
        sessions.push(session.id().clone());
    }
    
    // Wait for all to answer
    let mut answered = Vec::new();
    for session_id in sessions {
        match SessionControl::wait_for_answer(
            coordinator, 
            &session_id, 
            Duration::from_secs(30)
        ).await {
            Ok(session) => answered.push(session.id().clone()),
            Err(e) => warn!("Participant failed to answer: {}", e),
        }
    }
    
    // Set up conference mixing (application-specific)
    setup_conference_bridge(&answered).await?;
    
    Ok(answered)
}
```

### Recipe 11: Failover and Retry

```rust
async fn call_with_failover(
    coordinator: &Arc<SessionCoordinator>,
    primary_uri: &str,
    backup_uris: Vec<&str>
) -> Result<CallSession> {
    // Try primary first
    match try_call(coordinator, primary_uri).await {
        Ok(session) => return Ok(session),
        Err(e) => warn!("Primary failed: {}", e),
    }
    
    // Try backups
    for backup_uri in backup_uris {
        match try_call(coordinator, backup_uri).await {
            Ok(session) => return Ok(session),
            Err(e) => warn!("Backup {} failed: {}", backup_uri, e),
        }
    }
    
    Err(anyhow::anyhow!("All endpoints failed"))
}

async fn try_call(
    coordinator: &Arc<SessionCoordinator>,
    uri: &str
) -> Result<CallSession> {
    let (sdp_offer, _) = SessionControl::prepare_outgoing_call(
        coordinator, 
        &format!("call-{}", uuid::Uuid::new_v4())
    ).await?;
    
    let session = SessionControl::create_outgoing_call(
        coordinator,
        uri,
        "sip:user@local",
        Some(sdp_offer)
    ).await?;
    
    // Short timeout for failover scenario
    SessionControl::wait_for_answer(
        coordinator, 
        session.id(), 
        Duration::from_secs(10)
    ).await
}
```

## Error Handling

### Recipe 12: Comprehensive Error Handling

```rust
use rvoip_session_core::api::{SessionError, ErrorKind};

async fn handle_call_with_recovery(
    coordinator: &Arc<SessionCoordinator>,
    uri: &str
) -> Result<()> {
    match make_call(coordinator, uri).await {
        Ok(session) => {
            info!("Call successful: {}", session.id());
            Ok(())
        }
        Err(e) => {
            match e.downcast_ref::<SessionError>() {
                Some(session_err) => match session_err.kind() {
                    ErrorKind::Timeout => {
                        warn!("Call timed out, retrying...");
                        make_call(coordinator, uri).await?;
                        Ok(())
                    }
                    ErrorKind::InvalidState => {
                        error!("Invalid state: {}", session_err);
                        Err(e)
                    }
                    ErrorKind::ResourceExhausted => {
                        error!("No resources available");
                        // Wait and retry
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        make_call(coordinator, uri).await?;
                        Ok(())
                    }
                    _ => Err(e),
                },
                None => Err(e),
            }
        }
    }
}
```

## Performance & Monitoring

### Recipe 13: Real-time Quality Monitoring

```rust
async fn monitor_call_quality(
    coordinator: Arc<SessionCoordinator>,
    session_id: SessionId
) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    let mut degraded_count = 0;
    
    loop {
        interval.tick().await;
        
        match MediaControl::get_media_statistics(&coordinator, &session_id).await {
            Ok(Some(stats)) => {
                if let Some(quality) = &stats.quality_metrics {
                    let mos = quality.mos_score.unwrap_or(0.0);
                    
                    if mos < 3.0 {
                        degraded_count += 1;
                        warn!("Poor quality detected: MOS={:.1}", mos);
                        
                        if degraded_count >= 3 {
                            // Take action on sustained poor quality
                            notify_poor_quality(&session_id, mos).await;
                        }
                    } else {
                        degraded_count = 0;
                    }
                    
                    // Log metrics
                    metrics::gauge!("call_mos_score", mos, "session_id" => session_id.to_string());
                    metrics::gauge!("call_packet_loss", quality.packet_loss_percent, "session_id" => session_id.to_string());
                    metrics::gauge!("call_jitter_ms", quality.jitter_ms, "session_id" => session_id.to_string());
                }
            }
            Ok(None) => {
                info!("Call {} ended", session_id);
                break;
            }
            Err(e) => {
                error!("Failed to get statistics: {}", e);
                break;
            }
        }
    }
}
```

### Recipe 14: Load Testing Pattern

```rust
async fn load_test_calls(
    coordinator: Arc<SessionCoordinator>,
    target_uri: &str,
    concurrent_calls: usize,
    call_duration: Duration
) -> Result<()> {
    let mut handles = vec![];
    
    for i in 0..concurrent_calls {
        let coord = coordinator.clone();
        let uri = target_uri.to_string();
        let duration = call_duration;
        
        let handle = tokio::spawn(async move {
            match test_single_call(&coord, &uri, duration).await {
                Ok(stats) => {
                    info!("Call {} completed: {:?}", i, stats);
                    Ok(stats)
                }
                Err(e) => {
                    error!("Call {} failed: {}", i, e);
                    Err(e)
                }
            }
        });
        
        handles.push(handle);
        
        // Stagger call initiation
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    // Wait for all calls to complete
    let results = futures::future::join_all(handles).await;
    
    // Analyze results
    let successful = results.iter().filter(|r| r.is_ok()).count();
    info!("Load test complete: {}/{} calls successful", successful, concurrent_calls);
    
    Ok(())
}
```

## Best Practices

1. **Always use the public API** - Never access internal fields like `coordinator.dialog_manager`
2. **Handle errors gracefully** - Network operations can fail
3. **Clean up resources** - Always call `terminate_session` when done
4. **Monitor call quality** - Use statistics API for production monitoring
5. **Use appropriate timeouts** - Don't wait forever for responses
6. **Log important events** - But avoid logging sensitive data like passwords
7. **Test edge cases** - Network failures, timeouts, busy responses

## Common Pitfalls

### ❌ Don't do this:
```rust
// Direct internal access
coordinator.media_manager.create_session(...).await;
coordinator.dialog_manager.accept_call(...).await;
```

### ✅ Do this instead:
```rust
// Use public API methods
MediaControl::create_media_session(&coordinator, ...).await;
SessionControl::accept_incoming_call(&coordinator, ...).await;
```

### ❌ Don't forget error handling:
```rust
let session = SessionControl::create_outgoing_call(...).await.unwrap(); // Bad!
```

### ✅ Always handle errors:
```rust
let session = SessionControl::create_outgoing_call(...).await?; // Good!
// or
match SessionControl::create_outgoing_call(...).await {
    Ok(session) => { /* handle success */ },
    Err(e) => { /* handle error */ }
}
```

## Further Reading

- [API Reference](src/api/mod.rs) - Complete API documentation
- [Examples](examples/) - Full working examples
- [Migration Guide](src/api/MIGRATION_GUIDE.md) - Upgrading from older versions 