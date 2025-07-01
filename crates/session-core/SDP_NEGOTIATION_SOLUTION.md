# SDP Negotiation Solution for rvoip

## Overview

This document describes the long-term solution for SDP negotiation in the rvoip stack that works with both client-core and call-engine through session-core.

## Architecture

### 1. **Centralized SDP Negotiation in session-core**

The SDP negotiation logic is centralized in session-core, which acts as the coordinator between:
- **dialog-core**: SIP signaling and protocol handling
- **media-core**: RTP/RTCP media streaming
- **Applications**: client-core, call-engine, or any other application using session-core

### 2. **Key Components**

#### SdpNegotiator (`session-core/src/sdp/negotiator.rs`)
- Handles offer/answer negotiation for both UAC and UAS roles
- Uses MediaConfig preferences to generate and negotiate SDP
- Applies negotiated configuration to media-core
- Stores negotiated results for query

#### MediaConfig (`session-core/src/api/builder.rs`)
- Defines media preferences: codecs, audio processing, bandwidth, etc.
- Passed from applications (client-core/call-engine) to session-core
- Used by SdpNegotiator for generating offers/answers

#### NegotiatedMediaConfig (`session-core/src/sdp/types.rs`)
- Contains the result of SDP negotiation
- Includes: negotiated codec, local/remote RTP addresses, role, etc.
- Queryable through session-core API

### 3. **Integration Points**

#### For client-core:
```rust
// 1. Configure media preferences when building coordinator
let coordinator = SessionManagerBuilder::new()
    .with_media_config(MediaConfig {
        preferred_codecs: vec!["opus", "G722", "PCMU"],
        echo_cancellation: true,
        // ... other preferences
    })
    .build()
    .await?;

// 2. For deferred incoming calls, generate SDP answer
let answer = generate_sdp_answer(&coordinator, &call.id, &their_offer).await?;
SessionControl::accept_incoming_call(&coordinator, &call, Some(answer)).await?;

// 3. Query negotiated configuration
if let Some(config) = get_negotiated_media_config(&coordinator, &session_id).await? {
    println!("Negotiated codec: {}", config.codec);
}
```

#### For call-engine:
Same API through session-core, allowing consistent behavior across all applications.

## Implementation Details

### 1. **Media Preference Flow**

```
Application (client-core/call-engine)
    ↓ MediaConfig via SessionManagerBuilder
session-core (SessionCoordinator)
    ↓ Stores in SdpNegotiator
SdpNegotiator
    ↓ Uses preferences for SDP generation
MediaManager
    ↓ Generates SDP with preferred codecs
dialog-core
    ↓ Sends SDP in SIP messages
```

### 2. **SDP Negotiation Flow**

#### UAC (Outgoing Call):
1. MediaManager generates SDP offer with preferred codecs
2. dialog-core sends INVITE with offer
3. Receives 200 OK with answer
4. SdpNegotiator parses answer and determines negotiated codec
5. Applies configuration to media-core
6. Stores result in negotiated_configs

#### UAS (Incoming Call):
1. Receives INVITE with offer
2. SdpNegotiator generates answer based on preferences
3. Selects first mutually supported codec
4. dialog-core sends 200 OK with answer
5. Applies configuration to media-core after ACK
6. Stores result in negotiated_configs

### 3. **Key Files Modified**

- `session-core/src/sdp/mod.rs` - New SDP negotiation module
- `session-core/src/sdp/types.rs` - Negotiation types
- `session-core/src/sdp/negotiator.rs` - Core negotiation logic
- `session-core/src/coordinator/coordinator.rs` - Integration with coordinator
- `session-core/src/api/control.rs` - Public API functions
- `session-core/src/media/manager.rs` - Enhanced with preferences
- `session-core/src/media/config.rs` - SDP generation with preferences

## Usage Examples

### 1. **Simple Auto-Answer with Default Codecs**

```rust
struct AutoAnswerHandler;

#[async_trait]
impl CallHandler for AutoAnswerHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Let session-core handle SDP negotiation with defaults
        CallDecision::Accept(None)
    }
}
```

### 2. **Deferred Processing with Custom Negotiation**

```rust
struct DeferringHandler {
    call_queue: Arc<Mutex<Vec<IncomingCall>>>,
}

#[async_trait]
impl CallHandler for DeferringHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        self.call_queue.lock().unwrap().push(call);
        CallDecision::Defer
    }
}

// Process deferred calls
async fn process_calls(coordinator: &Arc<SessionCoordinator>, queue: Arc<Mutex<Vec<IncomingCall>>>) {
    while let Some(call) = queue.lock().unwrap().pop() {
        if let Some(offer) = &call.sdp {
            // Generate answer based on configured preferences
            let answer = generate_sdp_answer(coordinator, &call.id, offer).await?;
            SessionControl::accept_incoming_call(coordinator, &call, Some(answer)).await?;
            
            // Check what was negotiated
            if let Some(config) = get_negotiated_media_config(coordinator, &call.id).await? {
                println!("Using codec: {}", config.codec);
            }
        }
    }
}
```

### 3. **Enterprise PBX with Codec Policies**

```rust
// Configure different codec preferences for different user groups
let executive_config = MediaConfig {
    preferred_codecs: vec!["opus", "G722"],  // High quality
    max_bandwidth_kbps: Some(128),
    ..Default::default()
};

let standard_config = MediaConfig {
    preferred_codecs: vec!["PCMU", "PCMA"],  // Standard quality
    max_bandwidth_kbps: Some(64),
    ..Default::default()
};
```

## Benefits

1. **Centralized Logic**: All SDP negotiation happens in session-core
2. **Consistent Behavior**: Same negotiation for client-core and call-engine
3. **Flexible Configuration**: Applications can specify preferences
4. **Proper Separation**: Applications don't need to understand SDP details
5. **Queryable Results**: Applications can check what was negotiated

## Migration Path

For existing code:
1. Update to use `SessionManagerBuilder.with_media_config()`
2. For deferred calls, use `generate_sdp_answer()` helper
3. Query negotiated results with `get_negotiated_media_config()`
4. Remove any manual SDP generation/parsing code

## Future Enhancements

1. **Advanced Codec Negotiation**: Support for codec parameters, fmtp lines
2. **Video Support**: Extend to handle video codecs
3. **Dynamic Updates**: Support for mid-call codec changes
4. **SDP Attributes**: Support for custom SDP attributes
5. **Bandwidth Management**: Adaptive codec selection based on bandwidth 