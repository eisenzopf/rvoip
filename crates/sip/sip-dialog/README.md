# rvoip-sip-dialog

[![Crates.io](https://img.shields.io/crates/v/rvoip-sip-dialog.svg)](https://crates.io/crates/rvoip-sip-dialog)
[![Documentation](https://docs.rs/rvoip-sip-dialog/badge.svg)](https://docs.rs/rvoip-sip-dialog)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](../../../LICENSE)

> **Beta scope notice:** for the `rvoip-sip` beta, dialog-layer claims are
> governed by `crates/sip/rvoip-sip/docs/COMPATIBILITY_MATRIX.md` and
> `crates/sip/rvoip-sip/docs/RFC_COMPLIANCE_MATRIX.md`.

RFC 3261 SIP transaction and dialog layers for the [rvoip](../../../README.md) VoIP stack, providing clean separation between session coordination and SIP protocol operations.

## Overview

`rvoip-sip-dialog` implements the SIP dialog layer as defined in RFC 3261, and also contains the SIP transaction layer (the former `transaction-core` crate was merged into this crate as the `transaction` module). It sits between the `rvoip-sip` umbrella crate (session, proxy, and registrar coordination) and `rvoip-sip-transport` (network I/O). This crate manages SIP dialogs, routes messages within dialog contexts, and coordinates with the session layer through well-defined events.

## Features

### ✅ Completed Features

- **SIP Protocol Processing**
  - ✅ INVITE dialog creation and management
  - ✅ BYE dialog termination handling
  - ✅ REGISTER processing with registration coordination
  - ✅ ACK routing within confirmed dialogs
  - ✅ CANCEL request handling for early dialogs
  - ✅ Re-INVITE support for session modifications
  - ✅ OPTIONS, INFO, PRACK (RFC 3262), and UPDATE (RFC 3311) handling
  - ✅ REFER (RFC 3515) with implicit subscription and NOTIFY progress
  - ✅ SUBSCRIBE/NOTIFY (RFC 6665) dialogs (`subscription` module) and presence (`presence` module)

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

- **Transaction Layer** (`transaction` module)
  - ✅ RFC 3261 client/server INVITE and non-INVITE transactions with timers
  - ✅ `TransactionManager` over any `rvoip_sip_transport::Transport`
  - ✅ Ingress authorization seam: `SipRequestIngressAuthorizer` returns
    `SipRequestAuthorization::{Authorized, Rejected, Dropped}`; `Dropped`
    silently discards requests from a source that has exhausted its request
    budget (no response is sent, so floods are not amplified)
  - ✅ RFC 3263 resolution and failover of INVITE targets
    (`tests/rfc3263_resolution.rs`, `tests/rfc3263_failover.rs`)

- **Session Coordination**
  - ✅ Event-driven architecture via `SessionCoordinationEvent` and the
    `rvoip-infra-common` `GlobalEventCoordinator`
  - ✅ SDP negotiation coordination
  - ✅ Incoming call notification events
  - ✅ Call answered/terminated event propagation
  - ✅ Registration event handling

- **Recovery & Reliability**
  - ✅ Dialog recovery from failures (`Recovering` state)
  - ✅ Transaction correlation with dialogs
  - ✅ Graceful error handling and cleanup
  - ✅ Dialog expiration and cleanup

### 🚧 Planned Features

- **Advanced Dialog Management**
  - 🚧 Dialog forking support for parallel searches
  - 🚧 Dialog replacement (RFC 3891) support
  - 🚧 Enhanced dialog recovery mechanisms

- **Protocol Extensions**
  - 🚧 MESSAGE method for instant messaging

- **Performance Optimizations**
  - 🚧 Dialog caching and indexing improvements
  - 🚧 Memory-optimized dialog storage
  - 🚧 Concurrent dialog operation batching

## Architecture

### 🏗️ **Architecture Position**

```
┌─────────────────────────────────────────┐
│      Application Layer                  │
├─────────────────────────────────────────┤
│        Session Layer                    │
│         (rvoip-sip)                     │
├─────────────────────────────────────────┤
│   Dialog + Transaction Layers           │
│   (rvoip-sip-dialog) ⬅️ YOU ARE HERE    │
├─────────────────────────────────────────┤
│       Transport Layer                   │
│     (rvoip-sip-transport)               │
└─────────────────────────────────────────┘
```

### Dialog Management Architecture

`DialogManager` (in `src/manager/core.rs`) owns an `Arc<TransactionManager>`
and the local bind address, and keeps dialogs in `DashMap`s keyed by
`DialogId` plus lookup tables keyed by Call-ID and tags. Session
coordination events are emitted with `emit_session_coordination_event(..)`
through the global event coordinator rather than a per-manager channel.

`UnifiedDialogManager` wraps `DialogManager` with a `DialogManagerConfig`
(client, server, or hybrid behaviour) and exposes the one-liner
`send_*` helpers used by `UnifiedDialogApi`, `DialogClient`, and
`DialogServer`.

### Dialog State Machine

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DialogState {
    Initial,    // Created, before any response
    Early,      // After 1xx response received/sent
    Confirmed,  // After 2xx response received/sent
    Recovering, // Recovery in progress after a failure
    Terminated, // After BYE or error
}
```

## Usage

### Basic Dialog Creation

```rust
use rvoip_sip_dialog::api::{DialogApi, DialogServer};
use rvoip_sip_dialog::api::config::ServerConfig;
use rvoip_sip_dialog::transaction::transport::{TransportManager, TransportManagerConfig};
use rvoip_sip_dialog::transaction::TransactionManager;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Transport layer (UDP on 0.0.0.0:5060)
    let config = TransportManagerConfig {
        enable_udp: true,
        bind_addresses: vec!["0.0.0.0:5060".parse()?],
        ..Default::default()
    };
    let (transport, transport_rx) = TransportManager::new(config).await?;

    // Transaction layer
    let (transaction_manager, transaction_rx) =
        TransactionManager::with_transport_manager(transport, transport_rx, Some(100)).await?;

    // Dialog layer; session coordination events flow via the
    // GlobalEventCoordinator wired up by with_global_events(..)
    let server = DialogServer::with_global_events(
        Arc::new(transaction_manager),
        transaction_rx,
        ServerConfig::default(),
    )
    .await?;

    // Start processing
    server.start().await?;

    Ok(())
}
```

If you want the low-level manager directly, the constructors are:

```rust
use rvoip_sip_dialog::transaction::TransactionManager;
use rvoip_sip_dialog::DialogManager;
use rvoip_sip_transport::UdpTransport;
use std::sync::Arc;

let local_addr = "0.0.0.0:5060".parse()?;
let (transport, transport_rx) = UdpTransport::bind(local_addr, None).await?;
let (transaction_manager, _transaction_rx) =
    TransactionManager::new(Arc::new(transport), transport_rx, None).await?;
let dialog_manager = DialogManager::new(Arc::new(transaction_manager), local_addr).await?;
dialog_manager.start().await?;
```

### Outgoing Call Example

```rust
use rvoip_sip_dialog::api::{DialogApi, DialogClient};
use rvoip_sip_dialog::{DialogError, DialogId};

async fn make_call(
    client: &DialogClient,
    from_uri: &str,
    to_uri: &str,
) -> Result<DialogId, Box<dyn std::error::Error>> {
    // Create an outgoing dialog; the API generates Call-ID, tags, and CSeq
    let dialog = client.create_dialog(from_uri, to_uri).await?;
    let dialog_id = dialog.id().clone();

    // In-dialog one-liners
    let _info_tx = client.send_info(&dialog_id, "Application data".to_string()).await?;
    let _bye_tx = client.send_bye(&dialog_id).await?;

    Ok(dialog_id)
}
```

### Registration Handling

```rust
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_dialog::{DialogError, DialogManager};
use std::net::SocketAddr;

async fn handle_registration(
    dialog_manager: &DialogManager,
    source: SocketAddr,
) -> Result<(), DialogError> {
    let register_request = SimpleRequestBuilder::register("sip:example.com")?
        .from("Alice", "sip:alice@example.com", Some("tag-1"))
        .to("Alice", "sip:alice@example.com", None)
        .call_id("register-1")
        .cseq(1)
        .via("192.0.2.10:5060", "UDP", Some("z9hG4bK-1"))
        .contact("sip:alice@192.0.2.10:5060", None)
        .build();

    // Emits SessionCoordinationEvent::RegistrationRequest for the session layer
    dialog_manager.handle_register(register_request, source).await
}
```

## Relationship to Other Crates

### Core Dependencies

- **`rvoip-sip-core`**: Provides SIP message types, parsing, and core protocol structures
- **`rvoip-sip-transport`**: Provides network transport abstraction (`dns` feature enabled for RFC 3263)
- **`rvoip-infra-common`**: Global event coordinator used for session coordination
- **`rvoip-core-traits`**: Shared identity/principal types used by the ingress authorizer
- **`tokio`**: Async runtime for concurrent dialog processing
- **`async-trait`**: Async trait support for transport abstraction

### Integration with rvoip Stack

```
┌─────────────────────────────────────────┐
│            Application Layer            │
├─────────────────────────────────────────┤
│              rvoip-sip                  │ ← Coordinates sessions
│                    ↕️                    │
│   rvoip-sip-dialog  ⬅️ YOU ARE HERE      │ ← Manages SIP dialogs
│   (transaction module inside)           │ ← Handles reliability
├─────────────────────────────────────────┤
│         rvoip-sip-transport             │ ← Network transport
└─────────────────────────────────────────┘
```

The dialog layer provides:

- **Upward Interface**: `SessionCoordinationEvent`s to `rvoip-sip` via the global event coordinator
- **Downward Interface**: Transaction requests to the in-crate `transaction` module
- **Horizontal Interface**: Dialog state queries for other components

## Performance Characteristics

### Dialog Operations

- **Dialog Creation**: O(1) with `DashMap`-based storage
- **Dialog Lookup**: O(1) average case via Call-ID/tag lookup tables
- **Dialog Cleanup**: Background cleanup tasks to avoid blocking

### Concurrency

- **Lock-free reads**: `DashMap` dialog storage and `arc-swap` event subscriber lists
- **Event Processing**: Async processing with configurable buffer sizes

## Error Handling

`DialogError` uses struct-style variants; `DialogResult<T>` is the alias for
`Result<T, DialogError>`:

```rust
use rvoip_sip_dialog::{DialogError, DialogResult};

match dialog_result {
    Err(DialogError::DialogNotFound { id }) => {
        // Handle missing dialog - often recoverable for new requests
        log::warn!("dialog {} not found", id);
    }
    Err(DialogError::InvalidState { expected, actual }) => {
        // Handle state violations - typically not recoverable
        log::error!("Dialog state error: expected {}, got {}", expected, actual);
    }
    Err(DialogError::TransactionError { message }) => {
        // Handle transaction layer errors
        log::error!("transaction error: {}", message);
    }
    Err(other) => {
        // Every variant maps to a stable, payload-free class for logs/metrics
        log::error!("dialog error class {}: {}", other.diagnostic_class(), other);
    }
    Ok(result) => {
        // Handle success
    }
}
```

Other variants include `DialogAlreadyExists`, `ProtocolError`, `RoutingError`,
`SdpError`, `InternalError`, `NetworkError`, `TimeoutError`, and
`ConfigError` (see `src/errors/dialog_errors.rs`).

## Testing

Run the test suite:

```bash
# Run all tests
cargo test -p rvoip-sip-dialog

# Run with specific features
cargo test -p rvoip-sip-dialog --features "recovery events testing"

# Dialog lifecycle and RFC compliance suites
cargo test -p rvoip-sip-dialog --test dialog_lifecycle
cargo test -p rvoip-sip-dialog --test sip_compliance
cargo test -p rvoip-sip-dialog --features generated-validation --test generated_sip_compliance

# REFER, subscriptions, and RFC 3263 failover
cargo test -p rvoip-sip-dialog --test refer_transfer_tests
cargo test -p rvoip-sip-dialog --test subscription_dialogs
cargo test -p rvoip-sip-dialog --test rfc3263_failover
```

Other suites in [`tests/`](tests/) cover the API layer, BYE termination,
dialog recovery and state, identity signing/verification, MTU failover,
OPTIONS, PRACK, REGISTER flows, request routing, rport restamping, SDP
negotiation, multi-ID subscriptions, and the unified API.

## Features

The crate supports the following optional features:

- **`recovery`** (default): Dialog recovery and persistence capabilities
- **`events`** (default): Enhanced event system with filtering
- **`testing`**: Additional test utilities and mock implementations
- **`generated-validation`**: Forwards to `rvoip-sip-core/generated-validation` for the generated compliance suite
- **`ws`**: WebSocket transport cfg gates (a no-op until `rvoip-sip-transport`'s `ws` feature is wired through)
- **`dev`**: `recovery` + `events` + `testing`
- **`dev-insecure-tls`**: Dev-only; forwards to `rvoip-sip-transport/dev-insecure-tls`

Disable default features and enable only what you need:

```toml
[dependencies]
rvoip-sip-dialog = { version = "0.3.10", default-features = false, features = ["recovery"] }
```

## Examples

The `examples/` directory contains:

- **`basic_dialog.rs`** - Basic dialog creation and management
- **`dialog_recovery.rs`** - Dialog recovery and failure handling
- **`multi_dialog.rs`** - Managing multiple concurrent dialogs
- **`global_events_test.rs`** - Global event coordinator wiring
- **`phase3_integration_showcase.rs`** - The one-liner `send_*` helpers end to end
- **`debug_state_error.rs`**, **`simple_test.rs`** - Small diagnostic programs

Run examples:

```bash
cargo run -p rvoip-sip-dialog --example basic_dialog
cargo run -p rvoip-sip-dialog --example dialog_recovery --features "recovery"
```

## 🔧 **Core API**

### DialogManager
The low-level interface for dialog management:

```rust
impl DialogManager {
    // Lifecycle
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        local_address: SocketAddr,
    ) -> DialogResult<Self>;
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        local_address: SocketAddr,
    ) -> DialogResult<Self>;

    pub async fn start(&self) -> DialogResult<()>;
    pub async fn stop(&self) -> DialogResult<()>;

    // Dialog operations
    pub async fn create_dialog(&self, request: &Request) -> DialogResult<DialogId>;
    pub async fn create_outgoing_dialog(&self, local_uri: Uri, remote_uri: Uri, call_id: Option<String>) -> DialogResult<DialogId>;
    pub async fn find_dialog_for_request(&self, request: &Request) -> Option<DialogId>;
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> DialogResult<()>;

    // Protocol handling
    pub async fn handle_invite(&self, request: Request, source: SocketAddr) -> DialogResult<()>;
    pub async fn handle_bye(&self, request: Request) -> DialogResult<()>;
    pub async fn handle_register(&self, request: Request, source: SocketAddr) -> DialogResult<()>;
    pub async fn handle_refer(&self, request: Request, source: SocketAddr) -> DialogResult<()>;
    pub async fn handle_subscribe(&self, request: Request, source: SocketAddr) -> DialogResult<()>;
    pub async fn handle_notify(&self, request: Request, source: SocketAddr) -> DialogResult<()>;

    // Request/Response operations
    pub async fn send_request(&self, dialog_id: &DialogId, method: Method, body: Option<Bytes>) -> DialogResult<TransactionKey>;
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> DialogResult<()>;

    // Session coordination (delivered through the GlobalEventCoordinator)
    pub async fn emit_session_coordination_event(&self, event: SessionCoordinationEvent);

    // Monitoring and diagnostics
    pub fn dialog_count(&self) -> usize;
    pub fn retention_counts(&self) -> DialogManagerRetentionCounts;
}
```

### UnifiedDialogManager one-liners

`UnifiedDialogManager` (and the `UnifiedDialogApi` / `DialogClient` /
`DialogServer` wrappers) add in-dialog helpers, each returning
`ApiResult<TransactionKey>`:

- `send_bye(&dialog_id)`, `send_cancel(&dialog_id)`, `send_prack(&dialog_id, rseq)`
- `send_refer(&dialog_id, target_uri, refer_body)` and `send_refer_notify(&dialog_id, status_code, reason)`
- `send_notify(&dialog_id, event, body, subscription_state)`
- `send_update(&dialog_id, sdp)` and `send_info(&dialog_id, info_body)`
- `send_subscribe_out_of_dialog_for_session(..)` and `send_subscribe_refresh(..)`
- `send_invite_with_auth(..)`, `send_invite_with_options(..)`, and the session-timer variants

`UnifiedDialogApi::get_stats()` returns `DialogStats` (active/total dialogs,
successful/failed calls).

### Session Coordination Events
Events delivered to the session layer (`src/events/session_coordination.rs`):

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
        new_state: String,
        previous_state: String,
    },
    // ... plus ReInvite, CallRinging, CallTerminating, ByeReceived, CallCancelled,
    // ResponseReceived, EarlyMedia, TransferRequest, SessionRefreshed,
    // OutboundFlowFailed, RegisteredFlowClosed, and others
}
```

## 🔍 **Integration with RVOIP**

`rvoip-sip` uses this crate as its dialog and transaction layer. It builds a
`UnifiedDialogApi` with the shared global event coordinator
(`UnifiedDialogApi::with_shared_global_events_and_coordinator(..)`) and
subscribes to `SessionCoordinationEvent`s there; there is no per-manager
`set_session_coordinator` channel any more.

```rust
use rvoip_sip_dialog::{DialogManagerConfig, UnifiedDialogApi};
use std::sync::Arc;

let config = DialogManagerConfig::client("0.0.0.0:5060".parse()?)
    .with_from_uri("sip:alice@example.com")
    .build();

let api = UnifiedDialogApi::with_global_events_and_coordinator(
    Arc::new(transaction_manager),
    transaction_rx,
    config,
    global_coordinator, // Arc<rvoip_infra_common::events::coordinator::GlobalEventCoordinator>
)
.await?;
```

## Future Improvements

See [TODO.md](./TODO.md) for a comprehensive list of planned enhancements, including:

- Advanced dialog forking and parallel search support
- Enhanced dialog recovery mechanisms with persistent state
- Performance optimizations for high-scale deployments
- MESSAGE method support
- Advanced monitoring and diagnostics capabilities

## 🚀 **Development Status**

- ✅ Core dialog management implemented
- ✅ Transaction layer merged into this crate (`transaction` module)
- ✅ Protocol handling for INVITE, BYE, CANCEL, ACK, REGISTER, OPTIONS, INFO, PRACK, UPDATE, REFER, SUBSCRIBE, NOTIFY
- ✅ Session coordination events via the global event coordinator
- ✅ RFC 3263 resolution and failover
- 🚧 Advanced recovery mechanisms
- 🚧 Performance optimizations
- 🚧 MESSAGE method

## Contributing

Contributions are welcome! Please see the main [rvoip contributing guidelines](../../../README.md#contributing) for details.

When contributing to `rvoip-sip-dialog`:
1. Ensure proper RFC 3261 compliance
2. Maintain clean layer separation
3. Add comprehensive tests for new functionality
4. Update documentation and examples
5. Follow the existing API patterns

## License

This project is licensed under the [MIT license](../../../LICENSE).
