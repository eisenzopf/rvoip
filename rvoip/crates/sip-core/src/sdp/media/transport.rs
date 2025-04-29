// Transport protocol parsing for SDP
//
// Handles parsing of transport protocols like RTP/AVP, UDP/TLS/RTP/SAVPF, etc.

use nom::{
    IResult,
    branch::alt,
    combinator::value,
};
use crate::sdp::media::utils::tag_no_case;

/// Parse transport protocol
pub(crate) fn parse_transport_protocol(input: &str) -> IResult<&str, String> {
    alt((
        value("RTP/AVP".to_string(), tag_no_case("RTP/AVP")),
        value("RTP/SAVP".to_string(), tag_no_case("RTP/SAVP")),
        value("RTP/AVPF".to_string(), tag_no_case("RTP/AVPF")),
        value("RTP/SAVPF".to_string(), tag_no_case("RTP/SAVPF")),
        value("UDP/TLS/RTP/SAVP".to_string(), tag_no_case("UDP/TLS/RTP/SAVP")),
        value("UDP/TLS/RTP/SAVPF".to_string(), tag_no_case("UDP/TLS/RTP/SAVPF")),
        value("UDP".to_string(), tag_no_case("UDP")),
        value("TCP".to_string(), tag_no_case("TCP")),
        value("DCCP".to_string(), tag_no_case("DCCP")),
        value("SCTP".to_string(), tag_no_case("SCTP"))
    ))(input)
}

/// Check if a protocol string is secure (uses DTLS/TLS/SAVP)
pub(crate) fn is_secure_protocol(protocol: &str) -> bool {
    protocol.contains("TLS") || 
    protocol.contains("SAVP") ||
    protocol.contains("DTLS")
}

/// Check if a protocol string is for RTP-based media
pub(crate) fn is_rtp_protocol(protocol: &str) -> bool {
    protocol.contains("RTP")
} 