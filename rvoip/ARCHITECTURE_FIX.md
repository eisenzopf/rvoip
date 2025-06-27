# RVOIP Architecture Layer Separation Fix

## 🚨 **Problem Identified**

We've violated proper SIP layer separation by having SessionManager (session layer) do SIP protocol work (dialog layer) and application logic (client-core/call-engine).

## ✅ **Correct Architecture (Per README)**

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
│              Dialog Layer (in session-core)                 │
│                   DialogManager                             │
│      • SIP Protocol Processing   • INVITE/BYE/REGISTER      │
│      • Dialog State Management   • SIP Response Creation    │
├─────────────────────────────────────────────────────────────┤
│         Processing Layer                                    │
│  transaction-core              │  media-core               │
│  (SIP Reliability & State)     │  (Media Processing)       │
├─────────────────────────────────────────────────────────────┤
│              Transport Layer                                │
│  sip-transport    │  rtp-core    │  ice-core               │
└─────────────────────────────────────────────────────────────┘
```

## 🔧 **Required Fixes**

### **1. Move SIP Protocol Work from SessionManager to DialogManager**

**Current (WRONG):**
```rust
// SessionManager.handle_transaction_event()
match request.method() {
    &Method::Invite => {
        self.handle_invite_request(...).await?; // ❌ PROTOCOL WORK IN SESSION LAYER
    },
    &Method::Register => {
        self.handle_register_request(...).await?; // ❌ PROTOCOL WORK IN SESSION LAYER
    },
}
```

**Correct:**
```rust
// SessionManager.handle_transaction_event()
// ✅ DELEGATE ALL PROTOCOL WORK TO DIALOG LAYER
self.dialog_manager.process_transaction_event(event).await;

// DialogManager.process_transaction_event()
match event {
    TransactionEvent::IncomingRequest { transaction_id, request, source } => {
        match request.method() {
            Method::Invite => self.handle_invite_protocol(transaction_id, request, source).await,
            Method::Register => self.handle_register_protocol(transaction_id, request, source).await,
            // ... other SIP methods
        }
    }
}
```

### **2. Move Server Logic from session-core to call-engine**

**Current (WRONG):**
```rust
// session-core/src/api/server/mod.rs
pub trait IncomingCallNotification: Send + Sync { // ❌ SERVER LOGIC IN SESSION-CORE
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision;
}
```

**Correct:**
```rust
// call-engine/src/server/mod.rs  
pub trait IncomingCallNotification: Send + Sync { // ✅ SERVER LOGIC IN CALL-ENGINE
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision;
}

// session-core provides building blocks, call-engine provides policy
```

### **3. Move Client Logic from session-core to client-core**

**Current (WRONG):**
```rust
// SessionManager.initiate_outgoing_call() - ❌ CLIENT LOGIC IN SESSION-CORE
```

**Correct:**
```rust
// client-core/src/call.rs
impl CallManager {
    pub async fn make_call(&self, target: &str) -> Result<CallId> {
        // ✅ CLIENT COORDINATION IN CLIENT-CORE
        // Uses session-core SessionManager as building block
        let session = self.session_manager.create_outgoing_session().await?;
        self.session_manager.initiate_call(&session.id, target).await?;
        // ... client-specific logic
    }
}
```

## 📋 **Message Flow Fix**

### **Current Flow (BROKEN):**
```
sip-transport → transaction-core → SessionManager (doing everything!)
                                  ├── SIP protocol ❌
                                  ├── Session coordination ✅  
                                  ├── Server policy ❌
                                  └── Client logic ❌
```

### **Correct Flow:**
```
sip-transport → transaction-core → DialogManager → SessionManager → client-core/call-engine
              │                   │                │                │
              │                   │                │                └── Application Logic
              │                   │                └── Session Coordination
              │                   └── SIP Protocol Processing
              └── Message Reliability
```

## 🎯 **Implementation Plan**

### **Phase 1: Extract SIP Protocol to DialogManager**
1. Move `handle_invite_request()` from SessionManager to DialogManager
2. Move `handle_bye_request()` from SessionManager to DialogManager  
3. Move `handle_register_request()` from SessionManager to DialogManager
4. Update SessionManager to delegate all protocol work to DialogManager

### **Phase 2: Extract Server Logic to call-engine**
1. Move `IncomingCallNotification` trait to call-engine
2. Move `CallDecision` enum to call-engine
3. Move server-specific APIs to call-engine
4. Update session-core to provide building blocks only

### **Phase 3: Extract Client Logic to client-core**
1. Move `initiate_outgoing_call()` to client-core
2. Move client-specific coordination to client-core
3. Update client-core to use session-core as building blocks

### **Phase 4: Clean up APIs**
1. Define clean interfaces between layers
2. Remove layer-violating dependencies
3. Update examples and tests

## ✅ **Expected Benefits**

1. **RFC 3261 Compliance**: Proper separation between transaction, dialog, and session layers
2. **Maintainability**: Each layer has clear responsibilities
3. **Testability**: Layers can be tested independently
4. **Extensibility**: Easy to extend without violating architecture
5. **Reusability**: Components can be reused in different contexts

## 🧪 **Testing Strategy**

1. **Layer Tests**: Test each layer independently
2. **Integration Tests**: Test layer interactions
3. **SIPp Tests**: Ensure RFC compliance maintained
4. **Client Tests**: Test client-core integration
5. **Server Tests**: Test call-engine integration 