// Parser for the P-Asserted-Identity / P-Preferred-Identity headers
// (RFC 3325 §9.1, §9.2)
//
// PAssertedID       = "P-Asserted-Identity" HCOLON PAssertedID-value
//                       *(COMMA PAssertedID-value)
// PAssertedID-value = name-addr / addr-spec
//
// P-Preferred-Identity has identical syntax.

use nom::combinator::map;

use crate::parser::address::name_addr_or_addr_spec;
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;
use crate::types::address::Address;
use crate::types::p_asserted_identity::{PAssertedIdentity, PPreferredIdentity};

/// Parse the value of a `P-Asserted-Identity` (or `P-Preferred-Identity`)
/// header — a comma-separated list of name-addr / addr-spec entries with no
/// trailing parameters per RFC 3325.
pub fn parse_p_asserted_identity_value(input: &[u8]) -> ParseResult<Vec<Address>> {
    comma_separated_list1(name_addr_or_addr_spec)(input)
}

/// Parse a `P-Asserted-Identity` header value into the typed wrapper.
pub fn parse_p_asserted_identity(input: &[u8]) -> ParseResult<PAssertedIdentity> {
    map(parse_p_asserted_identity_value, PAssertedIdentity)(input)
}

/// Parse a `P-Preferred-Identity` header value into the typed wrapper.
pub fn parse_p_preferred_identity(input: &[u8]) -> ParseResult<PPreferredIdentity> {
    map(parse_p_asserted_identity_value, PPreferredIdentity)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::Scheme;

    #[test]
    fn parses_single_sip_uri() {
        let input = b"<sip:alice@example.com>";
        let (rem, list) = parse_p_asserted_identity_value(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].uri.scheme, Scheme::Sip);
    }

    #[test]
    fn parses_addr_spec_form() {
        let input = b"sip:alice@example.com";
        let (rem, list) = parse_p_asserted_identity_value(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn parses_two_entries_sip_and_tel() {
        let input = b"\"Alice\" <sip:alice@example.com>, <tel:+14155551234>";
        let (rem, list) = parse_p_asserted_identity_value(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].display_name, Some("Alice".to_string()));
        assert_eq!(list[0].uri.scheme, Scheme::Sip);
        assert_eq!(list[1].uri.scheme, Scheme::Tel);
    }

    #[test]
    fn rejects_empty_input() {
        let input = b"";
        assert!(parse_p_asserted_identity_value(input).is_err());
    }

    #[test]
    fn typed_pai_wrapper() {
        let input = b"<sip:alice@example.com>";
        let (_, pai) = parse_p_asserted_identity(input).unwrap();
        assert_eq!(pai.len(), 1);
    }

    #[test]
    fn typed_ppi_wrapper() {
        let input = b"<sip:bob@example.com>";
        let (_, ppi) = parse_p_preferred_identity(input).unwrap();
        assert_eq!(ppi.len(), 1);
    }
}
