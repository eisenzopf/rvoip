// Parser for Priority header (RFC 3261 Section 20.28)
// Priority = "Priority" HCOLON priority-value
// priority-value = "emergency" / "urgent" / "normal" / "non-urgent" / other-priority
// other-priority = token

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case},
    combinator::{map, map_res},
    sequence::{pair, preceded},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::token::token;
use crate::parser::ParseResult;

// Import types (assuming)
use crate::types::priority::PriorityValue; // Assuming enum { Emergency, Urgent, Normal, NonUrgent, Other(String) }

// priority-value = "emergency" / "urgent" / "normal" / "non-urgent" / other-priority
// other-priority = token
fn priority_value(input: &[u8]) -> ParseResult<PriorityValue> {
    map_res(
        token, // Any token is valid first
        |bytes| {
            let s = str::from_utf8(bytes)?;
            Ok(match s.to_ascii_lowercase().as_str() {
                "emergency" => PriorityValue::Emergency,
                "urgent" => PriorityValue::Urgent,
                "normal" => PriorityValue::Normal,
                "non-urgent" => PriorityValue::NonUrgent,
                other => PriorityValue::Other(other.to_string()), // Store original case? Or lower?
            })
        }
    )(input)
}

pub fn parse_priority(input: &[u8]) -> ParseResult<PriorityValue> {
    priority_value(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_priority() {
        let (rem, val) = parse_priority(b"normal").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, PriorityValue::Normal);

        let (rem_u, val_u) = parse_priority(b"UrGeNT").unwrap();
        assert!(rem_u.is_empty());
        assert_eq!(val_u, PriorityValue::Urgent);
        
        let (rem_o, val_o) = parse_priority(b"Business").unwrap();
        assert!(rem_o.is_empty());
        assert_eq!(val_o, PriorityValue::Other("Business".to_string()));
    }

    #[test]
    fn test_invalid_priority() {
        // Only fails if token itself fails (e.g., empty input)
        assert!(parse_priority(b"").is_err());
        assert!(parse_priority(b" with space").is_err());
    }
} 