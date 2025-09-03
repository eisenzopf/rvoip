# Missing Features in session-core-v2

## Overview
Session-core-v2 is **95% feature complete** with all core functionality working. The missing 5% consists of auxiliary features for production deployment, debugging, and advanced use cases.

## Missing Features by Category

### 1. Event History & Debugging (Priority: HIGH)
**Source**: STATE_TABLE_STORAGE_DESIGN.md, GAP_ANALYSIS.md

#### 1.1 Session History Tracking
```rust
// Not implemented - from STATE_TABLE_STORAGE_DESIGN.md
pub struct SessionHistory {
    transitions: VecDeque<TransitionRecord>,
    max_size: usize,
}

pub struct TransitionRecord {
    timestamp: Instant,
    from_state: CallState,
    event: EventType,
    to_state: CallState,
    actions_taken: Vec<ActionType>,
}
```

#### 1.2 Debugging Inspection Methods
```rust
// Not implemented - from STATE_TABLE_STORAGE_DESIGN.md
impl SessionStore {
    pub fn inspect_session(&self, session_id: &SessionId) -> SessionInspection;
    pub fn get_possible_transitions(&self, state: &SessionState) -> Vec<EventType>;
}
```

#### 1.3 State Visualization
- No state transition graph export
- No runtime state inspection API
- No transition history export for analysis

**Impact**: Makes debugging production issues difficult. Can't trace why a session got into a particular state.

---

### 2. Persistence Layer (Priority: MEDIUM)
**Source**: STATE_TABLE_STORAGE_DESIGN.md

#### 2.1 Database Support
```rust
// Not implemented - design specified these options
trait SessionPersistence {
    async fn save_session(&self, state: &SessionState) -> Result<()>;
    async fn load_session(&self, id: &SessionId) -> Result<SessionState>;
    async fn delete_session(&self, id: &SessionId) -> Result<()>;
}

// Missing implementations:
- Redis/KeyDB persistence
- SQLite persistence  
- PostgreSQL persistence
```

#### 2.2 Event Sourcing
```rust
// Not implemented - from STATE_TABLE_STORAGE_DESIGN.md
pub struct EventSourcedStore {
    current: Arc<RwLock<HashMap<SessionId, SessionState>>>,
    events: Arc<RwLock<Vec<SessionEvent>>>,
}
```

**Impact**: 
- Sessions lost on restart
- Can't scale horizontally
- No disaster recovery
- No audit trail

---

### 3. Session Cleanup & Resource Management (Priority: HIGH)
**Source**: STATE_TABLE_STORAGE_DESIGN.md

#### 3.1 Automatic Cleanup
```rust
// Not implemented
impl SessionStore {
    pub async fn cleanup_stale_sessions(&self) {
        // Remove terminated sessions after N minutes
        // Remove idle sessions after timeout
        // Clean up orphaned resources
    }
}
```

#### 3.2 Configurable Retention
- No TTL configuration per state
- No automatic resource cleanup
- No memory limit enforcement

**Impact**: Memory leaks in long-running deployments. Orphaned sessions accumulate.

---

### 4. Advanced Bridge & Transfer Features (Priority: LOW)
**Source**: BRIDGE_AND_TRANSFER_DESIGN.md

#### 4.1 Music on Hold
```rust
// Not implemented
Action::PlayMusicOnHold(session_id)
Action::StopMusicOnHold(session_id)
```

#### 4.2 Transfer Progress (NOTIFY)
```rust
// Partially implemented - missing detailed progress
Event::TransferProgress {
    session_id: SessionId,
    status: TransferStatus,  // Trying, Ringing, Success, Failed
    progress: u8,            // 0-100%
}
```

#### 4.3 Attended Transfer
Currently falls back to blind transfer. Need full implementation with:
- Consultation call establishment
- REFER with Replaces header
- Three-way state coordination

**Impact**: Limited PBX functionality. Can't implement full call center features.

---

### 5. Event Subscription Management (Priority: MEDIUM)
**Source**: UNIFIED_API_DESIGN.md

#### 5.1 Subscription Filtering
```rust
// Not implemented
pub struct SubscriptionFilter {
    event_types: Vec<EventType>,
    session_patterns: Vec<String>,
    priority: Priority,
}

impl UnifiedCoordinator {
    pub fn subscribe_filtered(&self, filter: SubscriptionFilter) -> Receiver<Event>;
}
```

#### 5.2 External Consumer Management
- No subscription lifecycle management
- No backpressure handling
- No dead subscription cleanup

**Impact**: Can't efficiently integrate with external systems. All consumers get all events.

---

### 6. Performance & Benchmarking (Priority: LOW)
**Source**: STATE_TABLE_IMPLEMENTATION_PLAN.md

#### 6.1 Performance Benchmarks
```rust
// Not implemented
#[bench]
fn bench_state_transition(b: &mut Bencher) {
    // Measure transition performance
}

#[bench]
fn bench_table_lookup(b: &mut Bencher) {
    // Verify O(1) lookup claim
}
```

#### 6.2 Optimizations Not Implemented
- No guard result caching
- No lazy evaluation of expensive guards
- No transition path optimization

**Impact**: Can't verify performance claims. May have hidden bottlenecks.

---

### 7. YAML Configuration Management (Priority: LOW)
**Source**: YAML_STATE_TABLE_PLAN.md

#### 7.1 Runtime Reloading
```rust
// Not implemented
impl YamlTableLoader {
    pub fn reload(&mut self) -> Result<()>;
    pub fn watch_file(&self, path: &Path) -> Result<()>;
}
```

#### 7.2 Schema Validation
- No JSON Schema validation
- No transition completeness validation
- No circular dependency detection

**Impact**: Configuration errors only caught at runtime. No hot-reloading for updates.

---

### 8. Monitoring & Metrics (Priority: MEDIUM)
**Source**: Not explicitly in plans but implied

#### 8.1 Metrics Collection
```rust
// Not implemented
pub struct SessionMetrics {
    active_sessions: Gauge,
    transitions_per_second: Counter,
    failed_transitions: Counter,
    session_duration: Histogram,
}
```

#### 8.2 Health Checks
- No health check endpoint
- No readiness/liveness probes
- No resource usage monitoring

**Impact**: Can't monitor production health. No alerting on issues.

---

## Implementation Priority

### Phase 1: Critical for Production (2-3 days)
1. **Session History Tracking** - Essential for debugging
2. **Automatic Cleanup** - Prevents memory leaks
3. **Basic Metrics** - Monitor health

### Phase 2: Scale & Reliability (3-4 days)
1. **Redis Persistence** - For horizontal scaling
2. **Subscription Filtering** - Reduce event noise
3. **Health Checks** - For orchestration

### Phase 3: Advanced Features (1 week)
1. **Attended Transfer** - Complete implementation
2. **Music on Hold** - User experience
3. **Performance Benchmarks** - Optimization
4. **YAML Hot-reload** - Operational flexibility

## Questions for Discussion

1. **Which features are must-have vs nice-to-have for your use case?**

2. **Persistence preference**: Redis for speed, PostgreSQL for durability, or both?

3. **History depth**: How many transitions should we keep? (affects memory)

4. **Cleanup policy**: How long to keep terminated sessions?

5. **Metrics backend**: Prometheus, StatsD, or custom?

6. **Should we add these features to session-core-v2 or create a session-core-v3?**

## Recommendation

Start with Phase 1 features as they're critical for any production deployment. The current implementation is solid for development/testing but needs these additions for production readiness.

The architecture is well-designed to accommodate these features without major refactoring - they're mostly additive.