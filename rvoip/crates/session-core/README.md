# rvoip-session-core

High-level SIP session and media management for building VoIP applications in Rust.

## Overview

`rvoip-session-core` provides a clean, type-safe API for building SIP clients and servers. It handles the complexity of SIP dialogs, media negotiation, and call control while exposing a simple interface for application developers.

## 📚 Documentation

- **[API Documentation](src/api/mod.rs)** - Comprehensive API reference with examples
- **[COOKBOOK.md](COOKBOOK.md)** - Practical recipes for common VoIP scenarios
- **[Examples](examples/)** - Full working examples including:
  - [Clean API Examples](examples/api_best_practices/) - Best practices demonstrations
  - [Client-Server Demo](examples/client-server/) - Complete UAC/UAS implementation

## Features

- 🎯 **Clean API** - Simple traits for session and media control
- 📞 **Complete Call Management** - Make, receive, hold, transfer calls
- 🎵 **Media Integration** - Built-in RTP/RTCP with quality monitoring
- 🔄 **Two Calling Patterns** - Immediate or deferred call decisions
- 📊 **Real-time Statistics** - Monitor call quality and performance
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
        "sip:bob@example.com",
        "sip:alice@192.168.1.100",
        None  // SDP will be generated automatically
    ).await?;
    
    println!("Call initiated: {}", session.id());
    
    // Clean shutdown
    SessionControl::stop(&coordinator).await?;
    Ok(())
}
```

## Core Concepts

### SessionControl Trait

The main interface for call control operations:

```rust
use rvoip_session_core::api::*;

// Create outgoing calls
let session = SessionControl::create_outgoing_call(
    &coordinator,
    "sip:callee@example.com",
    "sip:caller@myserver.com",
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

// Establish media flow
MediaControl::establish_media_flow(
    &coordinator,
    &session_id,
    "192.168.1.100:5004"  // Remote RTP endpoint
).await?;

// Monitor quality
let stats = MediaControl::get_media_statistics(&coordinator, &session_id).await?;
if let Some(quality) = stats.and_then(|s| s.quality_metrics) {
    println!("MOS Score: {:.1}", quality.mos_score.unwrap_or(0.0));
    println!("Packet Loss: {:.1}%", quality.packet_loss_percent);
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
            CallDecision::Accept(Some(generate_sdp_answer()))
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
struct DeferHandler;

#[async_trait::async_trait]
impl CallHandler for DeferHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Defer for async processing
        CallDecision::Defer
    }
}

// Process deferred calls asynchronously
async fn process_deferred_call(
    coordinator: &Arc<SessionCoordinator>,
    call: IncomingCall
) -> Result<()> {
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
    
    Ok(())
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
│           Coordinates Dialogs and Media                     │
│         Implements SessionControl & MediaControl            │
└─────────────────────────────────────────────────────────────┘
                    ↓               ↓
┌─────────────────────┬─────────────────────────────────────┐
│   DialogManager     │      MediaManager                    │
│  SIP Dialog State   │   RTP/Media Sessions                │
│  RFC 3261 Compliant │   Audio Processing                  │
└─────────────────────┴─────────────────────────────────────┘
                    ↓               ↓
┌─────────────────────┬─────────────────────────────────────┐
│ transaction-core    │      media-core                     │
│ SIP Transactions    │   RTP Processing                    │
│ Protocol Handling   │   Codec Support                     │
└─────────────────────┴─────────────────────────────────────┘
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
router.add_route("sip:support@", "sip:queue@support.local");
router.add_route("sip:sales@", "sip:queue@sales.local");
router.add_route("sip:+1800", "sip:tollfree@gateway.com");

let coordinator = SessionManagerBuilder::new()
    .with_sip_port(5060)
    .with_handler(Arc::new(router))
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

1. **Use the Public API** - Never access internal fields like `coordinator.dialog_manager`
2. **Handle Errors** - All operations can fail due to network issues
3. **Monitor Quality** - Use `MediaControl::get_media_statistics()` for production monitoring
4. **Clean Resources** - Always call `terminate_session()` when done
5. **Choose Patterns Wisely** - Use immediate decisions for simple cases, deferred for complex logic

## Dependencies

- `rvoip-sip-core` - Core SIP types and parsing
- `rvoip-dialog-core` - Dialog state management
- `rvoip-transaction-core` - SIP transaction handling
- `rvoip-media-core` - Media processing and codecs
- `rvoip-rtp-core` - RTP/RTCP protocol support

## Testing

Run the test suite:

```bash
cargo test

# Run integration tests
cargo test --test '*'

# Run examples
cd examples/api_best_practices
./run_clean_examples.sh
```

## License

Licensed under either:

- MIT License
- Apache License 2.0

at your option. 