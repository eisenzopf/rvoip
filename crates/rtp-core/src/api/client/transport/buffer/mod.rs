//! Buffer management components
//!
//! This module contains functionality related to high-performance buffer management:
//! - Transmit buffer for outgoing packets
//! - Buffer statistics and monitoring
//! - Packet priority handling

// Re-export modules
pub mod transmit;
pub mod stats;

// Re-export important types and functions
pub use transmit::{
    init_transmit_buffer, send_frame_with_priority,
    update_transmit_buffer_config, set_priority_threshold
};

pub use stats::{
    get_transmit_buffer_stats
}; 