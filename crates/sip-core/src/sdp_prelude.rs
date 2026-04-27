//! # SDP Prelude
//!
//! This module re-exports common SDP types and functions.
//!
//! This is typically imported with `use rvoip_sip_core::sdp_prelude::*;`

pub use crate::sdp;
pub use crate::sdp::attributes::rid::{RidAttribute, RidDirection};
pub use crate::sdp::attributes::MediaDirection;
pub use crate::sdp::builder::SdpBuilder;
pub use crate::sdp::parser::{
    is_valid_address, is_valid_hostname, is_valid_ipv4, is_valid_ipv6, parse_bandwidth_line,
    parse_sdp, validate_address_type, validate_network_type, validate_sdp,
};
pub use crate::types::sdp::{
    CandidateAttribute, FmtpAttribute, ParsedAttribute, RepeatTime, RtpMapAttribute, SsrcAttribute,
};
pub use crate::types::sdp::{
    ConnectionData, MediaDescription, Origin, SdpSession, TimeDescription,
}; // For the macro
