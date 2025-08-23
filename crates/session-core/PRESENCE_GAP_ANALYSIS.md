# Presence Support Gap Analysis

## Executive Summary

This document analyzes the current support for SIP presence functionality across the rvoip codebase and identifies gaps that need to be filled to implement the Presence API Plan.

## Current State Analysis

### 1. SIP-Core Layer ✅ GOOD

**What's Supported:**
- ✅ **Methods**: PUBLISH, SUBSCRIBE, NOTIFY are all defined in `Method` enum
- ✅ **Event Header**: Full implementation in `types/event.rs`
  - EventType (Token/Package)
  - Event parameters including 'id'
  - Parser and builder support
- ✅ **Headers**: Event, Expires, Accept, Content-Type headers all present
- ✅ **Content Types**: Support for arbitrary content types (needed for PIDF XML)

**Gaps:**
- ❌ No PIDF (Presence Information Data Format) XML parser/builder
- ❌ No Subscription-State header implementation
- ❌ No Allow-Events header

### 2. Transaction-Core Layer ⚠️ PARTIAL

**What's Supported:**
- ✅ Non-INVITE client transactions (can handle PUBLISH, SUBSCRIBE)
- ✅ Basic dialog support

**Gaps:**
- ❌ No specific SUBSCRIBE dialog state machine
- ❌ No subscription refresh mechanism
- ❌ No NOTIFY transaction handling as UAS
- ❌ No subscription expiry management

### 3. Dialog-Core Layer ⚠️ PARTIAL

**What's Supported:**
- ✅ SUBSCRIBE method mentioned in various files
- ✅ NOTIFY method referenced
- ✅ Dialog creation for SUBSCRIBE (treats it like INVITE)

**Gaps:**
- ❌ No subscription-specific dialog state machine
- ❌ No subscription refresh timer
- ❌ No forked subscription handling
- ❌ No subscription termination logic (Subscription-State: terminated)

### 4. Session-Core Layer ⚠️ MINIMAL

**What's Supported:**
- ✅ Tests exist for SUBSCRIBE/NOTIFY (`tests/dialog_subscribe.rs`, `tests/dialog_notify.rs`)
- ✅ Basic dialog operations that could support subscriptions
- ✅ Global event system via infra-common (`InfraSessionEventSystem`, `SessionEventAdapter`)
- ✅ Event adapters for cross-layer communication
- ✅ Existing SessionEvent enum that can be extended

**Gaps:**
- ❌ No PresenceCoordinator (parallel to SessionCoordinator)
- ❌ No PresenceEvent type definition
- ❌ No presence event routing in adapters
- ❌ No subscription management
- ❌ No PIDF generation/parsing
- ❌ No presence API layer

## Detailed Gap Analysis

### Gap 1: PIDF XML Support

**Location**: sip-core  
**Priority**: HIGH  
**Work Required**:
```rust
// New file: sip-core/src/types/pidf.rs
pub struct PresenceDocument {
    entity: String,
    tuples: Vec<Tuple>,
}

pub struct Tuple {
    id: String,
    status: BasicStatus,
    note: Option<String>,
    timestamp: Option<DateTime>,
}

pub enum BasicStatus {
    Open,
    Closed,
}
```

### Gap 2: Subscription-State Header

**Location**: sip-core  
**Priority**: HIGH  
**Work Required**:
```rust
// New file: sip-core/src/types/subscription_state.rs
pub struct SubscriptionState {
    pub state: SubState,
    pub expires: Option<u32>,
    pub reason: Option<String>,
    pub retry_after: Option<u32>,
}

pub enum SubState {
    Active,
    Pending,
    Terminated,
}
```

### Gap 3: Subscription Dialog State Machine

**Location**: dialog-core  
**Priority**: HIGH  
**Work Required**:
- Extend dialog state machine to handle subscription-specific states
- Add subscription refresh timer
- Handle NOTIFY within subscription dialog
- Track subscription expiry

### Gap 4: Presence Events

**Location**: session-core  
**Priority**: HIGH  
**Work Required**:
```rust
// New file: session-core/src/events/presence_events.rs
pub enum PresenceEvent {
    PresencePublished { ... },
    SubscriptionRequest { ... },
    PresenceNotification { ... },
    // etc.
}

impl Event for PresenceEvent { ... }
```

### Gap 5: PresenceCoordinator  

**Location**: session-core  
**Priority**: HIGH  
**Work Required**:
```rust
// New file: session-core/src/presence/coordinator.rs
pub struct PresenceCoordinator {
    event_bus: EventBus,
    subscriptions: HashMap<String, Subscription>,
    watchers: HashMap<String, mpsc::Sender<PresenceInfo>>,
    presence_state: HashMap<String, PresenceDocument>,
}
```

### Gap 6: Presence API Layer

**Location**: session-core/src/api/presence.rs  
**Priority**: HIGH  
**Work Required**:
- Implement the API design from PRESENCE_API_PLAN.md
- SimplePeer extensions
- PresenceWatcher implementation
- BuddyList functionality
- Integration with EventBus

## Implementation Priority

### Phase 1: Foundation (1-2 days)
1. **sip-core**: Add PIDF XML support
2. **sip-core**: Add Subscription-State header
3. **sip-core**: Add Allow-Events header

### Phase 2: Events & Coordination (1-2 days)
1. **session-core**: Define PresenceEvent types
2. **session-core**: Extend SessionEventAdapter for presence
3. **dialog-core**: Extend DialogEventAdapter for subscriptions
4. **session-core**: Create PresenceCoordinator with EventBus integration

### Phase 3: Dialog Support (2-3 days)
1. **dialog-core**: Implement subscription dialog state machine
2. **dialog-core**: Add subscription refresh timers
3. **dialog-core**: Handle NOTIFY in subscription context
4. **transaction-core**: Ensure NOTIFY server transactions work

### Phase 4: API Layer (1-2 days)
1. **session-core**: Implement api/presence.rs
2. **session-core**: Add SimplePeer extensions
3. **session-core**: Create PresenceWatcher (EventBus subscriber)
4. **session-core**: Implement BuddyList

### Phase 5: Testing & Polish (1-2 days)
1. Unit tests for each layer
2. Integration tests for P2P presence
3. Integration tests for B2BUA presence
4. Documentation and examples

## Risk Assessment

### Low Risk ✅
- SIP-core already has good foundation
- Event header fully implemented
- Methods already supported

### Medium Risk ⚠️
- Dialog-core modifications needed
- Subscription state machine complexity
- Timer management for refreshes

### High Risk ❌
- PIDF XML parsing/generation (new dependency?)
- Subscription forking scenarios
- Presence authorization/privacy

## Recommendations

1. **Start with PIDF**: Consider using existing XML library (quick-xml or roxmltree)
2. **Simplify Phase 1**: Focus on basic presence (open/closed) before rich presence
3. **Reuse Existing**: Leverage existing dialog infrastructure where possible
4. **Test Early**: Build integration tests alongside implementation
5. **Document Well**: Presence is complex - good docs essential

## Conclusion

The foundation for presence support exists, but significant work is needed across all layers:
- **sip-core**: 20% complete (methods and Event header done)
- **transaction-core**: 10% complete (basic infrastructure exists)
- **dialog-core**: 15% complete (dialog basics present, subscription-specific missing)
- **session-core**: 5% complete (only test stubs exist)

**Total Estimated Effort**: 8-12 days for full implementation

## Next Steps

1. Review and approve this gap analysis
2. Decide on XML library for PIDF
3. Create detailed technical design for subscription dialog state machine
4. Begin Phase 1 implementation (PIDF and headers)
5. Establish testing strategy early

## Open Questions

1. Should we use an external XML library or write minimal PIDF parser?
2. How much rich presence do we support initially?
3. Should presence authorization be in scope for v1?
4. Do we need presence aggregation for multiple devices?
5. Should we support XCAP for presence rules?