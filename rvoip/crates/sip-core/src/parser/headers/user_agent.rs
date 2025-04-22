// Parser for User-Agent header (RFC 3261 Section 20.41)
// User-Agent = "User-Agent" HCOLON server-val *(LWS server-val)

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
use super::server_val::server_val; // Use the shared server_val parser
use crate::parser::ParseResult;

// Import the return type from server_val
use super::server_val::ServerValComponent;

// Import types (assuming)
use crate::types::server::ServerVal;

// server-val *(LWS server-val)
fn server_val_list(input: &[u8]) -> ParseResult<Vec<ServerVal>> {
    // separated_list1 ensures at least one server_val, separated by LWS
    separated_list1(lws, server_val)(input)
}

pub(crate) fn parse_user_agent(input: &[u8]) -> ParseResult<Vec<ServerVal>> {
    server_val_list(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::server::{Product};

    #[test]
    fn test_parse_user_agent_single_product() {
        let input = b"ExampleClient/2.1";
        let result = parse_user_agent(input);
        assert!(result.is_ok());
        let (rem, vals) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vals.len(), 1);
        assert!(matches!(&vals[0], ServerVal::Product(p) if p.name == "ExampleClient" && p.version == Some("2.1".to_string())));
    }
    
    #[test]
    fn test_parse_user_agent_multiple() {
        let input = b"Softphone Beta1 (Debug build)";
        let result = parse_user_agent(input);
        assert!(result.is_ok());
        let (rem, vals) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(vals.len(), 2);
        assert!(matches!(&vals[0], ServerVal::Product(p) if p.name == "Softphone" && p.version == Some("Beta1".to_string())));
        assert!(matches!(&vals[1], ServerVal::Comment(c) if c == "Debug build"));
    }
}