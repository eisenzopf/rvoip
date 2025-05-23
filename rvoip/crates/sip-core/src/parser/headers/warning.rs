// Parser for the Warning header (RFC 3261 Section 20.43)
// Warning = "Warning" HCOLON warning-value *(COMMA warning-value)
// warning-value = warn-code SP warn-agent SP warn-text
// warn-code = 3DIGIT
// warn-agent = hostport / pseudonym (token)
// warn-text = quoted-string

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while_m_n, take_while1},
    bytes::complete::tag,
    character::complete::{digit1, space1},
    combinator::{map, map_res, opt, recognize},
    multi::{separated_list1},
    sequence::{pair, preceded, tuple},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError}, // Import NomError
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::token::token;
use crate::parser::quoted::quoted_string;
use crate::parser::uri::host::hostport;
use crate::parser::ParseResult;

use crate::types::uri::Host;
use crate::types::warning::{Warning as WarningHeader, WarnAgent, WarningValue}; // Import types
use crate::types::uri::Uri;

use std::str::FromStr;
use crate::parser::values::delta_seconds; // Use delta_seconds for duration
use crate::parser::whitespace::sws;

// WarningValue struct is now imported from types/warning.rs

// warn-code = 3DIGIT
fn warn_code(input: &[u8]) -> ParseResult<u16> { // Return u16 directly
    map_res(
        take_while_m_n(3, 3, |c: u8| c.is_ascii_digit()),
        |bytes| {
            let s = str::from_utf8(bytes).map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Char)))?;
            s.parse::<u16>().map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Digit)))
            // Removed Ok(WarnCode(code)) -> Just return the parsed u16
        }
    )(input)
}

// warn-agent = hostport / pseudonym (token)
fn warn_agent(input: &[u8]) -> ParseResult<WarnAgent> {
    alt((
        // First try to parse as a hostport (which includes IP addresses)
        map(hostport, |(host, port)| WarnAgent::HostPort(host, port)),
        // Then try to parse as a pseudonym (token)
        map_res(token, |t| {
            str::from_utf8(t).map(|s| WarnAgent::Pseudonym(s.to_string()))
                .map_err(|_| nom::Err::Error(NomError::from_error_kind(t, ErrorKind::AlphaNumeric)))
        })
    ))(input)
}

// warn-text = quoted-string
fn warn_text(input: &[u8]) -> ParseResult<&[u8]> {
    quoted_string(input) // Returns bytes within quotes
}

// warning-value = warn-code SP warn-agent SP warn-text
// Changed return type to ParseResult<WarningValue>
fn warning_value(input: &[u8]) -> ParseResult<WarningValue> {
    map(
        tuple((
            warn_code,
            preceded(space1, warn_agent),
            preceded(space1, warn_text)
        )),
        |(code, agent, text_b)| {
            WarningValue { code, agent, text: text_b.to_vec() } // Convert &[u8] to Vec<u8>
        }
    )(input)
}

/// Parses a Warning header value (list of warning-values).
// Warning = "Warning" HCOLON warning-value *(COMMA warning-value)
// Note: HCOLON handled elsewhere if parsing just the value part
pub fn parse_warning_value_list(input: &[u8]) -> ParseResult<Vec<WarningValue>> {
    separated_list1(comma, warning_value)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, IpAddr};

    #[test]
    fn test_warning_value() {
        let input = b"307 isi.edu \"Session parameter \'foo\' not understood\"";
        let result = warning_value(input); // Test the single value parser
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.code, 307);
        assert!(matches!(val.agent, WarnAgent::Pseudonym(d) if d == "isi.edu"));
        assert_eq!(val.text, b"Session parameter 'foo' not understood".to_vec()); // Compare Vec<u8>
    }

    #[test]
    fn test_warning_value_pseudonym() {
        let input = b"399 p1.example.net \"Response too large\"";
        let result = warning_value(input); // Test the single value parser
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.code, 399);
        assert!(matches!(val.agent, WarnAgent::Pseudonym(d) if d == "p1.example.net"));
        assert_eq!(val.text, b"Response too large".to_vec()); // Compare Vec<u8>
    }

    #[test]
    fn test_parse_warning_multiple() {
        // Test the list parser
        let input = b"307 isi.edu \"Session parameter \'foo\' not understood\", 392 192.168.1.1 \"Something else\"";
        let result = parse_warning_value_list(input); // Use the list parser
        assert!(result.is_ok());
        let (rem, warnings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(warnings.len(), 2);
        assert_eq!(warnings[0].code, 307);
        assert_eq!(warnings[1].code, 392);
        assert!(matches!(warnings[1].agent, WarnAgent::HostPort(Host::Address(a), None) if a == IpAddr::from(Ipv4Addr::new(192,168,1,1))));
    }
} 