// Parser for Server header (RFC 3261 Section 20.36)
// Server = "Server" HCOLON server-val *(LWS server-val)

use nom::{
    bytes::complete::tag_no_case,
    combinator::{map, opt},
    multi::{many0, separated_list1},
    sequence::{pair, preceded},
    IResult,
};

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::whitespace::lws;
use super::server_val::server_val_parser; // Use the shared server_val parser
use crate::parser::ParseResult;

// Import the return type from server_val
use super::server_val::ServerValComponent;

// Import shared parsers
use super::server_val::server_val;
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;

// Import types (assuming)
use crate::types::server::ServerVal;

// server-val *(LWS server-val)
fn server_val_list(input: &[u8]) -> ParseResult<Vec<ServerVal>> {
    // separated_list1 ensures at least one server_val, separated by LWS
    separated_list1(lws, server_val)(input)
}

// Server = "Server" HCOLON server-val *(LWS server-val)
// Note: HCOLON handled elsewhere
pub fn parse_server(input: &[u8]) -> ParseResult<Vec<ServerVal>> {
    server_val_list(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::server::{Product};

    #[test]
    fn test_parse_server_single_product() {
        let input = b"ExampleServer/1.1";
        let result = parse_server(input);
        assert!(result.is_ok());
        let (rem, vals) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vals.len(), 1);
        assert!(matches!(&vals[0], ServerVal::Product(p) if p.name == "ExampleServer" && p.version == Some("1.1".to_string())));
    }
    
    #[test]
    fn test_parse_server_multiple() {
        let input = b"ProductA/2.0 (Compatible) ProductB";
        let result = parse_server(input);
        assert!(result.is_ok());
        let (rem, vals) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vals.len(), 3);
        assert!(matches!(&vals[0], ServerVal::Product(p) if p.name == "ProductA" && p.version == Some("2.0".to_string())));
        assert!(matches!(&vals[1], ServerVal::Comment(c) if c == "Compatible"));
        assert!(matches!(&vals[2], ServerVal::Product(p) if p.name == "ProductB" && p.version == None));
    }

    #[test]
    fn test_parse_server_comment_only() {
        let input = b"(Internal Test Build)";
        let result = parse_server(input);
        assert!(result.is_ok());
        let (rem, vals) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vals.len(), 1);
        assert!(matches!(&vals[0], ServerVal::Comment(c) if c == "Internal Test Build"));
    }

    #[test]
    fn test_parse_server_empty_fail() {
        // Must have at least one server-val
        assert!(parse_server(b"").is_err());
    }
} 