# Session-Core API Extension Plan for Call-Engine Integration

## 🎯 Objective

Extend the session-core public API to provide the functionality that call-engine requires while maintaining clean architecture and backwards compatibility.

## 📋 Overview of Required Changes

### New API Modules to Add:
1. **`api/server.rs`** - Server-oriented session management
2. **`api/bridge.rs`** - Bridge management functionality  
3. **`api/notifications.rs`** - Enhanced notification system

### Existing Modules to Modify:
1. **`api/mod.rs`** - Export new modules and conference functionality
2. **`api/types.rs`** - Add missing types (CallerInfo, IncomingCallEvent)
3. **`api/builder.rs`** - Add server builder methods
4. **`bridge/types.rs`** - Enhance bridge types with events

### Internal Modules to Expose:
1. **`conference/*`** - Expose conference management in public API
2. **`bridge/*`** - Enhance and expose bridge functionality

## 📂 Detailed File Changes

### 1. **NEW FILE: `src/api/server.rs`**
```rust
//! Server-oriented Session Management API
//! 
//! Provides server-specific functionality for call centers, PBXs, and other
//! server applications that need to manage multiple sessions and bridges.

use async_trait::async_trait;
use tokio::sync::mpsc;
use crate::api::types::{SessionId, CallSession};
use crate::bridge::types::{BridgeId, BridgeConfig};
use crate::errors::Result;

/// Server-oriented session manager with bridge capabilities
#[async_trait]
pub trait ServerSessionManager: Send + Sync {
    /// Create a bridge between two or more sessions
    async fn bridge_sessions(&self, session1: &SessionId, session2: &SessionId) -> Result<BridgeId>;
    
    /// Destroy an existing bridge
    async fn destroy_bridge(&self, bridge_id: &BridgeId) -> Result<()>;
    
    /// Get the bridge a session is part of (if any)
    async fn get_session_bridge(&self, session_id: &SessionId) -> Result<Option<BridgeId>>;
    
    /// Remove a session from a bridge
    async fn remove_session_from_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<()>;
    
    /// List all active bridges
    async fn list_bridges(&self) -> Vec<BridgeInfo>;
    
    /// Subscribe to bridge events
    async fn subscribe_to_bridge_events(&self) -> mpsc::UnboundedReceiver<BridgeEvent>;
    
    /// Create a pre-allocated outgoing session (for agent registration)
    async fn create_outgoing_session(&self) -> Result<SessionId>;
    
    /// Get underlying session manager for basic operations
    fn session_manager(&self) -> &SessionManager;
}

/// Configuration for server-oriented session management
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_address: std::net::SocketAddr,
    pub transport_protocol: TransportProtocol,
    pub max_sessions: usize,
    pub session_timeout: std::time::Duration,
    pub transaction_timeout: std::time::Duration,
    pub enable_media: bool,
    pub server_name: String,
    pub contact_uri: Option<String>,
}

/// Transport protocol for SIP
#[derive(Debug, Clone)]
pub enum TransportProtocol {
    Udp,
    Tcp,
    Tls,
    Ws,
    Wss,
}

/// Factory function to create a server session manager
pub async fn create_full_server_manager(
    transaction_manager: Arc<TransactionManager>,
    config: ServerConfig,
) -> Result<Arc<dyn ServerSessionManager>>;
```

**Tasks:**
- [ ] Create `src/api/server.rs` file
- [ ] Define `ServerSessionManager` trait with all required methods
- [ ] Define `ServerConfig` struct
- [ ] Define `TransportProtocol` enum
- [ ] Implement `create_full_server_manager` factory function
- [ ] Add comprehensive documentation

### 2. **NEW FILE: `src/api/bridge.rs`**
```rust
//! Bridge Management API
//! 
//! Types and traits for managing bridges between sessions.

use std::time::Instant;
use crate::api::types::SessionId;
use crate::bridge::types::BridgeId;

/// Information about an active bridge
#[derive(Debug, Clone)]
pub struct BridgeInfo {
    pub id: BridgeId,
    pub sessions: Vec<SessionId>,
    pub created_at: Instant,
    pub participant_count: usize,
}

/// Bridge event notifications
#[derive(Debug, Clone)]
pub struct BridgeEvent {
    pub bridge_id: BridgeId,
    pub event_type: BridgeEventType,
    pub session_id: Option<SessionId>,
    pub timestamp: Instant,
}

/// Types of bridge events
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeEventType {
    /// Bridge was created
    Created,
    /// Session was added to bridge
    SessionAdded,
    /// Session was removed from bridge
    SessionRemoved,
    /// Bridge was destroyed
    Destroyed,
    /// Media started flowing
    MediaEstablished,
    /// Media stopped
    MediaStopped,
}

/// Bridge management trait
#[async_trait]
pub trait BridgeManager: Send + Sync {
    /// Create a new bridge with multiple sessions
    async fn create_bridge(&self, sessions: Vec<SessionId>, config: Option<BridgeConfig>) -> Result<BridgeId>;
    
    /// Add a session to existing bridge
    async fn add_to_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<()>;
    
    /// Get information about a bridge
    async fn get_bridge_info(&self, bridge_id: &BridgeId) -> Result<Option<BridgeInfo>>;
}
```

**Tasks:**
- [ ] Create `src/api/bridge.rs` file
- [ ] Define `BridgeInfo` struct
- [ ] Define `BridgeEvent` and `BridgeEventType`
- [ ] Define `BridgeManager` trait
- [ ] Add documentation and examples

### 3. **NEW FILE: `src/api/notifications.rs`**
```rust
//! Enhanced Notification System
//! 
//! Provides server-oriented call notification handling.

use async_trait::async_trait;
use crate::api::types::{SessionId, CallDecision};
use std::collections::HashMap;

/// Enhanced incoming call event with detailed information
#[derive(Debug, Clone)]
pub struct IncomingCallEvent {
    pub session_id: SessionId,
    pub caller_info: CallerInfo,
    pub call_id: String,
    pub headers: HashMap<String, String>,
    pub sdp: Option<String>,
}

/// Detailed caller information
#[derive(Debug, Clone)]
pub struct CallerInfo {
    pub from: String,
    pub to: String,
    pub display_name: Option<String>,
    pub user_agent: Option<String>,
    pub contact: Option<String>,
}

/// Server-oriented incoming call notification handler
#[async_trait]
pub trait IncomingCallNotification: Send + Sync {
    /// Handle incoming call with detailed event information
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision;
    
    /// Handle call termination by remote party
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, call_id: String);
    
    /// Handle call ended by server
    async fn on_call_ended_by_server(&self, session_id: SessionId, call_id: String);
}

/// Extension trait for setting notification handlers
#[async_trait]
pub trait NotificationSetter {
    /// Set the incoming call notification handler
    async fn set_incoming_call_notifier(&self, handler: Arc<dyn IncomingCallNotification>);
}
```

**Tasks:**
- [ ] Create `src/api/notifications.rs` file
- [ ] Define `IncomingCallEvent` struct
- [ ] Define `CallerInfo` struct
- [ ] Define `IncomingCallNotification` trait
- [ ] Define `NotificationSetter` extension trait
- [ ] Add documentation

### 4. **MODIFY: `src/api/mod.rs`**
```rust
// Add to existing exports

// New API modules
pub mod server;
pub mod bridge;
pub mod notifications;

// Re-export server functionality
pub use server::{
    ServerSessionManager, ServerConfig, TransportProtocol,
    create_full_server_manager,
};

// Re-export bridge functionality
pub use bridge::{
    BridgeManager, BridgeInfo, BridgeEvent, BridgeEventType,
};

// Re-export notification functionality
pub use notifications::{
    IncomingCallEvent, CallerInfo, IncomingCallNotification,
    NotificationSetter,
};

// Re-export conference functionality
pub use crate::conference::{
    ConferenceManager, ConferenceApi, ConferenceCoordinator,
    ConferenceId, ConferenceConfig, ConferenceEvent,
    ConferenceRoom, ConferenceParticipant,
};

// Re-export additional bridge types
pub use crate::bridge::{BridgeId, BridgeConfig};
```

**Tasks:**
- [ ] Add new module declarations
- [ ] Add server re-exports
- [ ] Add bridge re-exports  
- [ ] Add notification re-exports
- [ ] Add conference re-exports
- [ ] Update documentation

### 5. **MODIFY: `src/api/types.rs`**
```rust
// Add backwards compatibility type aliases

/// Alias for CallSession for compatibility
pub type Session = CallSession;

// Add missing CallDecision variants
impl CallDecision {
    /// Create a reject decision with status code
    pub fn reject_with_code(status_code: StatusCode, reason: Option<String>) -> Self {
        CallDecision::Reject(reason.unwrap_or_else(|| status_code.to_string()))
    }
}

// Re-export StatusCode for convenience
pub use rvoip_sip_core::StatusCode;
```

**Tasks:**
- [ ] Add `Session` type alias
- [ ] Enhance `CallDecision` with status code support
- [ ] Re-export commonly needed types
- [ ] Maintain backwards compatibility

### 6. **MODIFY: `src/api/builder.rs`**
```rust
// Add server builder methods

impl SessionManagerBuilder {
    /// Build a server-oriented session manager
    pub async fn build_server(self, transaction_manager: Arc<TransactionManager>) -> Result<Arc<dyn ServerSessionManager>> {
        // Convert builder config to ServerConfig
        let server_config = ServerConfig {
            bind_address: self.sip_addr.parse()?,
            transport_protocol: TransportProtocol::Udp,
            max_sessions: self.max_sessions,
            session_timeout: self.session_timeout,
            transaction_timeout: Duration::from_secs(32),
            enable_media: true,
            server_name: self.domain.clone(),
            contact_uri: Some(self.local_address.clone()),
        };
        
        create_full_server_manager(transaction_manager, server_config).await
    }
    
    /// Set transaction manager for server mode
    pub fn with_transaction_manager(mut self, tm: Arc<TransactionManager>) -> Self {
        self.transaction_manager = Some(tm);
        self
    }
}
```

**Tasks:**
- [ ] Add `build_server` method
- [ ] Add `with_transaction_manager` method  
- [ ] Update builder to support server mode
- [ ] Add configuration mapping

### 7. **CREATE: Internal Server Implementation**
```rust
// src/server/mod.rs - Internal implementation

mod session_manager;
mod bridge_manager;
mod notification_handler;

use self::session_manager::ServerSessionManagerImpl;

pub(crate) async fn create_server_manager_impl(
    transaction_manager: Arc<TransactionManager>,
    config: ServerConfig,
) -> Result<Arc<ServerSessionManagerImpl>> {
    ServerSessionManagerImpl::new(transaction_manager, config).await
}
```

**Tasks:**
- [ ] Create `src/server/` directory
- [ ] Create `src/server/mod.rs`
- [ ] Create `src/server/session_manager.rs`
- [ ] Create `src/server/bridge_manager.rs`
- [ ] Create `src/server/notification_handler.rs`
- [ ] Implement `ServerSessionManagerImpl`

### 8. **ENHANCE: Bridge Module**
```rust
// src/bridge/manager.rs - Add bridge management implementation

pub struct BridgeManagerImpl {
    bridges: Arc<RwLock<HashMap<BridgeId, Bridge>>>,
    event_sender: mpsc::UnboundedSender<BridgeEvent>,
}

impl BridgeManagerImpl {
    pub async fn create_bridge(&self, sessions: Vec<SessionId>) -> Result<BridgeId> {
        // Implementation
    }
    
    pub async fn destroy_bridge(&self, bridge_id: &BridgeId) -> Result<()> {
        // Implementation
    }
}
```

**Tasks:**
- [ ] Create `src/bridge/manager.rs`
- [ ] Implement bridge lifecycle management
- [ ] Add event notification system
- [ ] Integrate with media-core for RTP bridging
- [ ] Add tests

### 9. **EXPOSE: Conference Module**
```rust
// Ensure conference module is properly exposed

// In src/conference/mod.rs - ensure public visibility
pub use self::api::ConferenceApi;
pub use self::manager::ConferenceManager;
pub use self::coordinator::ConferenceCoordinator;
```

**Tasks:**
- [ ] Review conference module exports
- [ ] Ensure all needed types are public
- [ ] Add any missing functionality
- [ ] Update documentation

## 🧪 Testing Plan

### Unit Tests
- [ ] Test `ServerSessionManager` trait implementation
- [ ] Test bridge creation and destruction
- [ ] Test notification system
- [ ] Test conference integration

### Integration Tests  
- [ ] Create `tests/server_api_test.rs`
- [ ] Create `tests/bridge_management_test.rs`
- [ ] Create `tests/call_engine_compatibility_test.rs`
- [ ] Test full call-engine integration scenario

### Examples
- [ ] Create `examples/server_mode.rs`
- [ ] Create `examples/call_center_bridge.rs`
- [ ] Update existing examples for compatibility

## 📚 Documentation Updates

- [ ] Update `README.md` with server API documentation
- [ ] Update `COOKBOOK.md` with server patterns
- [ ] Add API migration guide
- [ ] Document bridge management patterns
- [ ] Add call-engine integration example

## ⚠️ Backwards Compatibility

### Ensure No Breaking Changes:
- [ ] All existing public APIs remain unchanged
- [ ] New functionality is additive only
- [ ] Type aliases for compatibility where needed
- [ ] Deprecation warnings for any changes

## 🚀 Implementation Order

### Phase 1: Core Types and Traits (Day 1)
1. Create type definitions in `api/types.rs`
2. Create traits in new API modules
3. Update `api/mod.rs` exports

### Phase 2: Internal Implementation (Day 2-3)
1. Create server implementation structure
2. Implement bridge management
3. Wire up notification system

### Phase 3: Integration (Day 4)
1. Connect to existing SessionCoordinator
2. Integrate with TransactionManager
3. Bridge to conference module

### Phase 4: Testing (Day 5)
1. Write comprehensive tests
2. Test call-engine integration
3. Performance testing

## ✅ Success Criteria

- [ ] Call-engine compiles with new session-core API
- [ ] All call-engine tests pass
- [ ] No breaking changes to existing session-core API
- [ ] Bridge management works correctly
- [ ] Notification system delivers events
- [ ] Conference integration functional
- [ ] Performance acceptable (< 10ms overhead)

## 📝 Notes

- This plan maintains backwards compatibility while adding needed functionality
- The server-oriented API wraps existing SessionCoordinator functionality
- Bridge management will use conference module internally where applicable
- Event system will be built on top of existing event infrastructure

---

**Estimated Timeline:** 5 days of focused development

**Risk Mitigation:**
- All changes are additive to prevent breaking existing code
- Internal implementation details hidden behind traits
- Comprehensive testing before integration 