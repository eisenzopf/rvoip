# B2BUA Implementation Plan

## Executive Summary

This plan describes building `b2bua-core` directly on `dialog-core`, bypassing `session-core-v2` entirely. This creates a purpose-built B2BUA library optimized for server scenarios (IVR, queues, conferences) while keeping `session-core-v2` simple for SIP endpoints.

## Architecture Overview

```
┌────────────────────────────────────────────────────────┐
│                 B2BUA Applications                     │
│  (IVR, Queue, Conference, Recording, Call Center)      │
└────────────────────────┬───────────────────────────────┘
                         │
┌────────────────────────▼───────────────────────────────┐
│                    b2bua-core                          │
│  - State Engine (B2BUA-specific)                       │
│  - Dialog Pair Management                              │
│  - Media Server Client                                 │
│  - Application Handlers                                │
└────────────┬───────────────────────┬───────────────────┘
             │                       │
    ┌────────▼─────────┐   ┌────────▼─────────┐
    │   dialog-core    │   │ media-server-core │
    │                  │   │   (remote media)   │
    └────────┬─────────┘   └──────────────────┘
             │
    ┌────────▼─────────┐
    │   infra-common   │
    │ (events, config) │
    └──────────────────┘
```

## Design Principles

1. **Direct Control**: b2bua-core directly manages SIP dialogs via dialog-core
2. **Remote Media**: All media processing delegated to media servers
3. **Event-Driven**: Uses atomic event bus from infra-common
4. **Application-Specific**: State machines designed for B2BUA patterns
5. **No Session Abstraction**: Not constrained by session-core-v2's model

## Core Components

### 1. B2BUA Core Engine

```rust
// b2bua-core/src/core.rs
pub struct B2buaCore {
    // Direct dialog management
    dialog_manager: Arc<DialogManager>,

    // Remote media control
    media_controller: Arc<dyn MediaServerController>,

    // Event coordination
    event_bus: Arc<AtomicEventBus>,

    // B2BUA state management
    state_engine: Arc<B2buaStateEngine>,

    // Call storage
    calls: Arc<DashMap<CallId, B2buaCall>>,

    // Application handlers
    handlers: Arc<DashMap<String, Box<dyn ApplicationHandler>>>,

    // Configuration
    config: B2buaConfig,
}

pub struct B2buaCall {
    id: CallId,
    leg_a: DialogHandle,
    leg_b: Option<DialogHandle>,
    state: B2buaState,
    media_sessions: Vec<MediaSessionId>,
    metadata: CallMetadata,
    timers: HashMap<String, TimerHandle>,
}
```

### 2. State Management

```rust
// b2bua-core/src/state.rs
pub enum B2buaState {
    // Basic states
    Idle,
    IncomingCall,
    OutgoingCall,
    Bridged,

    // Application states
    InIvr(IvrState),
    InQueue(QueueState),
    InConference(ConferenceState),
    Recording(RecordingState),

    // Transition states
    Transferring(TransferState),
    Parking(ParkState),
    Merging(MergeState),
}

pub struct B2buaStateEngine {
    // State transition rules
    transitions: Arc<StateTransitionTable>,

    // Active states
    states: Arc<DashMap<CallId, B2buaState>>,

    // State change notifications
    notifier: Arc<EventBus>,
}

impl B2buaStateEngine {
    pub async fn transition(
        &self,
        call_id: CallId,
        event: B2buaEvent,
    ) -> Result<B2buaState> {
        let current = self.states.get(&call_id)?;
        let next = self.transitions.evaluate(&current, &event)?;

        // Validate transition
        if !self.is_valid_transition(&current, &next) {
            return Err(Error::InvalidTransition);
        }

        // Execute transition
        self.states.insert(call_id, next.clone());

        // Notify listeners
        self.notifier.publish(StateChanged {
            call_id,
            from: current,
            to: next.clone(),
        }).await;

        Ok(next)
    }
}
```

### 3. Dialog Pair Management

```rust
// b2bua-core/src/dialog_pair.rs
pub struct DialogPair {
    leg_a: Dialog,
    leg_b: Dialog,
    bridge_mode: BridgeMode,
}

pub enum BridgeMode {
    Transparent,     // Pass everything through
    Intercepting,    // B2BUA processes messages
    Translating,     // Modify messages in transit
}

impl DialogPair {
    pub async fn bridge(&mut self) -> Result<()> {
        match self.bridge_mode {
            BridgeMode::Transparent => {
                self.transparent_bridge().await
            }
            BridgeMode::Intercepting => {
                self.intercepting_bridge().await
            }
            BridgeMode::Translating => {
                self.translating_bridge().await
            }
        }
    }

    pub async fn handle_reinvite(&mut self, from: DialogLeg, sdp: Sdp) -> Result<()> {
        // Handle hold, resume, codec changes
        match from {
            DialogLeg::A => {
                // Process and forward to leg B
                let modified_sdp = self.process_sdp(sdp)?;
                self.leg_b.send_reinvite(modified_sdp).await
            }
            DialogLeg::B => {
                // Process and forward to leg A
                let modified_sdp = self.process_sdp(sdp)?;
                self.leg_a.send_reinvite(modified_sdp).await
            }
        }
    }
}
```

### 4. Media Server Integration

```rust
// b2bua-core/src/media_client.rs
#[async_trait]
pub trait MediaServerController: Send + Sync {
    // Endpoint management
    async fn allocate_endpoint(&self) -> Result<MediaEndpoint>;
    async fn release_endpoint(&self, id: EndpointId) -> Result<()>;

    // Bridging
    async fn bridge(&self, a: EndpointId, b: EndpointId) -> Result<BridgeId>;
    async fn unbridge(&self, bridge: BridgeId) -> Result<()>;

    // IVR operations
    async fn play_prompt(&self, endpoint: EndpointId, file: &str) -> Result<()>;
    async fn collect_dtmf(&self, endpoint: EndpointId, max: u8) -> Result<String>;
    async fn play_and_collect(&self, endpoint: EndpointId, prompt: &str, max: u8) -> Result<String>;

    // Recording
    async fn start_recording(&self, endpoint: EndpointId) -> Result<RecordingId>;
    async fn stop_recording(&self, recording: RecordingId) -> Result<RecordingFile>;

    // Conference
    async fn create_conference(&self) -> Result<ConferenceId>;
    async fn add_to_conference(&self, conf: ConferenceId, endpoint: EndpointId) -> Result<()>;
    async fn remove_from_conference(&self, conf: ConferenceId, endpoint: EndpointId) -> Result<()>;

    // Music on hold
    async fn start_moh(&self, endpoint: EndpointId, class: &str) -> Result<()>;
    async fn stop_moh(&self, endpoint: EndpointId) -> Result<()>;
}

// Implementation using REST API
pub struct RestMediaServerClient {
    base_url: Url,
    client: reqwest::Client,
}

// Implementation using gRPC
pub struct GrpcMediaServerClient {
    channel: tonic::Channel,
}
```

### 5. Event System

```rust
// b2bua-core/src/events.rs
pub enum B2buaEvent {
    // SIP Events (from dialog-core)
    IncomingInvite(SipMessage),
    InviteResponse(SipResponse),
    Bye(SipMessage),
    Refer(SipMessage),
    Info(SipMessage),

    // Media Events (from media server)
    DtmfReceived { endpoint: EndpointId, digit: char },
    PlaybackComplete { endpoint: EndpointId, file: String },
    RecordingStarted { recording: RecordingId },
    RecordingStopped { recording: RecordingId, file: String },

    // Application Events
    AgentAvailable { agent: AgentId, skills: Vec<String> },
    QueueTimeout { queue: QueueId, call: CallId },
    ConferenceStarted { conference: ConferenceId },
    TransferRequested { from: CallId, to: String },

    // System Events
    TimerExpired { timer: TimerId, context: TimerContext },
    HealthCheck,
    Shutdown,
}

impl B2buaCore {
    async fn setup_event_handlers(&self) {
        // Subscribe to dialog events
        self.event_bus.subscribe("dialog.*", {
            let core = self.clone();
            move |event| core.handle_dialog_event(event)
        });

        // Subscribe to media events
        self.event_bus.subscribe("media.*", {
            let core = self.clone();
            move |event| core.handle_media_event(event)
        });

        // Subscribe to application events
        self.event_bus.subscribe("app.*", {
            let core = self.clone();
            move |event| core.handle_app_event(event)
        });
    }
}
```

### 6. Application Handlers

```rust
// b2bua-core/src/handlers/mod.rs
#[async_trait]
pub trait ApplicationHandler: Send + Sync {
    fn name(&self) -> &str;

    async fn can_handle(&self, call: &B2buaCall) -> bool;

    async fn handle_incoming_call(
        &self,
        call: &mut B2buaCall,
        invite: &SipMessage,
    ) -> Result<HandlerAction>;

    async fn handle_event(
        &self,
        call: &mut B2buaCall,
        event: &B2buaEvent,
    ) -> Result<HandlerAction>;
}

pub enum HandlerAction {
    Continue,           // Continue processing
    Complete,           // Handler completed
    TransferTo(String), // Transfer to another handler
    Bridge(Uri),        // Bridge to another endpoint
    Disconnect,         // End call
}
```

## Application Implementations

### IVR Handler

```rust
// b2bua-core/src/handlers/ivr.rs
pub struct IvrHandler {
    flows: HashMap<String, IvrFlow>,
    media_server: Arc<dyn MediaServerController>,
}

impl IvrHandler {
    async fn execute_flow(
        &self,
        call: &mut B2buaCall,
        flow: &IvrFlow,
    ) -> Result<()> {
        let mut current_node = flow.start_node.clone();

        loop {
            match &flow.nodes[&current_node] {
                IvrNode::PlayPrompt { file, next } => {
                    self.media_server.play_prompt(
                        call.media_endpoint(),
                        file
                    ).await?;
                    current_node = next.clone();
                }

                IvrNode::CollectInput { prompt, max_digits, next } => {
                    let input = self.media_server.play_and_collect(
                        call.media_endpoint(),
                        prompt,
                        *max_digits
                    ).await?;

                    call.set_variable("last_input", input);
                    current_node = next.clone();
                }

                IvrNode::Menu { prompt, options } => {
                    let digit = self.media_server.play_and_collect(
                        call.media_endpoint(),
                        prompt,
                        1
                    ).await?;

                    if let Some(next) = options.get(&digit) {
                        current_node = next.clone();
                    } else {
                        // Invalid option
                        current_node = flow.invalid_option_node.clone();
                    }
                }

                IvrNode::Transfer { destination } => {
                    call.transfer_to(destination).await?;
                    break;
                }

                IvrNode::Hangup => {
                    call.disconnect().await?;
                    break;
                }
            }
        }

        Ok(())
    }
}
```

### Queue Handler

```rust
// b2bua-core/src/handlers/queue.rs
pub struct QueueHandler {
    queues: Arc<QueueManager>,
    agents: Arc<AgentManager>,
    media_server: Arc<dyn MediaServerController>,
}

impl QueueHandler {
    async fn handle_queued_call(
        &self,
        call: &mut B2buaCall,
        queue: &Queue,
    ) -> Result<()> {
        // Add to queue
        let position = queue.add_call(call.id).await?;

        // Play initial announcement
        self.media_server.play_prompt(
            call.media_endpoint(),
            &queue.welcome_prompt
        ).await?;

        // Start music on hold
        self.media_server.start_moh(
            call.media_endpoint(),
            &queue.moh_class
        ).await?;

        // Wait for agent
        loop {
            tokio::select! {
                // Check for available agent
                agent = self.agents.get_available(&queue.skills) => {
                    if let Some(agent) = agent {
                        self.bridge_to_agent(call, agent).await?;
                        break;
                    }
                }

                // Play position announcement
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    self.announce_position(call, position).await?;
                }

                // Check for timeout
                _ = tokio::time::sleep(queue.timeout) => {
                    self.handle_timeout(call, queue).await?;
                    break;
                }

                // Handle hangup
                _ = call.wait_for_hangup() => {
                    queue.remove_call(call.id).await?;
                    break;
                }
            }
        }

        Ok(())
    }
}
```

### Conference Handler

```rust
// b2bua-core/src/handlers/conference.rs
pub struct ConferenceHandler {
    rooms: Arc<ConferenceRoomManager>,
    media_server: Arc<dyn MediaServerController>,
}

impl ConferenceHandler {
    async fn handle_conference_join(
        &self,
        call: &mut B2buaCall,
        room_id: &str,
    ) -> Result<()> {
        // Get or create room
        let room = self.rooms.get_or_create(room_id).await?;

        // Check PIN if required
        if let Some(pin) = &room.pin {
            let entered_pin = self.collect_pin(call).await?;
            if entered_pin != *pin {
                self.media_server.play_prompt(
                    call.media_endpoint(),
                    "invalid_pin.wav"
                ).await?;
                call.disconnect().await?;
                return Ok(());
            }
        }

        // Add to conference
        let conf_id = room.media_conference_id;
        self.media_server.add_to_conference(
            conf_id,
            call.media_endpoint()
        ).await?;

        // Announce join
        if room.announce_join {
            self.announce_participant_join(room, call).await?;
        }

        // Monitor for events
        loop {
            tokio::select! {
                // DTMF commands
                digit = self.media_server.wait_for_dtmf(call.media_endpoint()) => {
                    self.handle_conference_command(call, room, digit).await?;
                }

                // Participant leaves
                _ = call.wait_for_hangup() => {
                    self.media_server.remove_from_conference(
                        conf_id,
                        call.media_endpoint()
                    ).await?;

                    if room.announce_leave {
                        self.announce_participant_leave(room, call).await?;
                    }
                    break;
                }
            }
        }

        Ok(())
    }
}
```

## Integration Points

### 1. With dialog-core

```rust
impl B2buaCore {
    async fn create_dialog_handlers(&self) {
        // Handle incoming INVITE
        self.dialog_manager.on_invite({
            let core = self.clone();
            move |invite| core.handle_incoming_invite(invite)
        });

        // Handle BYE
        self.dialog_manager.on_bye({
            let core = self.clone();
            move |bye| core.handle_bye(bye)
        });

        // Handle REFER
        self.dialog_manager.on_refer({
            let core = self.clone();
            move |refer| core.handle_refer(refer)
        });
    }
}
```

### 2. With media-server-core

```rust
impl B2buaCore {
    async fn setup_media(&self, call: &mut B2buaCall) -> Result<()> {
        // Allocate media endpoint
        let endpoint = self.media_controller.allocate_endpoint().await?;

        // Update SDP to point to media server
        let media_sdp = endpoint.generate_sdp();
        call.set_local_sdp(media_sdp);

        // Store endpoint reference
        call.media_sessions.push(endpoint.id);

        Ok(())
    }
}
```

### 3. With infra-common

```rust
impl B2buaCore {
    async fn setup_infrastructure(&self) {
        // Use event bus
        self.event_bus.subscribe("*", {
            let core = self.clone();
            move |event| core.handle_event(event)
        });

        // Use configuration
        let config = self.config_manager.get("b2bua").await?;
        self.apply_config(config).await?;

        // Use metrics
        self.metrics.register_counter("b2bua.calls.total");
        self.metrics.register_gauge("b2bua.calls.active");
    }
}
```

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_basic_bridge() {
        let b2bua = B2buaCore::new_test();

        // Simulate incoming call
        let invite = create_test_invite();
        let call = b2bua.handle_incoming_invite(invite).await.unwrap();

        // Bridge to another leg
        b2bua.bridge_to("sip:bob@example.com").await.unwrap();

        // Verify state
        assert_eq!(call.state(), B2buaState::Bridged);
        assert!(call.leg_b().is_some());
    }

    #[tokio::test]
    async fn test_ivr_flow() {
        let b2bua = B2buaCore::new_test();
        let ivr = IvrHandler::new_test();

        // Create simple flow
        let flow = IvrFlow::builder()
            .add_prompt("welcome.wav")
            .add_menu("menu.wav", vec![
                ('1', "sales"),
                ('2', "support"),
            ])
            .build();

        // Execute flow
        let result = ivr.execute_flow(call, &flow).await.unwrap();

        // Verify navigation
        assert_eq!(result, "sales");
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_queue_to_agent_flow() {
    let b2bua = create_test_b2bua().await;

    // Setup queue
    let queue = Queue::new("support");

    // Add available agent
    let agent = Agent::new("alice");
    agent.set_available().await;

    // Incoming call
    let call = b2bua.handle_incoming_call(test_invite()).await?;

    // Add to queue
    queue.add_call(call).await?;

    // Should bridge to agent
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(call.state(), B2buaState::Bridged);
}
```

## Migration Strategy

### From Existing session-core-v2 Based Design

1. **Remove B2BUA code from session-core-v2**
   - Remove B2BUA-specific hooks
   - Remove bridge management
   - Keep only endpoint functionality

2. **Create new b2bua-core**
   - Start with basic bridging
   - Add IVR handler
   - Add queue handler
   - Add conference handler

3. **Update existing B2BUA applications**
   - Change from session-core-v2 to b2bua-core
   - Update state management
   - Update media handling

## Timeline

### Week 1-2: Core Foundation
- Basic b2bua-core structure
- Dialog pair management
- State engine
- Event system

### Week 3-4: Media Integration
- Media server client interface
- Endpoint management
- Basic bridging

### Week 5-6: Application Handlers
- IVR handler
- Queue handler
- Basic transfer/hold

### Week 7-8: Advanced Features
- Conference handler
- Recording
- Advanced transfer scenarios

### Week 9-10: Testing & Documentation
- Comprehensive tests
- Performance testing
- Documentation
- Example applications

## Success Metrics

1. **Functionality**
   - All B2BUA patterns supported
   - Clean integration with media servers
   - No dependency on session-core-v2

2. **Performance**
   - < 10ms call setup time
   - Support 10,000+ concurrent calls
   - < 100MB memory per 1000 calls

3. **Maintainability**
   - Clear separation from session-core-v2
   - Well-defined interfaces
   - Comprehensive test coverage (> 80%)

4. **Developer Experience**
   - Simple API for common patterns
   - Extensible for custom handlers
   - Good documentation and examples