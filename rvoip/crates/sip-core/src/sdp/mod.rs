pub mod parser;
pub mod attributes;
pub mod tests;

pub use parser::parse_sdp;
pub use attributes::{MediaDirection, parse_rid, parse_simulcast, parse_ice_options, 
                    parse_end_of_candidates, parse_sctp_port, parse_max_message_size,
                    parse_sctpmap, validate_attributes};

// Placeholder for SDP parsing logic 