//! Media synchronization functionality
//!
//! This module handles synchronization of multiple media streams, including
//! timestamp conversion, clock drift measurement, and reference stream management.

use std::sync::Arc;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, warn, info};

use crate::api::common::error::MediaTransportError;
use crate::packet::rtcp::NtpTimestamp;
use crate::api::client::transport::MediaSyncInfo;

/// Enable media synchronization
///
/// This function enables the media synchronization feature if it was not enabled
/// in the configuration.
pub async fn enable_media_sync(
    media_sync_enabled: &Arc<std::sync::atomic::AtomicBool>,
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
    ssrc: u32,
    clock_rate: u32,
) -> Result<bool, MediaTransportError> {
    // Placeholder for the extracted enable_media_sync functionality
    // We would extract this from client_transport_impl.rs
    if media_sync_enabled.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok(true);
    }
    
    // Create media sync context if it doesn't exist
    let mut sync_guard = media_sync.write().await;
    if sync_guard.is_none() {
        *sync_guard = Some(crate::sync::MediaSync::new());
    }
    
    // Set enabled flag
    media_sync_enabled.store(true, std::sync::atomic::Ordering::SeqCst);
    
    // Register default stream
    if let Some(sync) = &mut *sync_guard {
        sync.register_stream(ssrc, clock_rate);
    }
    
    Ok(true)
}

/// Check if media synchronization is enabled
pub fn is_media_sync_enabled(
    media_sync_enabled: &Arc<std::sync::atomic::AtomicBool>,
) -> bool {
    media_sync_enabled.load(std::sync::atomic::Ordering::SeqCst)
}

/// Register a stream for synchronization
pub async fn register_sync_stream(
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
    ssrc: u32,
    clock_rate: u32,
) -> Result<(), MediaTransportError> {
    let mut sync_guard = media_sync.write().await;
    if let Some(sync) = &mut *sync_guard {
        sync.register_stream(ssrc, clock_rate);
        debug!("Registered sync stream: SSRC={:08x}, clock_rate={}", ssrc, clock_rate);
        Ok(())
    } else {
        Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
    }
}

/// Set the reference stream for synchronization
pub async fn set_sync_reference_stream(
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
    ssrc: u32,
) -> Result<(), MediaTransportError> {
    let mut sync_guard = media_sync.write().await;
    if let Some(sync) = &mut *sync_guard {
        sync.set_reference_stream(ssrc);
        debug!("Set sync reference stream: SSRC={:08x}", ssrc);
        Ok(())
    } else {
        Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
    }
}

/// Get synchronization information for a stream
pub async fn get_sync_info(
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
    ssrc: u32,
) -> Result<Option<MediaSyncInfo>, MediaTransportError> {
    let sync_guard = media_sync.read().await;
    if let Some(sync) = sync_guard.as_ref() {
        // Get the stream data from core MediaSync
        let streams = sync.get_streams();
        if let Some(stream_data) = streams.get(&ssrc) {
            // Convert core StreamSyncData to API MediaSyncInfo
            let sync_info = MediaSyncInfo {
                ssrc: stream_data.ssrc,
                clock_rate: stream_data.clock_rate,
                last_ntp: stream_data.last_ntp,
                last_rtp: stream_data.last_rtp,
                clock_drift_ppm: stream_data.clock_drift_ppm,
            };
            debug!("Retrieved sync info for SSRC={:08x}: drift={:.2} PPM", ssrc, sync_info.clock_drift_ppm);
            Ok(Some(sync_info))
        } else {
            debug!("No sync info found for SSRC={:08x}", ssrc);
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

/// Get synchronization information for all registered streams
pub async fn get_all_sync_info(
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
) -> Result<HashMap<u32, MediaSyncInfo>, MediaTransportError> {
    let sync_guard = media_sync.read().await;
    if let Some(sync) = sync_guard.as_ref() {
        let mut result = HashMap::new();
        
        // Convert all streams from core MediaSync to API MediaSyncInfo
        for (ssrc, stream_data) in sync.get_streams() {
            let sync_info = MediaSyncInfo {
                ssrc: *ssrc,
                clock_rate: stream_data.clock_rate,
                last_ntp: stream_data.last_ntp,
                last_rtp: stream_data.last_rtp,
                clock_drift_ppm: stream_data.clock_drift_ppm,
            };
            result.insert(*ssrc, sync_info);
        }
        
        debug!("Retrieved sync info for {} registered streams", result.len());
        Ok(result)
    } else {
        Ok(HashMap::new())
    }
}

/// Convert an RTP timestamp from one stream to the equivalent timestamp in another stream
pub async fn convert_timestamp(
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
    from_ssrc: u32,
    to_ssrc: u32,
    rtp_ts: u32,
) -> Result<Option<u32>, MediaTransportError> {
    let sync_guard = media_sync.read().await;
    if let Some(sync) = sync_guard.as_ref() {
        let result = sync.convert_timestamp(from_ssrc, to_ssrc, rtp_ts);
        if result.is_some() {
            debug!("Converted timestamp {} from SSRC={:08x} to SSRC={:08x}: result={:?}", 
                   rtp_ts, from_ssrc, to_ssrc, result);
        } else {
            debug!("Failed to convert timestamp {} from SSRC={:08x} to SSRC={:08x} - insufficient sync data", 
                   rtp_ts, from_ssrc, to_ssrc);
        }
        Ok(result)
    } else {
        Ok(None)
    }
}

/// Convert an RTP timestamp to an NTP timestamp
pub async fn rtp_to_ntp(
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
    ssrc: u32,
    rtp_ts: u32,
) -> Result<Option<NtpTimestamp>, MediaTransportError> {
    let sync_guard = media_sync.read().await;
    if let Some(sync) = sync_guard.as_ref() {
        let result = sync.rtp_to_ntp(ssrc, rtp_ts);
        debug!("Converted RTP timestamp {} to NTP for SSRC={:08x}: success={}", 
               rtp_ts, ssrc, result.is_some());
        Ok(result)
    } else {
        Ok(None)
    }
}

/// Convert an NTP timestamp to an RTP timestamp
pub async fn ntp_to_rtp(
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
    ssrc: u32,
    ntp: NtpTimestamp,
) -> Result<Option<u32>, MediaTransportError> {
    let sync_guard = media_sync.read().await;
    if let Some(sync) = sync_guard.as_ref() {
        let result = sync.ntp_to_rtp(ssrc, ntp);
        debug!("Converted NTP timestamp to RTP for SSRC={:08x}: success={}", 
               ssrc, result.is_some());
        Ok(result)
    } else {
        Ok(None)
    }
}

/// Get clock drift for a stream in parts per million
pub async fn get_clock_drift_ppm(
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
    ssrc: u32,
) -> Result<Option<f64>, MediaTransportError> {
    let sync_guard = media_sync.read().await;
    if let Some(sync) = sync_guard.as_ref() {
        let drift = sync.get_clock_drift_ppm(ssrc);
        if let Some(drift_val) = drift {
            debug!("Clock drift for SSRC={:08x}: {:.2} PPM", ssrc, drift_val);
        }
        Ok(drift)
    } else {
        Ok(None)
    }
}

/// Check if two streams are sufficiently synchronized
pub async fn are_streams_synchronized(
    media_sync: &Arc<RwLock<Option<crate::sync::MediaSync>>>,
    ssrc1: u32,
    ssrc2: u32,
    tolerance_ms: f64,
) -> Result<bool, MediaTransportError> {
    let sync_guard = media_sync.read().await;
    if let Some(sync) = sync_guard.as_ref() {
        let synchronized = sync.are_synchronized(ssrc1, ssrc2, tolerance_ms);
        debug!("Streams synchronized check: SSRC1={:08x}, SSRC2={:08x}, tolerance={}ms, result={}", 
               ssrc1, ssrc2, tolerance_ms, synchronized);
        Ok(synchronized)
    } else {
        Ok(false)
    }
} 