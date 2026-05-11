# rvoip-dialog-core

[![Crates.io](https://img.shields.io/crates/v/rvoip-dialog-core.svg)](https://crates.io/crates/rvoip-dialog-core)
[![Documentation](https://docs.rs/rvoip-dialog-core/badge.svg)](https://docs.rs/rvoip-dialog-core)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

RFC 3261 SIP Dialog Management Layer for the [rvoip](../README.md) VoIP stack, providing clean separation between session coordination and SIP protocol operations.

## Overview

`rvoip-dialog-core` implements the SIP dialog layer as defined in RFC 3261, serving as the protocol processing engine between session coordination (handled by `session-core`) and transaction reliability (handled by `transaction-core`). This crate manages SIP dialogs, routes messages within dialog contexts, and coordinates with the session layer through well-defined events.

## Features

### ✅ Completed Features

- **SIP Protocol Processing**
  - ✅ INVITE dialog creation and management
  - ✅ BYE dialog termination handling
  - ✅ REGISTER processing with registration coordination
  - ✅ ACK routing within confirmed dialogs
  - ✅ CANCEL request handling for early dialogs
  - ✅ Re-INVITE support for session modifications

- **Dialog State Management**
  - ✅ RFC 3261 compliant dialog state machine
  - ✅ Early dialog handling (1xx responses)
  - ✅ Confirmed dialog management (2xx responses)
  - ✅ Dialog identification using Call-ID, tags, and CSeq
  - ✅ Proper dialog lifetime management
  - ✅ Dialog routing table maintenance

- **SIP Header Management**
  - ✅ Call-ID generation and validation
  - ✅ From/To tag management
  - ✅ CSeq number sequencing
  - ✅ Via header processing for routing
  - ✅ Contact header management
  - ✅ Route/Record-Route header handling

- **Session Coordination**
  - ✅ Event-driven architecture with `session-core`
  - ✅ SDP negotiation coordination
  - ✅ Incoming call notification events
  - ✅ Call answered/terminated event propagation
  - ✅ Registration event handling

- **Recovery & Reliability**
  - ✅ Dialog recovery from failures
  - ✅ Transaction correlation with dialogs
  - ✅ Graceful error handling and cleanup
  - ✅ Dialog expiration and cleanup

### 🚧 Planned Features

- **Advanced Dialog Management**
  - 🚧 Dialog forking support for parallel searches
  - 🚧 Dialog replacement (RFC 3891) support
  - 🚧 Enhanced dialog recovery mechanisms
  - 🚧 Dialog transfer coordination

- **Protocol Extensions**
  - 🚧 SUBSCRIBE/NOTIFY dialog handling
  - 🚧 REFER method support for call transfers
  - 🚧 MESSAGE method for instant messaging
  - 🚧 UPDATE method for mid-dialog updates

- **Performance Optimizations**
  - 🚧 Dialog caching and indexing improvements
  - 🚧 Memory-optimized dialog storage
  - 🚧 High-throughput dialog processing
  - 🚧 Concurrent dialog operation batching

- **Event System Integration**
  - 🚧 Integration with infra-common event bus
  - 🚧 Priority-based event processing
  - 🚧 Advanced event filtering and routing

## Architecture

### 🏗️ **Architecture Position**

```
┌─────────────────────────────────────────┐
│      Application Layer                  │
│    (client-core, call-engine)           │
├─────────────────────────────────────────┤
│        Session Layer                    │
│       (session-core)                    │
├─────────────────────────────────────────┤
│        Dialog Layer                     │
│      (dialog-core) ⬅️ YOU ARE HERE      │
├─────────────────────────────────────────┤
│      Transaction Layer                  │
│     (transaction-core)                  │
├─────────────────────────────────────────┤
│       Transport Layer                   │
│      (sip-transport)                    │
└─────────────────────────────────────────┘
```

### Dialog Management Architecture

```rust
pub struct DialogManager {
    // Core components
    transaction_manager: Arc<TransactionManager>,
    transport: Arc<dyn SipTransport>,
    
    // Dialog storage and routing
    dialogs: Arc<RwLock<DialogStore>>,
    routing_table: Arc<RwLock<DialogRoutingTable>>,
    
    // Session coordination
    session_coordinator: Option<mpsc::Sender<SessionCoordinationEvent>>,
    
    // Event processing
    event_processor: Arc<DialogEventProcessor>,
}
```

### Dialog State Machine

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum DialogState {
    Early,      // After 1xx response received/sent
    Confirmed,  // After 2xx response received/sent
    Terminated, // After BYE or error
}
```

## Usage

### Basic Dialog Creation

```rust
use rvoip_dialog_core::{DialogManager, DialogError, SessionCoordinationEvent};
use rvoip_transaction_core::TransactionManager;
use rvoip_sip_transport::UdpTransport;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), DialogError> {
    // Create dependencies
    let transaction_manager = Arc::new(TransactionManager::new().await?);
    let transport = Arc::new(UdpTransport::new("0.0.0.0:5060").await?);
    
    // Create dialog manager
    let dialog_manager = DialogManager::new(
        transaction_manager,
        transport
    ).await?;
    
    // Set up session coordination
    let (session_tx, mut session_rx) = tokio::sync::mpsc::channel(100);
    dialog_manager.set_session_coordinator(session_tx);
    
    // Handle session events
    tokio::spawn(async move {
        while let Some(event) = session_rx.recv().await {
            match event {
                SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
                    println!("New incoming call: {:?}", dialog_id);
                }
                SessionCoordinationEvent::CallAnswered { dialog_id, .. } => {
                    println!("Call answered: {:?}", dialog_id);
                }
                _ => {}
            }
        }
    });
    
    // Start processing
    dialog_manager.start().await?;
    
    Ok(())
}
```

### Outgoing Call Example

```rust
use rvoip_dialog_core::{DialogManager, DialogId};
use rvoip_sip_core::{Method, Request, Uri};

async fn make_call(
    dialog_manager: &DialogManager,
    from_uri: Uri,
    to_uri: Uri,
    sdp_offer: String,
) -> Result<DialogId, DialogError> {
    // Create INVITE request
    let invite_request = Request::builder()
        .method(Method::INVITE)
        .uri(to_uri.clone())
        .header("From", format!("<{}>;tag={}", from_uri, generate_tag()))
        .header("To", format!("<{}>", to_uri))
        .header("Call-ID", generate_call_id())
        .header("CSeq", "1 INVITE")
        .header("Content-Type", "application/sdp")
        .body(sdp_offer)
        .build()?;
    
    // Create dialog and send INVITE
    let dialog_id = dialog_manager.create_dialog(&invite_request).await?;
    let transaction_id = dialog_manager.send_request(
        &dialog_id,
        Method::INVITE,
        Some(invite_request.body().clone())
    ).await?;
    
    Ok(dialog_id)
}
```

### Registration Handling

```rust
async fn handle_registration(
    dialog_manager: &DialogManager,
    user_uri: Uri,
    contact_uri: Uri,
    expires: u32,
) -> Result<(), DialogError> {
    let register_request = Request::builder()
        .method(Method::REGISTER)
        .uri(user_uri.clone())
        .header("From", format!("<{}>", user_uri))
        .header("To", format!("<{}>", user_uri))
        .header("Contact", format!("<{}>;expires={}", contact_uri, expires))
        .header("Call-ID", generate_call_id())
        .header("CSeq", "1 REGISTER")
        .build()?;
    
    dialog_manager.handle_register(
        register_request,
        "0.0.0.0:5060".parse()?
    ).await?;
    
    Ok(())
}
```

### Dialog Recovery

```rust
use rvoip_dialog_core::recovery::{DialogRecoveryManager, RecoveryConfig};

async fn setup_dialog_recovery(
    dialog_manager: &DialogManager,
) -> Result<(), DialogError> {
    let recovery_config = RecoveryConfig {
        enable_state_persistence: true,
        recovery_timeout: Duration::from_secs(30),
        max_recovery_attempts: 3,
    };
    
    let recovery_manager = DialogRecoveryManager::new(
        recovery_config,
        dialog_manager.clone()
    ).await?;
    
    // Enable automatic recovery
    recovery_manager.enable_auto_recovery().await?;
    
    Ok(())
}
```

## Relationship to Other Crates

### Core Dependencies

- **`rvoip-sip-core`**: Provides SIP message types, parsing, and core protocol structures
- **`rvoip-transaction-core`**: Handles transaction reliability and retransmission
- **`rvoip-sip-transport`**: Provides network transport abstraction
- **`tokio`**: Async runtime for concurrent dialog processing
- **`async-trait`**: Async trait support for transport abstraction

### Optional Dependencies

- **`rvoip-infra-common`**: Event bus integration (planned)
- **`serde`**: Serialization support for dialog persistence (recovery feature)
- **`tracing`**: Enhanced logging and observability (monitoring feature)

### Integration with rvoip Stack

```
┌─────────────────────────────────────────┐
│            Application Layer            │
│         (client-core, call-engine)      │
├─────────────────────────────────────────┤
│          rvoip-session-core             │ ← Coordinates sessions
│                    ↕️                    │
│         rvoip-dialog-core  ⬅️ YOU ARE HERE │ ← Manages SIP dialogs
│                    ↕️                    │
│         rvoip-transaction-core          │ ← Handles reliability
├─────────────────────────────────────────┤
│         rvoip-sip-transport             │ ← Network transport
└─────────────────────────────────────────┘
```

The dialog layer provides:

- **Upward Interface**: Session coordination events to `session-core`
- **Downward Interface**: Transaction requests to `transaction-core`
- **Horizontal Interface**: Dialog state queries for other components

## Performance Characteristics

### Dialog Operations

- **Dialog Creation**: O(1) with optimized hash-based storage
- **Dialog Lookup**: O(1) average case with efficient routing table
- **Message Routing**: O(1) for established dialogs, O(log n) for routing decisions
- **Dialog Cleanup**: Batched cleanup to minimize lock contention

### Memory Management

- **Dialog Storage**: Memory-efficient with reference counting for shared data
- **Event Processing**: Zero-copy event propagation where possible
- **Header Processing**: Cached header parsing to avoid repeated work
- **Transaction Correlation**: Optimized correlation tables

### Concurrency

- **Read-Heavy Workloads**: Optimized with `RwLock` for dialog access
- **Write Operations**: Minimized lock scope for dialog modifications
- **Event Processing**: Async processing with configurable buffer sizes
- **Resource Cleanup**: Background cleanup tasks to avoid blocking

## Error Handling

The crate provides comprehensive error handling with categorized error types:

```rust
use rvoip_dialog_core::{DialogError, DialogResult};

match dialog_result {
    Err(DialogError::DialogNotFound(dialog_id)) => {
        // Handle missing dialog - often recoverable for new requests
        if request.method() == Method::INVITE {
            create_new_dialog(request).await?;
        }
    }
    Err(DialogError::InvalidDialogState { current, expected }) => {
        // Handle state violations - typically not recoverable
        log::error!("Dialog state error: expected {:?}, got {:?}", expected, current);
        terminate_dialog(dialog_id).await?;
    }
    Err(DialogError::TransactionError(tx_error)) => {
        // Handle transaction layer errors - may be recoverable
        if tx_error.is_recoverable() {
            retry_operation().await?;
        }
    }
    Err(DialogError::SessionCoordinationFailed(msg)) => {
        // Handle session layer communication errors
        log::warn!("Session coordination failed: {}", msg);
        // Continue dialog processing without session coordination
    }
    Ok(result) => {
        // Handle success
    }
}
```

### Error Categories

```rust
impl DialogError {
    pub fn is_recoverable(&self) -> bool {
        match self {
            DialogError::DialogNotFound(_) => true,
            DialogError::TransactionError(e) => e.is_recoverable(),
            DialogError::InvalidDialogState { .. } => false,
            DialogError::SessionCoordinationFailed(_) => true,
            _ => false,
        }
    }
}
```

## Testing

Run the comprehensive test suite:

```bash
# Run all tests
cargo test -p rvoip-dialog-core

# Run with specific features
cargo test -p rvoip-dialog-core --features "recovery events testing"

# Run integration tests
cargo test -p rvoip-dialog-core --test integration_tests

# Run RFC compliance tests
cargo test -p rvoip-dialog-core --test rfc_compliance

# Run with SIPp interoperability tests
cargo test -p rvoip-dialog-core --test sipp_integration

# Run performance benchmarks
cargo bench -p rvoip-dialog-core
```

## Features

The crate supports the following optional features:

- **`recovery`** (default): Dialog recovery and persistence capabilities
- **`events`** (default): Enhanced event system with filtering
- **`monitoring`** (default): Metrics and observability support
- **`testing`**: Additional test utilities and mock implementations

Disable default features and enable only what you need:

```toml
[dependencies]
rvoip-dialog-core = { version = "0.1", default-features = false, features = ["recovery"] }
```

## Examples

The `examples/` directory contains comprehensive examples:

- **`basic_dialog.rs`** - Basic dialog creation and management
- **`dialog_recovery.rs`** - Dialog recovery and failure handling
- **`multi_dialog.rs`** - Managing multiple concurrent dialogs
- **`outgoing_call.rs`** - Complete outgoing call flow
- **`registration_server.rs`** - Registration processing example
- **`session_coordination.rs`** - Integration with session-core

Run examples:

```bash
cargo run -p rvoip-dialog-core --example basic_dialog
cargo run -p rvoip-dialog-core --example outgoing_call --features "recovery"
```

## 🔧 **Core API**

### DialogManager
The main interface for dialog management:

```rust
impl DialogManager {
    // Lifecycle
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        transport: Arc<dyn SipTransport>
    ) -> Result<Self, DialogError>;
    
    pub async fn start(&self) -> Result<(), DialogError>;
    pub async fn stop(&self) -> Result<(), DialogError>;
    
    // Dialog operations
    pub async fn create_dialog(&self, request: &Request) -> Result<DialogId, DialogError>;
    pub async fn find_dialog(&self, request: &Request) -> Option<DialogId>;
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> Result<(), DialogError>;
    
    // Protocol handling
    pub async fn handle_invite(&self, request: Request, source: SocketAddr) -> Result<(), DialogError>;
    pub async fn handle_bye(&self, request: Request) -> Result<(), DialogError>;
    pub async fn handle_register(&self, request: Request, source: SocketAddr) -> Result<(), DialogError>;
    
    // Request/Response operations
    pub async fn send_request(&self, dialog_id: &DialogId, method: Method, body: Option<Bytes>) -> Result<TransactionKey, DialogError>;
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> Result<(), DialogError>;
    
    // Session coordination
    pub fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>);
    
    // Monitoring and diagnostics
    pub fn get_dialog_count(&self) -> usize;
    pub fn get_dialog_stats(&self) -> DialogStats;
}
```

### Session Coordination Events
Events sent to `session-core` for session management:

```rust
#[derive(Debug, Clone)]
pub enum SessionCoordinationEvent {
    IncomingCall {
        dialog_id: DialogId,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr,
    },
    CallAnswered {
        dialog_id: DialogId,
        session_answer: String, // SDP
    },
    CallTerminated {
        dialog_id: DialogId,
        reason: String,
    },
    RegistrationRequest {
        transaction_id: TransactionKey,
        from_uri: Uri,
        contact_uri: Uri,
        expires: u32,
    },
    DialogStateChanged {
        dialog_id: DialogId,
        old_state: DialogState,
        new_state: DialogState,
    },
}
```

## 🔍 **Integration with RVOIP**

This crate is designed to be used by `session-core` as its dialog management layer:

```rust
// In session-core
use rvoip_dialog_core::{DialogManager, SessionCoordinationEvent};

impl SessionManager {
    pub async fn new() -> Result<Self, Error> {
        let dialog_manager = DialogManager::new(
            transaction_manager,
            transport
        ).await?;
        
        // Set up coordination
        let (coord_tx, coord_rx) = mpsc::channel(100);
        dialog_manager.set_session_coordinator(coord_tx);
        
        // Handle coordination events
        self.spawn_coordination_handler(coord_rx);
        
        Ok(SessionManager {
            dialog_manager,
            // ... other fields
        })
    }
    
    async fn spawn_coordination_handler(&self, mut coord_rx: mpsc::Receiver<SessionCoordinationEvent>) {
        tokio::spawn(async move {
            while let Some(event) = coord_rx.recv().await {
                match event {
                    SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
                        // Create new session for incoming call
                        self.create_session(dialog_id).await;
                    }
                    SessionCoordinationEvent::CallTerminated { dialog_id, .. } => {
                        // Clean up session resources
                        self.terminate_session(dialog_id).await;
                    }
                    _ => {}
                }
            }
        });
    }
}
```

## Future Improvements

See [TODO.md](./TODO.md) for a comprehensive list of planned enhancements, including:

- Advanced dialog forking and parallel search support
- Enhanced dialog recovery mechanisms with persistent state
- Integration with infra-common event bus for high-throughput processing
- Performance optimizations for high-scale deployments
- Protocol extensions (SUBSCRIBE/NOTIFY, REFER, MESSAGE)
- Advanced monitoring and diagnostics capabilities

## 🚀 **Development Status**

This crate is part of the RVOIP architecture refactoring to establish clean layer separation. Current status:

- ✅ Core dialog management implemented
- ✅ Basic protocol handling (INVITE, BYE, REGISTER)
- ✅ Session coordination events
- 🚧 Advanced recovery mechanisms
- 🚧 Performance optimizations
- 🚧 Protocol extensions

## Contributing

Contributions are welcome! Please see the main [rvoip contributing guidelines](../../README.md#contributing) for details.

When contributing to dialog-core:
1. Ensure proper RFC 3261 compliance
2. Maintain clean layer separation
3. Add comprehensive tests for new functionality  
4. Update documentation and examples
5. Follow the existing API patterns

## License

This project is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option. 