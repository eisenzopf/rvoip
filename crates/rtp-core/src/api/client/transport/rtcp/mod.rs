//! RTCP components
//!
//! This module contains functionality related to RTCP (RTP Control Protocol):
//! - Sender and receiver reports
//! - Application-defined packets
//! - Goodbye packets
//! - Extended reports (XR)

// Re-export modules
pub mod reports;
pub mod app_packets;

// Re-export important types and functions
pub use reports::{
    send_rtcp_receiver_report, send_rtcp_sender_report,
    get_rtcp_stats, set_rtcp_interval
};

pub use app_packets::{
    send_rtcp_app, send_rtcp_bye, send_rtcp_xr_voip_metrics
}; 