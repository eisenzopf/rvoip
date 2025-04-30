//! SDP parsing and validation according to RFC 8866
//!
//! This module provides the parser implementation for SDP messages.
//! It's organized into smaller components for maintainability.

mod line_parser;
mod validation;
mod session_parser;
mod attribute_parser;
mod media_parser;
pub mod time_parser;
mod sdp_parser;

// Re-export the parsing functions 
pub use self::line_parser::parse_sdp_line;
pub use self::validation::validate_sdp;
pub use self::attribute_parser::parse_attribute;
pub use self::media_parser::parse_media_description_line;
pub use self::time_parser::{parse_time_description_line, parse_repeat_time_line, parse_time_with_unit};
pub use self::sdp_parser::parse_sdp;

use crate::error::{Error, Result};
use crate::types::sdp::{SdpSession, Origin, ConnectionData, MediaDescription, 
                      TimeDescription, ParsedAttribute};
use crate::types::MediaType;
use crate::sdp::attributes::MediaDirection;
// Use validation functions from our own module
use self::validation::{is_valid_address, is_valid_hostname,
                      is_valid_ipv4, is_valid_ipv6,
                      validate_network_type, validate_address_type};
use bytes::Bytes;
use std::str;