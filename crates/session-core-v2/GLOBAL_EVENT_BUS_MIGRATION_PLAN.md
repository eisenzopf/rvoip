# Global Event Bus Migration Plan

## Overview

This plan details the migration of dialog-core, media-core, and session-core from tokio channels to the infra-common global event bus using the cross-crate event definitions.

## Current State

All three crates currently use tokio `mpsc::channel` for inter-crate communication:
- **dialog-core**: Has `DialogEventAdapter` but doesn't use it
- **media-core**: Has `MediaEventAdapter` but doesn't use it  
- **session-core**: Uses channels exclusively, has unused event adapter code

## Target Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     GlobalEventCoordinator                       │
│                    (infra-common event bus)                      │
└──────────────────┬────────────────┬────────────────┬────────────┘
                   │                │                │
                   │                │                │
         ┌─────────▼────────┐ ┌────▼────────┐ ┌────▼────────┐
         │  DialogEventHub  │ │ MediaEventHub│ │SessionEventHub│
         │  (dialog-core)   │ │ (media-core) │ │(session-core) │
         └──────────────────┘ └──────────────┘ └──────────────┘
```

## Migration Steps

### Phase 1: Update Event Adapters

#### 1.1 Dialog-Core Changes

**File: `crates/dialog-core/src/events/adapter.rs`**

Currently exists but unused. Need to:

1. **Rename to `event_hub.rs`** for clarity
2. **Remove channel-based backward compatibility**
3. **Implement proper event conversion and publishing**

```rust
// crates/dialog-core/src/events/event_hub.rs
use infra_common::events::cross_crate::{
    RvoipCrossCrateEvent, DialogToSessionEvent, SessionToDialogEvent,
    TransportToDialogEvent, DialogToTransportEvent
};

pub struct DialogEventHub {
    global_coordinator: Arc<GlobalEventCoordinator>,
    dialog_manager: Arc<DialogManager>,
}

impl DialogEventHub {
    pub async fn new(
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_manager: Arc<DialogManager>,
    ) -> Result<Arc<Self>> {
        let hub = Arc::new(Self {
            global_coordinator,
            dialog_manager,
        });
        
        // Register as handler for session-to-dialog events
        global_coordinator
            .register_handler("session_to_dialog", hub.clone())
            .await?;
            
        // Register as handler for transport-to-dialog events
        global_coordinator
            .register_handler("transport_to_dialog", hub.clone())
            .await?;
            
        Ok(hub)
    }
    
    /// Publish dialog events to the global bus
    pub async fn publish_dialog_event(&self, event: DialogEvent) -> Result<()> {
        let cross_crate_event = self.convert_to_cross_crate(event)?;
        self.global_coordinator.publish(Arc::new(cross_crate_event)).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for DialogEventHub {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        // Handle incoming events from session-core or transport
        // Route to dialog manager methods
    }
}
```

**File: `crates/dialog-core/src/manager/core.rs`**

Replace channel usage:

```rust
// REMOVE:
// session_coordinator: Arc<tokio::sync::RwLock<Option<mpsc::Sender<SessionCoordinationEvent>>>>,
// dialog_event_sender: Arc<tokio::sync::RwLock<Option<mpsc::Sender<DialogEvent>>>>,

// ADD:
event_hub: Arc<DialogEventHub>,

// In emit_dialog_event():
pub async fn emit_dialog_event(&self, event: DialogEvent) {
    if let Err(e) = self.event_hub.publish_dialog_event(event).await {
        warn!("Failed to publish dialog event: {}", e);
    }
}

// In emit_session_coordination_event():
pub async fn emit_session_coordination_event(&self, event: SessionCoordinationEvent) {
    // Convert to DialogToSessionEvent
    let cross_crate_event = match event {
        SessionCoordinationEvent::IncomingCall { .. } => {
            RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::IncomingCall { .. })
        }
        // ... other conversions
    };
    
    if let Err(e) = self.event_hub.global_coordinator.publish(Arc::new(cross_crate_event)).await {
        warn!("Failed to publish session coordination event: {}", e);
    }
}
```

#### 1.2 Media-Core Changes

**File: `crates/media-core/src/events/event_hub.rs`** (rename from adapter.rs)

```rust
use infra_common::events::cross_crate::{
    RvoipCrossCrateEvent, MediaToSessionEvent, SessionToMediaEvent,
    MediaToRtpEvent, RtpToMediaEvent
};

pub struct MediaEventHub {
    global_coordinator: Arc<GlobalEventCoordinator>,
    controller: Arc<MediaSessionController>,
}

impl MediaEventHub {
    pub async fn new(
        global_coordinator: Arc<GlobalEventCoordinator>,
        controller: Arc<MediaSessionController>,
    ) -> Result<Arc<Self>> {
        let hub = Arc::new(Self {
            global_coordinator,
            controller,
        });
        
        // Register handlers
        global_coordinator
            .register_handler("session_to_media", hub.clone())
            .await?;
            
        global_coordinator
            .register_handler("rtp_to_media", hub.clone())
            .await?;
            
        Ok(hub)
    }
    
    pub async fn publish_media_event(&self, event: MediaSessionEvent) -> Result<()> {
        let cross_crate_event = match event {
            MediaSessionEvent::SessionCreated { dialog_id, .. } => {
                RvoipCrossCrateEvent::MediaToSession(MediaToSessionEvent::MediaStreamStarted {
                    session_id: self.get_session_id(&dialog_id),
                    local_port: self.get_local_port(&dialog_id),
                    codec: "PCMU".to_string(),
                })
            }
            // ... other conversions
        };
        
        self.global_coordinator.publish(Arc::new(cross_crate_event)).await?;
        Ok(())
    }
}
```

**File: `crates/media-core/src/relay/controller/mod.rs`**

Replace channel-based event distribution:

```rust
// REMOVE:
// event_tx: Option<mpsc::Sender<MediaSessionEvent>>,

// ADD:
event_hub: Arc<MediaEventHub>,

// In methods that emit events:
async fn emit_event(&self, event: MediaSessionEvent) -> Result<()> {
    self.event_hub.publish_media_event(event).await
}
```

#### 1.3 Session-Core Changes

**File: `crates/session-core/src/events/event_hub.rs`**

```rust
use infra_common::events::cross_crate::{
    RvoipCrossCrateEvent, SessionToDialogEvent, DialogToSessionEvent,
    SessionToMediaEvent, MediaToSessionEvent
};

pub struct SessionEventHub {
    global_coordinator: Arc<GlobalEventCoordinator>,
    session_manager: Arc<SessionManager>,
}

impl SessionEventHub {
    pub async fn new(
        global_coordinator: Arc<GlobalEventCoordinator>,
        session_manager: Arc<SessionManager>,
    ) -> Result<Arc<Self>> {
        let hub = Arc::new(Self {
            global_coordinator,
            session_manager,
        });
        
        // Register handlers for incoming events
        global_coordinator
            .register_handler("dialog_to_session", hub.clone())
            .await?;
            
        global_coordinator
            .register_handler("media_to_session", hub.clone())
            .await?;
            
        Ok(hub)
    }
    
    /// Publish session events to dialog-core
    pub async fn send_to_dialog(&self, event: SessionToDialogEvent) -> Result<()> {
        let cross_crate_event = RvoipCrossCrateEvent::SessionToDialog(event);
        self.global_coordinator.publish(Arc::new(cross_crate_event)).await
    }
    
    /// Publish session events to media-core
    pub async fn send_to_media(&self, event: SessionToMediaEvent) -> Result<()> {
        let cross_crate_event = RvoipCrossCrateEvent::SessionToMedia(event);
        self.global_coordinator.publish(Arc::new(cross_crate_event)).await
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for SessionEventHub {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        // Downcast and handle events
        match event.event_type() {
            "dialog_to_session" => {
                // Handle dialog events
            }
            "media_to_session" => {
                // Handle media events
            }
            _ => {}
        }
        Ok(())
    }
}
```

### Phase 2: Remove Channel-Based Communication

#### 2.1 Dialog-Core Channel Removal

**Files to modify:**
- `crates/dialog-core/src/manager/core.rs`
- `crates/dialog-core/src/api/unified.rs`
- `crates/dialog-core/src/dialog/coordinator.rs`

Remove:
```rust
// All instances of:
mpsc::channel(100)
mpsc::Sender<SessionCoordinationEvent>
mpsc::Receiver<SessionCoordinationEvent>
mpsc::Sender<DialogEvent>
mpsc::Receiver<DialogEvent>
set_session_coordinator()
set_dialog_event_sender()
```

Replace with event hub calls:
```rust
// Instead of:
if let Some(sender) = self.session_coordinator.read().await.as_ref() {
    sender.send(event).await?;
}

// Use:
self.event_hub.publish_dialog_event(event).await?;
```

#### 2.2 Media-Core Channel Removal

**Files to modify:**
- `crates/media-core/src/relay/controller/mod.rs`
- `crates/media-core/src/engine/media_engine.rs`

Remove:
```rust
// All instances of:
mpsc::channel for media events
take_event_receiver()
event_tx.send()
```

#### 2.3 Session-Core Channel Removal

**Files to modify:**
- `crates/session-core/src/manager/core.rs`
- `crates/session-core/src/coordinator/*.rs`
- `crates/session-core/src/dialog/coordinator.rs`
- `crates/session-core/src/media/bridge.rs`

Remove all channel creation and replace with event hub methods.

### Phase 3: Update Initialization

#### 3.1 Dialog-Core Initialization

```rust
// In UnifiedDialogApi::new()
pub async fn new(
    transaction_manager: Arc<TransactionManager>,
    config: DialogManagerConfig,
) -> Result<Arc<Self>> {
    // Create global coordinator
    let global_coordinator = Arc::new(
        GlobalEventCoordinator::monolithic()
            .await
            .map_err(|e| DialogError::Internal(format!("Failed to create coordinator: {}", e)))?
    );
    
    // Create dialog manager
    let dialog_manager = Arc::new(DialogManager::new(...));
    
    // Create and start event hub
    let event_hub = DialogEventHub::new(global_coordinator, dialog_manager.clone()).await?;
    dialog_manager.set_event_hub(event_hub);
    
    // ...
}
```

#### 3.2 Media-Core Initialization

```rust
// In MediaSessionController::new()
pub fn new() -> Self {
    // ...
}

pub async fn with_event_hub(mut self, global_coordinator: Arc<GlobalEventCoordinator>) -> Result<Self> {
    let event_hub = MediaEventHub::new(global_coordinator, Arc::new(self.clone())).await?;
    self.event_hub = Some(event_hub);
    Ok(self)
}
```

#### 3.3 Session-Core Initialization

```rust
// In SessionManager::new()
pub async fn new(config: SessionConfig) -> Result<Arc<Self>> {
    // Create global coordinator
    let global_coordinator = Arc::new(GlobalEventCoordinator::monolithic().await?);
    
    // Create dialog and media APIs
    let dialog_api = Self::create_dialog_api(&config, global_coordinator.clone()).await?;
    let media_controller = Self::create_media_controller(&config, global_coordinator.clone()).await?;
    
    // Create session manager
    let session_manager = Arc::new(SessionManager { ... });
    
    // Create event hub
    let event_hub = SessionEventHub::new(global_coordinator, session_manager.clone()).await?;
    session_manager.set_event_hub(event_hub);
    
    Ok(session_manager)
}
```

### Phase 4: Testing Strategy

1. **Unit Tests**: Test event conversion and publishing
2. **Integration Tests**: Test cross-crate event flow
3. **Migration Tests**: Run old and new systems in parallel to verify behavior

### Phase 5: Gradual Rollout

1. **Feature Flag**: Add feature flag to switch between channel and event bus
2. **Parallel Run**: Run both systems side-by-side initially
3. **Monitoring**: Add metrics for event flow
4. **Cutover**: Switch to event bus exclusively
5. **Cleanup**: Remove channel-based code

## Benefits

1. **Centralized Event Flow**: All events go through GlobalEventCoordinator
2. **Better Debugging**: Single point to monitor all cross-crate communication
3. **Loose Coupling**: Crates don't need direct references to each other
4. **Scalability**: Ready for distributed deployment
5. **Type Safety**: Using defined cross-crate event types

## Implementation Timeline

- **Week 1**: Update event adapters/hubs
- **Week 2**: Remove channels from dialog-core
- **Week 3**: Remove channels from media-core
- **Week 4**: Remove channels from session-core
- **Week 5**: Integration testing
- **Week 6**: Rollout and monitoring

## Key Files to Modify

### Dialog-Core
- `/src/events/event_hub.rs` (new)
- `/src/manager/core.rs`
- `/src/api/unified.rs`
- `/src/manager/transaction_integration.rs`
- `/src/protocol/response_handler.rs`

### Media-Core
- `/src/events/event_hub.rs` (new)
- `/src/relay/controller/mod.rs`
- `/src/engine/media_engine.rs`
- `/src/session/media_session.rs`

### Session-Core
- `/src/events/event_hub.rs` (new)
- `/src/manager/core.rs`
- `/src/coordinator/event_handler.rs`
- `/src/dialog/coordinator.rs`
- `/src/media/bridge.rs`

## Event Audit Checklist

### Dialog-Core Events

#### Incoming Events (Dialog-Core Receives)

**From Session-Core (SessionToDialogEvent):**
- [ ] `InitiateCall` - Start a new outgoing call
  - Fields: session_id, from, to, sdp_offer, headers
- [ ] `TerminateSession` - End an active call
  - Fields: session_id, reason
- [ ] `HoldSession` - Put call on hold
  - Fields: session_id
- [ ] `ResumeSession` - Resume from hold
  - Fields: session_id, sdp_offer
- [ ] `TransferCall` - Transfer (blind/attended)
  - Fields: session_id, target, transfer_type
- [ ] `SendDtmf` - Send DTMF tones
  - Fields: session_id, tones

**From Transport (TransportToDialogEvent):**
- [ ] `SipMessageReceived` - Incoming SIP request
  - Fields: source, method, headers, body, transaction_id
- [ ] `SipResponseReceived` - Incoming SIP response
  - Fields: transaction_id, status_code, reason_phrase, headers, body
- [ ] `TransportError` - Transport layer error
  - Fields: error, transaction_id
- [ ] `RegistrationStatusUpdate` - Registration status change
  - Fields: uri, status, expires

#### Outgoing Events (Dialog-Core Sends)

**To Session-Core (DialogToSessionEvent):**
- [ ] `IncomingCall` - New incoming call
  - Fields: session_id, call_id, from, to, sdp_offer, headers
- [ ] `CallStateChanged` - Call state transition
  - Fields: session_id, new_state, reason
- [ ] `CallEstablished` - Call connected (200 OK)
  - Fields: session_id, sdp_answer
- [ ] `CallTerminated` - Call ended
  - Fields: session_id, reason
- [ ] `DtmfReceived` - Received DTMF tones
  - Fields: session_id, tones
- [ ] `DialogError` - Dialog-level error
  - Fields: session_id, error, error_code

**To Transport (DialogToTransportEvent):**
- [ ] `SendSipMessage` - Send SIP request
  - Fields: destination, method, headers, body, transaction_id
- [ ] `SendSipResponse` - Send SIP response
  - Fields: transaction_id, status_code, reason_phrase, headers, body
- [ ] `RegisterEndpoint` - Register SIP endpoint
  - Fields: uri, expires, contact
- [ ] `UnregisterEndpoint` - Unregister endpoint
  - Fields: uri

**Current Implementation Events to Map:**
- [ ] `SessionCoordinationEvent::IncomingCall` → `DialogToSessionEvent::IncomingCall`
- [ ] `SessionCoordinationEvent::ResponseReceived` → `DialogToSessionEvent::CallStateChanged`
- [ ] `SessionCoordinationEvent::CallAnswered` → `DialogToSessionEvent::CallEstablished`
- [ ] `SessionCoordinationEvent::CallTerminating` → `DialogToSessionEvent::CallTerminated`
- [ ] `SessionCoordinationEvent::CallCancelled` → `DialogToSessionEvent::CallTerminated`
- [ ] `DialogEvent::Created` → (internal only, no cross-crate equivalent)
- [ ] `DialogEvent::StateChanged` → `DialogToSessionEvent::CallStateChanged`
- [ ] `DialogEvent::Terminated` → `DialogToSessionEvent::CallTerminated`

### Media-Core Events

#### Incoming Events (Media-Core Receives)

**From Session-Core (SessionToMediaEvent):**
- [ ] `StartMediaStream` - Initialize media session
  - Fields: session_id, local_sdp, remote_sdp, media_config
- [ ] `StopMediaStream` - Terminate media session
  - Fields: session_id
- [ ] `UpdateMediaStream` - Update media parameters
  - Fields: session_id, local_sdp, remote_sdp
- [ ] `HoldMedia` - Pause media flow
  - Fields: session_id
- [ ] `ResumeMedia` - Resume media flow
  - Fields: session_id
- [ ] `StartRecording` - Begin recording
  - Fields: session_id, file_path, format
- [ ] `StopRecording` - End recording
  - Fields: session_id
- [ ] `PlayAudio` - Play audio file
  - Fields: session_id, file_path, loop_count
- [ ] `StopAudio` - Stop audio playback
  - Fields: session_id

**From RTP-Core (RtpToMediaEvent):**
- [ ] `RtpStreamStarted` - RTP stream initialized
  - Fields: session_id, local_port
- [ ] `RtpStreamStopped` - RTP stream terminated
  - Fields: session_id, reason
- [ ] `RtpPacketReceived` - Incoming RTP packet
  - Fields: session_id, payload, timestamp, sequence_number, payload_type
- [ ] `RtpStatisticsUpdate` - RTP statistics
  - Fields: session_id, stats
- [ ] `RtpError` - RTP-level error
  - Fields: session_id, error

#### Outgoing Events (Media-Core Sends)

**To Session-Core (MediaToSessionEvent):**
- [ ] `MediaStreamStarted` - Media session ready
  - Fields: session_id, local_port, codec
- [ ] `MediaStreamStopped` - Media session ended
  - Fields: session_id, reason
- [ ] `MediaQualityUpdate` - Quality metrics update
  - Fields: session_id, quality_metrics
- [ ] `RecordingStarted` - Recording begun
  - Fields: session_id, file_path
- [ ] `RecordingStopped` - Recording ended
  - Fields: session_id, file_path, duration_ms
- [ ] `AudioPlaybackFinished` - Audio file completed
  - Fields: session_id
- [ ] `MediaError` - Media-level error
  - Fields: session_id, error, error_code

**To RTP-Core (MediaToRtpEvent):**
- [ ] `StartRtpStream` - Initialize RTP
  - Fields: session_id, local_port, remote_address, remote_port, payload_type, codec
- [ ] `StopRtpStream` - Terminate RTP
  - Fields: session_id
- [ ] `SendRtpPacket` - Outgoing RTP packet
  - Fields: session_id, payload, timestamp, sequence_number
- [ ] `UpdateRtpStream` - Update RTP parameters
  - Fields: session_id, remote_address, remote_port

**Current Implementation Events to Map:**
- [ ] `MediaSessionEvent::SessionCreated` → `MediaToSessionEvent::MediaStreamStarted`
- [ ] `MediaSessionEvent::SessionDestroyed` → `MediaToSessionEvent::MediaStreamStopped`
- [ ] `MediaSessionEvent::SessionFailed` → `MediaToSessionEvent::MediaError`
- [ ] `MediaSessionEvent::RemoteAddressUpdated` → (internal only)
- [ ] `IntegrationEventType::MediaSessionReady` → `MediaToSessionEvent::MediaStreamStarted`
- [ ] `IntegrationEventType::QualityUpdate` → `MediaToSessionEvent::MediaQualityUpdate`

### Session-Core Events

#### Incoming Events (Session-Core Receives)

**From Dialog-Core (DialogToSessionEvent):**
- [ ] `IncomingCall` - Handle incoming call
- [ ] `CallStateChanged` - Update session state
- [ ] `CallEstablished` - Complete call setup
- [ ] `CallTerminated` - Clean up terminated call
- [ ] `DtmfReceived` - Process DTMF input
- [ ] `DialogError` - Handle dialog errors

**From Media-Core (MediaToSessionEvent):**
- [ ] `MediaStreamStarted` - Media ready notification
- [ ] `MediaStreamStopped` - Media terminated notification
- [ ] `MediaQualityUpdate` - Update quality metrics
- [ ] `RecordingStarted` - Recording active notification
- [ ] `RecordingStopped` - Recording complete notification
- [ ] `AudioPlaybackFinished` - Playback complete notification
- [ ] `MediaError` - Handle media errors

#### Outgoing Events (Session-Core Sends)

**To Dialog-Core (SessionToDialogEvent):**
- [ ] `InitiateCall` - Start outgoing call
- [ ] `TerminateSession` - End call
- [ ] `HoldSession` - Put on hold
- [ ] `ResumeSession` - Resume from hold
- [ ] `TransferCall` - Transfer call
- [ ] `SendDtmf` - Send DTMF

**To Media-Core (SessionToMediaEvent):**
- [ ] `StartMediaStream` - Initialize media
- [ ] `StopMediaStream` - Terminate media
- [ ] `UpdateMediaStream` - Update media
- [ ] `HoldMedia` - Pause media
- [ ] `ResumeMedia` - Resume media
- [ ] `StartRecording` - Begin recording
- [ ] `StopRecording` - End recording
- [ ] `PlayAudio` - Play file
- [ ] `StopAudio` - Stop playback

**Current Implementation Events to Convert:**
- [ ] `SessionEvent::SessionCreated` → Internal + `SessionToDialogEvent::InitiateCall`
- [ ] `SessionEvent::IncomingCall` → Internal (received from dialog)
- [ ] `SessionEvent::StateChanged` → Internal state management
- [ ] `SessionEvent::SessionTerminating` → `SessionToDialogEvent::TerminateSession`
- [ ] `SessionEvent::SessionTerminated` → Cleanup complete
- [ ] `SessionEvent::DtmfReceived` → `SessionToDialogEvent::SendDtmf` (if outgoing)
- [ ] `SessionEvent::SessionHeld` → `SessionToDialogEvent::HoldSession`
- [ ] `SessionEvent::SessionResumed` → `SessionToDialogEvent::ResumeSession`
- [ ] `SessionEvent::MediaUpdate` → `SessionToMediaEvent::UpdateMediaStream`

## Event Flow Verification

### Critical Event Flows to Test

1. **Incoming Call Flow:**
   - Transport → Dialog: `SipMessageReceived` (INVITE)
   - Dialog → Session: `IncomingCall`
   - Session → Media: `StartMediaStream`
   - Media → Session: `MediaStreamStarted`
   - Session → Dialog: (accept decision)
   - Dialog → Transport: `SendSipResponse` (200 OK)

2. **Outgoing Call Flow:**
   - Session → Dialog: `InitiateCall`
   - Dialog → Transport: `SendSipMessage` (INVITE)
   - Transport → Dialog: `SipResponseReceived` (200 OK)
   - Dialog → Session: `CallEstablished`
   - Session → Media: `StartMediaStream`
   - Media → Session: `MediaStreamStarted`

3. **Call Termination Flow:**
   - Session → Dialog: `TerminateSession`
   - Dialog → Transport: `SendSipMessage` (BYE)
   - Session → Media: `StopMediaStream`
   - Media → Session: `MediaStreamStopped`
   - Dialog → Session: `CallTerminated`

4. **Hold/Resume Flow:**
   - Session → Dialog: `HoldSession`
   - Session → Media: `HoldMedia`
   - Dialog → Transport: `SendSipMessage` (re-INVITE)
   - Session → Dialog: `ResumeSession`
   - Session → Media: `ResumeMedia`

## Success Criteria

1. No tokio channels used for cross-crate communication
2. All events flow through GlobalEventCoordinator
3. All tests pass with new architecture
4. Performance metrics remain acceptable
5. Event tracing shows proper flow
6. **All events in the audit checklist are implemented**
7. **All current events are properly mapped to cross-crate events**

## Risks and Mitigations

1. **Risk**: Breaking existing functionality
   - **Mitigation**: Feature flag and parallel run

2. **Risk**: Performance degradation
   - **Mitigation**: Benchmark before/after, optimize hot paths

3. **Risk**: Complex debugging
   - **Mitigation**: Add comprehensive event tracing

4. **Risk**: Type conversion errors
   - **Mitigation**: Extensive unit tests for conversions
