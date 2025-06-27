# Session-Core API Developer Guide

## Table of Contents

1. [Introduction](#introduction)
2. [Quick Start](#quick-start)
3. [Core Concepts](#core-concepts)
4. [API Reference](#api-reference)
5. [Common Patterns](#common-patterns)
6. [Examples](#examples)
7. [Best Practices](#best-practices)
8. [Troubleshooting](#troubleshooting)

## Introduction

The session-core API is the primary interface for building SIP/VoIP applications with rvoip. It provides a high-level, type-safe Rust API that abstracts the complexity of SIP protocol handling, media management, and call control.

### Key Features

- **Unified Interface**: Single `SessionCoordinator` manages all aspects
- **Event-Driven**: Async callbacks for call events
- **Type-Safe**: Leverages Rust's type system for compile-time safety
- **Flexible**: Support for both simple and complex use cases
- **Production-Ready**: Built for real-world telephony applications

## Quick Start

### Basic SIP Phone in 20 Lines

```rust
use rvoip_session_core::api::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Create and configure the session coordinator
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_handler(Arc::new(AutoAnswerHandler))
        .build()
        .await?;
    
    // Start accepting incoming calls
    SessionControl::start(&coordinator).await?;
    
    // Make an outgoing call
    let session = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:alice@ourserver.com",
        "sip:bob@example.com",
        None  // SDP will be auto-generated
    ).await?;
    
    println!("Call initiated: {}", session.id());
    
    // Keep running
    tokio::signal::ctrl_c().await?;
    SessionControl::stop(&coordinator).await?;
    Ok(())
}
```

## Core Concepts

### SessionCoordinator

The `SessionCoordinator` is the central hub that coordinates all components:

```text
                    SessionCoordinator
                           |
        +------------------+------------------+
        |                  |                  |
   DialogManager      MediaManager      ConferenceManager
        |                  |                  |
  TransactionMgr      RTP/RTCP          Bridges/Mixing
```

### Session Lifecycle

```text
Outgoing Call:
create_outgoing_call() → Initiating → Ringing → Active → Terminated
                              ↓
                          Cancelled

Incoming Call:
INVITE → on_incoming_call() → CallDecision → Active/Rejected
                                    ↓
                                  Defer → Async Processing
```

### Call Decisions

When an incoming call arrives, your handler must return one of:

- `CallDecision::Accept(sdp)` - Accept with optional SDP answer
- `CallDecision::Reject(reason)` - Reject with reason
- `CallDecision::Defer` - Defer for async processing
- `CallDecision::Forward(target)` - Forward to another destination

## API Reference

### SessionManagerBuilder

Configure and build a SessionCoordinator:

```rust
let coordinator = SessionManagerBuilder::new()
    // Network Configuration
    .with_sip_port(5060)
    .with_rtp_port_range(10000, 20000)
    .with_local_address("sip:pbx@192.168.1.100:5060")
    
    // Call Handling
    .with_handler(Arc::new(MyCallHandler))
    
    // Advanced Options
    .with_max_sessions(1000)
    .with_session_timeout(Duration::from_secs(3600))
    
    .build()
    .await?;
```

### SessionControl Trait

Main control operations:

```rust
// Call Management
SessionControl::create_outgoing_call(&coordinator, from, to, sdp).await?;
SessionControl::terminate_session(&coordinator, &session_id).await?;

// Call Features  
SessionControl::hold_session(&coordinator, &session_id).await?;
SessionControl::resume_session(&coordinator, &session_id).await?;
SessionControl::transfer_session(&coordinator, &session_id, target).await?;
SessionControl::send_dtmf(&coordinator, &session_id, "1234#").await?;

// Programmatic Call Handling
SessionControl::accept_incoming_call(&coordinator, &call, sdp).await?;
SessionControl::reject_incoming_call(&coordinator, &call, reason).await?;

// Monitoring
SessionControl::get_session(&coordinator, &session_id).await?;
SessionControl::list_active_sessions(&coordinator).await?;
SessionControl::get_stats(&coordinator).await?;
```

### MediaControl Trait

Media and quality management:

```rust
// SDP Negotiation
MediaControl::generate_sdp_offer(&coordinator, &session_id).await?;
MediaControl::generate_sdp_answer(&coordinator, &session_id, offer).await?;

// Media Flow
MediaControl::establish_media_flow(&coordinator, &session_id, "192.168.1.100:5004").await?;
MediaControl::start_audio_transmission(&coordinator, &session_id).await?;
MediaControl::stop_audio_transmission(&coordinator, &session_id).await?;

// Quality Monitoring
MediaControl::get_media_statistics(&coordinator, &session_id).await?;
MediaControl::start_statistics_monitoring(&coordinator, &session_id, interval).await?;
```

### CallHandler Trait

Implement to customize call handling:

```rust
#[async_trait]
impl CallHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Your logic here
        CallDecision::Accept(None)
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call {} ended: {}", call.id(), reason);
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        println!("Call {} established", call.id());
    }
}
```

## Common Patterns

### Pattern 1: Simple Auto-Answer

```rust
struct AutoAnswerHandler;

#[async_trait]
impl CallHandler for AutoAnswerHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Accept(None)
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        log::info!("Call {} ended: {}", call.id(), reason);
    }
}
```

### Pattern 2: Queue with Async Processing

```rust
struct QueueHandler {
    queue: Arc<Mutex<VecDeque<IncomingCall>>>,
}

#[async_trait]
impl CallHandler for QueueHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        self.queue.lock().unwrap().push_back(call);
        CallDecision::Defer
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        // Update queue statistics
    }
}

// Process queue in background task
async fn process_queue(coordinator: Arc<SessionCoordinator>, handler: Arc<QueueHandler>) {
    loop {
        if let Some(call) = handler.queue.lock().unwrap().pop_front() {
            // Async processing: database lookup, authentication, etc.
            let result = authenticate_caller(&call.from).await;
            
            if result.is_authorized {
                let sdp = MediaControl::generate_sdp_answer(
                    &coordinator,
                    &call.id,
                    &call.sdp.unwrap()
                ).await.unwrap();
                
                SessionControl::accept_incoming_call(
                    &coordinator,
                    &call,
                    Some(sdp)
                ).await.unwrap();
            } else {
                SessionControl::reject_incoming_call(
                    &coordinator,
                    &call,
                    "Authentication failed"
                ).await.unwrap();
            }
        }
        
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

### Pattern 3: Advanced Call Routing

```rust
struct SmartRouter {
    rules: Vec<RoutingRule>,
    default_action: CallDecision,
}

impl SmartRouter {
    fn route(&self, call: &IncomingCall) -> CallDecision {
        // Time-based routing
        let hour = Local::now().hour();
        if hour < 9 || hour >= 17 {
            return CallDecision::Forward("sip:afterhours@voicemail.com".to_string());
        }
        
        // VIP routing
        if self.is_vip(&call.from) {
            return CallDecision::Forward("sip:vip@priority.queue".to_string());
        }
        
        // Department routing
        for rule in &self.rules {
            if call.to.contains(&rule.pattern) {
                return CallDecision::Forward(rule.target.clone());
            }
        }
        
        self.default_action.clone()
    }
}
```

## Examples

### Example 1: Call Center Agent

```rust
async fn agent_example() -> Result<()> {
    // Configure for call center
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_handler(Arc::new(AgentHandler::new()))
        .build()
        .await?;
    
    SessionControl::start(&coordinator).await?;
    
    // Wait for calls to be routed to this agent
    // Calls are automatically answered by AgentHandler
    
    Ok(())
}

struct AgentHandler {
    agent_id: String,
    status: Arc<Mutex<AgentStatus>>,
}

#[async_trait]
impl CallHandler for AgentHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        let status = self.status.lock().unwrap().clone();
        
        match status {
            AgentStatus::Available => {
                *self.status.lock().unwrap() = AgentStatus::OnCall;
                CallDecision::Accept(None)
            }
            AgentStatus::OnBreak => {
                CallDecision::Reject("Agent on break".to_string())
            }
            AgentStatus::OnCall => {
                CallDecision::Reject("Agent busy".to_string())
            }
        }
    }
    
    async fn on_call_ended(&self, _call: CallSession, _reason: &str) {
        *self.status.lock().unwrap() = AgentStatus::Available;
    }
}
```

### Example 2: Conference Bridge

```rust
async fn conference_example(coordinator: Arc<SessionCoordinator>) -> Result<()> {
    // Create two calls
    let call1 = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:conference@server.com",
        "sip:alice@example.com",
        None
    ).await?;
    
    let call2 = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:conference@server.com",
        "sip:bob@example.com",
        None
    ).await?;
    
    // Wait for both to answer
    SessionControl::wait_for_answer(&coordinator, &call1.id, Duration::from_secs(30)).await?;
    SessionControl::wait_for_answer(&coordinator, &call2.id, Duration::from_secs(30)).await?;
    
    // Bridge them together
    let bridge_id = coordinator.bridge_sessions(&call1.id, &call2.id).await?;
    
    println!("Conference bridge created: {}", bridge_id);
    
    Ok(())
}
```

### Example 3: Quality Monitoring

```rust
async fn monitor_quality(coordinator: Arc<SessionCoordinator>, session_id: SessionId) -> Result<()> {
    // Start automatic monitoring
    MediaControl::start_statistics_monitoring(
        &coordinator,
        &session_id,
        Duration::from_secs(5)
    ).await?;
    
    // Manual quality checks
    let mut poor_quality_count = 0;
    
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        
        let stats = MediaControl::get_media_statistics(&coordinator, &session_id).await?;
        
        if let Some(stats) = stats {
            if let Some(quality) = stats.quality_metrics {
                let mos = quality.mos_score.unwrap_or(0.0);
                
                if mos < 3.0 {
                    poor_quality_count += 1;
                    log::warn!("Poor quality detected: MOS={:.1}", mos);
                    
                    if poor_quality_count >= 3 {
                        // Sustained poor quality - take action
                        notify_operations_team(&session_id, mos).await?;
                    }
                } else {
                    poor_quality_count = 0;
                }
                
                // Log metrics
                log::info!(
                    "Call {}: MOS={:.1}, Loss={:.1}%, Jitter={:.1}ms",
                    session_id,
                    mos,
                    quality.packet_loss_percent,
                    quality.jitter_ms
                );
            }
        }
        
        // Check if call is still active
        if let Ok(Some(session)) = SessionControl::get_session(&coordinator, &session_id).await {
            if session.state().is_final() {
                break;
            }
        }
    }
    
    Ok(())
}
```

## Best Practices

### 1. Resource Management

Always clean up sessions:

```rust
// Use a guard pattern
struct SessionGuard {
    coordinator: Arc<SessionCoordinator>,
    session_id: SessionId,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let coordinator = self.coordinator.clone();
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            let _ = SessionControl::terminate_session(&coordinator, &session_id).await;
        });
    }
}
```

### 2. Error Handling

Handle all error cases:

```rust
match SessionControl::create_outgoing_call(&coordinator, from, to, None).await {
    Ok(session) => {
        log::info!("Call created: {}", session.id());
        Ok(session)
    }
    Err(SessionError::InvalidUri(uri)) => {
        log::error!("Invalid URI: {}", uri);
        Err(AppError::BadRequest(format!("Invalid SIP URI: {}", uri)))
    }
    Err(SessionError::ResourceExhausted) => {
        log::error!("No available RTP ports");
        Err(AppError::ServiceUnavailable("System at capacity"))
    }
    Err(e) => {
        log::error!("Call failed: {}", e);
        Err(AppError::Internal(e.to_string()))
    }
}
```

### 3. Async Processing

Don't block in handlers:

```rust
// BAD - Blocks the event loop
impl CallHandler for BadHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        let result = expensive_database_query(&call.from).await; // Don't do this!
        if result.is_authorized {
            CallDecision::Accept(None)
        } else {
            CallDecision::Reject("Unauthorized")
        }
    }
}

// GOOD - Defer for async processing
impl CallHandler for GoodHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        self.pending.push(call);
        CallDecision::Defer
    }
}
```

### 4. Monitoring

Always monitor production calls:

```rust
// Set up monitoring for all calls
coordinator.set_on_session_created(|session_id| {
    tokio::spawn(monitor_quality(coordinator.clone(), session_id));
});
```

### 5. Testing

Write comprehensive tests:

```rust
#[tokio::test]
async fn test_call_rejection() {
    let coordinator = create_test_coordinator().await;
    let handler = Arc::new(RejectAllHandler);
    
    // Simulate incoming call
    let call = IncomingCall {
        id: SessionId::new(),
        from: "sip:test@example.com".to_string(),
        to: "sip:pbx@ourserver.com".to_string(),
        sdp: Some(test_sdp()),
        headers: HashMap::new(),
        received_at: Instant::now(),
    };
    
    let decision = handler.on_incoming_call(call).await;
    assert!(matches!(decision, CallDecision::Reject(_)));
}
```

## Troubleshooting

### Common Issues

1. **"No available RTP ports"**
   - Increase port range: `.with_rtp_port_range(10000, 60000)`
   - Check firewall rules

2. **"Failed to bind SIP port"**
   - Port already in use: `lsof -i :5060`
   - Need root/sudo for ports < 1024

3. **"Media not established"**
   - Check NAT/firewall configuration
   - Verify SDP addresses are reachable
   - Enable STUN/TURN if behind NAT

4. **Poor call quality**
   - Monitor with `get_media_statistics()`
   - Check network bandwidth and latency
   - Verify codec compatibility

### Debug Logging

Enable detailed logs:

```rust
env_logger::Builder::from_env(env_logger::Env::default())
    .filter_module("rvoip_session_core", log::LevelFilter::Debug)
    .filter_module("rvoip_sip_core", log::LevelFilter::Trace)
    .init();
```

### Health Checks

Implement health monitoring:

```rust
async fn health_check(coordinator: &Arc<SessionCoordinator>) -> HealthStatus {
    let stats = SessionControl::get_stats(coordinator).await?;
    
    HealthStatus {
        active_calls: stats.active_sessions,
        total_calls: stats.total_sessions,
        failed_calls: stats.failed_sessions,
        uptime: coordinator.uptime(),
        sip_status: if coordinator.is_running() { "OK" } else { "DOWN" },
    }
}
```

## Next Steps

- Explore the [examples/](examples/) directory for complete applications
- Read the [COOKBOOK.md](COOKBOOK.md) for common recipes
- Check [ARCHITECTURE.md](ARCHITECTURE.md) for system design details
- Join our community for support and discussions

## API Stability

The session-core API follows semantic versioning:

- Types in `api::types` are stable
- Trait methods are stable (new methods get default implementations)
- Handler interfaces are stable
- Breaking changes require major version bump

We're committed to maintaining a stable API for production use. 