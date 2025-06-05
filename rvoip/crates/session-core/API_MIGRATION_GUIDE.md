# Session-Core API Migration Guide (Phase 12)

## 🎯 **New Architecture Overview**

After Phase 12 architectural refactoring, we have **clean separation of concerns** across three layers:

```
┌─────────────────────────────────────────────────────────────┐
│                    client-core                              │
│              (Client-specific UX/UI patterns)              │
│    • Make call / Answer call UX logic                      │
│    • Client registration behavior                          │
│    • Client-specific call management                       │
│    • Uses SessionManager from session-core                 │
├─────────────────────────────────────────────────────────────┤
│                   call-engine                               │
│              (Business Logic & Policies)                   │
│    • Call routing decisions    • Accept/reject policies    │
│    • Conference orchestration  • Business rules            │
│    • Policy enforcement       • Authentication             │
│    • Uses SessionManager from session-core                 │
├─────────────────────────────────────────────────────────────┤
│                  session-core                               │
│            (Session Coordination Infrastructure)            │
│    • SessionManager ✅         • Session primitives ✅      │
│    • Bridge infrastructure ✅  • Basic coordination ✅      │
│    • Media coordination ✅     • Event infrastructure ✅    │
└─────────────────────────────────────────────────────────────┘
```

## ✅ **What session-core Provides (Infrastructure)**

### **Core Infrastructure**
- **SessionManager** - Central session coordination infrastructure
- **Session, SessionId, SessionState** - Session primitives
- **SessionBridge** - Multi-session bridging infrastructure
- **MediaManager** - Media coordination infrastructure
- **EventBus** - Session event coordination

### **Factory APIs**
```rust
use rvoip_session_core::api::*;

// Create SessionManager infrastructure
let session_manager = create_session_manager(
    dialog_api, 
    media_manager, 
    config
).await?;

// Or create complete infrastructure
let infrastructure = create_session_infrastructure(
    dialog_api,
    media_manager, 
    config
).await?;
```

### **Basic Coordination Primitives**
- **BasicSessionGroup** - Session grouping data structures
- **BasicResourceLimits** - Resource tracking primitives  
- **BasicSessionPriority** - Priority classification primitives
- **BasicEventBus** - Simple pub/sub event communication

## 🚀 **Migration Examples**

### **Before (Mixed Responsibilities)**
```rust
// ❌ Old way - business logic mixed with infrastructure
use rvoip_session_core::api::server::*;

let server = create_full_server_manager(config).await?;
server.accept_call(&session_id).await?; // Business decision in session-core!
```

### **After (Clean Separation)**

#### **Call-Engine Usage (Business Logic)**
```rust
// ✅ New way - call-engine handles business logic
use rvoip_session_core::api::*;
use rvoip_call_engine::*;

// session-core provides infrastructure
let session_manager = create_session_manager(dialog_api, media_manager, config).await?;

// call-engine orchestrates business logic
let call_engine = CallEngineServer::new(session_manager, engine_config).await?;
call_engine.accept_call(&session_id).await?; // Business decision in call-engine!
```

#### **Client-Core Usage (Client Patterns)**  
```rust
// ✅ New way - client-core handles client-specific patterns
use rvoip_session_core::api::*;
use rvoip_client_core::*;

// session-core provides infrastructure  
let session_manager = create_session_manager(dialog_api, media_manager, config).await?;

// client-core orchestrates client behavior
let client = ClientManager::new(session_manager, client_config).await?;
client.make_call("sip:alice@example.com").await?; // Client behavior in client-core!
```

## 📋 **Complete Migration Table**

| **Old API (session-core)** | **New API (where moved)** | **Reason** |
|---------------------------|---------------------------|------------|
| `create_sip_server()` | `call-engine::CallEngineServer::new()` | Server business logic |
| `create_sip_client()` | `client-core::ClientManager::new()` | Client UX patterns |
| `ServerManager::accept_call()` | `CallEngineServer::accept_call()` | Business policy decision |
| `ClientManager::make_call()` | `ClientManager::make_call()` | Client UX pattern |
| `create_session_manager()` | ✅ **STAYS in session-core** | Infrastructure, not business logic |
| `SessionManager::create_bridge()` | ✅ **STAYS in session-core** | Infrastructure, not business logic |

## 🎯 **Architecture Benefits**

### **For session-core**
- ✅ **Focused scope**: Only session coordination infrastructure
- ✅ **Reusable**: Used by both call-engine and client-core
- ✅ **Maintainable**: Much cleaner codebase
- ✅ **Testable**: Infrastructure-focused testing

### **For call-engine**  
- ✅ **Complete business logic**: All call routing, policies, authentication
- ✅ **Rich features**: Can add complex PBX features without affecting session-core
- ✅ **Integration**: Business logic properly integrated with routing and agent management

### **For client-core**
- ✅ **Client-focused**: UX patterns, registration behavior, client-specific features
- ✅ **UI integration**: Can integrate with different UI frameworks
- ✅ **Client policies**: Handle client-specific call management patterns

## 📖 **Usage Patterns**

### **For Application Developers**

#### **Building a SIP Server (PBX)**
```rust
use rvoip_session_core::api::*;
use rvoip_call_engine::*;

// 1. Create session infrastructure
let infrastructure = create_session_infrastructure_simple(
    local_addr,
    SessionInfrastructureConfig::new(local_addr)
).await?;

// 2. Create call-engine with business logic
let call_engine = CallEngineServer::new(
    infrastructure.session_manager(),
    CallEngineConfig::pbx_default()
).await?;

// 3. Handle business logic through call-engine
call_engine.start().await?;
```

#### **Building a SIP Client (Softphone)**
```rust
use rvoip_session_core::api::*;
use rvoip_client_core::*;

// 1. Create session infrastructure
let infrastructure = create_session_infrastructure_simple(
    local_addr,
    SessionInfrastructureConfig::new(local_addr)
).await?;

// 2. Create client manager with UX patterns
let client = ClientManager::new(
    infrastructure.session_manager(),
    ClientConfig::softphone_default()
).await?;

// 3. Handle client behavior through client-core
let session_id = client.make_call("sip:bob@example.com").await?;
```

### **For Library Developers**

#### **Extending Call-Engine**
```rust
use rvoip_session_core::api::*;

// Your custom business logic can use SessionManager infrastructure
pub struct CustomCallRouter {
    session_manager: Arc<SessionManager>,
}

impl CustomCallRouter {
    pub async fn route_call(&self, session_id: &SessionId) -> Result<()> {
        // Use session-core infrastructure for coordination
        let bridge_id = self.session_manager.create_bridge(
            BridgeConfig::default()
        ).await?;
        
        // Your business logic here...
        Ok(())
    }
}
```

## 🔄 **Backward Compatibility**

All old APIs are **deprecated but functional** during the transition period:

```rust
#[deprecated(since = "1.0.0", note = "Use call-engine instead")]
pub async fn create_sip_server(config: ServerConfig) -> Result<SipServer>

#[deprecated(since = "1.0.0", note = "Use client-core instead")]  
pub async fn create_sip_client(config: ClientConfig) -> Result<SipClient>
```

**Migration timeline**: 
- **Phase 1** (current): Old APIs deprecated but working
- **Phase 2** (next release): Old APIs removed, clean architecture only

## 🚀 **Getting Started**

1. **Update imports**: Change from business logic APIs to infrastructure APIs
2. **Add call-engine/client-core**: Import the appropriate business logic layer
3. **Separate concerns**: Move business decisions to call-engine, UX patterns to client-core
4. **Test**: Verify functionality with new architecture

**The result**: Cleaner, more maintainable, and more powerful SIP applications! 🎉 