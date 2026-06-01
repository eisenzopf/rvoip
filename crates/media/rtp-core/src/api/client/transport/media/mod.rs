//! Media handling components
//!
//! This module contains media-specific functionality for the transport client:
//! - Media synchronization between streams
//! - CSRC (Contributing Source) management
//! - RTP header extensions
//! - SSRC demultiplexing

// Re-export modules
pub mod csrc;
pub mod extensions;
pub mod ssrc;
pub mod sync;

// Re-export important types and functions
pub use sync::{
    are_streams_synchronized, convert_timestamp, enable_media_sync, get_all_sync_info,
    get_clock_drift_ppm, get_sync_info, is_media_sync_enabled, ntp_to_rtp, register_sync_stream,
    rtp_to_ntp, set_sync_reference_stream,
};

pub use csrc::{
    add_csrc_mapping, add_simple_csrc_mapping, enable_csrc_management, get_active_csrcs,
    get_all_csrc_mappings, get_csrc_mapping_by_ssrc, is_csrc_management_enabled,
    remove_csrc_mapping_by_ssrc,
};

pub use extensions::{
    add_audio_level_extension, add_header_extension, add_transport_cc_extension,
    add_video_orientation_extension, configure_header_extension, configure_header_extensions,
    enable_header_extensions, get_received_audio_level, get_received_header_extensions,
    get_received_transport_cc, get_received_video_orientation, is_header_extensions_enabled,
};

pub use ssrc::{get_all_ssrcs, get_sequence_number, is_ssrc_demultiplexing_enabled, register_ssrc};
