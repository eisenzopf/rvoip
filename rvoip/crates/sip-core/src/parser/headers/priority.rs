// Parser for Priority header (RFC 3261 Section 20.28)
// Priority = "Priority" HCOLON priority-value
// priority-value = "emergency" / "urgent" / "normal" / "non-urgent" / other-priority
// other-priority = token

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while1},
    combinator::{map, map_res, verify, all_consuming},
    sequence::{pair, preceded},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::token::token;
use crate::parser::ParseResult;

// Import types
use crate::types::Priority;

// priority-value = "emergency" / "urgent" / "normal" / "non-urgent" / other-priority
// other-priority = token
fn priority_value(input: &[u8]) -> ParseResult<Priority> {
    // Use all_consuming to ensure we don't accept any trailing content
    all_consuming(
        map_res(
            token, // Any token is valid per RFC 3261
            |bytes| {
                let s = str::from_utf8(bytes)
                    .map_err(|_| nom::Err::Failure(nom::error::Error::new(bytes, nom::error::ErrorKind::Char)))?;
                
                // Check if it's a numeric string and verify it's within u8 range
                if s.chars().all(|c| c.is_ascii_digit()) {
                    // Try to parse as a number
                    match s.parse::<u8>() {
                        Ok(val) => Ok(Priority::Other(val)),
                        // If parsing fails, it's likely too large for u8
                        Err(_) => Err(nom::Err::Failure(nom::error::Error::new(bytes, nom::error::ErrorKind::Digit)))
                    }
                } else {
                    // Use the from_token method for non-numeric tokens
                    Ok(Priority::from_token(s))
                }
            }
        )
    )(input)
}

pub fn parse_priority(input: &[u8]) -> ParseResult<Priority> {
    priority_value(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_priority() {
        // Test all specified priority values with different casings
        let test_cases = &[
            (&b"emergency"[..], Priority::Emergency),
            (&b"EMERGENCY"[..], Priority::Emergency),
            (&b"EmErGeNcY"[..], Priority::Emergency),
            (&b"urgent"[..], Priority::Urgent),
            (&b"URGENT"[..], Priority::Urgent),
            (&b"UrGeNt"[..], Priority::Urgent),
            (&b"normal"[..], Priority::Normal),
            (&b"NORMAL"[..], Priority::Normal),
            (&b"NoRmAl"[..], Priority::Normal),
            (&b"non-urgent"[..], Priority::NonUrgent),
            (&b"NON-URGENT"[..], Priority::NonUrgent),
            (&b"Non-Urgent"[..], Priority::NonUrgent),
        ];

        for (input, expected) in test_cases {
            let (rem, val) = parse_priority(input).unwrap();
            assert!(rem.is_empty(), "Remaining input should be empty for {:?}", input);
            assert_eq!(val, *expected, "Failed to parse priority value {:?}", input);
        }
    }

    #[test]
    fn test_numeric_priority() {
        // Test numeric priority values
        let numeric_cases = &[
            (&b"0"[..], 0),
            (&b"1"[..], 1),
            (&b"5"[..], 5),
            (&b"10"[..], 10),
            (&b"42"[..], 42),
            (&b"255"[..], 255),
        ];

        for (input, expected) in numeric_cases {
            let (rem, val) = parse_priority(input).unwrap();
            assert!(rem.is_empty(), "Remaining input should be empty for {:?}", input);
            assert_eq!(val, Priority::Other(*expected), "Failed to parse numeric priority {:?}", input);
        }
    }

    #[test]
    fn test_token_priority() {
        // Test token-based priority values
        let token_cases = &[
            &b"high"[..], 
            &b"low"[..],
            &b"critical"[..],
            &b"non_critical"[..],   // With underscore
            &b"high-priority"[..],  // With hyphen
            &b"priority.level"[..], // With period
            &b"priority+"[..],      // With plus
            &b"urgent!"[..],        // With exclamation
            &b"~special"[..],       // With tilde
        ];

        for input in token_cases {
            let (rem, val) = parse_priority(input).unwrap();
            assert!(rem.is_empty(), "Remaining input should be empty for {:?}", input);
            let expected_str = str::from_utf8(input).unwrap();
            assert_eq!(val, Priority::Token(expected_str.to_string()), 
                       "Failed to parse token priority {:?}", input);
        }
    }

    #[test]
    fn test_invalid_priority() {
        // Test invalid priority values
        let invalid_cases = &[
            &b""[..],                  // Empty input
            &b" "[..],                 // Just whitespace
            &b"emergency "[..],        // Trailing whitespace
            &b" emergency"[..],        // Leading whitespace
            &b"emergency;param"[..],   // With parameter
            &b"256"[..],               // Numeric value too large for u8
            &b"1000"[..],              // Numeric value too large for u8
            &b"emergency\r\n"[..],     // With line ending
        ];

        for input in invalid_cases {
            assert!(parse_priority(input).is_err(), "Should reject invalid input {:?}", input);
        }
    }

    #[test]
    fn test_rfc3261_compliance() {
        // Test RFC 3261 Section 20.28 examples and edge cases
        // RFC 3261 states: Priority = "Priority" HCOLON priority-value
        // priority-value = "emergency" / "urgent" / "normal" / "non-urgent" / other-priority
        // other-priority = token
        
        // This test checks the compliance with the ABNF for priority-value
        
        // Test that we can handle all examples from RFC 3261
        
        // Standard priority values
        let (_, val) = parse_priority(b"emergency").unwrap();
        assert_eq!(val, Priority::Emergency);
        
        let (_, val) = parse_priority(b"urgent").unwrap();
        assert_eq!(val, Priority::Urgent);
        
        let (_, val) = parse_priority(b"normal").unwrap();
        assert_eq!(val, Priority::Normal);
        
        let (_, val) = parse_priority(b"non-urgent").unwrap();
        assert_eq!(val, Priority::NonUrgent);
        
        // Numeric other-priority
        let (_, val) = parse_priority(b"0").unwrap();
        assert_eq!(val, Priority::Other(0));
        
        // Token other-priority
        // token = 1*(alphanum / "-" / "." / "!" / "%" / "*" / "_" / "+" / "`" / "'" / "~")
        let (_, val) = parse_priority(b"high-priority").unwrap();
        assert_eq!(val, Priority::Token("high-priority".to_string()));
        
        let (_, val) = parse_priority(b"non_urgent").unwrap();
        assert_eq!(val, Priority::Token("non_urgent".to_string()));
        
        // Ensure we reject invalid inputs
        assert!(parse_priority(b"").is_err());         // Empty
        assert!(parse_priority(b" ").is_err());        // Space
        assert!(parse_priority(b"urgent;q=0.8").is_err()); // With parameter
    }
} 