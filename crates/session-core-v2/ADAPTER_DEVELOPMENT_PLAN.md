# Dialog-Core and Media-Core Adapter Development Plan

## Event Flow Analysis

### Critical Event Ordering for State Table

The state table expects events in this sequence:

#### UAC (Outbound Call) Flow:
```
1. User: MakeCall
2. Dialog: SendINVITE → (network)
3. Dialog: ← 180 Ringing → EventType::Dialog180Ringing
4. Dialog: ← 200 OK (with SDP) → EventType::Dialog200OK
5. State Machine: NegotiateSDPAsUAC action
6. Dialog: SendACK → (network)
7. Media: StartMediaSession → EventType::MediaSessionCreated
8. Media: MediaReady → EventType::MediaSessionReady
9. Media: NegotiateComplete → EventType::MediaNegotiated
10. Media: RTP flows → EventType::MediaFlowEstablished
11. User/Dialog: HangupCall/BYE → EventType::DialogBYE
```

#### UAS (Inbound Call) Flow:
```
1. Dialog: ← INVITE (with SDP) → EventType::DialogInvite
2. State Machine: StoreRemoteSDP action
3. User: AcceptCall → EventType::AcceptCall
4. State Machine: NegotiateSDPAsUAS action
5. Dialog: Send200OK → (network)
6. Dialog: ← ACK → EventType::DialogACK
7. Media: StartMediaSession → EventType::MediaSessionCreated
8. Media: MediaReady → EventType::MediaSessionReady
9. Media: RTP flows → EventType::MediaFlowEstablished
```

## Adapter Architecture

### Design Principles
1. **Thin Translation Layer** - Adapters only translate events, no business logic
2. **Direct State Table Mapping** - Events map directly to EventType enum
3. **Minimal State** - Adapters maintain minimal mapping state (session IDs only)
4. **No Buffering** - Events flow immediately to state machine

## Development Tasks

### Phase 1: Dialog Adapter Simplification

#### Task 1.1: Create Minimal DialogAdapter
**File**: `src/adapters/dialog_adapter.rs` (refactor existing)
```rust
pub struct DialogAdapter {
    dialog_api: Arc<UnifiedDialogApi>,
    event_tx: mpsc::Sender<(SessionId, EventType)>,
    session_map: Arc<DashMap<DialogId, SessionId>>,
}

impl DialogAdapter {
    // Outbound actions (from state machine)
    async fn send_invite(&self, session_id: &SessionId, target: &str, sdp: Option<String>);
    async fn send_response(&self, session_id: &SessionId, code: u16, sdp: Option<String>);
    async fn send_ack(&self, session_id: &SessionId);
    async fn send_bye(&self, session_id: &SessionId);
    async fn send_cancel(&self, session_id: &SessionId);
    
    // Inbound events (from dialog-core)
    async fn handle_dialog_event(&self, event: DialogEvent);
}
```
**Lines**: ~200 (down from ~800)

#### Task 1.2: Direct Event Translation
**Implementation**:
```rust
fn translate_dialog_event(event: DialogEvent) -> Option<EventType> {
    match event {
        DialogEvent::IncomingInvite { .. } => Some(EventType::DialogInvite),
        DialogEvent::Response(180) => Some(EventType::Dialog180Ringing),
        DialogEvent::Response(200) => Some(EventType::Dialog200OK),
        DialogEvent::AckReceived => Some(EventType::DialogACK),
        DialogEvent::ByeReceived => Some(EventType::DialogBYE),
        DialogEvent::Error(e) => Some(EventType::DialogError(e.to_string())),
        _ => None, // Ignore non-state-changing events
    }
}
```
**Lines**: ~50

#### Task 1.3: Remove Intermediate Types
- Remove SessionDialogHandle
- Remove DialogBridge
- Remove DialogConfigConverter
- Use dialog-core types directly
**Reduction**: -600 lines

### Phase 2: Media Adapter Simplification

#### Task 2.1: Create Minimal MediaAdapter
**File**: `src/adapters/media_adapter.rs` (refactor existing)
```rust
pub struct MediaAdapter {
    controller: Arc<MediaSessionController>,
    event_tx: mpsc::Sender<(SessionId, EventType)>,
    session_map: Arc<DashMap<SessionId, MediaSessionId>>,
}

impl MediaAdapter {
    // Outbound actions (from state machine)
    async fn start_session(&self, session_id: &SessionId) -> Result<()>;
    async fn stop_session(&self, session_id: &SessionId) -> Result<()>;
    async fn negotiate_sdp_as_uac(&self, session_id: &SessionId, remote_sdp: &str) -> Result<(String, NegotiatedConfig)>;
    async fn negotiate_sdp_as_uas(&self, session_id: &SessionId, remote_sdp: &str) -> Result<(String, NegotiatedConfig)>;
    
    // Inbound events (from media-core)
    async fn handle_media_event(&self, event: MediaEvent);
}
```
**Lines**: ~200 (down from ~1200)

#### Task 2.2: Streamlined SDP Negotiation
```rust
async fn negotiate_sdp_as_uas(&self, session_id: &SessionId, remote_sdp: &str) -> Result<(String, NegotiatedConfig)> {
    // 1. Parse remote SDP
    let remote = parse_sdp(remote_sdp)?;
    
    // 2. Allocate local port
    let local_port = self.controller.allocate_port().await?;
    
    // 3. Create media session
    let media_id = self.controller.create_session(local_port, remote.port).await?;
    self.session_map.insert(session_id.clone(), media_id);
    
    // 4. Generate answer SDP
    let local_sdp = generate_answer_sdp(local_port, &remote)?;
    
    // 5. Return negotiated config
    let config = NegotiatedConfig {
        local_addr: SocketAddr::new(self.local_ip, local_port),
        remote_addr: SocketAddr::new(remote.ip, remote.port),
        codec: remote.codec,
    };
    
    Ok((local_sdp, config))
}
```
**Lines**: ~100

#### Task 2.3: Critical Media Events Only
```rust
fn translate_media_event(event: MediaEvent) -> Option<EventType> {
    match event {
        MediaEvent::SessionCreated { .. } => Some(EventType::MediaSessionCreated),
        MediaEvent::MediaReady { .. } => Some(EventType::MediaSessionReady),
        MediaEvent::RtpFlowing { .. } => Some(EventType::MediaFlowEstablished),
        MediaEvent::Error(e) => Some(EventType::MediaError(e.to_string())),
        _ => None, // Ignore stats, quality reports, etc.
    }
}
```
**Lines**: ~30

### Phase 3: Event Synchronization

#### Task 3.1: Event Router
**File**: `src/adapters/event_router.rs`
```rust
pub struct EventRouter {
    state_machine: Arc<StateMachineExecutor>,
    dialog_adapter: Arc<DialogAdapter>,
    media_adapter: Arc<MediaAdapter>,
}

impl EventRouter {
    // Route events from adapters to state machine
    async fn route_to_state_machine(&self, session_id: SessionId, event: EventType) {
        self.state_machine.process_event(&session_id, event).await;
    }
    
    // Route actions from state machine to adapters
    async fn execute_action(&self, session_id: &SessionId, action: Action) {
        match action {
            Action::SendINVITE => self.dialog_adapter.send_invite(session_id, ..).await,
            Action::StartMediaSession => self.media_adapter.start_session(session_id).await,
            // ... other actions
        }
    }
}
```
**Lines**: ~150

#### Task 3.2: Ensure Event Ordering
- Dialog events must complete before media events
- Use async/await naturally for sequencing
- No complex buffering or queuing needed
**Implementation**: Built into state table guards

### Phase 4: Integration Testing

#### Task 4.1: Event Flow Tests
```rust
#[tokio::test]
async fn test_uac_event_flow() {
    // 1. Create adapters with mock backends
    // 2. Trigger MakeCall
    // 3. Simulate dialog responses
    // 4. Verify media events fire in order
    // 5. Check MediaFlowEstablished published
}
```

#### Task 4.2: Real Integration Tests
- Test with actual dialog-core
- Test with actual media-core
- Verify RTP flows

## Line Count Comparison

### Old session-core Integration:
```
dialog/
  manager.rs: ~400 lines
  bridge.rs: ~300 lines
  coordinator.rs: ~350 lines
  config.rs: ~200 lines
  types.rs: ~150 lines
  builder.rs: ~200 lines
  Total: ~1,600 lines

media/
  manager.rs: ~800 lines
  bridge.rs: ~250 lines
  coordinator.rs: ~400 lines
  config.rs: ~200 lines
  types.rs: ~300 lines
  engine.rs: ~500 lines
  Total: ~2,450 lines

Grand Total: ~4,050 lines
```

### New session-core-v2 Adapters:
```
adapters/
  dialog_adapter.rs: ~200 lines
  media_adapter.rs: ~200 lines
  event_router.rs: ~150 lines
  types.rs: ~50 lines
  Total: ~600 lines

Reduction: 85% fewer lines!
```

## Key Simplifications

1. **No Intermediate Types** - Use dialog-core and media-core types directly
2. **No Business Logic** - All logic in state table
3. **No Event Buffering** - Direct flow to state machine
4. **No Complex Mapping** - Simple ID mappings only
5. **No Codec Management** - Let media-core handle it
6. **No SDP Storage** - Store in SessionState
7. **No Separate Bridges** - Adapters handle translation directly

## Implementation Order

1. **Week 1**: Dialog Adapter
   - Day 1-2: Refactor existing adapter
   - Day 3-4: Event translation
   - Day 5: Testing

2. **Week 2**: Media Adapter  
   - Day 1-2: Refactor existing adapter
   - Day 3-4: SDP negotiation
   - Day 5: Testing

3. **Week 3**: Integration
   - Day 1-2: Event router
   - Day 3-4: End-to-end testing
   - Day 5: Performance optimization

## Success Metrics

- ✅ All state transitions triggered by correct events
- ✅ MediaFlowEstablished fires when all conditions met
- ✅ Under 600 total lines of adapter code
- ✅ No adapter-specific state beyond ID mappings
- ✅ Events flow in correct order for both UAC and UAS