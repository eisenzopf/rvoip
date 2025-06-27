//! Media handling components
//!
//! This module contains media-specific functionality for the transport client:
//! - Media synchronization between streams
//! - CSRC (Contributing Source) management
//! - RTP header extensions
//! - SSRC demultiplexing

// Re-export modules
pub mod sync;
pub mod csrc;
pub mod extensions;
pub mod ssrc;

// Re-export important types and functions
pub use sync::{
    enable_media_sync, is_media_sync_enabled, register_sync_stream,
    set_sync_reference_stream, get_sync_info, get_all_sync_info,
    convert_timestamp, rtp_to_ntp, ntp_to_rtp, get_clock_drift_ppm,
    are_streams_synchronized
};

pub use csrc::{
    is_csrc_management_enabled, enable_csrc_management,
    add_csrc_mapping, add_simple_csrc_mapping, remove_csrc_mapping_by_ssrc,
    get_csrc_mapping_by_ssrc, get_all_csrc_mappings, get_active_csrcs
};

pub use extensions::{
    is_header_extensions_enabled, enable_header_extensions,
    configure_header_extension, configure_header_extensions,
    add_header_extension, add_audio_level_extension,
    add_video_orientation_extension, add_transport_cc_extension,
    get_received_header_extensions, get_received_audio_level,
    get_received_video_orientation, get_received_transport_cc
};

pub use ssrc::{
    is_ssrc_demultiplexing_enabled, register_ssrc,
    get_sequence_number, get_all_ssrcs
}; 