// Session Description Protocol (SDP) module
//
// This module contains parsers and types for working with SDP messages
// as defined in RFC 8866.

pub mod parser;
pub mod session;
pub mod time_parser;
pub mod media;
pub mod attributes;
pub mod sdp_macros;

#[cfg(test)]
mod tests;

pub use parser::parse_sdp;
pub use sdp_macros::*;

// For backward compatibility
pub mod media_parser {
    pub use crate::sdp::media::*;
}

// For backward compatibility
pub mod session_parser {
    pub use crate::sdp::session::*;
} 