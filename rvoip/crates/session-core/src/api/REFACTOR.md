# Session-Core API Refactor Plan

## Overview

This document outlines improvements needed for the session-core public API based on real-world usage patterns and identified limitations.

## Current Limitations

### 1. Limited Event Handler Callbacks

**Problem**: The existing `CallHandler` trait only provides three callbacks, missing important events like state changes, media quality updates, and DTMF.

**Current approach**:
```rust
// Limited to just these three callbacks
pub trait CallHandler: Send + Sync + std::fmt::Debug {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision;
    async fn on_call_ended(&self, call: CallSession, reason: &str);
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>);
}

// Applications must poll for other state changes
loop {
    if let Ok(Some(session)) = SessionControl::get_session(&coordinator, &session_id).await {
        if session.state() != last_state {
            // Handle state change
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
}
```

**Impact**: 
- Cannot receive real-time notifications for all state transitions
- Missing media quality events
- No DTMF event callbacks
- Applications resort to inefficient polling

### 2. Statistics API Changes

**Problem**: `MediaSessionStats` was removed from exports, causing confusion.

**Current approach**:
```rust
// Must use multiple methods to get complete statistics
let media_info = MediaControl::get_media_info(&coordinator, &session_id).await?;
let media_stats = MediaControl::get_media_statistics(&coordinator, &session_id).await?;
let rtp_stats = MediaControl::get_rtp_statistics(&coordinator, &session_id).await?;
```

**Impact**:
- Multiple async calls needed for related data
- Unclear which method provides what statistics
- No convenience methods for common metrics

### 3. Limited SDP Control

**Problem**: While automatic SDP negotiation works well, there's no way to customize behavior.

**Current approach**:
```rust
// Fixed negotiation strategy
let answer = MediaControl::generate_sdp_answer(&coordinator, &session_id, offer).await?;
```

**Impact**:
- Cannot enforce specific codec preferences per call
- Cannot add custom SDP attributes
- Cannot implement advanced negotiation strategies

## Proposed Improvements

### 1. Extend Existing CallHandler Trait

**Important**: Session-core already has a working event system through the `CallHandler` trait that client-core successfully uses. Instead of creating a duplicate event system, we'll extend the existing trait with new optional methods.

```rust
/// Extended CallHandler trait with comprehensive event callbacks
#[async_trait]
pub trait CallHandler: Send + Sync + std::fmt::Debug {
    // === Existing methods (keep as-is) ===
    
    /// Handle an incoming call and decide what to do with it
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision;

    /// Handle when a call ends
    async fn on_call_ended(&self, call: CallSession, reason: &str);
    
    /// Handle when a call is established (200 OK received/sent)
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        // Default implementation
    }
    
    // === New optional methods with default implementations ===
    
    /// Called on any session state change
    async fn on_call_state_changed(&self, event: StateChangeEvent) {
        // Default: do nothing - maintains backward compatibility
    }
    
    /// Called when media quality metrics are available
    async fn on_media_quality(&self, event: MediaQualityEvent) {
        // Default: do nothing
    }
    
    /// Called when DTMF digit is received
    async fn on_dtmf(&self, event: DtmfEvent) {
        // Default: do nothing
    }
    
    /// Called when media starts/stops flowing
    async fn on_media_flow(&self, event: MediaFlowEvent) {
        // Default: do nothing
    }
    
    /// Called on non-fatal warnings
    async fn on_warning(&self, event: WarningEvent) {
        // Default: do nothing
    }
}

/// State change event
pub struct StateChangeEvent {
    pub session_id: SessionId,
    pub old_state: CallState,
    pub new_state: CallState,
    pub timestamp: Instant,
    pub reason: Option<String>,
}

/// Media quality event
pub struct MediaQualityEvent {
    pub session_id: SessionId,
    pub mos_score: f32,
    pub packet_loss: f32,
    pub jitter_ms: f32,
    pub round_trip_ms: f32,
    pub alert_level: QualityAlertLevel,
}

/// DTMF event
pub struct DtmfEvent {
    pub session_id: SessionId,
    pub digit: char,
    pub duration_ms: u32,
    pub timestamp: Instant,
}

/// Media flow event
pub struct MediaFlowEvent {
    pub session_id: SessionId,
    pub direction: MediaDirection,
    pub active: bool,
    pub codec: String,
}

/// Warning event for non-fatal issues
pub struct WarningEvent {
    pub session_id: Option<SessionId>,
    pub category: WarningCategory,
    pub message: String,
}

pub enum QualityAlertLevel {
    Good,      // MOS >= 4.0
    Fair,      // MOS >= 3.0
    Poor,      // MOS >= 2.0
    Critical,  // MOS < 2.0
}

pub enum MediaDirection {
    Send,
    Receive,
    Both,
}

pub enum WarningCategory {
    Network,
    Media,
    Protocol,
    Resource,
}
```

**Key advantages of extending CallHandler**:
- No breaking changes - existing implementations continue to work
- Client-core already uses this trait, so events flow naturally
- Consistent with existing architecture
- Optional methods allow gradual adoption

**Integration with internal events**:
```rust
// SessionCoordinator already processes SessionEvent internally
// We just need to ensure it calls the appropriate CallHandler methods
match event {
    SessionEvent::StateChanged { session_id, old_state, new_state } => {
        // Call existing handler methods when appropriate
        if let Some(handler) = &self.handler {
            // New: Always call on_call_state_changed
            handler.on_call_state_changed(StateChangeEvent {
                session_id: session_id.clone(),
                old_state: old_state.clone(),
                new_state: new_state.clone(),
                timestamp: Instant::now(),
                reason: None,
            }).await;
            
            // Existing: Call specific methods for important transitions
            match (old_state, new_state) {
                (_, CallState::Active) => {
                    // Still call on_call_established for compatibility
                    if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                        handler.on_call_established(session, local_sdp, remote_sdp).await;
                    }
                }
                (_, CallState::Terminated) | (_, CallState::Failed(_)) => {
                    // Still call on_call_ended
                    if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                        handler.on_call_ended(session, &reason).await;
                    }
                }
                _ => {}
            }
        }
    }
    // Handle other events similarly...
}
```

### 2. Improved Statistics API

Add convenience methods and unified statistics retrieval:

```rust
/// Comprehensive call statistics
pub struct CallStatistics {
    pub session_id: SessionId,
    pub duration: Duration,
    pub state: State,
    pub media: MediaStatistics,
    pub rtp: RtpSessionStats,
    pub quality: QualityMetrics,
}

impl MediaControl for Arc<SessionCoordinator> {
    /// Get all statistics in one call
    async fn get_call_statistics(&self, session_id: &SessionId) -> Result<Option<CallStatistics>>;
    
    /// Convenience: Get just the MOS score
    async fn get_call_quality_score(&self, session_id: &SessionId) -> Result<Option<f32>>;
    
    /// Convenience: Get packet loss percentage
    async fn get_packet_loss_rate(&self, session_id: &SessionId) -> Result<Option<f32>>;
    
    /// Convenience: Get current bitrate
    async fn get_current_bitrate(&self, session_id: &SessionId) -> Result<Option<u32>>;
    
    /// Start quality monitoring with automatic alerts
    async fn monitor_call_quality(
        &self, 
        session_id: &SessionId,
        thresholds: QualityThresholds
    ) -> Result<()>;
}

/// Quality monitoring thresholds
pub struct QualityThresholds {
    pub min_mos: f32,              // Default: 3.0
    pub max_packet_loss: f32,       // Default: 5.0%
    pub max_jitter_ms: f32,         // Default: 50.0
    pub check_interval: Duration,    // Default: 5 seconds
}
```

### 3. Enhanced SDP Control

Add negotiation options and custom SDP handling:

```rust
/// SDP negotiation options
pub struct SdpNegotiationOptions {
    /// Force specific codecs for this call
    pub required_codecs: Vec<String>,
    
    /// Additional SDP attributes to include
    pub custom_attributes: Vec<SdpAttribute>,
    
    /// Negotiation strategy
    pub strategy: NegotiationStrategy,
    
    /// Media preferences override
    pub media_config_override: Option<MediaConfig>,
}

pub struct SdpAttribute {
    pub name: String,
    pub value: String,
}

pub enum NegotiationStrategy {
    /// Use first mutually supported codec (default)
    FirstMatch,
    
    /// Prefer wideband codecs
    PreferWideband,
    
    /// Prefer narrowband for bandwidth savings
    PreferNarrowband,
    
    /// Custom priority function
    Custom(Box<dyn Fn(&[String], &[String]) -> Option<String> + Send + Sync>),
}

impl MediaControl for Arc<SessionCoordinator> {
    /// Generate SDP answer with options
    async fn generate_sdp_answer_with_options(
        &self,
        session_id: &SessionId,
        offer: &str,
        options: SdpNegotiationOptions
    ) -> Result<String>;
    
    /// Generate SDP offer with options
    async fn generate_sdp_offer_with_options(
        &self,
        session_id: &SessionId,
        options: SdpNegotiationOptions
    ) -> Result<String>;
    
    /// Get the negotiated media configuration
    async fn get_negotiated_config(
        &self,
        session_id: &SessionId
    ) -> Result<Option<NegotiatedMediaConfig>>;
}
```

### 4. Session Control Enhancements

Add missing session operations:

```rust
impl SessionControl for Arc<SessionCoordinator> {
    /// Get all active sessions
    async fn get_active_sessions(&self) -> Result<Vec<SessionInfo>>;
    
    /// Transfer a call to another destination
    async fn transfer_call(
        &self,
        session_id: &SessionId,
        transfer_to: &str,
        transfer_type: TransferType
    ) -> Result<()>;
    
    /// Conference multiple sessions
    async fn create_conference(
        &self,
        session_ids: Vec<SessionId>
    ) -> Result<ConferenceId>;
    
    /// Add early media support
    async fn send_early_media(
        &self,
        session_id: &SessionId,
        sdp: &str
    ) -> Result<()>;
}

pub enum TransferType {
    Blind,
    Attended,
}
```

## Implementation Plan

### Phase 1: Extend CallHandler (Priority: High)
1. Add new optional methods to CallHandler trait
2. Update SessionCoordinator to call new methods from internal events
3. Test with existing client-core implementation
4. Update examples to showcase new callbacks

### Phase 2: Statistics API (Priority: Medium)
1. Create `CallStatistics` aggregate type
2. Implement convenience methods
3. Add quality monitoring
4. Document statistics fields

### Phase 3: SDP Enhancements (Priority: Medium)
1. Create options types
2. Extend negotiator with strategies
3. Add codec enforcement
4. Support custom attributes

### Phase 4: Session Control (Priority: Low)
1. Add `get_active_sessions`
2. Implement call transfer
3. Add conference support
4. Early media handling

## Migration Guide

### For Event Handling

**Before** (still supported):
```rust
// Existing handlers continue to work unchanged
struct MyHandler;

#[async_trait]
impl CallHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        CallDecision::Accept(None)
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call ended: {}", reason);
    }
}
```

**After** (with new events):
```rust
// Same handler can now opt into additional events
struct MyHandler;

#[async_trait]
impl CallHandler for MyHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        CallDecision::Accept(None)
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call ended: {}", reason);
    }
    
    // Opt into state change events
    async fn on_call_state_changed(&self, event: StateChangeEvent) {
        println!("State changed from {:?} to {:?}", event.old_state, event.new_state);
    }
    
    // Opt into quality monitoring
    async fn on_media_quality(&self, event: MediaQualityEvent) {
        if event.alert_level == QualityAlertLevel::Critical {
            eprintln!("Poor call quality: MOS={}", event.mos_score);
        }
    }
}
```

**Client-core integration** (no changes needed):
```rust
// Client-core's existing ClientCallHandler already implements CallHandler
// It can gradually adopt new methods to emit more ClientEvents
impl CallHandler for ClientCallHandler {
    // Existing methods work as before
    
    // New: Can now forward state changes directly
    async fn on_call_state_changed(&self, event: StateChangeEvent) {
        // Map to ClientEvent and emit to application
        if let Some(call_id) = self.call_mapping.get(&event.session_id) {
            let client_event = ClientEvent::CallStateChanged {
                info: CallStatusInfo {
                    call_id: *call_id,
                    new_state: self.map_session_state_to_client_state(&event.new_state),
                    previous_state: Some(self.map_session_state_to_client_state(&event.old_state)),
                    reason: event.reason,
                    timestamp: event.timestamp,
                },
                priority: EventPriority::Normal,
            };
            // Emit to application...
        }
    }
}
```

### For Statistics

**Before**:
```rust
let info = MediaControl::get_media_info(&coord, &id).await?;
let stats = MediaControl::get_media_statistics(&coord, &id).await?;
// Manually combine...
```

**After**:
```rust
let stats = MediaControl::get_call_statistics(&coord, &id).await?;
// Everything in one place
```

### For SDP Control

**Before**:
```rust
// Fixed behavior
let answer = MediaControl::generate_sdp_answer(&coord, &id, offer).await?;
```

**After**:
```rust
// Customizable
let options = SdpNegotiationOptions {
    required_codecs: vec!["G722".to_string()],
    strategy: NegotiationStrategy::PreferWideband,
    ..Default::default()
};
let answer = MediaControl::generate_sdp_answer_with_options(
    &coord, &id, offer, options
).await?;
```

## Benefits

1. **Better Developer Experience**
   - More comprehensive event callbacks
   - No breaking changes to existing code
   - Natural integration with client-core
   - Fewer async calls needed

2. **Performance**
   - No polling overhead
   - Leverages existing event flow
   - Efficient event dispatching

3. **Flexibility**
   - Backward compatible
   - Optional adoption of new features
   - Extensible for future events

4. **Maintainability**
   - Builds on existing architecture
   - No duplicate event systems
   - Clear upgrade path

## Open Questions

1. Should we add event filtering to CallHandler methods?
2. How to handle back-pressure if handler is slow?
3. Should quality metrics be pushed or pulled?
4. Frequency of media quality events?
5. Should we provide a CallHandlerAdapter with empty defaults?

## Next Steps

1. Review and approve design
2. Extend CallHandler trait
3. Update SessionCoordinator event processing
4. Test with client-core
5. Update documentation and examples 