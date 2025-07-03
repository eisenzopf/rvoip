//! Buffer statistics
//!
//! This module handles statistics and monitoring of the high-performance
//! transmit buffer, including buffer fullness, packet counts, and congestion metrics.

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn, info};

use crate::api::common::error::MediaTransportError;
use crate::buffer::{TransmitBuffer, TransmitBufferStats};

/// Get current transmit buffer statistics
///
/// This function returns information about the state of the transmit buffer, including
/// packet counts, buffer fullness, and congestion control metrics.
pub async fn get_transmit_buffer_stats(
    high_performance_buffers_enabled: bool,
    transmit_buffer: &Arc<RwLock<Option<TransmitBuffer>>>,
) -> Result<TransmitBufferStats, MediaTransportError> {
    // Placeholder for the extracted get_transmit_buffer_stats functionality
    if high_performance_buffers_enabled {
        let tx_buffer_guard = transmit_buffer.read().await;
        if let Some(tx_buffer) = tx_buffer_guard.as_ref() {
            // get_stats is not an async method, so no need to await
            let stats = tx_buffer.get_stats();
            debug!("Retrieved transmit buffer stats: {} packets queued, {:.1}% full", 
                   stats.packets_queued, stats.buffer_fullness * 100.0);
            return Ok(stats);
        }
    }
    
    // Return default stats if not enabled or not available
    debug!("No transmit buffer available, returning default stats");
    Ok(TransmitBufferStats::default())
} 