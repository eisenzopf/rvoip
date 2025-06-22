# Session-Core API Enhancements Demonstration

This document describes the API enhancements demonstrated in the updated UAC and UAS examples, based on the REFACTOR.md plan.

## Overview

The updated examples showcase the new extended CallHandler trait methods and improved statistics APIs, providing:
- Rich event callbacks for comprehensive call lifecycle tracking
- Convenient statistics methods for easy monitoring
- Automatic quality alerts based on configurable thresholds
- Better developer experience with less polling

## New CallHandler Callbacks Demonstrated

### 1. `on_call_state_changed`
Provides real-time notifications for all state transitions.

**UAC Example:**
```rust
async fn on_call_state_changed(&self, session_id: &SessionId, old_state: &CallState, new_state: &CallState, reason: Option<&str>) {
    // Track specific transitions
    match (old_state, new_state) {
        (CallState::Trying, CallState::Proceeding) => stats.calls_proceeding += 1,
        (_, CallState::Ringing) => stats.calls_ringing += 1,
        (_, CallState::Failed(_)) => stats.calls_failed += 1,
        _ => {}
    }
}
```

### 2. `on_media_quality`
Automatic quality monitoring with severity levels.

**UAS Example:**
```rust
async fn on_media_quality(&self, session_id: &SessionId, mos_score: f32, packet_loss: f32, alert_level: MediaQualityAlertLevel) {
    let emoji = match alert_level {
        MediaQualityAlertLevel::Good => "🟢",      // MOS >= 4.0
        MediaQualityAlertLevel::Fair => "🟡",      // MOS >= 3.0
        MediaQualityAlertLevel::Poor => "🟠",      // MOS >= 2.0
        MediaQualityAlertLevel::Critical => "🔴",  // MOS < 2.0
    };
    
    if matches!(alert_level, MediaQualityAlertLevel::Poor | MediaQualityAlertLevel::Critical) {
        warn!("⚠️  Poor quality detected on call {}: Consider network optimization", session_id);
    }
}
```

### 3. `on_dtmf`
DTMF digit notifications for IVR and feature codes.

```rust
async fn on_dtmf(&self, session_id: &SessionId, digit: char, duration_ms: u32) {
    match digit {
        '#' => info!("Hash key detected - could trigger special action"),
        '*' => info!("Star key detected - could open menu"),
        _ => {}
    }
}
```

### 4. `on_media_flow`
Media flow start/stop notifications with codec information.

```rust
async fn on_media_flow(&self, session_id: &SessionId, direction: MediaFlowDirection, active: bool, codec: &str) {
    let arrow = match direction {
        MediaFlowDirection::Send => "→",
        MediaFlowDirection::Receive => "←",
        MediaFlowDirection::Both => "↔",
    };
    info!("🎵 Media {} {} using {}", arrow, if active { "started" } else { "stopped" }, codec);
}
```

### 5. `on_warning`
Non-fatal warnings for proactive issue detection.

```rust
async fn on_warning(&self, session_id: Option<&SessionId>, category: WarningCategory, message: &str) {
    match category {
        WarningCategory::Resource => warn!("Consider increasing server resources"),
        WarningCategory::Network => warn!("Check network connectivity"),
        _ => {}
    }
}
```

## New Statistics APIs Demonstrated

### 1. `get_call_statistics`
Single method to get comprehensive call statistics.

**UAC Example:**
```rust
match MediaControl::get_call_statistics(&coordinator, &session_id).await {
    Ok(Some(call_stats)) => {
        info!("Duration: {:?}", call_stats.duration);
        info!("State: {:?}", call_stats.state);
        
        // Media info
        info!("Codec: {}", call_stats.media.codec);
        info!("Media flowing: {}", call_stats.media.media_flowing);
        
        // RTP stats
        info!("Packets sent: {}", call_stats.rtp.packets_sent);
        info!("Bitrate: {} kbps", call_stats.rtp.current_bitrate_kbps);
        
        // Quality metrics
        info!("MOS Score: {:.1}", call_stats.quality.mos_score);
        info!("Packet Loss: {:.1}%", call_stats.quality.packet_loss_rate);
        info!("Acceptable: {}", call_stats.quality.is_acceptable);
    }
}
```

### 2. Convenience Methods
Quick access to specific metrics without parsing full statistics.

```rust
// Get individual metrics
if let Ok(Some(mos)) = MediaControl::get_call_quality_score(&coordinator, session.id()).await {
    info!("📈 Current MOS score: {:.1}", mos);
}

if let Ok(Some(loss)) = MediaControl::get_packet_loss_rate(&coordinator, session.id()).await {
    info!("📉 Current packet loss: {:.1}%", loss);
}

if let Ok(Some(bitrate)) = MediaControl::get_current_bitrate(&coordinator, session.id()).await {
    info!("📶 Current bitrate: {} kbps", bitrate);
}
```

### 3. Quality Monitoring
Automatic quality monitoring with configurable thresholds.

```rust
let thresholds = QualityThresholds {
    min_mos: 3.0,
    max_packet_loss: 5.0,
    max_jitter_ms: 50.0,
    check_interval: Duration::from_secs(5),
};

MediaControl::monitor_call_quality(&coordinator, session.id(), thresholds).await?;
// Will automatically trigger on_media_quality callbacks when thresholds are exceeded
```

## Enhanced Statistics Tracking

Both examples now track comprehensive metrics:

**UAC Stats:**
```rust
struct UacStats {
    calls_initiated: usize,
    calls_connected: usize,
    calls_failed: usize,
    calls_proceeding: usize,    // NEW: Track state transitions
    calls_ringing: usize,       // NEW: Track state transitions
    quality_warnings: usize,    // NEW: Track quality issues
    total_duration: Duration,
}
```

**UAS Stats:**
```rust
struct UasStats {
    calls_received: usize,
    calls_accepted: usize,
    calls_rejected: usize,
    calls_active: usize,
    total_duration: Duration,
    state_changes: usize,       // NEW: Total state changes
    media_flow_events: usize,   // NEW: Media start/stop events
    quality_alerts: usize,      // NEW: Quality threshold breaches
    dtmf_received: usize,       // NEW: DTMF digits received
    warnings_received: usize,   // NEW: System warnings
}
```

## Benefits Demonstrated

1. **No More Polling**: State changes and quality updates are pushed via callbacks
2. **Rich Monitoring**: Comprehensive statistics available with a single call
3. **Automatic Alerts**: Quality monitoring runs in background with configurable thresholds
4. **Better Debugging**: Detailed event tracking helps diagnose issues
5. **Backward Compatible**: Existing code continues to work; new features are opt-in

## Running the Examples

Start the UAS server:
```bash
cargo run --bin uas_server_clean -- --port 5062 --auto-accept true
```

In another terminal, run the UAC client:
```bash
cargo run --bin uac_client_clean -- --port 5061 --target 127.0.0.1:5062 --num-calls 3 --duration 30
```

Watch the enhanced logging showing:
- Real-time state transitions
- Media flow events with codec information
- Quality metrics with visual indicators (🟢🟡🟠🔴)
- Comprehensive statistics updates
- Automatic quality alerts when thresholds are exceeded

## Future Enhancements

The REFACTOR.md plan also includes:
- SDP negotiation strategies for codec selection
- Custom SDP attributes support
- Enhanced conference call support
- Recording and transcoding events

These will be demonstrated in future example updates. 