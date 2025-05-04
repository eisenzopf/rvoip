//! # SDP Prelude
//!
//! This module re-exports common SDP types and functions.
//!
//! This is typically imported with `use rvoip_sip_core::sdp_prelude::*;`

pub use crate::types::sdp::{SdpSession, Origin, ConnectionData, TimeDescription, MediaDescription};
pub use crate::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute, CandidateAttribute, SsrcAttribute, RepeatTime};
pub use crate::sdp::attributes::MediaDirection;
pub use crate::sdp::attributes::rid::{RidAttribute, RidDirection};
pub use crate::sdp::parser::{
    validate_sdp,
    validate_network_type,
    validate_address_type,
    is_valid_address,
    is_valid_ipv4,
    is_valid_ipv6,
    is_valid_hostname,
    parse_bandwidth_line,
    parse_sdp
};
pub use crate::sdp::builder::SdpBuilder;
pub use crate::sdp;  // For the macro 