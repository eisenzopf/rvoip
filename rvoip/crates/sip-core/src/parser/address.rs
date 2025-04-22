// Parsers for common address formats (name-addr / addr-spec)

use nom::{
    branch::alt,
    combinator::{map, map_res, opt},
    sequence::{delimited, pair, terminated},
    IResult,
};
use std::str;
use ordered_float::NotNan;

// Import necessary parsers
use super::token::token;
use super::whitespace::lws;
use super::quoted::quoted_string;
use super::separators::{laquot, raquot};
use super::uri::{parse_uri}; // Removed parse_absolute_uri for now
use super::common_params::unquote_string; // Import helper
use super::ParseResult;

// Import necessary types
use crate::types::uri::Uri;
use crate::types::address::Address; // Changed to use Address struct
use crate::error::Error; // For unquote error

// display-name = *(token LWS)/ quoted-string
// Simplified: Parses either a single token or an unquoted string.
fn display_name(input: &[u8]) -> ParseResult<String> {
    alt((
        // Quoted string path (handles unquoting)
        map_res(quoted_string, |bytes| unquote_string(bytes)),
        // Single token path
        map_res(token, |bytes| str::from_utf8(bytes).map(String::from))
    ))(input)
}

// addr-spec = SIP-URI / SIPS-URI / absoluteURI
// Modified: For headers like From/To/Contact, only parse SIP/SIPS URIs for now.
// Returns Uri struct directly.
pub(crate) fn addr_spec(input: &[u8]) -> ParseResult<Uri> {
    parse_uri(input)
    // If absoluteURI support is needed later, this needs adjustment, perhaps returning an enum.
}

// name-addr = [ display-name ] LAQUOT addr-spec RAQUOT
// Returns Address struct (params added by caller)
pub fn name_addr(input: &[u8]) -> ParseResult<Address> {
    map(
        pair(
            opt(display_name), // Optional display name (String)
            delimited(laquot, addr_spec, raquot) // < sip-uri / sips-uri >
        ),
        |(display_opt, uri)| Address {
            display_name: display_opt,
            uri: uri, // uri is Uri struct
            params: Vec::new(), // Params added later
        }
    )(input)
}

// Helper to parse either name-addr or addr-spec, used by From/To/etc.
// Returns Address struct (params added by caller)
pub fn name_addr_or_addr_spec(input: &[u8]) -> ParseResult<Address> {
    alt((
        name_addr, // Try name-addr first (<> required)
        // If just addr-spec (URI directly), map it into an Address struct
        map(addr_spec, |uri| Address {
            display_name: None,
            uri: uri,
            params: Vec::new(),
        })
    ))(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Host;
    use std::net::Ipv4Addr;

    #[test]
    fn test_display_name_simple() {
        let (rem, name) = display_name(b"Alice").unwrap();
        assert!(rem.is_empty());
        assert_eq!(name, "Alice");
    }

    #[test]
    fn test_display_name_quoted() {
        let (rem, name) = display_name(b"\"Alice Smith\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(name, "Alice Smith");
    }
    
    #[test]
    fn test_display_name_quoted_escaped() {
        let (rem, name) = display_name(b"\"\\\"Agent\\\" Smith\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(name, "\"Agent\" Smith");
    }

    #[test]
    fn test_addr_spec_sip() {
        let (rem, uri) = addr_spec(b"sip:user@example.com").unwrap();
        assert!(rem.is_empty());
        assert_eq!(uri.scheme, "sip"); // Check Uri fields
    }

    #[test]
    fn test_name_addr_full() {
        let (rem, addr) = name_addr(b"\"Bob\" <sip:bob@host.com>").unwrap();
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, Some("Bob".to_string()));
        assert_eq!(addr.uri.scheme, "sip");
        assert!(addr.params.is_empty()); // Params not parsed here
    }
    
    #[test]
    fn test_name_addr_no_display() {
        let (rem, addr) = name_addr(b"<sip:bob@host.com>").unwrap();
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, None);
        assert_eq!(addr.uri.scheme, "sip");
        assert!(addr.params.is_empty());
    }

     #[test]
    fn test_name_addr_or_addr_spec_name_addr() {
        let (rem, addr) = name_addr_or_addr_spec(b"\"Test\" <sip:t@test.com>").unwrap();
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, Some("Test".to_string()));
        assert_eq!(addr.uri.scheme, "sip");
    }
    
    #[test]
    fn test_name_addr_or_addr_spec_addr_spec_only() {
        let (rem, addr) = name_addr_or_addr_spec(b"sip:t@test.com").unwrap();
        assert!(rem.is_empty());
        assert_eq!(addr.display_name, None); 
        assert_eq!(addr.uri.scheme, "sip");
    }
} 