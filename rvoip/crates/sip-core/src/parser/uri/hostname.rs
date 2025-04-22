use nom::{
    bytes::complete::{tag, take_while1},
    combinator::{map_res, recognize},
    multi::many0,
    sequence::pair,
    IResult,
};
use std::str;

use crate::types::uri::Host;
use crate::parser::ParseResult;

// hostname = *( domainlabel "." ) toplabel [ "." ]
// domainlabel = alphanum / alphanum *( alphanum / "-" ) alphanum
// toplabel = ALPHA / ALPHA *( alphanum / "-" ) alphanum
// Simplified: Recognizes sequences of alphanumeric/hyphen labels separated by dots.
// Does not enforce toplabel/domainlabel specific content rules, relies on higher-level validation if needed.
pub(crate) fn hostname(input: &[u8]) -> ParseResult<Host> {
    map_res(
        recognize(
            pair(
                many0(pair(take_while1(|c:u8| c.is_ascii_alphanumeric() || c == b'-'), tag(b"."))),
                take_while1(|c:u8| c.is_ascii_alphanumeric() || c == b'-')
                // Optional trailing dot is ignored by recognize logic here, handled if needed by callers.
            )
        ),
         |bytes| {
            // Basic validation: Ensure not empty and doesn't start/end with hyphen (common basic check)
            if bytes.is_empty() || bytes[0] == b'-' || bytes[bytes.len() - 1] == b'-' {
                Err(nom::Err::Failure((input, nom::error::ErrorKind::Verify)))
            } else {
                str::from_utf8(bytes)
                    .map(|s| Host::Domain(s.to_string()))
                    .map_err(|_| nom::Err::Failure((input, nom::error::ErrorKind::Char)))
            }
        }
    )(input)
} 