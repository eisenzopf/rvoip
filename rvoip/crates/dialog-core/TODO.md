# Dialog-Core: SIP Dialog Management Layer

## 🎯 **Purpose & Scope**

The `dialog-core` crate implements RFC 3261 dialog management as a dedicated layer between `session-core` and the transport/transaction layers. This addresses the architectural violations we've been encountering by providing proper separation of concerns.

## 🏗️ **Architecture Position**

```
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
├─────────────────────────────────────────────────────────────┤
│  client-core                   │  call-engine               │
│  (Client Logic & Coordination) │  (Server Logic & Policy)   │
├─────────────────────────────────────────────────────────────┤
│                 *** session-core ***                        │
│           (Session Manager - Central Coordinator)           │
│      • Session Coordination      • Media Coordination       │
│      • Session State Management  • Event Orchestration      │  
├─────────────────────────────────────────────────────────────┤
│              *** dialog-core *** (NEW!)                     │
│                   DialogManager                             │
│      • SIP Protocol Processing   • INVITE/BYE/REGISTER      │
│      • Dialog State Management   • SIP Response Creation    │
│      • RFC 3261 Dialog Layer     • Request Routing         │
├─────────────────────────────────────────────────────────────┤
│         Processing Layer                                    │
│  transaction-core              │  media-core               │
│  (SIP Reliability & State)     │  (Media Processing)       │
├─────────────────────────────────────────────────────────────┤
│              Transport Layer                                │
│  sip-transport    │  rtp-core    │  ice-core               │
└─────────────────────────────────────────────────────────────┘
```

## 📋 **Responsibilities**

### ✅ **What dialog-core DOES (Dialog Layer)**
- **SIP Protocol Handling**: Process INVITE, BYE, REGISTER, UPDATE, etc.
- **Dialog State Management**: Track dialog state per RFC 3261
- **Request/Response Routing**: Route SIP messages within dialogs
- **SIP Header Management**: Call-ID, tags, CSeq handling
- **Early/Confirmed Dialog Logic**: Handle provisional and success responses
- **SDP Negotiation Coordination**: Track offer/answer state
- **Dialog Recovery**: Handle network failures and recovery

### ❌ **What dialog-core DOES NOT DO**
- **Session Coordination**: That's session-core's job
- **Media Management**: That's media-core's job  
- **Application Logic**: That's client-core/call-engine's job
- **Transaction Reliability**: That's transaction-core's job
- **Transport**: That's sip-transport's job

### 🔄 **What dialog-core DELEGATES TO**
- **transaction-core**: For reliable SIP message delivery
- **sip-transport**: For actual network transport
- **Session coordination events**: Back to session-core for session management

## 📁 **Directory Structure**

```
dialog-core/
├── Cargo.toml
├── README.md
├── TODO.md (this file)
├── CHANGELOG.md
├── examples/
│   ├── basic_dialog.rs
│   ├── dialog_recovery.rs
│   └── multi_dialog.rs
├── tests/
│   ├── integration/
│   │   ├── dialog_lifecycle.rs
│   │   ├── dialog_recovery.rs
│   │   └── sip_compliance.rs
│   └── unit/
│       ├── dialog_state.rs
│       ├── request_routing.rs
│       └── sdp_negotiation.rs
└── src/
    ├── lib.rs
    ├── errors/
    │   ├── mod.rs
    │   ├── dialog_errors.rs
    │   └── recovery_errors.rs
    ├── dialog/
    │   ├── mod.rs
    │   ├── dialog_id.rs
    │   ├── dialog_impl.rs
    │   ├── dialog_state.rs
    │   └── dialog_utils.rs
    ├── manager/
    │   ├── mod.rs
    │   ├── dialog_manager.rs
    │   ├── event_processing.rs
    │   └── transaction_coordination.rs
    ├── protocol/
    │   ├── mod.rs
    │   ├── invite_handler.rs
    │   ├── bye_handler.rs
    │   ├── register_handler.rs
    │   ├── update_handler.rs
    │   └── response_handler.rs
    ├── routing/
    │   ├── mod.rs
    │   ├── request_router.rs
    │   ├── response_router.rs
    │   └── dialog_matcher.rs
    ├── sdp/
    │   ├── mod.rs
    │   ├── negotiation.rs
    │   ├── offer_answer.rs
    │   └── media_tracking.rs
    ├── recovery/
    │   ├── mod.rs
    │   ├── recovery_manager.rs
    │   ├── failure_detection.rs
    │   └── recovery_strategies.rs
    └── events/
        ├── mod.rs
        ├── dialog_events.rs
        └── session_coordination.rs
```

## 🔧 **Core API Design**

### DialogManager (Main Interface)
```rust
pub struct DialogManager {
    // Dependencies
    transaction_manager: Arc<TransactionManager>,
    transport: Arc<dyn SipTransport>,
    
    // Dialog state
    dialogs: DashMap<DialogId, Dialog>,
    dialog_lookup: DashMap<DialogTuple, DialogId>,
    
    // Event coordination
    session_coordinator: mpsc::Sender<SessionCoordinationEvent>,
}

impl DialogManager {
    // Lifecycle
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        transport: Arc<dyn SipTransport>
    ) -> Result<Self, DialogError>;
    
    // Dialog operations
    pub async fn create_dialog(&self, request: &Request) -> Result<DialogId, DialogError>;
    pub async fn find_dialog(&self, request: &Request) -> Option<DialogId>;
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> Result<(), DialogError>;
    
    // Protocol handling (delegates to specific handlers)
    pub async fn handle_invite(&self, request: Request, source: SocketAddr) -> Result<(), DialogError>;
    pub async fn handle_bye(&self, request: Request) -> Result<(), DialogError>;
    pub async fn handle_register(&self, request: Request, source: SocketAddr) -> Result<(), DialogError>;
    
    // Request/Response routing
    pub async fn send_request(&self, dialog_id: &DialogId, method: Method, body: Option<Bytes>) -> Result<TransactionKey, DialogError>;
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> Result<(), DialogError>;
    
    // Session coordination
    pub fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>);
}
```

### Session Coordination Events
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
    // ... other coordination events
}
```

## 🚀 **Implementation Phases**

### **Phase 1: Core Infrastructure**
- [ ] Create basic crate structure
- [ ] Define error types and handling
- [ ] Implement DialogId and basic Dialog struct
- [ ] Create DialogManager skeleton
- [ ] Add basic transaction coordination

### **Phase 2: Protocol Handlers**
- [ ] Implement InviteHandler
- [ ] Implement ByeHandler  
- [ ] Implement RegisterHandler
- [ ] Add response handling
- [ ] Implement dialog state machine

### **Phase 3: Request/Response Routing**
- [ ] Implement request router
- [ ] Implement response router
- [ ] Add dialog matching logic
- [ ] Handle in-dialog vs new dialog requests

### **Phase 4: SDP Negotiation**
- [ ] Track SDP offer/answer state
- [ ] Coordinate with media-core
- [ ] Handle re-INVITE scenarios
- [ ] Support early media

### **Phase 5: Recovery & Reliability**
- [ ] Implement dialog recovery
- [ ] Add failure detection
- [ ] Handle network failures
- [ ] Implement recovery strategies

### **Phase 6: Session-Core Integration**
- [ ] Move dialog management from session-core
- [ ] Update session-core to use dialog-core
- [ ] Add session coordination events
- [ ] Remove architectural violations

### **Phase 7: Testing & Validation**
- [ ] Unit tests for all components
- [ ] Integration tests with transaction-core
- [ ] SIPp interoperability testing
- [ ] Performance testing

## 🔄 **Migration Plan**

### **Step 1: Extract from session-core**
1. Move dialog-related code from `session-core/src/dialog/` to `dialog-core/src/`
2. Clean up dependencies and imports
3. Create proper API boundaries

### **Step 2: Update Dependencies**
1. Add dialog-core dependency to session-core
2. Update session-core to use dialog-core APIs
3. Remove duplicate dialog code from session-core

### **Step 3: Fix API Integration**
1. Implement session coordination events
2. Update transaction event handling
3. Test end-to-end functionality

### **Step 4: Validation**
1. Run existing integration tests
2. Run SIPp interoperability tests
3. Verify no regressions in call-engine

## 📊 **Dependencies**

### **dialog-core depends on:**
- `transaction-core` - for reliable message delivery
- `sip-transport` - for network transport
- `sip-core` - for SIP message types
- `infra-common` - for events and utilities

### **dialog-core provides to:**
- `session-core` - dialog management services
- Future crates needing dialog functionality

## 🎯 **Success Criteria**

1. **Layer Separation**: Clean separation between dialog and session layers
2. **RFC 3261 Compliance**: Proper dialog state machine implementation
3. **No Regressions**: All existing tests pass
4. **Performance**: No significant performance impact
5. **Maintainability**: Clear APIs and responsibility boundaries

## 📝 **Notes**

- This addresses the architectural violations we've been encountering
- Follows proper RFC 3261 layer separation
- Enables future SIP features to be added cleanly
- Provides foundation for advanced dialog features (forking, etc.)
- Makes testing easier by isolating dialog logic

## 🔍 **Current Issues This Solves**

1. **Layer Violation**: session-core doing protocol work
2. **Compilation Errors**: Missing imports and trait implementations  
3. **Architectural Confusion**: Mixed responsibilities
4. **Testing Complexity**: Hard to test dialog logic in isolation
5. **Maintenance Burden**: Dialog code scattered across session-core 