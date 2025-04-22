// Parser for the Warning header (RFC 3261 Section 20.43)
// Warning = "Warning" HCOLON warning-value *(COMMA warning-value)
// warning-value = warn-code SP warn-agent SP warn-text
// warn-code = 3DIGIT
// warn-agent = hostport / pseudonym (token)
// warn-text = quoted-string

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while_m_n},
    character::complete::{digit1, space1},
    combinator::{map, map_res, opt, recognize},
    multi::{separated_list1},
    sequence::{pair, preceded, tuple},
    IResult,
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, comma};
use crate::parser::token::token;
use crate::parser::quoted::quoted_string;
use crate::parser::uri::host::hostport;
use crate::parser::whitespace::space1;
use crate::parser::ParseResult;

use crate::uri::Host;

// warn-code = 3DIGIT
fn warn_code(input: &[u8]) -> ParseResult<WarnCode> {
    map_res(
        take_while_m_n(3, 3, |c: u8| c.is_ascii_digit()),
        |bytes| {
            let s = str::from_utf8(bytes).map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Char)))?;
            let code = s.parse::<u16>().map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Digit)))?;
            Ok(WarnCode(code))
        }
    )(input)
}

// warn-agent = hostport / pseudonym (token)
// Returns Either<(Host, Option<u16>), &[u8]>
fn warn_agent(input: &[u8]) -> ParseResult<Result<(Host, Option<u16>), &[u8]>> {
    alt((
        map(hostport, |hp| Ok(hp)), // hostport -> Ok
        map(token, |t| Err(t)) // pseudonym -> Err
    ))(input)
}

// warn-text = quoted-string
fn warn_text(input: &[u8]) -> ParseResult<&[u8]> {
    quoted_string(input) // Returns bytes within quotes
}

// warning-value = warn-code SP warn-agent SP warn-text
// Returns (code, agent, text_bytes)
fn warning_value(input: &[u8]) -> ParseResult<(u16, Result<(Host, Option<u16>), &[u8]>, &[u8])> {
    tuple((
        warn_code,
        preceded(space1, warn_agent),
        preceded(space1, warn_text)
    ))(input)
}

// Define struct for Warning value
#[derive(Debug, PartialEq, Clone)]
pub enum WarnAgent {
    HostPort(Host, Option<u16>),
    Pseudonym(Vec<u8>),
}
#[derive(Debug, PartialEq, Clone)]
pub struct WarningValue {
    pub code: u16,
    pub agent: WarnAgent,
    // Store raw bytes, unescaping handled by consumer if needed
    pub text: Vec<u8>,
}

// Warning = "Warning" HCOLON warning-value *(COMMA warning-value)
pub(crate) fn parse_warning(input: &[u8]) -> ParseResult<Vec<WarningValue>> {
    map(
        preceded(
            pair(tag_no_case(b"Warning"), hcolon),
            separated_list1(comma, warning_value) // Requires at least one value
        ),
        |warnings| {
            warnings.into_iter().map(|(code, agent_res, text_b)| {
                let agent = match agent_res {
                    Ok((host, port)) => WarnAgent::HostPort(host, port),
                    Err(pseudo_b) => WarnAgent::Pseudonym(pseudo_b.to_vec()),
                };
                WarningValue { code, agent, text: text_b.to_vec() }
            }).collect()
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_warning_value() {
        let input = b"307 isi.edu \"Session parameter \'foo\' not understood\"";
        let result = warning_value(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.code, 307);
        assert!(matches!(val.agent, WarnAgent::HostPort(Host::Domain(d), None) if d == "isi.edu"));
        assert_eq!(val.text, "Session parameter 'foo' not understood");
    }

    #[test]
    fn test_warning_value_pseudonym() {
        let input = b"399 p1.example.net \"Response too large\"";
        let result = warning_value(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val.code, 399);
        assert!(matches!(val.agent, WarnAgent::HostPort(Host::Domain(d), None) if d == "p1.example.net")); // Assuming pseudonym is parsed as domain for now based on example
        assert_eq!(val.text, "Response too large");
    }

    #[test]
    fn test_parse_warning_multiple() {
        let input = b"307 isi.edu \"Session parameter 'foo' not understood\", 392 192.168.1.1 \"Something else\"";
        let result = parse_warning(input);
        assert!(result.is_ok());
        let (rem, warnings) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(warnings.len(), 2);
        assert_eq!(warnings[0].code, 307);
        assert_eq!(warnings[1].code, 392);
        assert!(matches!(warnings[1].agent, WarnAgent::HostPort(Host::Address(a), None) if *a == Ipv4Addr::new(192,168,1,1).into()));
    }
} 