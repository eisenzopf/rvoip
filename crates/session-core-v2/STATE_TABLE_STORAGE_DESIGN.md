# State Table Storage Design

## Two Distinct Concepts

### 1. The Master State Table (Static Rules)
**What it is**: The transition rules - "if in state X and event Y happens, do Z"  
**Storage**: Compiled into the binary as a static, immutable data structure  
**Cardinality**: ONE table for the entire system  

### 2. Session State (Runtime Data)
**What it is**: Current state of each active session  
**Storage**: In-memory HashMap or similar structure  
**Cardinality**: ONE entry per active call/session  

## The Master State Table Storage

### Option 1: Static Compile-Time Table (Recommended)
```rust
lazy_static! {
    static ref MASTER_TABLE: HashMap<StateKey, Transition> = {
        let mut table = HashMap::new();
        
        // UAC transitions
        table.insert(
            StateKey { 
                role: Role::UAC, 
                state: CallState::Initiating, 
                event: EventType::Dialog200OK 
            },
            Transition {
                guards: Guards::has_sdp(),
                actions: vec![Action::SendACK, Action::NegotiateSDP],
                next_state: Some(CallState::Active),
                publish: vec![Event::StateChanged],
            }
        );
        // ... hundreds more entries
        
        table
    };
}
```

**Pros**: 
- Zero runtime overhead
- Immutable, thread-safe
- Can be verified at compile time

**Cons**:
- Cannot modify without recompiling
- Takes up binary space

### Option 2: Configuration File Table
```yaml
# state_table.yaml
transitions:
  - key:
      role: UAC
      state: Initiating
      event: Dialog200OK
    transition:
      guards: [has_sdp]
      actions: [SendACK, NegotiateSDP]
      next_state: Active
      publish: [StateChanged]
```

```rust
static TABLE: OnceCell<MasterTable> = OnceCell::new();

fn init_table() {
    let yaml = fs::read_to_string("state_table.yaml")?;
    let table = MasterTable::from_yaml(&yaml)?;
    TABLE.set(table);
}
```

**Pros**:
- Can modify without recompiling
- Easier to review/audit
- Can validate separately

**Cons**:
- Runtime parsing overhead
- Potential for runtime errors

### Option 3: Code-Generated Table
```rust
// build.rs
fn main() {
    let table_spec = load_table_specification();
    let rust_code = generate_state_table_code(table_spec);
    fs::write("src/generated_table.rs", rust_code)?;
}

// In main code
include!("generated_table.rs");
```

**Pros**:
- Best of both worlds
- Compile-time verification
- Easy to modify source

## Session State Storage

### In-Memory Storage Structure
```rust
pub struct SessionStore {
    // Primary storage - all active sessions
    sessions: Arc<RwLock<HashMap<SessionId, SessionState>>>,
    
    // Indexes for fast lookup
    by_dialog: Arc<RwLock<HashMap<DialogId, SessionId>>>,
    by_call_id: Arc<RwLock<HashMap<CallId, SessionId>>>,
    by_media_id: Arc<RwLock<HashMap<MediaSessionId, SessionId>>>,
}

pub struct SessionState {
    // Identity
    pub session_id: SessionId,
    pub role: Role,
    
    // Current state
    pub state: CallState,
    pub entered_state_at: Instant,
    
    // Readiness flags
    pub dialog_established: bool,
    pub media_ready: bool,
    pub sdp_negotiated: bool,
    
    // Data
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub dialog_id: Option<DialogId>,
    pub media_session_id: Option<MediaSessionId>,
    
    // History (optional, for debugging)
    pub history: Option<SessionHistory>,
}
```

### Optional: Session History Tracking
```rust
pub struct SessionHistory {
    // Ring buffer of last N transitions
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

impl SessionHistory {
    pub fn record_transition(&mut self, record: TransitionRecord) {
        if self.transitions.len() >= self.max_size {
            self.transitions.pop_front();
        }
        self.transitions.push_back(record);
    }
    
    pub fn get_history(&self) -> Vec<TransitionRecord> {
        self.transitions.iter().cloned().collect()
    }
}
```

## How They Work Together

```rust
pub struct StateMachine {
    // Static rules (shared by all sessions)
    table: &'static MasterTable,
    
    // Runtime state (per session)
    store: SessionStore,
}

impl StateMachine {
    pub async fn process_event(
        &self,
        session_id: &SessionId,
        event: Event,
    ) -> Result<()> {
        // 1. Get current session state
        let mut session_state = self.store.get_session(session_id)?;
        
        // 2. Look up transition rule in static table
        let key = StateKey {
            role: session_state.role,
            state: session_state.state,
            event: event.event_type(),
        };
        
        let transition = self.table.get(&key)
            .ok_or("No transition defined")?;
        
        // 3. Check guards
        if !transition.check_guards(&session_state) {
            return Ok(()); // Not ready
        }
        
        // 4. Execute actions
        for action in &transition.actions {
            self.execute_action(action, &mut session_state).await?;
        }
        
        // 5. Update state
        if let Some(next_state) = transition.next_state {
            // Record history if enabled
            if let Some(ref mut history) = session_state.history {
                history.record_transition(TransitionRecord {
                    timestamp: Instant::now(),
                    from_state: session_state.state,
                    event: event.event_type(),
                    to_state: next_state,
                    actions_taken: transition.actions.clone(),
                });
            }
            
            session_state.state = next_state;
            session_state.entered_state_at = Instant::now();
        }
        
        // 6. Save updated state
        self.store.update_session(session_state)?;
        
        // 7. Publish events
        for event in &transition.publish {
            self.publish_event(event).await?;
        }
        
        Ok(())
    }
}
```

## Storage Patterns

### Pattern 1: Write-Through Cache
```rust
pub struct CachedSessionStore {
    // Fast path - in-memory
    cache: Arc<RwLock<HashMap<SessionId, SessionState>>>,
    
    // Persistence layer (optional)
    persistent: Option<Box<dyn SessionPersistence>>,
}

impl CachedSessionStore {
    async fn update_session(&self, state: SessionState) -> Result<()> {
        // Update cache
        self.cache.write().await.insert(state.session_id.clone(), state.clone());
        
        // Write through to persistence if configured
        if let Some(ref persistent) = self.persistent {
            persistent.save_session(&state).await?;
        }
        
        Ok(())
    }
}
```

### Pattern 2: Event Sourcing (Optional)
```rust
pub struct EventSourcedStore {
    // Current state (derived from events)
    current: Arc<RwLock<HashMap<SessionId, SessionState>>>,
    
    // Event log (source of truth)
    events: Arc<RwLock<Vec<SessionEvent>>>,
}

impl EventSourcedStore {
    pub async fn replay_session(&self, session_id: &SessionId) -> Result<SessionState> {
        let events = self.events.read().await;
        let session_events: Vec<_> = events
            .iter()
            .filter(|e| e.session_id() == session_id)
            .collect();
        
        let mut state = SessionState::new(session_id);
        for event in session_events {
            state = self.apply_event(state, event)?;
        }
        
        Ok(state)
    }
}
```

## Memory Footprint

### Static Table Size
```rust
// Approximate sizes
StateKey: 3 bytes (role + state + event enums)
Transition: ~100 bytes (vectors of actions/events)
Total entries: ~200-300 for complete SIP/RTP

Total table size: ~30KB (negligible)
```

### Per-Session Memory
```rust
SessionState: ~500 bytes
- IDs: 100 bytes
- Flags: 3 bytes  
- SDPs: 200 bytes (optional)
- History: 1KB (optional)

For 10,000 concurrent calls: ~5MB (without history)
For 10,000 concurrent calls: ~15MB (with history)
```

## Persistence Options

### Option 1: No Persistence (Recommended for Most)
- Sessions only in memory
- Lost on restart
- Simplest, fastest

### Option 2: Redis/KeyDB
```rust
impl SessionPersistence for RedisStore {
    async fn save_session(&self, state: &SessionState) -> Result<()> {
        let key = format!("session:{}", state.session_id);
        let value = serde_json::to_string(state)?;
        self.redis.set_ex(key, value, 3600).await?; // 1 hour TTL
        Ok(())
    }
}
```

### Option 3: SQLite/PostgreSQL
```sql
CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    state TEXT NOT NULL,
    dialog_established BOOLEAN,
    media_ready BOOLEAN,
    sdp_negotiated BOOLEAN,
    local_sdp TEXT,
    remote_sdp TEXT,
    created_at TIMESTAMP,
    updated_at TIMESTAMP
);

CREATE TABLE session_history (
    id SERIAL PRIMARY KEY,
    session_id TEXT REFERENCES sessions(session_id),
    timestamp TIMESTAMP,
    from_state TEXT,
    event TEXT,
    to_state TEXT,
    actions JSONB
);
```

## Cleanup Strategy

```rust
impl SessionStore {
    pub async fn cleanup_stale_sessions(&self) {
        let mut sessions = self.sessions.write().await;
        let now = Instant::now();
        
        sessions.retain(|_, state| {
            match state.state {
                CallState::Terminated | CallState::Failed(_) => {
                    // Keep terminated sessions for N minutes for debugging
                    now.duration_since(state.entered_state_at) < Duration::from_secs(300)
                }
                _ => {
                    // Remove sessions idle for > 1 hour
                    now.duration_since(state.entered_state_at) < Duration::from_secs(3600)
                }
            }
        });
    }
}
```

## Debugging Features

### Session Inspector
```rust
impl SessionStore {
    pub async fn inspect_session(&self, session_id: &SessionId) -> SessionInspection {
        let state = self.sessions.read().await.get(session_id).cloned();
        
        SessionInspection {
            current_state: state.clone(),
            history: state.and_then(|s| s.history.map(|h| h.get_history())),
            possible_transitions: self.get_possible_transitions(&state),
            time_in_state: state.map(|s| Instant::now() - s.entered_state_at),
        }
    }
    
    fn get_possible_transitions(&self, state: &Option<SessionState>) -> Vec<EventType> {
        if let Some(state) = state {
            MASTER_TABLE.get_valid_events(state.role, state.state)
        } else {
            vec![]
        }
    }
}
```

## Summary

**The Master State Table**:
- Static, immutable rules
- ONE per system
- ~30KB in memory
- Never changes during runtime

**Session State Storage**:
- Dynamic, mutable state
- ONE per active call
- ~500 bytes per session
- Updated on every transition

**They work together**:
1. Event arrives for session X
2. Look up session X's current state
3. Look up transition rule in master table
4. Execute transition
5. Update session X's state
6. Optional: Record history

This separation gives you:
- **Correctness**: Rules can't be accidentally modified
- **Performance**: O(1) lookups for both
- **Debugging**: Can inspect any session's state and history
- **Scalability**: Can handle 100K+ concurrent sessions in ~50MB RAM