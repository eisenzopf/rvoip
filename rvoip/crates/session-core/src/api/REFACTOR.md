# Session-Core API Refactor Plan

## Overview

This document outlines improvements needed for the session-core public API based on real-world usage patterns and identified limitations.

## Architecture Principles

**Important**: The API module is a high-level abstraction that depends on lower-level modules. Dependencies must flow in this direction:
```
api → coordinator → manager → events
```

Events and types should be defined in lower-level modules and re-exported by the API layer, not the other way around.

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

### 1. Extend Events in manager/events.rs

First, we'll extend the existing `SessionEvent` enum in `manager/events.rs` with richer event types:

```rust
// In manager/events.rs - extend the existing SessionEvent enum
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    // ... existing events ...
    
    /// Enhanced state change event with metadata
    DetailedStateChange {
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
        timestamp: std::time::Instant,
        reason: Option<String>,
    },
    
    /// Media quality metrics event
    MediaQuality {
        session_id: SessionId,
        mos_score: f32,
        packet_loss: f32,
        jitter_ms: f32,
        round_trip_ms: f32,
        alert_level: MediaQualityAlertLevel,
    },
    
    /// DTMF digit received (enhanced version)
    DtmfDigit {
        session_id: SessionId,
        digit: char,
        duration_ms: u32,
        timestamp: std::time::Instant,
    },
    
    /// Media flow status change
    MediaFlowChange {
        session_id: SessionId,
        direction: MediaFlowDirection,
        active: bool,
        codec: String,
    },
    
    /// Non-fatal warning event
    Warning {
        session_id: Option<SessionId>,
        category: WarningCategory,
        message: String,
    },
}

// Also add supporting types in manager/events.rs
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MediaQualityAlertLevel {
    Good,      // MOS >= 4.0
    Fair,      // MOS >= 3.0
    Poor,      // MOS >= 2.0
    Critical,  // MOS < 2.0
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MediaFlowDirection {
    Send,
    Receive,
    Both,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum WarningCategory {
    Network,
    Media,
    Protocol,
    Resource,
}
```

### 2. Extend CallHandler to Consume Internal Events

In the API layer, extend `CallHandler` to translate internal events into handler callbacks:

```rust
// In api/handlers.rs - extend the existing trait
#[async_trait]
pub trait CallHandler: Send + Sync + std::fmt::Debug {
    // === Existing methods (keep as-is) ===
    
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision;
    async fn on_call_ended(&self, call: CallSession, reason: &str);
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>);
    
    // === New optional methods with default implementations ===
    
    /// Called on any session state change
    async fn on_call_state_changed(&self, session_id: &SessionId, old_state: &CallState, new_state: &CallState, reason: Option<&str>) {
        // Default: do nothing - maintains backward compatibility
    }
    
    /// Called when media quality metrics are available
    async fn on_media_quality(&self, session_id: &SessionId, mos_score: f32, packet_loss: f32, alert_level: MediaQualityAlertLevel) {
        // Default: do nothing
    }
    
    /// Called when DTMF digit is received
    async fn on_dtmf(&self, session_id: &SessionId, digit: char, duration_ms: u32) {
        // Default: do nothing
    }
    
    /// Called when media starts/stops flowing
    async fn on_media_flow(&self, session_id: &SessionId, direction: MediaFlowDirection, active: bool, codec: &str) {
        // Default: do nothing
    }
    
    /// Called on non-fatal warnings
    async fn on_warning(&self, session_id: Option<&SessionId>, category: WarningCategory, message: &str) {
        // Default: do nothing
    }
}
```

### 3. Update SessionCoordinator Event Processing

The `SessionCoordinator` will subscribe to internal events and translate them to CallHandler calls:

```rust
// In coordinator/event_handler.rs - process internal events
impl SessionCoordinator {
    async fn process_session_event(&self, event: SessionEvent) {
        if let Some(handler) = &self.handler {
            match event {
                SessionEvent::DetailedStateChange { session_id, old_state, new_state, reason, .. } => {
                    // Call the new handler method
                    handler.on_call_state_changed(&session_id, &old_state, &new_state, reason.as_deref()).await;
                    
                    // Also call legacy methods for compatibility
                    match (&old_state, &new_state) {
                        (_, CallState::Active) => {
                            if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                                handler.on_call_established(session, local_sdp, remote_sdp).await;
                            }
                        }
                        (_, CallState::Terminated) | (_, CallState::Failed(_)) => {
                            if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                                handler.on_call_ended(session, &reason.unwrap_or_default()).await;
                            }
                        }
                        _ => {}
                    }
                }
                SessionEvent::MediaQuality { session_id, mos_score, packet_loss, alert_level, .. } => {
                    handler.on_media_quality(&session_id, mos_score, packet_loss, alert_level).await;
                }
                SessionEvent::DtmfDigit { session_id, digit, duration_ms, .. } => {
                    handler.on_dtmf(&session_id, digit, duration_ms).await;
                }
                SessionEvent::MediaFlowChange { session_id, direction, active, codec } => {
                    handler.on_media_flow(&session_id, direction, active, &codec).await;
                }
                SessionEvent::Warning { session_id, category, message } => {
                    handler.on_warning(session_id.as_ref(), category, &message).await;
                }
                // Handle other events...
                _ => {}
            }
        }
    }
}
```

### 4. API Layer Re-exports

The API layer simply re-exports the event types for public use:

```rust
// In api/mod.rs - re-export event types from lower layers
pub use crate::manager::events::{
    MediaQualityAlertLevel,
    MediaFlowDirection,
    WarningCategory,
    // Note: We don't export SessionEvent itself - that's internal
};
```

### 5. Statistics API Improvements

Add convenience types in a lower-level module and re-export:

```rust
// In media/stats.rs - define comprehensive statistics types
#[derive(Debug, Clone)]
pub struct CallStatistics {
    pub session_id: SessionId,
    pub duration: Duration,
    pub state: CallState,
    pub media: MediaStatistics,
    pub rtp: RtpSessionStats,
    pub quality: QualityMetrics,
}

#[derive(Debug, Clone)]
pub struct QualityMetrics {
    pub mos_score: f32,
    pub packet_loss_rate: f32,
    pub jitter_ms: f32,
    pub round_trip_ms: f32,
}

// In api/media.rs - add convenience methods that use these types
impl MediaControl for Arc<SessionCoordinator> {
    /// Get all statistics in one call
    async fn get_call_statistics(&self, session_id: &SessionId) -> Result<Option<CallStatistics>> {
        // Implementation collects from various sources
    }
    
    /// Convenience: Get just the MOS score
    async fn get_call_quality_score(&self, session_id: &SessionId) -> Result<Option<f32>> {
        // Extract from CallStatistics
    }
}
```

### 6. SDP Control Options

Define negotiation options in a lower-level module:

```rust
// In sdp/negotiation.rs - define negotiation types
pub struct SdpNegotiationOptions {
    pub required_codecs: Vec<String>,
    pub custom_attributes: Vec<SdpAttribute>,
    pub strategy: NegotiationStrategy,
    pub media_config_override: Option<MediaConfig>,
}

pub enum NegotiationStrategy {
    FirstMatch,
    PreferWideband,
    PreferNarrowband,
    Custom(Box<dyn Fn(&[String], &[String]) -> Option<String> + Send + Sync>),
}

// In api/media.rs - expose methods that use these types
impl MediaControl for Arc<SessionCoordinator> {
    async fn generate_sdp_answer_with_options(
        &self,
        session_id: &SessionId,
        offer: &str,
        options: SdpNegotiationOptions
    ) -> Result<String> {
        // Delegate to sdp::negotiation module
    }
}
```

## Implementation Plan

### Phase 1: Extend Internal Events (Priority: High)
1. Add new event variants to `SessionEvent` in `manager/events.rs`
2. Update event publishers throughout the codebase to emit richer events
3. Ensure backward compatibility with existing event handling

### Phase 2: Extend CallHandler (Priority: High)
1. Add new optional methods to `CallHandler` trait in `api/handlers.rs`
2. Update `SessionCoordinator` to subscribe to internal events
3. Translate internal events to CallHandler method calls
4. Test with existing client-core implementation

### Phase 3: Statistics API (Priority: Medium)
1. Create statistics types in `media/stats.rs`
2. Implement aggregation logic in media module
3. Add convenience methods to `MediaControl` trait
4. Re-export types through API layer

### Phase 4: SDP Enhancements (Priority: Medium)
1. Create negotiation types in `sdp/negotiation.rs`
2. Extend SDP negotiator with strategy support
3. Add methods to `MediaControl` that use these types
4. Re-export necessary types through API

### Phase 5: Session Control (Priority: Low)
1. Add missing operations to `SessionControl` trait
2. Implement in `SessionCoordinator`
3. Add supporting types where needed

## Dependency Flow

The refactored architecture maintains proper dependency hierarchy:

```
┌─────────────┐
│     API     │ ← Public interface, re-exports types
├─────────────┤
│ Coordinator │ ← Orchestrates and translates events
├─────────────┤
│   Manager   │ ← Core session management
├─────────────┤
│   Events    │ ← Event definitions (lowest level)
└─────────────┘
```

## Migration Guide

### For Event Handling

**Before** (still works):
```rust
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
struct MyHandler;

#[async_trait]
impl CallHandler for MyHandler {
    // Existing methods continue to work
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        CallDecision::Accept(None)
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("Call ended: {}", reason);
    }
    
    // Opt into new events
    async fn on_call_state_changed(&self, session_id: &SessionId, old_state: &CallState, new_state: &CallState, reason: Option<&str>) {
        println!("State changed from {:?} to {:?}", old_state, new_state);
    }
    
    async fn on_media_quality(&self, session_id: &SessionId, mos_score: f32, packet_loss: f32, alert_level: MediaQualityAlertLevel) {
        if matches!(alert_level, MediaQualityAlertLevel::Critical) {
            eprintln!("Poor call quality: MOS={}", mos_score);
        }
    }
}
```

## Benefits

1. **Proper Architecture**
   - Respects dependency hierarchy
   - No circular dependencies
   - Clear separation of concerns

2. **Better Developer Experience**
   - More comprehensive event callbacks
   - No breaking changes to existing code
   - Natural integration with client-core

3. **Performance**
   - Leverages existing event system
   - No additional overhead
   - Efficient event translation

4. **Maintainability**
   - Events defined in appropriate modules
   - API layer remains thin
   - Easy to extend further

## Next Steps

1. Review and approve updated design
2. Extend `SessionEvent` in manager/events.rs
3. Update event publishers throughout codebase
4. Extend CallHandler trait with event translation
5. Update documentation and examples 