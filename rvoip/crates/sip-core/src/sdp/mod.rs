// Session Description Protocol (SDP) module
//
// This module contains parsers and types for working with SDP messages
// as defined in RFC 8866. It also provides utilities for creating SDP messages,
// both through helper functions (sdp_macros) and declarative Rust macros (new_macros).

pub mod parser;
pub mod session;
pub mod media;
pub mod attributes;
pub mod macros;
pub mod builder;

#[cfg(test)]
mod tests;

pub use parser::parse_sdp;
pub use parser::validate_sdp;
pub use macros::*;
pub use builder::SdpBuilder;

// For backward compatibility
pub mod media_parser {
    pub use crate::sdp::media::*;
}

// For backward compatibility
pub mod session_parser {
    pub use crate::sdp::session::*;
}

// For backward compatibility
pub mod time_parser {
    pub use crate::sdp::parser::time_parser::*;
} 