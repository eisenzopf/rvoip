# RVOIP Dialog-Core

RFC 3261 SIP Dialog Management Layer for RVOIP.

## 🎯 **Purpose**

`dialog-core` implements the SIP dialog layer as defined in RFC 3261, providing a clean separation between session coordination (handled by `session-core`) and SIP protocol operations. This crate is responsible for managing SIP dialogs and routing SIP messages according to the RFC 3261 specification.

## 🏗️ **Architecture Position**

```
Application Layer (client-core, call-engine)
                     ↓
        Session Layer (session-core) 
                     ↓
        Dialog Layer (dialog-core) ← YOU ARE HERE
                     ↓
      Transaction Layer (transaction-core)
                     ↓
       Transport Layer (sip-transport)
```

## 📋 **Responsibilities**

### ✅ **What dialog-core handles:**
- SIP protocol processing (INVITE, BYE, REGISTER, etc.)
- Dialog state management per RFC 3261
- Request/response routing within dialogs
- SIP header management (Call-ID, tags, CSeq)
- Early and confirmed dialog handling
- SDP negotiation coordination
- Dialog recovery and failure handling

### 🔄 **What dialog-core delegates:**
- **Transaction reliability** → `transaction-core`
- **Network transport** → `sip-transport`
- **Session coordination** → `session-core` (via events)
- **Media processing** → `media-core` (via session-core)

## 🚀 **Quick Start**

```rust
use rvoip_dialog_core::{DialogManager, DialogError};
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
    
    // Set up session coordination (connects to session-core)
    let (session_tx, session_rx) = tokio::sync::mpsc::channel(100);
    dialog_manager.set_session_coordinator(session_tx);
    
    // Start processing
    dialog_manager.start().await?;
    
    Ok(())
}
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
}
```

## 🧪 **Testing**

The crate includes comprehensive testing:

```bash
# Run all tests
cargo test

# Run with features
cargo test --features "recovery events testing"

# Run integration tests (requires SIPp)
cargo test --test integration

# Run with SIPp interoperability tests
cargo test --test sipp_compliance
```

## 📚 **Examples**

See the `examples/` directory for:
- `basic_dialog.rs` - Basic dialog creation and management
- `dialog_recovery.rs` - Dialog recovery and failure handling
- `multi_dialog.rs` - Managing multiple concurrent dialogs

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
}
```

## 🚀 **Development Status**

This crate is part of the RVOIP architecture refactoring to fix layer separation violations. See `TODO.md` for the implementation roadmap.

## 📄 **License**

Licensed under either of:
- Apache License, Version 2.0
- MIT License

## 🤝 **Contributing**

This crate follows RFC 3261 specifications. When contributing:
1. Ensure proper layer separation
2. Add tests for new functionality  
3. Update documentation
4. Follow the existing API patterns 