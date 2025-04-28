# rvoip-session-core

`rvoip-session-core` is a Rust library that provides Session Initiation Protocol (SIP) session and dialog management for the RVOIP stack. It serves as the middle layer between the low-level SIP transaction processing and high-level application logic for VoIP applications.

## Overview

This library implements RFC-compliant SIP dialog and session management, providing a clean, type-safe API for building SIP clients and servers. It handles the complex state machines defined in SIP RFCs while abstracting away many of the protocol intricacies.

## Role in SIP Applications

### For SIP Clients

In client applications (`rvoip-sip-client`), `session-core` manages:

- Outgoing call establishment (INVITE transactions)
- Dialog state tracking through call setup, in-call operations, and termination
- Media stream negotiation via SDP
- Mid-call operations (hold, transfer)
- Call termination (BYE transactions)
- Event notifications for UI updates

### For SIP Servers

In server applications (`rvoip-call-engine`), `session-core` provides:

- Incoming call processing
- Server-side dialog state management
- Session tracking for multiple concurrent calls
- Media negotiation for server-side media processing
- Call routing based on dialog relationships
- Resource management for active calls

## Architecture

The library is structured around several core components:

- **Dialog Management**: Tracks SIP dialogs (call relationships) according to RFC 3261
- **Session Management**: Provides higher-level call session abstraction on top of dialogs
- **Media Handling**: Manages media stream setup, configuration, and negotiation via SDP
- **Event System**: Provides asynchronous notifications for session and dialog state changes

```
┌─────────────────┐    ┌──────────────────┐    ┌───────────────┐
│ SIP Client/     │    │ Transaction Core │    │ SIP Core      │
│ Server App      │◄───┤ Dialog & Session │◄───┤ Messages &    │
│ (UI/Logic)      │    │ Management       │    │ Transports    │
└─────────────────┘    └──────────────────┘    └───────────────┘
                           │       ▲
                           ▼       │
                        ┌──────────────────┐
                        │ Media Core       │
                        │ (RTP/Audio)      │
                        └──────────────────┘
```

## RFC Compliance

### Supported RFC Components

The library implements the following key RFCs and sections:

#### Core SIP (RFC 3261)
- **Dialog Management** (Section 12) ✅
  - Dialog creation from INVITE responses
  - Dialog identification (Call-ID, tags)
  - Dialog state machine (early, confirmed, terminated states)
  - Route set management
  
- **Session Management** ✅
  - INVITE handling
  - ACK processing
  - BYE processing
  - Call establishment flows
  
- **In-Dialog Requests** ✅
  - CSeq handling
  - Route header generation
  - Target refresh processing

#### SDP for Media Negotiation (RFC 4566, RFC 3264) ✅
- SDP message parsing and generation
- Support for common audio codecs (G.711, etc.)
- Basic offer/answer model

#### Core Call Features ✅
- Basic call hold
- Session modification (re-INVITE)
- Call termination

### Partially Supported

#### Reliable Provisional Responses (RFC 3262) ⚠️
- Basic 1xx response handling
- Limited PRACK support

#### Call Transfer (RFC 3515) ⚠️
- Partial REFER processing
- Basic transfer support

#### Dialog Event Package (RFC 4235) ⚠️
- Limited dialog event notifications

### Not Currently Supported

- **SIP Authentication** (RFC 3261 Section 22) ❌
- **SIP Security (TLS, SIPS)** ❌
- **ICE for NAT Traversal** (RFC 8445) ❌
- **SRTP for Media Encryption** (RFC 3711) ❌
- **SIP Identity** (RFC 8224) ❌
- **Presence and Events Framework** (RFC 3856, RFC 3265) ❌
- **Advanced Call Transfers and Replaces** (RFC 3891) ❌

## Usage

### Basic Client Example

```rust
use rvoip_session_core::prelude::*;

async fn make_call(transaction_manager: Arc<TransactionManager>, uri: &str) -> Result<SessionId> {
    // Create session manager
    let config = SessionConfig::default();
    let event_bus = EventBus::new(100);
    let session_manager = Arc::new(SessionManager::new(
        transaction_manager,
        config,
        event_bus.clone()
    ));
    
    // Create outgoing session
    let session = session_manager.create_outgoing_session().await?;
    
    // Parse destination URI
    let target_uri = Uri::from_str(uri)?;
    
    // Send INVITE
    session.send_invite(target_uri).await?;
    
    Ok(session.id.clone())
}
```

### Basic Server Example

```rust
use rvoip_session_core::prelude::*;

async fn handle_incoming_call(
    transaction_manager: Arc<TransactionManager>,
    request: Request
) -> Result<Response> {
    // Create session manager
    let config = SessionConfig::default();
    let event_bus = EventBus::new(100);
    let session_manager = Arc::new(SessionManager::new(
        transaction_manager,
        config,
        event_bus.clone()
    ));
    
    // Create incoming session
    let session = session_manager.create_incoming_session(request.clone()).await?;
    
    // Process the INVITE request
    session.handle_request(request).await
}
```

## Dependencies

- `rvoip-sip-core`: Provides core SIP message types and parsing
- `rvoip-transaction-core`: Handles SIP transactions (request-response pairs)
- `rvoip-rtp-core`: Manages RTP media packets
- `rvoip-media-core`: Handles audio processing and codecs

## License

This crate is licensed under either:

- MIT License
- Apache License 2.0

at your option. 