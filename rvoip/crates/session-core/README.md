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

The library is structured around several core components with proper RFC 3261 separation of concerns:

- **Session Management**: Coordinates call flows and integrates with media processing
- **Dialog Management**: Handles pure SIP protocol dialog state per RFC 3261  
- **Media Handling**: Manages media stream setup, configuration, and negotiation via SDP
- **Event System**: Provides asynchronous notifications for session and dialog state changes

### **RFC 3261 Compliant Architecture**

```
┌─────────────────────────────────────────────────────────────┐
│               ServerManager (Policy Layer)                  │
│                 • Business Rules & Decisions                │
│                 • Call Acceptance Policies                  │
│                 • Resource Management                       │
└─────────────────────────────────────────────────────────────┘
           ↓ delegates implementation
┌─────────────────────────────────────────────────────────────┐
│             SessionManager (Coordination Layer)             │
│          • CallLifecycleCoordinator (call flows)            │
│          • Media stream coordination                        │
│          • Session state management                         │ 
│          • Multi-dialog coordination                        │
│          • Reacts to Transaction Events                     │
│          • SIGNALS transaction-core for responses           │
└─────────────────────────────────────────────────────────────┘
           ↓ delegates protocol work  
┌─────────────────────────────────────────────────────────────┐
│              DialogManager (Protocol Layer)                 │
│                • Pure RFC 3261 Dialog State                 │
│                • Dialog ID management                       │
│                • In-dialog request routing                  │
│                • SIP Protocol Compliance                    │
└─────────────────────────────────────────────────────────────┘
           ↓ integrates with core protocols
┌─────────────────────────────────────────────────────────────┤
│         Processing Layer                                    │
│  transaction-core              │  media-core               │
│  (SIP Protocol Handler)        │  (Media Processing)       │
│  • Sends SIP Responses ✅      │  • Real RTP Port Alloc ✅ │
│  • Manages SIP State Machine ✅│  • MediaSessionController ✅│
│  • Handles Retransmissions ✅  │  • RTP Stream Management ✅│
│  • Timer 100 (100 Trying) ✅   │  • SDP Generation ✅      │
├─────────────────────────────────────────────────────────────┤
│              Transport Layer                                │
│  sip-transport ✅  │  rtp-core ✅  │  ice-core ✅          │
└─────────────────────────────────────────────────────────────┘
```

**Key Architectural Principles:**

- **Clean Separation**: Policy, coordination, and protocol layers are properly separated
- **RFC 3261 Compliance**: DialogManager focuses purely on SIP protocol state machine
- **Session Coordination**: SessionManager coordinates call flows and media integration
- **No Double Handling**: Each SIP message processed once at appropriate layer
- **Proper Delegation**: Each layer delegates to lower layers, doesn't bypass them

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