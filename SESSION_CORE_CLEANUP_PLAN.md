# Session-Core-V2 Cleanup Plan

## Executive Summary

This plan removes B2BUA-specific features from session-core-v2, returning it to its original purpose: managing SIP sessions for **endpoints** (clients, phones, simple UAs). All B2BUA functionality moves to the new b2bua-core library.

## Current State (What to Remove)

### B2BUA-Specific Code to Remove

```rust
// REMOVE: These were added for B2BUA but don't belong
pub trait SessionInterceptor {
    async fn on_bridge_request(&self, ...);
    async fn should_intercept(&self, ...);
}

pub trait BridgeDriver {
    async fn create_bridge(&self, ...);
    async fn destroy_bridge(&self, ...);
}

pub enum SessionMode {
    Endpoint,  // KEEP
    B2BUA,     // REMOVE
    Proxy,     // REMOVE
}

pub struct BridgeManager {  // REMOVE ENTIRELY
    // This belongs in b2bua-core
}
```

### Hooks to Remove

```rust
// REMOVE: B2BUA-specific hooks
impl SessionCore {
    pub fn set_interceptor(&mut self, interceptor: Box<dyn SessionInterceptor>) {
        // REMOVE
    }

    pub fn set_bridge_driver(&mut self, driver: Box<dyn BridgeDriver>) {
        // REMOVE
    }

    pub fn bridge_to(&self, target: Uri) -> Result<()> {
        // REMOVE - endpoints don't bridge
    }
}
```

### State Machine Simplification

```rust
// CURRENT (Complex)
pub enum SessionState {
    Idle,
    Calling,
    Ringing,
    Connected,
    OnHold,
    // REMOVE THESE:
    Bridging,
    Bridged,
    Transferring,
    Queueing,
    InConference,
}

// AFTER CLEANUP (Simple)
pub enum SessionState {
    Idle,
    Calling,
    Ringing,
    Connected,
    OnHold,
    Disconnected,
}
```

## Target State (Pure Endpoint Library)

### Clean Session-Core-V2

```rust
// session-core-v2/src/lib.rs
pub struct SessionCore {
    // Dialog management
    dialog: Arc<Dialog>,

    // Local media processing
    media: Arc<MediaCore>,

    // Simple state machine
    state: SessionState,

    // Event notifications
    event_bus: Arc<EventBus>,
}

impl SessionCore {
    // Endpoint operations only
    pub async fn make_call(&self, target: &str) -> Result<()>;
    pub async fn answer_call(&self) -> Result<()>;
    pub async fn hangup(&self) -> Result<()>;
    pub async fn hold(&self) -> Result<()>;
    pub async fn resume(&self) -> Result<()>;
    pub async fn transfer(&self, target: &str) -> Result<()>;

    // Media operations (local processing)
    pub async fn send_dtmf(&self, digits: &str) -> Result<()>;
    pub async fn mute(&self, direction: MediaDirection) -> Result<()>;
    pub async fn unmute(&self, direction: MediaDirection) -> Result<()>;
}
```

### What Remains

1. **Core Session Management**
   - UAC (outgoing calls)
   - UAS (incoming calls)
   - Session timers (RFC 4028)
   - Reliability (PRACK/UPDATE)

2. **Local Media Processing**
   - RTP send/receive via media-core
   - Codec negotiation
   - DTMF generation/detection
   - Local audio device integration

3. **Standard SIP Operations**
   - Hold/Resume
   - Blind transfer (REFER)
   - Session refresh
   - Early media

## Migration Steps

### Phase 1: Identify Dependencies (Week 1)

1. **Audit Current Code**
   ```bash
   # Find all B2BUA references
   grep -r "B2BUA\|Bridge\|Interceptor" crates/session-core-v2/

   # Find who imports these features
   grep -r "use.*session_core.*Bridge" crates/
   ```

2. **Document External Dependencies**
   - Which examples use B2BUA features?
   - Which tests rely on bridging?
   - Which applications import these traits?

### Phase 2: Create Migration Path (Week 1)

1. **Deprecate B2BUA Features**
   ```rust
   #[deprecated(since = "0.2.0", note = "Use b2bua-core instead")]
   pub trait SessionInterceptor { ... }

   #[deprecated(since = "0.2.0", note = "Use b2bua-core instead")]
   pub fn bridge_to(&self, ...) { ... }
   ```

2. **Add Migration Guide**
   ```rust
   // OLD (session-core-v2)
   let session = SessionCore::new();
   session.set_mode(SessionMode::B2BUA);
   session.bridge_to(target).await?;

   // NEW (b2bua-core)
   let b2bua = B2buaCore::new();
   b2bua.handle_call(invite).await?;
   b2bua.bridge_to(target).await?;
   ```

### Phase 3: Remove Code (Week 2)

1. **Remove B2BUA Types**
   - Delete `bridge.rs`
   - Delete `interceptor.rs`
   - Remove B2BUA variants from enums

2. **Simplify State Machine**
   - Remove B2BUA states
   - Remove B2BUA transitions
   - Update state tests

3. **Clean Up Examples**
   - Move B2BUA examples to b2bua-core
   - Update remaining examples

### Phase 4: Optimize for Endpoints (Week 2)

1. **Improve Endpoint APIs**
   ```rust
   // Add convenience methods for clients
   impl SessionCore {
       pub async fn dial(&self, number: &str) -> Result<()> {
           // Simplified dialing for endpoints
       }

       pub async fn register(&self, registrar: &str) -> Result<()> {
           // Built-in registration support
       }
   }
   ```

2. **Add Client-Specific Features**
   ```rust
   pub struct ClientConfig {
       pub stun_server: Option<String>,
       pub audio_device: AudioDevice,
       pub codecs: Vec<Codec>,
   }
   ```

## Testing Changes

### Tests to Remove

```rust
// REMOVE: B2BUA-specific tests
#[test]
fn test_bridge_creation() { ... }

#[test]
fn test_interceptor_hooks() { ... }

#[test]
fn test_b2bua_state_transitions() { ... }
```

### Tests to Add

```rust
// ADD: Pure endpoint tests
#[test]
fn test_simple_outgoing_call() { ... }

#[test]
fn test_incoming_call_handling() { ... }

#[test]
fn test_hold_resume_cycle() { ... }

#[test]
fn test_codec_negotiation() { ... }
```

## Documentation Updates

### Remove B2BUA Documentation

- Remove B2BUA section from README
- Remove B2BUA examples
- Remove B2BUA API docs

### Add Endpoint Focus

```markdown
# session-core-v2

A SIP session management library for **endpoints** (clients, phones, softphones).

## What is session-core-v2?

This library helps you build SIP endpoints that:
- Make and receive calls
- Handle media locally
- Integrate with audio devices
- Support standard telephony features

## What session-core-v2 is NOT

- Not for B2BUA (use b2bua-core)
- Not for proxies (use proxy-core)
- Not for media servers (use media-server-core)

## Examples

### Simple Softphone
```rust
let session = SessionCore::new();
session.dial("1234").await?;
```
```

## API Breaking Changes

### Version 0.2.0 Breaking Changes

| Removed API | Replacement | Migration |
|-------------|-------------|-----------|
| `SessionMode::B2BUA` | Use b2bua-core | Change library |
| `SessionInterceptor` trait | Use b2bua-core handlers | Implement ApplicationHandler |
| `bridge_to()` method | Use b2bua-core | Call b2bua.bridge_to() |
| `BridgeDriver` trait | Use b2bua-core | Implement MediaServerController |
| `SessionState::Bridged` | Use b2bua-core states | Use B2buaState |

## Benefits After Cleanup

### For session-core-v2

1. **Simpler Code**
   - 30% less code
   - Clearer purpose
   - Easier to maintain

2. **Better Performance**
   - No B2BUA overhead
   - Optimized for endpoints
   - Smaller memory footprint

3. **Clearer API**
   - Endpoint-focused methods
   - No confusing dual-purpose APIs
   - Better documentation

### For Users

1. **Clear Library Choice**
   - Endpoint? Use session-core-v2
   - B2BUA? Use b2bua-core
   - No confusion

2. **Better Type Safety**
   - Endpoint-specific types
   - No accidentally using B2BUA features
   - Compile-time guarantees

3. **Improved Documentation**
   - Focused examples
   - Clear use cases
   - Better getting started guide

## Timeline

### Week 1
- Identify dependencies
- Create deprecation notices
- Write migration guide

### Week 2
- Remove B2BUA code
- Update tests
- Update documentation

### Week 3
- Release v0.2.0-beta
- Gather feedback
- Fix any issues

### Week 4
- Release v0.2.0
- Announce breaking changes
- Support migration

## Success Criteria

1. **Code Metrics**
   - Lines of code reduced by 30%
   - Cyclomatic complexity reduced by 40%
   - Test coverage remains > 80%

2. **API Clarity**
   - No B2BUA-specific methods
   - Clear endpoint focus
   - Simplified state machine

3. **Performance**
   - 20% faster session creation
   - 30% less memory per session
   - No B2BUA overhead

## Coordination with b2bua-core

### Ensure Feature Parity

Before removing from session-core-v2, verify b2bua-core has:
- [x] Bridge management
- [x] Dialog pair handling
- [x] Media server integration
- [x] Application handlers
- [x] State management

### Migration Support

1. Provide clear migration examples
2. Support both libraries during transition
3. Help users identify which library they need

## Conclusion

This cleanup returns session-core-v2 to its intended purpose: a simple, efficient library for SIP endpoints. By removing B2BUA features, we:

1. Reduce complexity
2. Improve performance
3. Clarify the API
4. Better serve endpoint use cases

The removed features aren't lost - they're better implemented in the purpose-built b2bua-core library.