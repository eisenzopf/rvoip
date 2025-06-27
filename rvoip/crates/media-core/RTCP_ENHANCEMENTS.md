# RTCP Statistics Enhancement Plan for media-core

## Overview

This document outlines the plan to expose comprehensive RTCP/RTP statistics from rtp-core through media-core to session-core, enabling real-time monitoring of call quality metrics.

## Architecture

```
┌─────────────┐     ┌─────────────┐     ┌──────────────┐
│  rtp-core   │────▶│ media-core  │────▶│ session-core │
├─────────────┤     ├─────────────┤     ├──────────────┤
│RtpSession   │     │MediaSession │     │MediaControl  │
│ - get_stats │     │Controller   │     │ - get_stats  │
│RtpSessionStats    │ - expose stats     │ - monitor    │
└─────────────┘     └─────────────┘     └──────────────┘
```

## Goals

1. **Expose RTP/RTCP Statistics**: Make rtp-core's RtpSessionStats available through media-core
2. **Quality Metrics**: Calculate quality indicators (MOS, packet loss %, jitter)
3. **Real-time Monitoring**: Enable periodic statistics collection and events
4. **API Integration**: Extend MediaControl trait in session-core
5. **Example Usage**: Update client/server examples to log statistics

## Task List

### Phase 1: Core Types and Imports
- [ ] **Task 1.1**: Import RtpSessionStats and RtpStreamStats in media-core/src/lib.rs
- [ ] **Task 1.2**: Re-export statistics types in public API
- [ ] **Task 1.3**: Add statistics types to prelude module
- [ ] **Task 1.4**: Create media-core/src/types/stats.rs with MediaStatistics struct
- [ ] **Task 1.5**: Define MediaProcessingStats structure
- [ ] **Task 1.6**: Define enhanced QualityMetrics structure with RTCP data

### Phase 2: MediaSessionController Enhancement
- [ ] **Task 2.1**: Add get_rtp_statistics() method to MediaSessionController
- [ ] **Task 2.2**: Add get_stream_statistics() method for per-stream stats
- [ ] **Task 2.3**: Add get_media_statistics() method for comprehensive stats
- [ ] **Task 2.4**: Implement calculate_mos_from_stats() helper function
- [ ] **Task 2.5**: Implement calculate_network_quality() helper function
- [ ] **Task 2.6**: Update MediaSessionInfo to include rtp_stats field
- [ ] **Task 2.7**: Update get_session_info() to populate statistics

### Phase 3: Event System Enhancement
- [ ] **Task 3.1**: Add StatisticsUpdated variant to MediaSessionEvent
- [ ] **Task 3.2**: Add QualityDegraded variant for quality alerts
- [ ] **Task 3.3**: Implement start_statistics_monitoring() method
- [ ] **Task 3.4**: Create background task for periodic statistics collection
- [ ] **Task 3.5**: Implement quality threshold detection logic

### Phase 4: Session-Core Integration
- [ ] **Task 4.1**: Add get_rtp_statistics() to MediaManager
- [ ] **Task 4.2**: Add get_media_statistics() to MediaManager
- [ ] **Task 4.3**: Extend MediaControl trait with statistics methods
- [ ] **Task 4.4**: Implement MediaControl trait methods in SessionCoordinator
- [ ] **Task 4.5**: Add start_statistics_monitoring() to MediaControl trait

### Phase 5: API Types Enhancement
- [ ] **Task 5.1**: Update MediaInfo struct in session-core to include statistics
- [ ] **Task 5.2**: Create RtpStatistics wrapper type for API consistency
- [ ] **Task 5.3**: Add quality metrics to session-core API types
- [ ] **Task 5.4**: Document new API methods and types

### Phase 6: Example Updates
- [ ] **Task 6.1**: Update uac_client.rs to log RTP statistics
- [ ] **Task 6.2**: Update uas_server.rs to log RTP statistics
- [ ] **Task 6.3**: Add statistics monitoring spawn task in examples
- [ ] **Task 6.4**: Format statistics output for readability
- [ ] **Task 6.5**: Add quality degradation alerts to examples

### Phase 7: Testing
- [ ] **Task 7.1**: Unit tests for statistics calculation functions
- [ ] **Task 7.2**: Integration tests for statistics flow
- [ ] **Task 7.3**: Test quality metric calculations
- [ ] **Task 7.4**: Test event generation for quality degradation
- [ ] **Task 7.5**: Performance tests for statistics collection overhead

### Phase 8: Documentation
- [ ] **Task 8.1**: Update media-core README with statistics API
- [ ] **Task 8.2**: Update session-core README with statistics usage
- [ ] **Task 8.3**: Add inline documentation for all new methods
- [ ] **Task 8.4**: Create statistics interpretation guide
- [ ] **Task 8.5**: Document quality thresholds and MOS calculation

## Implementation Details

### MediaStatistics Structure
```rust
pub struct MediaStatistics {
    pub session_id: MediaSessionId,
    pub dialog_id: DialogId,
    pub rtp_stats: Option<RtpSessionStats>,
    pub stream_stats: Vec<RtpStreamStats>,
    pub media_stats: MediaProcessingStats,
    pub quality_metrics: Option<QualityMetrics>,
    pub session_start: Instant,
    pub session_duration: Duration,
}
```

### Quality Metrics
```rust
pub struct QualityMetrics {
    pub packet_loss_percent: f32,
    pub jitter_ms: f64,
    pub rtt_ms: Option<f64>,
    pub mos_score: Option<f32>,
    pub network_quality: u8,
}
```

### MOS Score Calculation
- Based on E-model (simplified)
- Range: 1.0 (bad) to 5.0 (excellent)
- Factors: packet loss, jitter, codec type

### Quality Thresholds
- **Excellent**: Loss < 0.5%, Jitter < 20ms, MOS > 4.3
- **Good**: Loss < 1%, Jitter < 30ms, MOS > 4.0
- **Fair**: Loss < 3%, Jitter < 50ms, MOS > 3.5
- **Poor**: Loss < 5%, Jitter < 100ms, MOS > 2.5
- **Bad**: Loss >= 5%, Jitter >= 100ms, MOS <= 2.5

## Dependencies

- rtp-core: Already integrated, provides RtpSessionStats
- No new external dependencies required

## Migration Path

1. Changes are additive - no breaking changes
2. Existing code continues to work
3. New statistics API is opt-in
4. Examples show best practices

## Performance Considerations

- Statistics collection is lightweight (already tracked by rtp-core)
- Monitoring interval configurable (default: 5 seconds)
- Event-driven updates minimize overhead
- No impact on media path performance

## Future Enhancements

1. **RTCP XR Support**: Extended reports for detailed metrics
2. **Historical Statistics**: Time-series data storage
3. **Predictive Quality**: ML-based quality prediction
4. **SIP Integration**: Statistics in SIP headers/SDP
5. **WebRTC Stats**: Compatibility with WebRTC statistics API

## Success Criteria

- [ ] RTP statistics accessible from session-core
- [ ] Quality metrics calculated and available
- [ ] Examples successfully log statistics
- [ ] No performance regression
- [ ] All tests passing
- [ ] Documentation complete

## Timeline Estimate

- Phase 1-2: 2 days (Core implementation)
- Phase 3-4: 2 days (Integration)
- Phase 5-6: 1 day (API and examples)
- Phase 7-8: 2 days (Testing and documentation)

**Total: ~1 week of development**

## Notes

- Priority: High - Required for production monitoring
- Risk: Low - Additive changes only
- Testing: Comprehensive test coverage required 