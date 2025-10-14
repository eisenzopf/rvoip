# SimplePeer Builder Pattern & Default Handlers Plan

## Executive Summary

This document defines a comprehensive builder pattern for SimplePeer that solves the event handling problem by:
1. **Background event processing** - Prevents buffer overflow and blocking
2. **Configurable default handlers** - Automatic event handling based on policies
3. **Explicit user control** - Opt-in behaviors with safe defaults
4. **Event routing** - Users can override defaults for specific events

---

## Problem Statement

Current SimplePeer API has critical flaws:
- ❌ Events can be missed if not actively polled
- ❌ Blocking operations (like `exchange_audio`) prevent event processing
- ❌ No way to handle asynchronous events (like REFER) without explicit waiting
- ❌ Buffer can overflow (1000 events max)
- ❌ Users forced to handle all events manually or lose them

---

## Solution Architecture

### Core Components

```
┌─────────────────────────────────────────────────────────────┐
│                        SimplePeer                            │
│                                                              │
│  ┌────────────────────────────────────────────────────┐    │
│  │     Background Event Processor (tokio task)        │    │
│  │                                                     │    │
│  │  1. Receives all events from coordinator           │    │
│  │  2. Routes to user handlers (if registered)        │    │
│  │  3. Falls back to default handlers (per policy)    │    │
│  │  4. Never blocks - auto-drains if needed           │    │
│  └────────────────────────────────────────────────────┘    │
│                                                              │
│  ┌────────────────────────────────────────────────────┐    │
│  │         User Event Channels (optional)             │    │
│  │  - Filtered by event type                          │    │
│  │  - Non-blocking sends                              │    │
│  │  - User polls at their own pace                    │    │
│  └────────────────────────────────────────────────────┘    │
│                                                              │
│  ┌────────────────────────────────────────────────────┐    │
│  │            Default Handlers                        │    │
│  │  - Configured via policies                         │    │
│  │  - Execute automatically                           │    │
│  │  - Can be overridden per event type                │    │
│  └────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

---

## Builder Pattern API

### SimplePeerBuilder Structure

```rust
pub struct SimplePeerBuilder {
    // Basic configuration
    name: String,
    config: Config,
    
    // Event handling policies
    call_policy: IncomingCallPolicy,
    transfer_policy: TransferPolicy,
    dtmf_policy: DtmfPolicy,
    hold_policy: HoldPolicy,
    
    // Error/failure handling
    call_failure_policy: CallFailurePolicy,
    media_error_policy: MediaErrorPolicy,
    network_error_policy: NetworkErrorPolicy,
    
    // Event routing
    unhandled_event_policy: UnhandledEventPolicy,
    event_buffer_policy: BufferOverflowPolicy,
    
    // Logging and monitoring
    log_level: EventLogLevel,
    event_history_size: Option<usize>,
    
    // Advanced options
    background_event_processor: bool,
    auto_drain_events: bool,
}
```

---

## Policy Enums

### 1. IncomingCallPolicy

Controls how incoming calls are handled automatically.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncomingCallPolicy {
    /// Reject all incoming calls automatically with 603 Decline
    /// Use case: Outbound-only applications (telemarketers, monitoring)
    RejectAll,
    
    /// Reject incoming calls with busy signal (486 Busy Here)
    /// Use case: When peer is already on a call
    RejectWhenBusy,
    
    /// Accept all incoming calls automatically
    /// Use case: Auto-answering systems, voicemail, IVR
    /// ⚠️ WARNING: Security risk - answers all calls
    AcceptAll,
    
    /// Accept calls from whitelist only, reject others
    /// Use case: Enterprise systems with allowed caller list
    AcceptWhitelist {
        allowed_domains: Vec<String>,
        allowed_users: Vec<String>,
    },
    
    /// Require manual handling - error if no user handler registered
    /// Use case: Applications with custom call screening logic
    /// ✅ SAFE DEFAULT
    RequireManual,
    
    /// Queue for manual handling with timeout
    /// If not handled within timeout, auto-reject
    /// Use case: Interactive applications with UI
    QueueWithTimeout {
        timeout_secs: u32,
        on_timeout: TimeoutAction,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutAction {
    Reject,
    Accept,
    SendToVoicemail,
}
```

**Default:** `IncomingCallPolicy::RequireManual`

**Examples:**
```rust
// Call center (accept all)
.incoming_call_policy(IncomingCallPolicy::AcceptAll)

// Enterprise (whitelist)
.incoming_call_policy(IncomingCallPolicy::AcceptWhitelist {
    allowed_domains: vec!["company.com".to_string()],
    allowed_users: vec!["boss@external.com".to_string()],
})

// Interactive app (queue with timeout)
.incoming_call_policy(IncomingCallPolicy::QueueWithTimeout {
    timeout_secs: 30,
    on_timeout: TimeoutAction::Reject,
})
```

---

### 2. TransferPolicy

Controls how call transfer requests (REFER) are handled.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPolicy {
    /// Reject all transfer requests with 603 Decline
    /// Use case: Security-critical applications, prevent call hijacking
    /// ✅ SAFE DEFAULT
    RejectAll,
    
    /// Reject with specific SIP response code
    RejectWithCode {
        status_code: u16,
        reason: &'static str,
    },
    
    /// Accept blind transfers automatically and complete them
    /// Terminates current call, calls transfer target
    /// Use case: Automated call routing, simple forwarding
    AcceptBlind,
    
    /// Accept attended transfers automatically
    /// Waits for transferor to complete consultation
    /// Use case: Warm transfer scenarios
    AcceptAttended,
    
    /// Accept all transfer types
    /// Use case: Full-featured softphone
    AcceptAll,
    
    /// Accept transfers only from trusted domains
    AcceptTrusted {
        trusted_domains: Vec<String>,
    },
    
    /// Require manual handling - error if no user handler
    /// Use case: Applications with custom transfer logic
    RequireManual,
    
    /// Queue for manual approval with timeout
    QueueWithTimeout {
        timeout_secs: u32,
        on_timeout: TimeoutAction,
    },
    
    /// Log and ignore (accept REFER, send NOTIFY, but don't transfer)
    /// Use case: Testing, debugging
    LogAndIgnore,
}
```

**Default:** `TransferPolicy::RejectAll`

**Examples:**
```rust
// Secure application
.transfer_policy(TransferPolicy::RejectAll)

// Call center agent
.transfer_policy(TransferPolicy::AcceptAll)

// Enterprise with trusted domains
.transfer_policy(TransferPolicy::AcceptTrusted {
    trusted_domains: vec!["company.com".to_string(), "partner.com".to_string()],
})

// Interactive with approval
.transfer_policy(TransferPolicy::QueueWithTimeout {
    timeout_secs: 15,
    on_timeout: TimeoutAction::Reject,
})
```

---

### 3. DtmfPolicy

Controls how DTMF (touch-tone) digits are handled.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtmfPolicy {
    /// Ignore all DTMF digits (drop silently)
    /// Use case: Voice-only applications
    Ignore,
    
    /// Log DTMF digits but don't process
    /// Use case: Debugging, monitoring
    LogOnly,
    
    /// Buffer DTMF digits for later retrieval
    /// Use case: IVR systems that collect input
    Buffer {
        max_buffer_size: usize,
        clear_on_read: bool,
    },
    
    /// Forward to registered handler immediately
    /// Use case: Real-time DTMF processing
    ForwardImmediate,
    
    /// Execute commands based on digit pattern
    /// Use case: In-call commands (*1 = hold, *2 = transfer)
    ExecuteCommands {
        commands: Vec<DtmfCommand>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DtmfCommand {
    pub pattern: String,        // e.g., "*1", "#", "123#"
    pub action: DtmfAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtmfAction {
    Hold,
    Resume,
    Transfer,
    Mute,
    Unmute,
    Hangup,
    Custom(&'static str),
}
```

**Default:** `DtmfPolicy::ForwardImmediate`

**Examples:**
```rust
// IVR system
.dtmf_policy(DtmfPolicy::Buffer {
    max_buffer_size: 20,
    clear_on_read: false,
})

// Softphone with in-call commands
.dtmf_policy(DtmfPolicy::ExecuteCommands {
    commands: vec![
        DtmfCommand { pattern: "*1".to_string(), action: DtmfAction::Hold },
        DtmfCommand { pattern: "*2".to_string(), action: DtmfAction::Resume },
        DtmfCommand { pattern: "*3".to_string(), action: DtmfAction::Hangup },
    ],
})
```

---

### 4. HoldPolicy

Controls how hold/resume requests are handled.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldPolicy {
    /// Reject hold requests
    /// Use case: Simple applications without hold support
    Reject,
    
    /// Accept hold automatically
    /// Use case: Standard softphone behavior
    Accept,
    
    /// Accept hold and play music/message
    /// Use case: Professional call handling
    AcceptWithMedia {
        media_source: HoldMediaSource,
    },
    
    /// Require manual handling
    RequireManual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldMediaSource {
    Silence,
    ToneBeep,
    MusicFile(&'static str),
    Message(&'static str),
}
```

**Default:** `HoldPolicy::Accept`

**Examples:**
```rust
// Standard softphone
.hold_policy(HoldPolicy::Accept)

// Call center with music
.hold_policy(HoldPolicy::AcceptWithMedia {
    media_source: HoldMediaSource::MusicFile("/var/hold-music.wav"),
})
```

---

### 5. CallFailurePolicy

Controls how call failures (4xx, 5xx, 6xx) are handled.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallFailurePolicy {
    /// Log and return error to user
    /// ✅ SAFE DEFAULT
    ReturnError,
    
    /// Retry with exponential backoff
    RetryWithBackoff {
        max_retries: u32,
        initial_delay_ms: u64,
        max_delay_ms: u64,
    },
    
    /// Try alternate targets
    TryAlternates {
        alternates: Vec<String>,
    },
    
    /// Ignore silently (dangerous)
    Ignore,
    
    /// Log only, continue execution
    LogAndContinue,
}
```

**Default:** `CallFailurePolicy::ReturnError`

**Examples:**
```rust
// Robust dialer with retries
.call_failure_policy(CallFailurePolicy::RetryWithBackoff {
    max_retries: 3,
    initial_delay_ms: 1000,
    max_delay_ms: 10000,
})

// Failover system
.call_failure_policy(CallFailurePolicy::TryAlternates {
    alternates: vec![
        "sip:backup1@server.com".to_string(),
        "sip:backup2@server.com".to_string(),
    ],
})
```

---

### 6. MediaErrorPolicy

Controls how media errors (RTP issues, codec problems) are handled.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaErrorPolicy {
    /// Terminate call immediately
    TerminateCall,
    
    /// Attempt recovery (re-INVITE)
    AttemptRecovery {
        max_attempts: u32,
    },
    
    /// Continue with degraded quality
    ContinueDegraded,
    
    /// Log and return error
    /// ✅ SAFE DEFAULT
    ReturnError,
    
    /// Ignore (dangerous - silent failures)
    Ignore,
}
```

**Default:** `MediaErrorPolicy::ReturnError`

---

### 7. NetworkErrorPolicy

Controls how network errors (connection loss, timeouts) are handled.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkErrorPolicy {
    /// Terminate all active calls
    TerminateAll,
    
    /// Retry connection
    RetryConnection {
        max_retries: u32,
        timeout_secs: u32,
    },
    
    /// Enter offline mode, queue operations
    EnterOfflineMode,
    
    /// Return error
    /// ✅ SAFE DEFAULT
    ReturnError,
}
```

**Default:** `NetworkErrorPolicy::ReturnError`

---

### 8. UnhandledEventPolicy

Controls what happens to events not explicitly handled by user or defaults.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnhandledEventPolicy {
    /// Drop event silently
    /// ⚠️ Can hide bugs
    DropSilent,
    
    /// Log at DEBUG level
    LogDebug,
    
    /// Log at WARN level
    /// ✅ SAFE DEFAULT
    LogWarn,
    
    /// Log at ERROR level
    LogError,
    
    /// Return error (strict mode)
    /// Use case: Development, testing
    ReturnError,
    
    /// Panic (very strict mode)
    /// Use case: Critical systems where unhandled events are bugs
    Panic,
    
    /// Store in history buffer for later inspection
    StoreInHistory,
}
```

**Default:** `UnhandledEventPolicy::LogWarn`

---

### 9. BufferOverflowPolicy

Controls what happens when event buffer is full.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferOverflowPolicy {
    /// Drop oldest events (FIFO)
    /// ✅ SAFE DEFAULT - ensures newest events processed
    DropOldest,
    
    /// Drop newest events (keep history)
    DropNewest,
    
    /// Block until buffer has space (dangerous - can deadlock)
    Block,
    
    /// Return error
    ReturnError,
    
    /// Panic
    Panic,
    
    /// Dynamically expand buffer
    /// ⚠️ Can consume unbounded memory
    ExpandBuffer {
        max_size: usize,
    },
}
```

**Default:** `BufferOverflowPolicy::DropOldest`

---

### 10. EventLogLevel

Controls verbosity of event logging.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventLogLevel {
    /// No event logging
    Off,
    
    /// Log only errors
    ErrorOnly,
    
    /// Log errors and warnings
    ErrorAndWarn,
    
    /// Log all important events (default)
    /// ✅ RECOMMENDED
    Info,
    
    /// Log everything including internal events
    Debug,
    
    /// Log absolutely everything
    Trace,
}
```

**Default:** `EventLogLevel::Info`

---

## Complete Builder Example

```rust
use rvoip_session_core_v3::api::simple::{
    SimplePeerBuilder,
    IncomingCallPolicy,
    TransferPolicy,
    DtmfPolicy,
    HoldPolicy,
    CallFailurePolicy,
    MediaErrorPolicy,
    NetworkErrorPolicy,
    UnhandledEventPolicy,
    BufferOverflowPolicy,
    EventLogLevel,
    Config,
};

// Example 1: Secure Enterprise Softphone
let peer = SimplePeerBuilder::new("alice")
    .config(Config {
        sip_port: 5060,
        local_ip: "10.0.0.100".parse()?,
        ..Default::default()
    })
    // Security: Manual control required
    .incoming_call_policy(IncomingCallPolicy::AcceptWhitelist {
        allowed_domains: vec!["company.com".to_string()],
        allowed_users: vec![],
    })
    .transfer_policy(TransferPolicy::AcceptTrusted {
        trusted_domains: vec!["company.com".to_string()],
    })
    // Features: Full softphone features
    .dtmf_policy(DtmfPolicy::ExecuteCommands {
        commands: vec![
            DtmfCommand { pattern: "*1".to_string(), action: DtmfAction::Hold },
            DtmfCommand { pattern: "*2".to_string(), action: DtmfAction::Resume },
        ],
    })
    .hold_policy(HoldPolicy::AcceptWithMedia {
        media_source: HoldMediaSource::MusicFile("/etc/hold-music.wav"),
    })
    // Reliability: Retry on failures
    .call_failure_policy(CallFailurePolicy::RetryWithBackoff {
        max_retries: 3,
        initial_delay_ms: 1000,
        max_delay_ms: 5000,
    })
    .media_error_policy(MediaErrorPolicy::AttemptRecovery {
        max_attempts: 2,
    })
    .network_error_policy(NetworkErrorPolicy::RetryConnection {
        max_retries: 5,
        timeout_secs: 10,
    })
    // Monitoring: Comprehensive logging
    .unhandled_event_policy(UnhandledEventPolicy::LogError)
    .buffer_overflow_policy(BufferOverflowPolicy::DropOldest)
    .event_log_level(EventLogLevel::Info)
    .event_history_size(Some(1000))
    // Execution: Background processing
    .enable_background_processor(true)
    .auto_drain_events(true)
    .build().await?;

// Example 2: Simple Outbound Dialer
let peer = SimplePeerBuilder::new("dialer")
    .incoming_call_policy(IncomingCallPolicy::RejectAll)
    .transfer_policy(TransferPolicy::RejectAll)
    .dtmf_policy(DtmfPolicy::Ignore)
    .build().await?;

// Example 3: Call Center Agent
let peer = SimplePeerBuilder::new("agent007")
    .incoming_call_policy(IncomingCallPolicy::AcceptAll)
    .transfer_policy(TransferPolicy::AcceptAll)
    .dtmf_policy(DtmfPolicy::ForwardImmediate)
    .hold_policy(HoldPolicy::AcceptWithMedia {
        media_source: HoldMediaSource::Message("Please hold..."),
    })
    .build().await?;

// Example 4: IVR System
let peer = SimplePeerBuilder::new("ivr")
    .incoming_call_policy(IncomingCallPolicy::AcceptAll)
    .transfer_policy(TransferPolicy::LogAndIgnore)
    .dtmf_policy(DtmfPolicy::Buffer {
        max_buffer_size: 50,
        clear_on_read: false,
    })
    .hold_policy(HoldPolicy::Reject)
    .build().await?;

// Example 5: Development/Testing (Strict)
let peer = SimplePeerBuilder::new("test")
    .incoming_call_policy(IncomingCallPolicy::RequireManual)
    .transfer_policy(TransferPolicy::RequireManual)
    .unhandled_event_policy(UnhandledEventPolicy::ReturnError)
    .buffer_overflow_policy(BufferOverflowPolicy::Panic)
    .event_log_level(EventLogLevel::Trace)
    .build().await?;
```

---

## Implementation Details

### Background Event Processor

```rust
struct EventProcessor {
    coordinator: Arc<UnifiedCoordinator>,
    policies: EventPolicies,
    user_handlers: HashMap<EventType, Box<dyn EventHandler>>,
    event_history: Option<RingBuffer<Event>>,
}

impl EventProcessor {
    async fn run(mut self, mut event_rx: mpsc::Receiver<Event>) {
        while let Some(event) = event_rx.recv().await {
            // Log event
            self.log_event(&event);
            
            // Store in history if enabled
            if let Some(ref mut history) = self.event_history {
                history.push(event.clone());
            }
            
            // Try user handler first
            if let Some(handler) = self.user_handlers.get(&event.event_type()) {
                if handler.handle(event.clone()).await.is_ok() {
                    continue; // User handled it
                }
            }
            
            // Fall back to default handler
            match event {
                Event::IncomingCall { .. } => {
                    self.handle_incoming_call_default(event).await;
                }
                Event::ReferReceived { .. } => {
                    self.handle_transfer_default(event).await;
                }
                Event::DtmfReceived { .. } => {
                    self.handle_dtmf_default(event).await;
                }
                _ => {
                    self.handle_unhandled_event(event).await;
                }
            }
        }
    }
    
    async fn handle_incoming_call_default(&self, event: Event) {
        match self.policies.call_policy {
            IncomingCallPolicy::RejectAll => {
                // Send 603 Decline
                self.coordinator.reject_call(&event.call_id(), "Declined").await;
            }
            IncomingCallPolicy::AcceptAll => {
                // Auto-accept
                self.coordinator.accept_call(&event.call_id()).await;
            }
            IncomingCallPolicy::RequireManual => {
                // Error - no handler registered
                error!("Incoming call but no handler registered (policy: RequireManual)");
            }
            // ... other policies
        }
    }
    
    async fn handle_transfer_default(&self, event: Event) {
        match self.policies.transfer_policy {
            TransferPolicy::RejectAll => {
                // Send 603 Decline
                warn!("Transfer request rejected (policy: RejectAll)");
            }
            TransferPolicy::AcceptBlind => {
                // Auto-complete transfer
                if let Event::ReferReceived { refer_to, .. } = event {
                    info!("Auto-accepting blind transfer to {}", refer_to);
                    // Execute transfer in background
                }
            }
            TransferPolicy::RequireManual => {
                error!("Transfer request but no handler (policy: RequireManual)");
            }
            // ... other policies
        }
    }
}
```

### User Override API

```rust
impl SimplePeer {
    /// Register a custom handler for a specific event type
    pub fn on_incoming_call<F>(&mut self, handler: F) -> &mut Self
    where F: Fn(IncomingCallInfo) -> BoxFuture<'static, Result<CallDecision>> + Send + Sync + 'static
    {
        self.event_processor.register_handler(
            EventType::IncomingCall,
            Box::new(handler)
        );
        self
    }
    
    pub fn on_transfer<F>(&mut self, handler: F) -> &mut Self
    where F: Fn(ReferRequest) -> BoxFuture<'static, Result<TransferDecision>> + Send + Sync + 'static
    {
        self.event_processor.register_handler(
            EventType::ReferReceived,
            Box::new(handler)
        );
        self
    }
    
    pub fn on_dtmf<F>(&mut self, handler: F) -> &mut Self
    where F: Fn(char, CallId) -> BoxFuture<'static, Result<()>> + Send + Sync + 'static
    {
        self.event_processor.register_handler(
            EventType::DtmfReceived,
            Box::new(handler)
        );
        self
    }
}

// Usage:
let mut peer = SimplePeerBuilder::new("alice")
    .transfer_policy(TransferPolicy::RequireManual) // Override default
    .build().await?;

// Register custom handler (overrides policy for this event type)
peer.on_transfer(|refer| Box::pin(async move {
    if refer.refer_to.contains("trusted.com") {
        Ok(TransferDecision::Accept)
    } else {
        Ok(TransferDecision::Reject)
    }
}));
```

---

## Migration Path

### Current API (Broken)
```rust
let mut alice = SimplePeer::new("alice").await?;
alice.call("sip:bob@...").await?;
// Events get lost during exchange_audio!
alice.exchange_audio(...).await?;
```

### New API (Fixed) - Option 1: Automatic
```rust
let peer = SimplePeerBuilder::new("alice")
    .transfer_policy(TransferPolicy::AcceptBlind) // Auto-handle
    .build().await?;

peer.call("sip:bob@...").await?;
// Transfers handled automatically in background
peer.exchange_audio(...).await?;
```

### New API (Fixed) - Option 2: Manual
```rust
let mut peer = SimplePeerBuilder::new("alice")
    .transfer_policy(TransferPolicy::RequireManual)
    .build().await?;

peer.on_transfer(|refer| Box::pin(async move {
    // Custom logic
    Ok(TransferDecision::Accept)
}));

peer.call("sip:bob@...").await?;
// Transfers handled by registered handler
peer.exchange_audio(...).await?;
```

---

## Default Configuration

```rust
impl Default for SimplePeerBuilder {
    fn default() -> Self {
        Self {
            name: "peer".to_string(),
            config: Config::default(),
            
            // Safe defaults - require explicit handling
            call_policy: IncomingCallPolicy::RequireManual,
            transfer_policy: TransferPolicy::RejectAll,
            dtmf_policy: DtmfPolicy::ForwardImmediate,
            hold_policy: HoldPolicy::Accept,
            
            // Sensible error handling
            call_failure_policy: CallFailurePolicy::ReturnError,
            media_error_policy: MediaErrorPolicy::ReturnError,
            network_error_policy: NetworkErrorPolicy::ReturnError,
            
            // Logging and monitoring
            unhandled_event_policy: UnhandledEventPolicy::LogWarn,
            buffer_overflow_policy: BufferOverflowPolicy::DropOldest,
            log_level: EventLogLevel::Info,
            event_history_size: None,
            
            // Background processing enabled by default
            background_event_processor: true,
            auto_drain_events: true,
        }
    }
}
```

---

## Benefits

### 1. **No More Lost Events**
- Background processor ensures all events are handled
- Buffer overflow handled gracefully
- No blocking during long operations

### 2. **Explicit Behavior**
- Developer chooses policies upfront
- No surprises or magic behavior
- Clear documentation of what happens

### 3. **Safe Defaults**
- Reject dangerous operations (transfers, auto-accept)
- Require manual handling for security-critical events
- Log unhandled events for debugging

### 4. **Flexible**
- Start with defaults
- Override per event type
- Mix automatic and manual handling

### 5. **Production Ready**
- Error recovery policies
- Network resilience
- Resource management

---

## Testing Strategy

### Unit Tests
```rust
#[tokio::test]
async fn test_transfer_policy_reject() {
    let peer = SimplePeerBuilder::new("test")
        .transfer_policy(TransferPolicy::RejectAll)
        .build().await.unwrap();
    
    // Simulate REFER
    // Verify 603 Decline sent
}

#[tokio::test]
async fn test_transfer_policy_accept_blind() {
    let peer = SimplePeerBuilder::new("test")
        .transfer_policy(TransferPolicy::AcceptBlind)
        .build().await.unwrap();
    
    // Simulate REFER
    // Verify transfer completed
}
```

### Integration Tests
```rust
#[tokio::test]
async fn test_background_processor_prevents_buffer_overflow() {
    let peer = SimplePeerBuilder::new("test")
        .buffer_overflow_policy(BufferOverflowPolicy::ReturnError)
        .build().await.unwrap();
    
    // Generate 10,000 events rapidly
    // Verify no overflow errors
    // Verify all events processed
}
```

---

## Implementation Timeline

### Phase 1: Core Infrastructure (1-2 days)
- Background event processor
- Policy enums
- Builder pattern structure

### Phase 2: Default Handlers (2-3 days)
- Implement each policy type
- Test automatic behaviors
- Ensure no regressions

### Phase 3: User Overrides (1 day)
- Handler registration API
- Priority system (user > default)
- Testing overrides

### Phase 4: Migration (1 day)
- Update examples
- Update documentation
- Deprecation warnings

**Total: 5-7 days**

---

## Open Questions

1. Should policies be runtime-configurable or build-time only?
2. Should we support policy changes after construction?
3. Do we need per-call policies (different behavior per call)?
4. Should event history be queryable at runtime?
5. Should we support policy plugins (user-defined policies)?

---

## Conclusion

This builder pattern + default handlers approach solves the fundamental event handling problem in SimplePeer while maintaining simplicity for basic use cases and providing power for advanced scenarios.

**Key Principles:**
- ✅ Safe defaults
- ✅ Explicit configuration  
- ✅ No lost events
- ✅ Flexible policies
- ✅ Production ready

Ready for implementation!

