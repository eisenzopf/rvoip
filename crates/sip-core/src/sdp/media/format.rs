// Media format parsing for SDP
//
// Handles parsing of media formats in m= lines

use nom::{
    IResult,
    character::complete::{space1},
    multi::separated_list1,
    bytes::complete::take_while1,
    combinator::map,
};

/// Parse media formats (space separated list of identifiers)
pub(crate) fn parse_formats(input: &str) -> IResult<&str, Vec<String>> {
    separated_list1(
        space1,
        map(
            take_while1(|c: char| c.is_ascii_digit() || c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' || c == '*'),
            |s: &str| s.to_string()
        )
    )(input)
}

/// Parse port and optional port count
pub(crate) fn parse_port_and_count(input: &str) -> IResult<&str, (u16, Option<u16>)> {
    use nom::{
        branch::alt,
        character::complete::{char, digit1},
        combinator::{map, map_res},
        sequence::tuple,
    };

    alt((
        // Port with count: "port/count"
        map(
            tuple((
                map_res(digit1, |s: &str| s.parse::<u16>()),
                char('/'),
                map_res(digit1, |s: &str| s.parse::<u16>())
            )),
            |(port, _, count)| (port, Some(count))
        ),
        // Just port
        map(
            map_res(digit1, |s: &str| s.parse::<u16>()),
            |port| (port, None)
        )
    ))(input)
}

/// Check if a format ID is a valid RTP payload type (0-127)
pub(crate) fn is_valid_payload_type(format: &str) -> bool {
    if let Ok(pt) = format.parse::<u8>() {
        return pt <= 127;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_formats_numeric() {
        // Test simple numeric formats
        let formats = "0 8 96";
        let result = parse_formats(formats);
        assert!(result.is_ok(), "Failed to parse numeric formats");
        
        let (rest, parsed) = result.unwrap();
        assert_eq!(rest, "", "Parser did not consume all input");
        assert_eq!(parsed, vec!["0", "8", "96"], "Incorrect parsed formats");
    }
    
    #[test]
    fn test_parse_formats_alphanumeric() {
        // Test alphanumeric formats (like used in application media lines)
        let formats = "wb netblt";
        let result = parse_formats(formats);
        assert!(result.is_ok(), "Failed to parse alphanumeric formats");
        
        let (rest, parsed) = result.unwrap();
        assert_eq!(parsed, vec!["wb", "netblt"], "Incorrect parsed formats");
    }
    
    #[test]
    fn test_parse_formats_special_chars() {
        // Test formats with special characters allowed by the parser
        let formats = "H264-1 PCMA.8 opus_48000";
        let result = parse_formats(formats);
        assert!(result.is_ok(), "Failed to parse formats with special chars");
        
        let (rest, parsed) = result.unwrap();
        assert_eq!(parsed, vec!["H264-1", "PCMA.8", "opus_48000"], "Incorrect parsed formats");
    }
    
    #[test]
    fn test_parse_formats_wildcard() {
        // Test wildcard format (used in some application protocols)
        let formats = "*";
        let result = parse_formats(formats);
        assert!(result.is_ok(), "Failed to parse wildcard format");
        
        let (rest, parsed) = result.unwrap();
        assert_eq!(parsed, vec!["*"], "Incorrect parsed wildcard");
    }
    
    #[test]
    fn test_parse_formats_single() {
        // Test single format
        let formats = "0";
        let result = parse_formats(formats);
        assert!(result.is_ok(), "Failed to parse single format");
        
        let (rest, parsed) = result.unwrap();
        assert_eq!(parsed, vec!["0"], "Incorrect parsed single format");
    }
    
    #[test]
    fn test_parse_port_basic() {
        // Test basic port number
        let port_str = "49170";
        let result = parse_port_and_count(port_str);
        assert!(result.is_ok(), "Failed to parse basic port");
        
        let (rest, (port, count)) = result.unwrap();
        assert_eq!(rest, "", "Parser did not consume all input");
        assert_eq!(port, 49170, "Incorrect port number");
        assert_eq!(count, None, "Port count should be None");
    }
    
    #[test]
    fn test_parse_port_with_count() {
        // Test port with count (for consecutive ports)
        let port_str = "49170/2";
        let result = parse_port_and_count(port_str);
        assert!(result.is_ok(), "Failed to parse port with count");
        
        let (rest, (port, count)) = result.unwrap();
        assert_eq!(port, 49170, "Incorrect port number");
        assert_eq!(count, Some(2), "Incorrect port count");
    }
    
    #[test]
    fn test_parse_port_zero() {
        // Test zero port (used for inactive media)
        let port_str = "0";
        let result = parse_port_and_count(port_str);
        assert!(result.is_ok(), "Failed to parse zero port");
        
        let (rest, (port, count)) = result.unwrap();
        assert_eq!(port, 0, "Incorrect port number");
        assert_eq!(count, None, "Port count should be None");
    }
    
    #[test]
    fn test_parse_port_max() {
        // Test max port value (65535)
        let port_str = "65535";
        let result = parse_port_and_count(port_str);
        assert!(result.is_ok(), "Failed to parse max port value");
        
        let (rest, (port, count)) = result.unwrap();
        assert_eq!(port, 65535, "Incorrect port number");
    }
    
    #[test]
    fn test_parse_port_with_large_count() {
        // Test larger count value
        let port_str = "16384/8";
        let result = parse_port_and_count(port_str);
        assert!(result.is_ok(), "Failed to parse port with large count");
        
        let (rest, (port, count)) = result.unwrap();
        assert_eq!(port, 16384, "Incorrect port number");
        assert_eq!(count, Some(8), "Incorrect port count");
    }
    
    #[test]
    fn test_parse_port_with_trailing_data() {
        // Test port with trailing data
        let port_str = "49170 RTP/AVP";
        let result = parse_port_and_count(port_str);
        assert!(result.is_ok(), "Failed to parse port with trailing data");
        
        let (rest, (port, count)) = result.unwrap();
        assert_eq!(rest, " RTP/AVP", "Parser should leave trailing data");
        assert_eq!(port, 49170, "Incorrect port number");
    }
    
    #[test]
    fn test_parse_port_with_count_and_trailing_data() {
        // Test port with count and trailing data
        let port_str = "49170/2 RTP/AVP";
        let result = parse_port_and_count(port_str);
        assert!(result.is_ok(), "Failed to parse port with count and trailing data");
        
        let (rest, (port, count)) = result.unwrap();
        assert_eq!(rest, " RTP/AVP", "Parser should leave trailing data");
        assert_eq!(port, 49170, "Incorrect port number");
        assert_eq!(count, Some(2), "Incorrect port count");
    }
    
    #[test]
    fn test_is_valid_payload_type() {
        // Test valid payload types
        assert!(is_valid_payload_type("0"), "0 should be a valid payload type");
        assert!(is_valid_payload_type("127"), "127 should be a valid payload type");
        assert!(is_valid_payload_type("96"), "96 should be a valid payload type");
        
        // Test invalid payload types
        assert!(!is_valid_payload_type("128"), "128 should not be a valid payload type");
        assert!(!is_valid_payload_type("256"), "256 should not be a valid payload type");
        assert!(!is_valid_payload_type("-1"), "-1 should not be a valid payload type");
        assert!(!is_valid_payload_type("abc"), "Non-numeric should not be a valid payload type");
        assert!(!is_valid_payload_type(""), "Empty string should not be a valid payload type");
    }
    
    #[test]
    fn test_parse_formats_from_rfc() {
        // Test formats from RFC 4566 examples
        let examples = [
            "0",                    // Basic PCMU
            "0 8 96",               // Multiple formats
            "31",                   // H.261 video
            "96 97 98",             // Dynamic payload types
            "wb"                    // Whiteboard (application)
        ];
        
        for example in examples.iter() {
            let result = parse_formats(example);
            assert!(result.is_ok(), "Failed to parse RFC example: {}", example);
        }
    }
} 