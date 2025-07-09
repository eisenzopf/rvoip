# Codec Negotiation Fix: Implementation Plan & Progress Tracker

## 📋 Overview

This document tracks the implementation of fixes for the critical codec negotiation issue where the media system always sends PCMU audio regardless of SDP negotiation results. The plan also includes robust handling for unexpected RTP stream formats.

**Issue**: Even when SDP negotiation successfully negotiates Opus codec, the server still sends audio encoded as PCMU.

**Root Cause**: Hardcoded payload type (0 = PCMU) in `MediaSessionController.start_media()` method.

---

## 🎯 Goals

1. **Fix Primary Issue**: Ensure negotiated codecs are properly used for RTP transmission
2. **Add Resilience**: Handle unexpected codec formats gracefully
3. **Maintain Compatibility**: Preserve backward compatibility with existing systems
4. **Add Monitoring**: Provide comprehensive statistics and logging

---

## 📊 Task Status Legend

- ⏳ **Pending**: Not started
- 🔄 **In Progress**: Currently being worked on
- ✅ **Complete**: Task finished and tested
- ❌ **Blocked**: Cannot proceed due to dependencies or issues

---

## 🗂️ Phase 1: Core Codec Mapping Infrastructure

### Task 1: Create Codec Mapping Utilities ✅
**File**: `crates/media-core/src/codec/mapping.rs` (new file)  
**Dependencies**: None  
**Estimated Time**: 4 hours  
**Status**: ✅ Complete

**Implementation Details**:
```rust
// Create bidirectional mapping between codec names and payload types
pub struct CodecMapper {
    name_to_payload: HashMap<String, u8>,
    payload_to_name: HashMap<u8, String>,
}

impl CodecMapper {
    pub fn new() -> Self {
        // Static payload types (RFC 3551)
        // - PCMU (0), PCMA (8), G722 (9), G729 (18)
        // Dynamic payload types
        // - Opus (111)
    }
    
    pub fn codec_to_payload(&self, codec_name: &str) -> Option<u8>
    pub fn payload_to_codec(&self, payload_type: u8) -> Option<String>
    pub fn get_clock_rate(&self, codec_name: &str) -> u32
    pub fn register_dynamic_codec(&mut self, name: String, payload_type: u8)
}
```

**Testing Requirements**:
- [x] Unit tests for all codec mappings
- [x] Test dynamic codec registration
- [x] Test edge cases (unknown codecs, case sensitivity)
- [x] Test clock rate mappings

**Notes**: Foundation for all other codec-related functionality. ✅ **COMPLETED** - All 9 tests passing, including comprehensive coverage of bidirectional mapping, case-insensitive lookup, dynamic registration, and edge cases.

---

## 🔧 Phase 2: Fix RTP Session Configuration

### Task 2: Fix Hardcoded Payload Type in start_media() ⏳
**File**: `crates/media-core/src/relay/controller/mod.rs`  
**Dependencies**: Task 1 (codec_mapping_util)  
**Estimated Time**: 3 hours  
**Status**: ⏳ Pending

**Key Changes**:
```rust
// Before (BROKEN):
payload_type: 0, // Default to PCMU

// After (FIXED):
let payload_type = config.preferred_codec
    .as_ref()
    .and_then(|codec| self.codec_mapper.codec_to_payload(codec))
    .unwrap_or(0); // Default to PCMU

let clock_rate = config.preferred_codec
    .as_ref()
    .map(|codec| self.codec_mapper.get_clock_rate(codec))
    .unwrap_or(8000);
```

**Testing Requirements**:
- [ ] Test with different negotiated codecs (PCMU, PCMA, Opus)
- [ ] Test fallback to PCMU when codec is unknown
- [ ] Verify RTP session uses correct payload type and clock rate
- [ ] Test logging output shows correct codec information

**Notes**: This is the core fix for the primary issue.

---

### Task 3: Add Codec Change Handling to update_media() ⏳
**File**: `crates/media-core/src/relay/controller/mod.rs`  
**Dependencies**: Task 1 (codec_mapping_util)  
**Estimated Time**: 4 hours  
**Status**: ⏳ Pending

**Key Features**:
- Detect codec changes during session updates
- Update RTP session payload type and clock rate
- Emit codec change events
- Handle re-INVITE scenarios

**Testing Requirements**:
- [ ] Test codec changes during active calls
- [ ] Test combined codec + remote address changes
- [ ] Verify events are emitted correctly
- [ ] Test re-INVITE scenarios

**Notes**: Handles dynamic codec changes during active sessions.

---

## 🔍 Phase 3: Dynamic Codec Detection and Fallback

### Task 4: Implement Dynamic Codec Detection ⏳
**File**: `crates/media-core/src/relay/controller/codec_detection.rs` (new file)  
**Dependencies**: Task 1 (codec_mapping_util)  
**Estimated Time**: 6 hours  
**Status**: ⏳ Pending

**Key Components**:
```rust
pub struct CodecDetector {
    mapper: Arc<CodecMapper>,
    detection_cache: Arc<RwLock<HashMap<DialogId, DetectionState>>>,
}

pub enum CodecDetectionResult {
    Expected { payload_type: u8, codec: Option<String> },
    UnexpectedCodec { 
        expected_payload_type: u8,
        detected_payload_type: u8,
        confidence: f32,
    },
}
```

**Testing Requirements**:
- [ ] Test detection of expected payload types
- [ ] Test detection of unexpected payload types
- [ ] Test confidence calculations
- [ ] Test cleanup functionality
- [ ] Test performance with high packet rates

**Notes**: Core component for handling unexpected codec formats.

---

### Task 5: Create Fallback Mechanism ⏳
**File**: `crates/media-core/src/relay/controller/codec_fallback.rs` (new file)  
**Dependencies**: Task 4 (dynamic_codec_detection)  
**Estimated Time**: 8 hours  
**Status**: ⏳ Pending

**Key Features**:
- Transcode between unexpected and expected codecs
- Graceful degradation to passthrough mode
- Statistics tracking for fallback operations
- Error handling and recovery

**Testing Requirements**:
- [ ] Test transcoding between different codec pairs
- [ ] Test fallback to passthrough on transcoding failures
- [ ] Test statistics tracking
- [ ] Test cleanup functionality
- [ ] Test error handling edge cases

**Notes**: Most complex component - handles codec mismatches gracefully.

---

## 🔗 Phase 4: Integration and Statistics

### Task 6: Update Session-Core Integration ⏳
**File**: `crates/session-core/src/media/manager.rs`  
**Dependencies**: Task 2 (rtp_config_fix)  
**Estimated Time**: 3 hours  
**Status**: ⏳ Pending

**Key Changes**:
- Ensure negotiated codecs are properly passed to media-core
- Update MediaConfig conversion to handle codec names
- Verify SDP negotiation results reach RTP layer

**Testing Requirements**:
- [ ] Test codec propagation from SDP negotiation to media-core
- [ ] Test different codec types (PCMU, PCMA, Opus)
- [ ] Test re-INVITE scenarios with codec changes
- [ ] Test integration with existing SIP clients

**Notes**: Critical bridge between SDP negotiation and media processing.

---

### Task 7: Add Payload Type Validation in RTP-Core ⏳
**File**: `crates/media-core/src/integration/rtp_bridge.rs`  
**Dependencies**: Task 4 (dynamic_codec_detection)  
**Estimated Time**: 4 hours  
**Status**: ⏳ Pending

**Key Features**:
- Validate incoming RTP packet payload types
- Integrate with codec detection system
- Handle fallback processing for unexpected formats
- Comprehensive logging and error handling

**Testing Requirements**:
- [ ] Test validation with correct payload types
- [ ] Test handling of unexpected payload types
- [ ] Test fallback mechanism integration
- [ ] Test error handling and logging
- [ ] Test performance impact

**Notes**: Entry point for processing incoming RTP packets.

---

### Task 8: Fix Codec Statistics Tracking ⏳
**File**: `crates/media-core/src/relay/controller/statistics.rs`  
**Dependencies**: Task 2 (rtp_config_fix)  
**Estimated Time**: 2 hours  
**Status**: ⏳ Pending

**Key Changes**:
```rust
// Before (BROKEN):
current_codec: Some("PCMU".to_string()), // Always PCMU

// After (FIXED):
current_codec: Some(current_codec.clone()), // Actual codec
fallback_active: fallback_stats.map(|s| s.transcoding_active).unwrap_or(false),
fallback_success_rate: fallback_stats.map(|s| s.success_rate).unwrap_or(1.0),
```

**Testing Requirements**:
- [ ] Test statistics with different codecs
- [ ] Test fallback statistics tracking
- [ ] Test statistics persistence across codec changes
- [ ] Test performance monitoring integration

**Notes**: Essential for monitoring and debugging codec issues.

---

## 🧪 Phase 5: Testing and Validation

### Task 9: Create Comprehensive Test Suite ⏳
**File**: `crates/media-core/tests/codec_negotiation_integration.rs` (new file)  
**Dependencies**: All previous tasks  
**Estimated Time**: 12 hours  
**Status**: ⏳ Pending

**Test Categories**:
1. **Unit Tests**: Individual component testing
2. **Integration Tests**: End-to-end codec negotiation
3. **Performance Tests**: Codec detection and fallback performance
4. **Stress Tests**: High-load scenarios with codec changes
5. **Compatibility Tests**: Backward compatibility verification

**Test Coverage Requirements**:
- [ ] `test_pcmu_to_opus_negotiation()`
- [ ] `test_unexpected_codec_fallback()`
- [ ] `test_codec_mapper_bidirectional()`
- [ ] `test_dynamic_codec_registration()`
- [ ] `test_fallback_performance()`
- [ ] `test_statistics_accuracy()`
- [ ] `test_concurrent_codec_changes()`
- [ ] `test_error_recovery()`

**Notes**: Critical for ensuring stability and correctness.

---

## 🛡️ Risk Mitigation & Rollback Plan

### Potential Risks:
1. **Performance Impact**: Codec detection and transcoding may increase CPU usage
2. **Compatibility Issues**: Changes might break existing integrations
3. **Memory Usage**: Codec detection caches and transcoding buffers
4. **Transcoding Quality**: Audio quality degradation during fallback

### Mitigation Strategies:
1. **Feature Flags**: Implement fallback handling as optional feature
2. **Gradual Rollout**: Deploy codec mapping fixes before fallback features
3. **Monitoring**: Add comprehensive logging and metrics
4. **Graceful Degradation**: Ensure system works even if new features fail
5. **Performance Budgets**: Set limits on transcoding operations

### Rollback Plan:
1. **Configuration Rollback**: Add config flag to disable new codec handling
2. **Code Rollback**: Maintain backward compatibility for 1 version
3. **Data Rollback**: Ensure statistics format is backward compatible
4. **Emergency Disable**: Quick way to disable fallback mechanisms

---

## 📅 Implementation Timeline

| Phase | Timeline | Tasks | Status |
|-------|----------|--------|--------|
| **Week 1** | Phase 1-2 | Tasks 1-2 | ⏳ Pending |
| **Week 2** | Phase 2-3 | Tasks 3-4 | ⏳ Pending |
| **Week 3** | Phase 3-4 | Tasks 5-6 | ⏳ Pending |
| **Week 4** | Phase 4 | Tasks 7-8 | ⏳ Pending |
| **Week 5** | Phase 5 | Task 9 | ⏳ Pending |

---

## 📝 Progress Log

### 2024-12-28 - Task 1 - ✅ COMPLETED
**Create Codec Mapping Utilities**: Successfully implemented `CodecMapper` with bidirectional mapping between codec names and payload types. Added comprehensive test suite (9 tests) covering:
- Static codec mappings (PCMU, PCMA, G722, G729)
- Dynamic codec registration (Opus)
- Case-insensitive lookups
- Clock rate mapping with fallbacks
- Codec capability information
- Edge case handling

All 116 tests in media-core continue to pass. Foundation ready for next phase.

---

## 🔧 Configuration Changes

### New Configuration Options:
```rust
// Add to MediaEngineConfig
pub struct CodecConfig {
    pub enable_dynamic_detection: bool,
    pub enable_fallback_transcoding: bool,
    pub fallback_passthrough_threshold: f32,
    pub detection_confidence_threshold: f32,
}
```

### Environment Variables:
- `RVOIP_CODEC_FALLBACK_ENABLED`: Enable/disable fallback mechanisms
- `RVOIP_CODEC_DETECTION_ENABLED`: Enable/disable dynamic codec detection
- `RVOIP_CODEC_STATS_ENABLED`: Enable/disable detailed codec statistics

---

## 📚 Documentation Updates Required

- [ ] Update API documentation for new codec handling
- [ ] Add troubleshooting guide for codec issues
- [ ] Document configuration options
- [ ] Update examples with codec negotiation scenarios
- [ ] Add performance tuning guide

---

## 🚀 Post-Implementation Tasks

- [ ] Monitor production metrics for codec usage
- [ ] Collect feedback from developers using the system
- [ ] Optimize performance based on real-world usage
- [ ] Consider adding support for additional codecs
- [ ] Plan for future codec-related features

---

**Last Updated**: [Current Date]  
**Next Review**: [Schedule next review] 