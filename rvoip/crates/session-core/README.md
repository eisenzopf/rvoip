# rvoip-session-core

High-level SIP session and media management for building VoIP applications in Rust.

## Overview

`rvoip-session-core` provides a clean, type-safe API for building SIP clients and servers. It handles the complexity of SIP dialogs, media negotiation, and call control while exposing a simple interface for application developers.

## 📚 Documentation

- **[API Documentation](src/api/mod.rs)** - Comprehensive API reference with examples
- **[API Guide](API_GUIDE.md)** - Complete developer guide with patterns and best practices
- **[COOKBOOK.md](COOKBOOK.md)** - Practical recipes for common VoIP scenarios
- **[Examples](examples/)** - Full working examples including:
  - [API Best Practices](examples/api_best_practices/) - Clean API usage demonstrations
  - [Client-Server Demo](examples/client-server/) - Complete UAC/UAS implementation
  - [SIPp Integration Tests](examples/sipp_tests/) - Interoperability testing

## Features

- 🎯 **Clean API** - Simple traits for session and media control
- 📞 **Complete Call Management** - Make, receive, hold, transfer calls
- 🎵 **Media Integration** - Built-in RTP/RTCP with quality monitoring
- 🔄 **Two Calling Patterns** - Immediate or deferred call decisions
- 🌉 **Bridge Management** - Connect calls for conferencing
- 📊 **Real-time Statistics** - Monitor call quality with MOS scores
- 🏗️ **Builder Pattern** - Easy configuration and setup
- ⚡ **Async/Await** - Modern async Rust throughout

## Quick Start

```rust
use rvoip_session_core::api::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Build and start a session coordinator
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:alice@192.168.1.100:5060")
        .with_handler(Arc::new(AutoAnswerHandler))
        .build()
        .await?;
    
    SessionControl::start(&coordinator).await?;
    
    // Make a call
    let session = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:alice@192.168.1.100",  // from
        "sip:bob@example.com",       // to
        None  // SDP will be generated automatically
    ).await?;
    
    println!("Call initiated: {}", session.id());
    
    // Wait for answer
    SessionControl::wait_for_answer(
        &coordinator,
        &session.id,
        Duration::from_secs(30)
    ).await?;
    
    println!("Call connected!");
    
    // Clean shutdown
    SessionControl::stop(&coordinator).await?;
    Ok(())
}
```

## Core API Components

### SessionControl Trait

The main interface for call control operations:

```rust
use rvoip_session_core::api::*;

// Create outgoing calls
let session = SessionControl::create_outgoing_call(
    &coordinator,
    "sip:caller@myserver.com",
    "sip:callee@example.com",
    None
).await?;

// Wait for answer
SessionControl::wait_for_answer(
    &coordinator,
    session.id(),
    Duration::from_secs(30)
).await?;

// Control calls
SessionControl::hold_session(&coordinator, session.id()).await?;
SessionControl::resume_session(&coordinator, session.id()).await?;
SessionControl::send_dtmf(&coordinator, session.id(), "1234#").await?;
SessionControl::terminate_session(&coordinator, session.id()).await?;
```

### MediaControl Trait

Interface for media stream management:

```rust
// Generate SDP for negotiation
let sdp_offer = MediaControl::generate_sdp_offer(&coordinator, &session_id).await?;
let sdp_answer = MediaControl::generate_sdp_answer(&coordinator, &session_id, &offer).await?;

// Control audio
MediaControl::start_audio_transmission(&coordinator, &session_id).await?;
MediaControl::stop_audio_transmission(&coordinator, &session_id).await?;  // Mute

// Monitor quality
let stats = MediaControl::get_media_statistics(&coordinator, &session_id).await?;
if let Some(quality) = stats.and_then(|s| s.quality_metrics) {
    println!("MOS Score: {:.1}", quality.mos_score.unwrap_or(0.0));
    println!("Packet Loss: {:.1}%", quality.packet_loss_percent);
    println!("Jitter: {:.1}ms", quality.jitter_ms);
}
```

### Call Handlers

Two patterns for handling incoming calls:

#### Pattern 1: Immediate Decision

```rust
#[derive(Debug)]
struct MyHandler;

#[async_trait::async_trait]
impl CallHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Make decision immediately
        if is_authorized(&call.from) {
            CallDecision::Accept(None)  // Auto-generate SDP answer
        } else {
            CallDecision::Reject("Unauthorized".to_string())
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call {} ended: {}", call.id(), reason);
    }
}
```

#### Pattern 2: Deferred Decision

```rust
#[derive(Debug)]
struct DeferHandler {
    pending_calls: Arc<Mutex<Vec<IncomingCall>>>,
}

#[async_trait::async_trait]
impl CallHandler for DeferHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Defer for async processing
        self.pending_calls.lock().unwrap().push(call);
        CallDecision::Defer
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        // Update records
    }
}

// Process deferred calls asynchronously
async fn process_deferred_calls(
    coordinator: &Arc<SessionCoordinator>,
    handler: &DeferHandler
) -> Result<()> {
    while let Some(call) = handler.pending_calls.lock().unwrap().pop() {
        // Async operations: database lookup, authentication, etc.
        let user = lookup_user(&call.from).await?;
        
        if user.is_authorized {
            let sdp_answer = MediaControl::generate_sdp_answer(
                coordinator,
                &call.id,
                &call.sdp.unwrap()
            ).await?;
            
            SessionControl::accept_incoming_call(
                coordinator,
                &call,
                Some(sdp_answer)
            ).await?;
        } else {
            SessionControl::reject_incoming_call(
                coordinator,
                &call,
                "Not authorized"
            ).await?;
        }
    }
    
    Ok(())
}
```

## Bridge Management

Connect two calls together (e.g., customer to agent):

```rust
// Create bridge between two active sessions
let bridge_id = coordinator.bridge_sessions(&session1_id, &session2_id).await?;

// Monitor bridge events
let mut events = coordinator.subscribe_to_bridge_events().await;
while let Some(event) = events.recv().await {
    match event {
        BridgeEvent::ParticipantAdded { bridge_id, session_id } => {
            println!("Session {} joined bridge {}", session_id, bridge_id);
        }
        BridgeEvent::ParticipantRemoved { bridge_id, session_id, reason } => {
            println!("Session {} left bridge {}: {}", session_id, bridge_id, reason);
        }
        BridgeEvent::BridgeDestroyed { bridge_id } => {
            println!("Bridge {} ended", bridge_id);
            break;
        }
    }
}
```

## Architecture

The library is organized into clean layers with well-defined responsibilities:

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
│                 Your VoIP Application                       │
└─────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────┐
│                      Public API Layer                       │
│         SessionControl & MediaControl Traits                │
│                Clean, Simple, Type-Safe                     │
└─────────────────────────────────────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────────┐
│                  SessionCoordinator                         │
│           Coordinates All Session Components                │
│         Implements SessionControl & MediaControl            │
└─────────────────────────────────────────────────────────────┘
        ↓               ↓                ↓                ↓
┌─────────────┬─────────────┬─────────────┬─────────────────┐
│DialogManager│MediaManager │ConferenceApi│SessionRegistry  │
│ SIP Dialogs │RTP Sessions │  Bridges    │ Session State   │
└─────────────┴─────────────┴─────────────┴─────────────────┘
        ↓               ↓                ↓
┌─────────────┬─────────────┬─────────────────────────────────┐
│transaction  │ media-core  │    dialog-core                  │
│   -core     │ RTP/Codecs  │  Dialog State Machine           │
└─────────────┴─────────────┴─────────────────────────────────┘
```

## Common Use Cases

### Basic Softphone

```rust
let coordinator = SessionManagerBuilder::new()
    .with_sip_port(5060)
    .with_handler(Arc::new(AutoAnswerHandler))
    .build()
    .await?;
```

### Call Center with Queue

```rust
let queue = Arc::new(QueueHandler::new(100));
let coordinator = SessionManagerBuilder::new()
    .with_sip_port(5060)
    .with_handler(queue.clone())
    .build()
    .await?;

// Process queued calls in separate task
tokio::spawn(async move {
    while let Some(call) = queue.dequeue() {
        process_queued_call(&coordinator, call).await?;
    }
});
```

### PBX with Routing

```rust
let mut router = RoutingHandler::new();
router.add_route("sip:support@*", "sip:queue@support.local");
router.add_route("sip:sales@*", "sip:queue@sales.local");
router.add_route("sip:+1800*", "sip:tollfree@gateway.com");

let coordinator = SessionManagerBuilder::new()
    .with_sip_port(5060)
    .with_handler(Arc::new(router))
    .build()
    .await?;
```

### Composite Handler Chain

```rust
let handler = CompositeHandler::new()
    .add_handler(Arc::new(BlacklistHandler::new()))
    .add_handler(Arc::new(BusinessHoursHandler::new()))
    .add_handler(Arc::new(RoutingHandler::new()))
    .add_handler(Arc::new(QueueHandler::new(50)));

let coordinator = SessionManagerBuilder::new()
    .with_handler(Arc::new(handler))
    .build()
    .await?;
```

## RFC Compliance

### ✅ Fully Supported

- **RFC 3261** - Core SIP: Dialog management, session handling, in-dialog requests
- **RFC 4566** - SDP: Session Description Protocol parsing and generation
- **RFC 3264** - Offer/Answer Model: SDP negotiation
- **RFC 3550** - RTP: Real-time Transport Protocol
- **RFC 3551** - RTP Profile: Audio codecs (G.711 µ-law/A-law)

### ⚠️ Partially Supported

- **RFC 3262** - Reliable Provisional Responses (basic PRACK)
- **RFC 3515** - REFER Method (basic call transfer)
- **RFC 4235** - Dialog Event Package (limited support)

### ❌ Not Currently Supported

- SIP Authentication (RFC 3261 Section 22)
- TLS/SIPS Security
- ICE for NAT Traversal (RFC 8445)
- SRTP Media Encryption (RFC 3711)
- Advanced Call Features (attended transfer, replaces)

## Best Practices

1. **Use the Public API** - Always use `SessionControl` and `MediaControl` traits
2. **Handle Errors** - All operations can fail due to network issues
3. **Monitor Quality** - Use `MediaControl::get_media_statistics()` for production
4. **Clean Resources** - Always call `terminate_session()` when done
5. **Choose Patterns Wisely** - Immediate for simple cases, deferred for complex
6. **Test Thoroughly** - Use the provided examples and SIPp tests

## Dependencies

- `rvoip-sip-core` - Core SIP types and parsing
- `rvoip-dialog-core` - Dialog state management
- `rvoip-transaction-core` - SIP transaction handling
- `rvoip-media-core` - Media processing and codecs
- `rvoip-rtp-core` - RTP/RTCP protocol support
- `rvoip-infra-common` - Common infrastructure

## Testing

Run the test suite:

```bash
# Unit tests
cargo test

# Integration tests
cargo test --test '*'

# Run examples
cd examples/api_best_practices
./run_clean_examples.sh

# SIPp interoperability tests
cd examples/sipp_tests
./run_all_tests.sh
```

## License

Licensed under either:

- MIT License
- Apache License 2.0

at your option.

## Contributing

Contributions are welcome! Please see our [Contributing Guide](../../CONTRIBUTING.md) for details.

## Support

- [API Documentation](src/api/mod.rs) - Complete reference
- [Cookbook](COOKBOOK.md) - Practical recipes
- [API Guide](API_GUIDE.md) - In-depth guide
- [Examples](examples/) - Working code samples
- [Issues](https://github.com/yourusername/rvoip/issues) - Bug reports and feature requests 