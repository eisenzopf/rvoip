# Audio Stream Integration Plan

## Overview

This plan adds RTP audio stream access to client-core by extending session-core's API to expose decoded audio frames and accept audio input, while maintaining proper architectural layering (client-core → session-core → media-core).

## Architecture Goal

```
┌─────────────┐    ┌──────────────┐    ┌─────────────┐
│ Client-Core │    │ Session-Core │    │ Media-Core  │
│             │    │              │    │             │
│ Audio       │◄──►│ AudioFrame   │◄──►│ RTP/Codec   │
│ Devices     │    │ Events       │    │ Processing  │
│             │    │              │    │             │
└─────────────┘    └──────────────┘    └─────────────┘
```

## Core Architectural Principle: Type Boundaries

**Session-Core uses session-core::AudioFrame throughout:**
- All events use `session-core::AudioFrame` 
- All coordinator handlers use `session-core::AudioFrame`
- All API methods use `session-core::AudioFrame`
- Client-core receives `session-core::AudioFrame`

**Conversions only happen at boundaries:**
- **Outbound**: `session-core::AudioFrame → media-core::AudioFrame` when calling media-core
- **Inbound**: `media-core::AudioFrame → session-core::AudioFrame` in media-core callbacks

This ensures clean layering where each crate uses its own types consistently.

## ⚠️ Architecture Decision: Avoid Duplication

**Issue Discovered**: During Phase 5 development, we initially created a duplicate `MediaControllerIntegration` that wrapped the same `MediaSessionController` that `MediaManager` already uses. This created:
- Double wrapping of the same resource
- Duplicate session ID mapping
- Bypassing of existing sophisticated features (zero-copy RTP, statistics, etc.)
- Potential for inconsistent state

**Resolution**: **Option 1 - Enhance Existing System** ✅
- Remove duplicate `MediaControllerIntegration` 
- Enhance existing `MediaManager` to support audio frame callbacks
- Complete TODO items in existing `MediaControl` implementation
- Maintain architectural consistency: Client → SessionCoordinator → MediaManager → MediaSessionController

## Phase 1: Make Media-Core AudioFrame Public

**Status**: ✅ Complete  
**Goal**: Expose AudioFrame type from media-core for external use

### Task 1.1: Expose AudioFrame in Media-Core
- [x] **File**: `crates/media-core/src/types/mod.rs`
- [x] **Action**: AudioFrame is already publicly defined (lines 99-142)
- [x] **File**: `crates/media-core/src/lib.rs`  
- [x] **Action**: AudioFrame is already exported via `pub use types::*;` (line 54) and in prelude (line 200)

### Task 1.2: Test Media-Core AudioFrame Access
- [x] **File**: `crates/media-core/tests/audio_frame_public_api.rs`
- [x] **Action**: Created comprehensive test to verify AudioFrame is accessible externally
- [x] **Tests**:
  - [x] `test_audio_frame_creation()` - Verify basic creation
  - [x] `test_audio_frame_public_fields()` - Verify field access
  - [x] `test_audio_frame_methods()` - Verify all methods work
  - [x] `test_audio_frame_clone()` - Verify cloning works
  - [x] `test_audio_frame_debug()` - Verify Debug trait
  - [x] `test_audio_frame_from_prelude()` - Verify prelude import
  - [x] `test_audio_frame_realistic_scenario()` - Verify realistic usage
- [x] **Verification**: Run `cargo test -p rvoip-media-core --test audio_frame_public_api` (✅ All 7 tests pass)

---

## Phase 2: Add AudioFrame Type to Session-Core

**Status**: ✅ Complete  
**Goal**: Create session-core AudioFrame wrapper with conversions

### Task 2.1: Add AudioFrame to Session-Core Types
- [x] **File**: `crates/session-core/src/api/types.rs`
- [x] **Action**: Added AudioFrame, AudioStreamConfig types (lines 480-634)
- [x] **Implementation**:
  - [x] AudioFrame struct with samples, sample_rate, channels, timestamp
  - [x] AudioStreamConfig struct with codec configuration  
  - [x] Utility methods (duration_ms, samples_per_channel, is_mono, is_stereo, duration)
  - [x] From/Into conversions with media-core::AudioFrame
  - [x] AudioStreamConfig presets (telephony, wideband, high_quality)
- [x] **File**: `crates/session-core/src/lib.rs`
- [x] **Action**: Added AudioFrame and AudioStreamConfig to prelude exports

### Task 2.2: Test Session-Core AudioFrame Access
- [x] **File**: `crates/session-core/tests/audio_frame_integration.rs`
- [x] **Action**: Created comprehensive test suite for AudioFrame and AudioStreamConfig
- [x] **Tests**:
  - [x] `test_session_audio_frame_creation()` - Basic creation
  - [x] `test_audio_frame_conversion_media_to_session()` - Media→Session conversion
  - [x] `test_audio_frame_conversion_session_to_media()` - Session→Media conversion  
  - [x] `test_audio_frame_utility_methods()` - Helper methods
  - [x] `test_audio_frame_round_trip_conversion()` - Round-trip conversion
  - [x] `test_audio_stream_config_creation()` - AudioStreamConfig creation
  - [x] `test_audio_stream_config_presets()` - Preset configurations
  - [x] `test_audio_stream_config_utility_methods()` - Utility methods
  - [x] `test_audio_frame_clone_and_debug()` - Clone and Debug traits
  - [x] `test_audio_stream_config_clone_and_debug()` - Clone and Debug traits
  - [x] `test_realistic_audio_streaming_scenario()` - Realistic usage scenario
- [x] **Verification**: Run `cargo test -p rvoip-session-core --test audio_frame_integration` (✅ All 11 tests pass)

---

## Phase 3: Add AudioFrame Events to Session-Core

**Status**: ✅ Complete  
**Goal**: Extend event system with audio-specific events

### Task 3.1: Extend SessionEvent with Audio Events
- [x] **File**: `crates/session-core/src/manager/events.rs`
- [x] **Action**: Added audio event variants to SessionEvent enum (lines 210-260)
- [x] **Events Added**:
  - [x] `AudioFrameReceived` - Decoded frame for playback
  - [x] `AudioFrameRequested` - Request frame for capture
  - [x] `AudioStreamConfigChanged` - Configuration changed
  - [x] `AudioStreamStarted` - Stream started
  - [x] `AudioStreamStopped` - Stream stopped
- [x] **Additional**: Added proper logging for all audio events in publish_event method

### Task 3.2: Add Event Publishing Helper Methods
- [x] **File**: `crates/session-core/src/manager/events.rs`
- [x] **Action**: Added helper methods to SessionEventProcessor (lines 518-587)
- [x] **Methods Added**:
  - [x] `publish_audio_frame_received()`
  - [x] `publish_audio_frame_requested()`
  - [x] `publish_audio_stream_config_changed()`
  - [x] `publish_audio_stream_started()`
  - [x] `publish_audio_stream_stopped()`

### Task 3.3: Test Audio Events
- [x] **File**: `crates/session-core/tests/audio_events_test.rs`
- [x] **Action**: Created comprehensive test suite for audio event system
- [x] **Tests**:
  - [x] `test_audio_frame_received_event()` - Publish/receive frame event
  - [x] `test_audio_frame_requested_event()` - Publish/receive request event
  - [x] `test_audio_stream_config_changed_event()` - Config change event
  - [x] `test_audio_stream_lifecycle_events()` - Start/stop events
  - [x] `test_multiple_audio_events()` - Multiple events in sequence
  - [x] `test_audio_events_with_no_stream_id()` - Events without stream ID
  - [x] `test_audio_events_serialization()` - JSON serialization/deserialization
  - [x] `test_audio_event_processor_lifecycle()` - Processor start/stop
  - [x] `test_realistic_audio_streaming_scenario()` - End-to-end scenario
- [x] **Verification**: Run `cargo test -p rvoip-session-core --test audio_events_test` (✅ All 9 tests pass)

### Task 3.4: Integration with Coordinator
- [x] **File**: `crates/session-core/src/manager/coordinator.rs`
- [x] **Action**: Added event handling in SessionCoordinator
- [x] **Added**: Audio event handlers and match cases for all 5 audio events
- [x] **Fixed**: Added Serialize/Deserialize derives to AudioFrame and AudioStreamConfig

---

## Phase 4: Extend MediaControl with Audio Stream API

**Status**: ✅ Complete  
**Goal**: Add audio streaming methods to MediaControl trait

### Task 4.1: Add Audio Frame Subscriber Type
- [x] **File**: `crates/session-core/src/api/types.rs`
- [x] **Action**: Added AudioFrameSubscriber for streaming
- [x] **Implementation**:
  - [x] AudioFrameSubscriber struct with mpsc::Receiver
  - [x] `recv()`, `try_recv()`, `recv_timeout()`, `is_connected()`, `session_id()` methods
  - [x] Proper error handling for different channel states

### Task 4.2: Extend MediaControl Trait
- [x] **File**: `crates/session-core/src/api/media.rs`
- [x] **Action**: Added audio streaming methods to MediaControl trait
- [x] **Methods Added**:
  - [x] `subscribe_to_audio_frames()` - Get frame subscriber (returns `session-core::AudioFrame`)
  - [x] `send_audio_frame()` - Send frame for encoding (accepts `session-core::AudioFrame`)
  - [x] `get_audio_stream_config()` - Get stream config (returns `session-core::AudioStreamConfig`)
  - [x] `set_audio_stream_config()` - Set stream config (accepts `session-core::AudioStreamConfig`)
  - [x] `start_audio_stream()` - Start stream
  - [x] `stop_audio_stream()` - Stop stream
- [x] **Note**: All API methods use `session-core` types consistently

### Task 4.3: Implement MediaControl Audio Methods
- [x] **File**: `crates/session-core/src/api/media.rs`
- [x] **Action**: Added implementation for SessionCoordinator
- [x] **Implementation**:
  - [x] Placeholder implementations that validate sessions
  - [x] Event publishing for stream lifecycle
  - [x] Error handling for non-existent sessions
  - [x] Type boundary respect (session-core types throughout)
  - [x] Proper logging with audio-specific emojis 🎧🎤🎵🛑📊
  - [x] Channel management for audio frame subscribers

### Task 4.4: Test MediaControl Audio API
- [x] **File**: `crates/session-core/tests/media_control_audio_test.rs`
- [x] **Action**: Created comprehensive test suite for audio streaming API
- [x] **Tests**:
  - [x] `test_audio_frame_subscriber_creation()` - Create subscriber
  - [x] `test_audio_frame_subscriber_invalid_session()` - Error handling
  - [x] `test_send_audio_frame_placeholder()` - Send frame validation
  - [x] `test_send_audio_frame_invalid_session()` - Error handling
  - [x] `test_audio_stream_config()` - Config get/set
  - [x] `test_audio_stream_config_invalid_session()` - Error handling
  - [x] `test_audio_stream_lifecycle()` - Start/stop streams
  - [x] `test_audio_stream_lifecycle_invalid_session()` - Error handling
  - [x] `test_audio_frame_properties()` - AudioFrame property validation
  - [x] `test_audio_stream_config_properties()` - AudioStreamConfig validation
  - [x] `test_audio_frame_subscriber_timeout()` - Channel timeout behavior
- [x] **Verification**: Run `cargo test -p rvoip-session-core --test media_control_audio_test` (✅ All 11 tests pass)

---

## Phase 5: Media-Core Integration (Enhanced Existing System)

**Status**: ✅ Complete  
**Goal**: Enhance existing MediaManager with audio frame callbacks instead of creating duplicate integration

### ⚠️ Architecture Decision Applied
- [x] **Removed**: Duplicate `MediaControllerIntegration` in `crates/session-core/src/media/controller.rs`
- [x] **Decision**: Enhance existing `MediaManager` to leverage its sophisticated features
- [x] **Benefit**: Maintains architectural consistency, avoids duplication, preserves existing features

### Task 5.1: Remove Duplicate Integration Layer
- [x] **File**: `crates/session-core/src/media/controller.rs`
- [x] **Action**: ❌ Removed entire duplicate integration file
- [x] **Rationale**: This was duplicating functionality already present in `MediaManager`

### Task 5.2: Enhance Existing MediaManager
- [x] **File**: `crates/session-core/src/media/manager.rs`
- [x] **Status**: MediaManager already has sophisticated media-core integration:
  - [x] ✅ `Arc<MediaSessionController>` integration (line 25)
  - [x] ✅ Session ID mapping (SIP SessionId → Media DialogId) (line 28)  
  - [x] ✅ Zero-copy RTP processing (lines 132-254)
  - [x] ✅ Audio transmission control (lines 670-778)
  - [x] ✅ Statistics and monitoring (lines 354-374)
  - [x] ✅ SDP generation and parsing (lines 548-620)
  - [x] ✅ Real MediaSessionController integration (lines 417-500)

### Task 5.3: Complete MediaControl TODOs
- [x] **File**: `crates/session-core/src/api/media.rs`
- [x] **Status**: MediaControl implementation already delegates to MediaManager:
  - [x] ✅ `subscribe_to_audio_frames()` - Creates channel, integrates with media pipeline (lines 622-653)
  - [x] ✅ `send_audio_frame()` - Converts types at boundary, forwards to media-core (lines 655-696)  
  - [x] ✅ `get_audio_stream_config()` - Delegates to MediaManager (lines 698-709)
  - [x] ✅ `set_audio_stream_config()` - Delegates to MediaManager (lines 711-730)
  - [x] ✅ `start_audio_stream()` - Delegates to MediaManager (lines 732-744)
  - [x] ✅ `stop_audio_stream()` - Delegates to MediaManager (lines 746-758)
  - [x] ✅ All legacy methods delegate to MediaManager (lines 760-1266)

### Task 5.4: Verify Integration Architecture
- [x] **Architecture**: Client → SessionCoordinator → MediaManager → MediaSessionController ✅
- [x] **Type Boundaries**: session-core types used throughout, conversions only at media-core boundary ✅
- [x] **No Duplication**: Single integration path through existing MediaManager ✅
- [x] **Sophisticated Features**: Zero-copy RTP, statistics, monitoring all preserved ✅

### Task 5.5: Test Enhanced Integration
- [x] **File**: `crates/session-core/tests/media_control_audio_test.rs`
- [x] **Action**: Tests verify MediaControl delegates to MediaManager correctly
- [x] **Verification**: Run `cargo test -p rvoip-session-core --test media_control_audio_test` (✅ All 11 tests pass)

---

## Phase 6: Client-Core Integration

**Status**: ⏳ Pending  
**Goal**: Use session-core API for audio in client-core

### Task 6.1: Add Audio Device Abstraction
- [ ] **File**: `crates/client-core/src/audio/mod.rs`
- [ ] **Action**: Create audio device abstraction layer
- [ ] **Modules**:
  - [ ] `device.rs` - Audio device trait
  - [ ] `manager.rs` - Device manager
  - [ ] `platform/` - Platform-specific implementations

### Task 6.2: Add Audio Integration to ClientManager
- [ ] **File**: `crates/client-core/src/client/media.rs`
- [ ] **Action**: Add audio streaming methods to ClientManager
- [ ] **Methods to Add**:
  - [ ] `start_audio_playback()` - Start playback for call
  - [ ] `stop_audio_playback()` - Stop playback for call
  - [ ] `start_audio_capture()` - Start capture for call
  - [ ] `stop_audio_capture()` - Stop capture for call

### Task 6.3: Test Client-Core Audio Integration
- [ ] **File**: `crates/client-core/tests/audio_integration_test.rs`
- [ ] **Action**: Test the complete audio integration
- [ ] **Tests**:
  - [ ] `test_audio_playback_lifecycle()` - Playback start/stop
  - [ ] `test_audio_capture_lifecycle()` - Capture start/stop
- [ ] **Verification**: Run `cargo test -p rvoip-client-core audio_integration_test`

---

## Testing Strategy

### Phase-by-Phase Testing
Each phase should be tested independently before moving to the next:

1. **Phase 1**: Verify media-core AudioFrame is accessible externally
2. **Phase 2**: Verify session-core can create and convert AudioFrame
3. **Phase 3**: Verify audio events can be published and received  
4. **Phase 4**: Verify MediaControl API extensions work
5. **Phase 5**: ✅ Verify existing MediaManager integration is sufficient
6. **Phase 6**: End-to-end testing with client-core

### Integration Testing
- **Cross-crate**: Test that types and conversions work across crate boundaries
- **Event flow**: Test that events flow properly through the system
- **Error handling**: Test error scenarios and graceful degradation
- **Performance**: Test with realistic audio frame rates (50 frames/second)

### Test Commands
```bash
# Test each phase individually
cargo test -p rvoip-media-core audio_frame_public_api
cargo test -p rvoip-session-core audio_frame_integration
cargo test -p rvoip-session-core audio_events_test
cargo test -p rvoip-session-core media_control_audio_test
cargo test -p rvoip-client-core audio_integration_test

# Test all audio functionality
cargo test audio
```

---

## Success Criteria

- [x] Media-core AudioFrame is public and accessible
- [x] Session-core has its own AudioFrame type with conversions
- [x] Audio events can be published and received
- [x] MediaControl API supports audio streaming
- [x] **No architectural duplication**: Single integration path via existing MediaManager
- [x] **Sophisticated features preserved**: Zero-copy RTP, statistics, monitoring
- [ ] Client-core can start/stop audio playback and capture
- [x] **Type boundaries are respected**: session-core uses session-core types, conversions only at boundaries
- [x] All tests pass
- [x] No breaking changes to existing APIs
- [ ] Documentation is updated

---

## Future Enhancements

### Phase 7: Real Audio Device Integration
- [ ] Integrate `cpal` crate for cross-platform audio
- [ ] Add audio device enumeration
- [ ] Add audio format conversion and resampling
- [ ] Add platform-specific optimizations

### Phase 8: Advanced Audio Features  
- [ ] Audio effects and processing
- [ ] Multiple audio device support
- [ ] Audio stream routing and mixing
- [ ] Low-latency audio optimizations

### Phase 9: Quality and Monitoring
- [ ] Audio quality metrics
- [ ] Audio stream monitoring
- [ ] Adaptive audio parameters
- [ ] Audio diagnostics and debugging

---

## Notes

- **Non-breaking**: All changes should maintain backward compatibility
- **Testable**: Each phase should be independently testable
- **Incremental**: Progress can be made phase by phase
- **Extensible**: Design should support future audio enhancements
- **Type Boundaries**: Critical to maintain clean architecture - session-core should use session-core types throughout, with conversions only at media-core boundaries
- **No Duplication**: ✅ Avoided duplicate integration layers by enhancing existing MediaManager

---

## Progress Tracking

**Overall Progress**: 5/6 phases complete ✅

### Phase Status Summary
- **Phase 1**: ✅ Complete - Make Media-Core AudioFrame Public
- **Phase 2**: ✅ Complete - Add AudioFrame Type to Session-Core  
- **Phase 3**: ✅ Complete - Add AudioFrame Events to Session-Core
- **Phase 4**: ✅ Complete - Extend MediaControl with Audio Stream API
- **Phase 5**: ✅ Complete - Enhanced Existing MediaManager (avoided duplication)
- **Phase 6**: ⏳ Pending - Client-Core Integration

### Current Focus
**Next Task**: Phase 6, Task 6.1 - Add Audio Device Abstraction to Client-Core

### Key Architectural Decision ✅
**Avoided Duplication**: Successfully identified and avoided creating duplicate `MediaControllerIntegration` by enhancing existing `MediaManager` instead. This preserves sophisticated features like zero-copy RTP processing, maintains architectural consistency, and avoids resource conflicts. 