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

**Status**: ⏳ Pending  
**Goal**: Create session-core AudioFrame wrapper with conversions

### Task 2.1: Add AudioFrame to Session-Core Types
- [ ] **File**: `crates/session-core/src/api/types.rs`
- [ ] **Action**: Add AudioFrame, AudioStreamConfig types
- [ ] **Implementation**:
  - [ ] AudioFrame struct with samples, sample_rate, channels, timestamp
  - [ ] AudioStreamConfig struct with codec configuration  
  - [ ] Utility methods (duration_ms, samples_per_channel)
  - [ ] From/Into conversions with media-core::AudioFrame

### Task 2.2: Test Session-Core AudioFrame Access
- [ ] **File**: `crates/session-core/tests/audio_frame_integration.rs`
- [ ] **Action**: Test AudioFrame creation and conversion
- [ ] **Tests**:
  - [ ] `test_session_audio_frame_creation()` - Basic creation
  - [ ] `test_audio_frame_conversion_media_to_session()` - Media→Session conversion
  - [ ] `test_audio_frame_conversion_session_to_media()` - Session→Media conversion  
  - [ ] `test_audio_frame_utility_methods()` - Helper methods
- [ ] **Verification**: Run `cargo test -p rvoip-session-core audio_frame_integration`

---

## Phase 3: Add AudioFrame Events to Session-Core

**Status**: ⏳ Pending  
**Goal**: Extend event system with audio-specific events

### Task 3.1: Extend SessionEvent with Audio Events
- [ ] **File**: `crates/session-core/src/manager/events.rs`
- [ ] **Action**: Add audio event variants to SessionEvent enum
- [ ] **Events to Add**:
  - [ ] `AudioFrameReceived` - Decoded frame for playback
  - [ ] `AudioFrameRequested` - Request frame for capture
  - [ ] `AudioStreamConfigChanged` - Configuration changed
  - [ ] `AudioStreamStarted` - Stream started
  - [ ] `AudioStreamStopped` - Stream stopped

### Task 3.2: Add Event Publishing Helper Methods
- [ ] **File**: `crates/session-core/src/manager/events.rs`
- [ ] **Action**: Add helper methods to SessionEventProcessor
- [ ] **Methods to Add**:
  - [ ] `publish_audio_frame_received()`
  - [ ] `publish_audio_frame_requested()`
  - [ ] `publish_audio_stream_config_changed()`
  - [ ] `publish_audio_stream_started()`
  - [ ] `publish_audio_stream_stopped()`

### Task 3.3: Test Audio Events
- [ ] **File**: `crates/session-core/tests/audio_events_test.rs`
- [ ] **Action**: Test audio event publishing and receiving
- [ ] **Tests**:
  - [ ] `test_audio_frame_received_event()` - Publish/receive frame event
  - [ ] `test_audio_frame_requested_event()` - Publish/receive request event
  - [ ] `test_audio_stream_config_changed_event()` - Config change event
  - [ ] `test_audio_stream_lifecycle_events()` - Start/stop events
- [ ] **Verification**: Run `cargo test -p rvoip-session-core audio_events_test`

---

## Phase 4: Extend MediaControl with Audio Stream API

**Status**: ⏳ Pending  
**Goal**: Add audio streaming methods to MediaControl trait

### Task 4.1: Add Audio Frame Subscriber Type
- [ ] **File**: `crates/session-core/src/api/types.rs`
- [ ] **Action**: Add AudioFrameSubscriber for streaming
- [ ] **Implementation**:
  - [ ] AudioFrameSubscriber struct with mpsc::Receiver
  - [ ] `recv()`, `try_recv()`, `session_id()` methods

### Task 4.2: Extend MediaControl Trait
- [ ] **File**: `crates/session-core/src/api/media.rs`
- [ ] **Action**: Add audio streaming methods to MediaControl trait
- [ ] **Methods to Add**:
  - [ ] `subscribe_to_audio_frames()` - Get frame subscriber
  - [ ] `send_audio_frame()` - Send frame for encoding
  - [ ] `get_audio_stream_config()` - Get stream config
  - [ ] `set_audio_stream_config()` - Set stream config
  - [ ] `start_audio_stream()` - Start stream
  - [ ] `stop_audio_stream()` - Stop stream

### Task 4.3: Implement MediaControl Audio Methods
- [ ] **File**: `crates/session-core/src/api/media.rs`
- [ ] **Action**: Add implementation for SessionCoordinator
- [ ] **Implementation**:
  - [ ] Placeholder implementations that validate sessions
  - [ ] Event publishing for stream lifecycle
  - [ ] Error handling for non-existent sessions

### Task 4.4: Test MediaControl Audio API
- [ ] **File**: `crates/session-core/tests/media_control_audio_test.rs`
- [ ] **Action**: Test the new audio streaming API
- [ ] **Tests**:
  - [ ] `test_audio_frame_subscriber_creation()` - Create subscriber
  - [ ] `test_send_audio_frame_placeholder()` - Send frame validation
  - [ ] `test_audio_stream_config()` - Config get/set
  - [ ] `test_audio_stream_lifecycle()` - Start/stop streams
- [ ] **Verification**: Run `cargo test -p rvoip-session-core media_control_audio_test`

---

## Phase 5: Media-Core Integration (Revisited from Phase 1)

**Status**: ⏳ Pending  
**Goal**: Add callback support to MediaSessionController

### Task 5.1: Add Audio Frame Callback to MediaSessionController
- [ ] **File**: `crates/media-core/src/relay/controller/mod.rs`
- [ ] **Action**: Add callback support for audio frames
- [ ] **Implementation**:
  - [ ] Add `audio_frame_callbacks` field to MediaSessionController
  - [ ] `set_audio_frame_callback()` method
  - [ ] `remove_audio_frame_callback()` method
  - [ ] `send_audio_frame()` method for transmission
  - [ ] Integration with RTP processing pipeline

### Task 5.2: Test Media-Core Callback Integration
- [ ] **File**: `crates/media-core/tests/audio_callback_test.rs`
- [ ] **Action**: Test callback functionality
- [ ] **Tests**:
  - [ ] Test callback registration/removal
  - [ ] Test audio frame forwarding
  - [ ] Test multiple callback scenarios

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
5. **Phase 5**: Integration testing with real media-core callbacks
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

- [ ] Media-core AudioFrame is public and accessible
- [ ] Session-core has its own AudioFrame type with conversions
- [ ] Audio events can be published and received
- [ ] MediaControl API supports audio streaming
- [ ] Client-core can start/stop audio playback and capture
- [ ] All tests pass
- [ ] No breaking changes to existing APIs
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

---

## Progress Tracking

**Overall Progress**: 1/6 phases complete

### Phase Status Summary
- **Phase 1**: ✅ Complete - Make Media-Core AudioFrame Public
- **Phase 2**: ⏳ Pending - Add AudioFrame Type to Session-Core  
- **Phase 3**: ⏳ Pending - Add AudioFrame Events to Session-Core
- **Phase 4**: ⏳ Pending - Extend MediaControl with Audio Stream API
- **Phase 5**: ⏳ Pending - Media-Core Integration
- **Phase 6**: ⏳ Pending - Client-Core Integration

### Current Focus
**Next Task**: Phase 2, Task 2.1 - Add AudioFrame to Session-Core Types 