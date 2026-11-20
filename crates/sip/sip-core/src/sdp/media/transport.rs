// Transport protocol parsing for SDP
//
// Handles parsing of transport protocols like RTP/AVP, UDP/TLS/RTP/SAVPF, etc.

use crate::sdp::media::utils::tag_no_case;
use nom::{branch::alt, combinator::value, IResult};

/// Parse transport protocol
pub(crate) fn parse_transport_protocol(input: &str) -> IResult<&str, String> {
    alt((
        // Order protocols from most specific to least specific to avoid partial matching
        value(
            "UDP/TLS/RTP/SAVPF".to_string(),
            tag_no_case("UDP/TLS/RTP/SAVPF"),
        ),
        value(
            "UDP/TLS/RTP/SAVP".to_string(),
            tag_no_case("UDP/TLS/RTP/SAVP"),
        ),
        value("RTP/SAVPF".to_string(), tag_no_case("RTP/SAVPF")),
        value("RTP/AVPF".to_string(), tag_no_case("RTP/AVPF")),
        value("RTP/SAVP".to_string(), tag_no_case("RTP/SAVP")),
        value("RTP/AVP".to_string(), tag_no_case("RTP/AVP")),
        value("UDP".to_string(), tag_no_case("UDP")),
        value("TCP".to_string(), tag_no_case("TCP")),
        value("DCCP".to_string(), tag_no_case("DCCP")),
        value("SCTP".to_string(), tag_no_case("SCTP")),
    ))(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_rtp_protocols() {
        // Test basic RTP/AVP protocol (RFC 4566)
        let input = "RTP/AVP rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse RTP/AVP");

        let (rest, protocol) = result.unwrap();
        assert_eq!(rest, " rest", "Parser should leave the rest of the input");
        assert_eq!(protocol, "RTP/AVP", "Incorrect protocol parsed");

        // Test RTP/SAVP protocol (RFC 3711 - SRTP)
        let input = "RTP/SAVP rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse RTP/SAVP");

        let (_rest, protocol) = result.unwrap();
        assert_eq!(protocol, "RTP/SAVP", "Incorrect protocol parsed");
    }

    #[test]
    fn test_parse_feedback_protocols() {
        // Test RTP/AVPF protocol (RFC 4585 - RTP with feedback)
        let input = "RTP/AVPF rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse RTP/AVPF");

        let (_rest, protocol) = result.unwrap();
        assert_eq!(protocol, "RTP/AVPF", "Incorrect protocol parsed");

        // Test RTP/SAVPF protocol (RFC 5124 - Secure RTP with feedback)
        let input = "RTP/SAVPF rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse RTP/SAVPF");

        let (_rest, protocol) = result.unwrap();
        assert_eq!(protocol, "RTP/SAVPF", "Incorrect protocol parsed");
    }

    #[test]
    fn test_parse_dtls_srtp_protocols() {
        // Test UDP/TLS/RTP/SAVP protocol (RFC 5764 - DTLS-SRTP)
        let input = "UDP/TLS/RTP/SAVP rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse UDP/TLS/RTP/SAVP");

        let (_rest, protocol) = result.unwrap();
        assert_eq!(protocol, "UDP/TLS/RTP/SAVP", "Incorrect protocol parsed");

        // Test UDP/TLS/RTP/SAVPF protocol (WebRTC common protocol)
        let input = "UDP/TLS/RTP/SAVPF rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse UDP/TLS/RTP/SAVPF");

        let (_rest, protocol) = result.unwrap();
        assert_eq!(protocol, "UDP/TLS/RTP/SAVPF", "Incorrect protocol parsed");
    }

    #[test]
    fn test_parse_datagram_protocols() {
        // Test UDP protocol
        let input = "UDP rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse UDP");

        let (_rest, protocol) = result.unwrap();
        assert_eq!(protocol, "UDP", "Incorrect protocol parsed");

        // Test TCP protocol
        let input = "TCP rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse TCP");

        let (_rest, protocol) = result.unwrap();
        assert_eq!(protocol, "TCP", "Incorrect protocol parsed");
    }

    #[test]
    fn test_parse_case_insensitivity() {
        // Test case insensitivity in parsing
        let input = "rtp/avp rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse lowercase protocol");

        let (_rest, protocol) = result.unwrap();
        assert_eq!(protocol, "RTP/AVP", "Should normalize to uppercase");

        // Mixed case should also work
        let input = "RtP/aVp rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_ok(), "Failed to parse mixed case protocol");

        let (_rest, protocol) = result.unwrap();
        assert_eq!(protocol, "RTP/AVP", "Should normalize to uppercase");
    }

    #[test]
    fn test_parse_invalid_protocols() {
        // Test protocol not in the list
        let input = "INVALID rest";
        let result = parse_transport_protocol(input);
        assert!(result.is_err(), "Should reject invalid protocol");

        // Test empty input
        let input = "";
        let result = parse_transport_protocol(input);
        assert!(result.is_err(), "Should reject empty input");

        // Test incomplete protocol
        let input = "RTP/";
        let result = parse_transport_protocol(input);
        assert!(result.is_err(), "Should reject incomplete protocol");
    }

    #[test]
    fn test_parse_rfc_examples() {
        // Examples based on RFC 4566 and WebRTC specs
        let examples = [
            "RTP/AVP",           // RFC 4566 basic RTP
            "RTP/SAVP",          // RFC 3711 SRTP
            "RTP/AVPF",          // RFC 4585 RTP with feedback
            "RTP/SAVPF",         // RFC 5124 Secure RTP with feedback
            "UDP/TLS/RTP/SAVP",  // RFC 5764 DTLS-SRTP
            "UDP/TLS/RTP/SAVPF", // WebRTC common profile
            "UDP",               // Basic UDP
            "TCP",               // Basic TCP
            "DCCP",              // DCCP
            "SCTP",              // SCTP
        ];

        for example in examples.iter() {
            let result = parse_transport_protocol(example);
            assert!(result.is_ok(), "Failed to parse example: {}", example);

            let (rest, protocol) = result.unwrap();
            assert_eq!(rest, "", "Parser should consume entire input");
            assert_eq!(protocol, *example, "Protocol should match example");
        }
    }
}
