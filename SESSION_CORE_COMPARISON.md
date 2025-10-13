# Session-Core vs Session-Core-V2: Architectural Comparison

## Executive Summary

Both libraries provide session management for SIP calls, but take fundamentally different architectural approaches:

- **session-core**: Traditional imperative coordination pattern with feature-rich APIs
- **session-core-v2**: State table-driven declarative pattern with simplified APIs

## Architectural Approaches

### session-core: Imperative Coordinator Pattern

**Philosophy**: Explicit coordination through a central coordinator that orchestrates all components.

**Core Architecture**:
```
┌────────────────────────────────────┐
│     SessionCoordinator             │
│  (Central Orchestration Hub)       │
├────────────────────────────────────┤
│  - DialogManager                   │
│  - MediaCoordinator                │
│  - BridgeManager                   │
│  - ConferenceManager               │
│  - EventSystem                     │
└────────────────────────────────────┘
         │              │
         ▼              ▼
   dialog-core    media-core
```

**Key Characteristics**:
- **Imperative Control**: Coordinator explicitly calls methods on subsystems
- **Rich Feature Set**: Extensive modules for bridges, conferences, coordination
- **Multiple APIs**: SessionControl, MediaControl, SipClient traits
- **Flexible Event System**: Comprehensive event propagation and handling
- **State Machine**: Embedded within session lifecycle management
- **Code Organization**: ~100+ source files across many specialized modules

### session-core-v2: State Table-Driven Pattern

**Philosophy**: Declarative state transitions defined in a master state table.

**Core Architecture**:
```
┌────────────────────────────────────┐
│      State Table (YAML)            │
│  (Single Source of Truth)          │
│  - All valid transitions           │
│  - Guards, Actions, Events         │
└────────────────────────────────────┘
         │
         ▼
┌────────────────────────────────────┐
│      State Machine Executor        │
│  - Processes events                │
│  - Validates transitions           │
│  - Executes actions                │
└────────────────────────────────────┘
         │              │
         ▼              ▼
   DialogAdapter  MediaAdapter
         │              │
         ▼              ▼
   dialog-core    media-core
```

**Key Characteristics**:
- **Declarative Transitions**: All state changes defined in YAML table
- **Deterministic Behavior**: State machine executes predefined transitions
- **Adapter Pattern**: Clean adapters to dialog-core and media-core
- **Unified API**: Single UnifiedCoordinator + SimplePeer wrapper
- **YAML Configuration**: State table can be customized without code changes
- **Code Organization**: ~50 source files with clear separation of concerns

## Detailed Comparison

### 1. State Management

#### session-core
```rust
// State transitions managed imperatively in code
pub async fn create_outgoing_call(
    coordinator: &Arc<SessionCoordinator>,
    from: &str,
    to: &str,
) -> Result<CallSession> {
    // Explicit steps:
    // 1. Create session
    // 2. Generate SDP
    // 3. Create dialog
    // 4. Send INVITE
    // 5. Update state
    // ... all in code
}
```

**Pros**:
- Direct control over state transitions
- Easy to add custom logic inline
- No external DSL to learn

**Cons**:
- State logic scattered across modules
- Hard to visualize all possible transitions
- Difficult to modify behavior without code changes

#### session-core-v2
```yaml
# state_tables/default.yaml
- from_state: Idle
  role: UAC
  event: MakeCall
  guards:
    - IsIdle
  actions:
    - CreateDialog
    - GenerateLocalSDP
    - CreateMediaSession
    - SendINVITE
  next_state: Initiating
  publish_events:
    - SessionCreated
```

**Pros**:
- All transitions visible in one place
- Easy to visualize state machine
- Can modify behavior by changing YAML
- Formal verification possible

**Cons**:
- Need to learn YAML DSL
- Complex logic harder to express
- Requires code for custom actions

### 2. API Complexity

#### session-core: Multi-Layered APIs

```rust
// Multiple trait-based APIs
trait SessionControl {
    async fn create_outgoing_call(...) -> Result<CallSession>;
    async fn accept_incoming_call(...) -> Result<CallSession>;
    async fn terminate_session(...) -> Result<()>;
    async fn hold_session(...) -> Result<()>;
    async fn transfer_session(...) -> Result<()>;
    // ... 20+ methods
}

trait MediaControl {
    async fn generate_sdp_offer(...) -> Result<String>;
    async fn establish_media_flow(...) -> Result<()>;
    async fn get_media_statistics(...) -> Result<Option<MediaStatistics>>;
    // ... 25+ methods
}

trait SipClient {
    async fn register(...) -> Result<RegistrationHandle>;
    async fn send_options(...) -> Result<SipResponse>;
    // ... 5+ methods
}

// Plus: unified client module wrapping all three
pub mod unified { /* 700+ lines of wrappers */ }
```

**Pros**:
- Rich feature set out of the box
- Granular control over all aspects
- Extensive options for advanced use cases

**Cons**:
- Steep learning curve
- 50+ API methods to learn
- Easy to use APIs incorrectly

#### session-core-v2: Simplified Unified API

```rust
// SimplePeer - ultra-simple wrapper
pub struct SimplePeer {
    async fn call(&self, to: &str) -> Result<CallId>;
    async fn accept(&self, call_id: &CallId) -> Result<()>;
    async fn hangup(&self, call_id: &CallId) -> Result<()>;
    async fn transfer(&self, call_id: &CallId, target: &str) -> Result<()>;
    // ... ~15 essential methods
}

// UnifiedCoordinator - full-featured but unified
pub struct UnifiedCoordinator {
    async fn make_call(&self, from: &str, to: &str) -> Result<SessionId>;
    async fn accept_call(&self, session_id: &SessionId) -> Result<()>;
    async fn hangup(&self, session_id: &SessionId) -> Result<()>;
    async fn blind_transfer(&self, session_id: &SessionId, target: &str) -> Result<()>;
    // ... ~20 methods total
}
```

**Pros**:
- Minimal learning curve
- Hard to use incorrectly
- Clear, consistent API surface

**Cons**:
- Less granular control
- Some advanced features require deeper access
- Fewer configuration options

### 3. Extensibility

#### session-core
**Strengths**:
- Add new features by creating new modules
- Hook into event system at many levels
- Override behavior through inheritance/traits
- Rich plugin points throughout

**Weaknesses**:
- Changes require code modifications
- Testing changes requires recompiling
- Hard to A/B test behaviors
- Version upgrades may break customizations

#### session-core-v2
**Strengths**:
- Extend behavior by modifying YAML table
- Test different state machines without code changes
- Easy to version control state tables
- Custom actions/guards via code
- Hot-reload state tables (with feature flag)

**Weaknesses**:
- YAML limited for complex logic
- Need to implement custom actions in Rust for advanced features
- State table validation required

### 4. Testing & Verification

#### session-core
- **Test Count**: 400+ tests
- **Coverage**: Comprehensive unit, integration, and end-to-end tests
- **Test Types**:
  - Media integration (14 tests)
  - State transitions (17 tests)
  - Conference tests
  - Bridge tests
  - Performance benchmarks
- **Verification**: Manual code review, extensive test suites

**Challenges**:
- Hard to test all state combinations
- State logic scattered makes verification difficult
- Implicit state transitions not obvious

#### session-core-v2
- **Test Count**: Fewer but more focused
- **Coverage**: State table validation, transition tests, adapter tests
- **Test Types**:
  - State table validation
  - Transition correctness
  - Guard/action execution
  - Adapter behavior
- **Verification**: State table validation, formal verification possible

**Advantages**:
- All transitions explicitly defined
- Can validate state table completeness
- Easier to spot missing transitions
- Can generate test cases from state table

### 5. Code Size & Complexity

| Metric | session-core | session-core-v2 |
|--------|--------------|-----------------|
| Source Files | ~100+ | ~50 |
| Total Lines | ~25,000+ | ~10,000+ |
| API Methods | 50+ | ~20 |
| Dependencies | Similar | Similar |
| Module Count | 15+ | 8 |
| Complexity | High | Medium |

### 6. Performance Characteristics

#### session-core
- **Session Creation**: <1ms (direct coordination)
- **State Transitions**: Immediate (in-memory)
- **Memory per Session**: ~3KB
- **Overhead**: Minimal (no lookups)

#### session-core-v2
- **Session Creation**: <1ms (+ table lookup)
- **State Transitions**: Table lookup + execution (~0.1ms overhead)
- **Memory per Session**: ~3KB + state table entry
- **Overhead**: HashMap lookup per transition

**Verdict**: Performance is comparable; v2 has slight overhead from table lookups but negligible in practice.

### 7. Production Readiness

#### session-core
**Maturity**: ✅ Production-ready
- 400+ tests passing
- Real media-core integration
- B2BUA bugs fixed (Phase 17)
- Extensive real-world usage examples
- Comprehensive documentation

**Known Issues**:
- Complexity can lead to maintenance burden
- Some architectural tech debt from evolution

#### session-core-v2
**Maturity**: ⚠️ Newer, less battle-tested
- Core functionality working
- State machine architecture proven
- Transfer support implemented
- Needs more real-world testing

**Known Issues**:
- Less mature than v1
- Fewer examples
- Some features may need state table updates

## Use Case Recommendations

### Choose session-core if:
✅ You need maximum control and flexibility
✅ You have complex business logic requirements
✅ You need all features immediately available
✅ You prefer imperative programming style
✅ You need battle-tested production code
✅ You have time to learn the comprehensive API

**Best For**:
- Large-scale PBX systems
- Complex call center applications
- Applications requiring extensive customization
- Teams comfortable with complex APIs

### Choose session-core-v2 if:
✅ You want simple, clean APIs
✅ You prefer declarative configuration
✅ You want to modify behavior without code changes
✅ You need to version control call flow logic
✅ You want easier testing and verification
✅ You value maintainability over features

**Best For**:
- Simple SIP applications
- Peer-to-peer calling
- Rapid prototyping
- Applications needing configurable call flows
- Teams wanting minimal learning curve

## Migration Path

If you're considering switching from v1 to v2 or vice versa:

### v1 → v2
**Difficulty**: Medium
**Steps**:
1. Identify your core call flows
2. Map them to state table transitions
3. Rewrite using SimplePeer or UnifiedCoordinator
4. Test thoroughly with your use cases

**Challenges**:
- Some advanced features may need custom actions
- Conference support less mature in v2
- Bridge management different approach

### v2 → v1
**Difficulty**: Easy
**Steps**:
1. Replace SimplePeer with SessionControl API
2. Update method names and signatures
3. Test integration

**Challenges**:
- Need to learn more complex API
- More code to maintain

## Future Direction

### session-core Roadmap
- Enhanced developer experience
- WebRTC integration
- Advanced transfer features
- Real-time dashboards

### session-core-v2 Roadmap
- State table visualization tools
- Hot-reload capabilities
- More example state tables
- Formal verification tools

## Conclusion

Both libraries are well-architected and capable. The choice depends on your priorities:

**session-core**: Choose for **maximum features and battle-tested reliability**
**session-core-v2**: Choose for **simplicity, maintainability, and declarative configuration**

For new projects, I recommend:
- **Start with session-core-v2** if you want fast development and simple APIs
- **Use session-core** if you need comprehensive features and proven production reliability

Both can coexist in the same system if needed, as they use the same underlying dialog-core and media-core libraries.
