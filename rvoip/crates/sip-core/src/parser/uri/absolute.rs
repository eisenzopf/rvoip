// RFC 2396 / 3261 absoluteURI parser (Full)

use nom::{
    branch::alt,
    bytes::complete::{tag, take_while, take_while1, take_while_m_n},
    character::complete::{alphanumeric1, char},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1},
    sequence::{delimited, pair, preceded, tuple},
    IResult,
};
use std::str;

// Import shared parsers from base parser
use crate::parser::common_chars::{alpha, digit, escaped, mark, reserved, unreserved};
use crate::parser::uri::host::hostport; // For srvr
use crate::parser.uri::userinfo::userinfo; // For srvr (assuming it returns bytes needed)
use crate::parser::ParseResult;

// --- URI Character Sets (RFC 2396 / 3261) ---

// uric = reserved / unreserved / escaped
fn uric(input: &[u8]) -> ParseResult<&[u8]> {
    alt((reserved, unreserved, escaped))(input)
}

// uric-no-slash = unreserved / escaped / ";" / "?" / ":" / "@" / "&" / "=" / "+" / "$" / ","
fn is_uric_no_slash_char(c: u8) -> bool {
    // Check unreserved first (alphanum / mark)
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')') ||
    // Check other allowed chars
    matches!(c, b';' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',')
}
fn uric_no_slash(input: &[u8]) -> ParseResult<&[u8]> {
    alt((escaped, take_while1(is_uric_no_slash_char)))(input)
}

// pchar = unreserved / escaped / ":" / "@" / "&" / "=" / "+" / "$" / ","
fn is_pchar_char(c: u8) -> bool {
    // Check unreserved first (alphanum / mark)
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')') ||
    // Check other allowed chars
    matches!(c, b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',')
}
fn pchar(input: &[u8]) -> ParseResult<&[u8]> {
    alt((escaped, take_while1(is_pchar_char)))(input)
}

// --- URI Components --- 

// scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
fn scheme(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        alpha,
        many0(alt((alpha, digit, tag(b"+"), tag(b"-"), tag(b"."))))
    ))(input)
}

// param = *pchar
fn param(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(pchar))(input)
}

// segment = *pchar *( ";" param )
fn segment(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        many0(pchar),
        many0(preceded(tag(b";"), param))
    ))(input)
}

// path-segments = segment *( "/" segment )
fn path_segments(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        segment,
        many0(preceded(tag(b"/"), segment))
    ))(input)
}

// abs-path = "/" path-segments
fn abs_path(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(preceded(tag(b"/"), path_segments))(input)
}

// query = *uric
fn query(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(uric))(input)
}

// reg-name-char = unreserved / escaped / "$" / "," / ";" / ":" / "@" / "&" / "=" / "+"
fn is_reg_name_char(c: u8) -> bool {
    // Check unreserved first (alphanum / mark)
    c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')') ||
    // Check other allowed chars
    matches!(c, b'$' | b',' | b';' | b':' | b'@' | b'&' | b'=' | b'+')
}

// reg-name = 1*( unreserved / escaped / "$" / "," / ";" / ":" / "@" / "&" / "=" / "+" )
fn reg_name(input: &[u8]) -> ParseResult<&[u8]> {
     recognize(many1(alt((escaped, take_while1(is_reg_name_char)))))(input)
}

// userinfo_bytes = userinfo parser returning bytes (internal detail, assume exists or adapt)
// Assumes userinfo from uri module returns the needed bytes before mapping
fn userinfo_bytes(input: &[u8]) -> ParseResult<&[u8]> {
     recognize(terminated(
            pair(crate::parser::uri::userinfo::user, 
                 opt(preceded(tag(b":"), crate::parser::uri::userinfo::password))),
            tag(b"@")
    ))(input)
}

// srvr = [ [ userinfo "@" ] hostport ]
// Note: userinfo includes the @
fn srvr(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(opt(userinfo_bytes), hostport))(input)
}

// authority = srvr / reg-name
fn authority(input: &[u8]) -> ParseResult<&[u8]> {
    alt((srvr, reg_name))(input)
}

// net-path = "//" authority [ abs-path ]
fn net_path(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(preceded(
        tag("//"), 
        pair(authority, opt(abs_path))
    ))(input)
}

// hier-part = ( net-path / abs-path ) [ "?" query ]
fn hier_part(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        alt((net_path, abs_path)),
        opt(preceded(tag(b"?"), query))
    ))(input)
}

// opaque-part = uric-no-slash *uric
fn opaque_part(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(uric_no_slash, many0(uric)))(input)
}

// absoluteURI = scheme ":" ( hier-part / opaque-part )
// Returns the full matched URI as &[u8]
pub fn parse_absolute_uri(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(
        tuple((
            scheme,
            tag(b":"),
            alt((hier_part, opaque_part))
        ))
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_absolute_uri_full() {
        // Hierarchical - net-path
        assert!(parse_absolute_uri(b"http://user:pass@example.com:80/p;a/t;h?q=1#f").is_ok());
        // Hierarchical - abs-path
        assert!(parse_absolute_uri(b"mailto:user@example.com?Subject=Test").is_ok()); 
        // Opaque
        assert!(parse_absolute_uri(b"urn:isbn:0451450523").is_ok());
        assert!(parse_absolute_uri(b"sip:conf%231@example.com").is_ok()); // Treat sip as scheme
        assert!(parse_absolute_uri(b"tel:+1-123-456-7890").is_ok());

        // Invalid scheme
        assert!(parse_absolute_uri(b"1http://example.com").is_err());
        // Invalid char in opaque
        assert!(parse_absolute_uri(b"urn:/some/path").is_err()); // / not allowed in uric-no-slash
        // Invalid hierarchical path start
        assert!(parse_absolute_uri(b"http:example.com").is_err()); 
        // Invalid authority
        assert!(parse_absolute_uri(b"http://user:pass@").is_err()); // Missing hostport
    }
} 