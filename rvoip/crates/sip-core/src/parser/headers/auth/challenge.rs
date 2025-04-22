// RFC 3261 Section 22 & 25.1
// Parser for the challenge part of Authenticate headers

use super::common::*;
use crate::parser::common::comma_separated_list1;
use crate::parser::common_chars::token;
use crate::parser::quoted::quoted_string;
use crate::parser::separators::{comma, equal, ldquot, rdquot};
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;
use crate::types::auth::{AuthParam, Challenge, DigestChallenge, DigestParam}; // Assume types exist
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    character::complete::space1,
    combinator::{map, map_res, opt, recognize},
    multi::{many0, separated_list0},
    sequence::{delimited, pair, preceded, tuple},
    IResult,
};
use std::str;

// domain = "domain" EQUAL LDQUOT URI *( 1*SP URI ) RDQUOT
// URI = absoluteURI / abs-path
// Simplified: Parses the raw content within the quotes.
// TODO: Implement actual URI list parsing within the quoted string if needed.
fn domain(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(
            tag_no_case("domain"),
            preceded(
                equal,
                delimited(ldquot, recognize(many0(alt((token, tag(b" "))))), rdquot), // Use b" "
            ),
        ),
        |bytes| str::from_utf8(bytes).map(String::from),
    )(input)
}

// digest-cln = realm / domain / nonce / opaque / stale / algorithm
//              / qop-options / auth-param
fn digest_cln(input: &[u8]) -> ParseResult<DigestParam> {
    alt((
        map(realm, DigestParam::Realm),
        map(domain, DigestParam::Domain),
        map(nonce, DigestParam::Nonce),
        map(opaque, DigestParam::Opaque),
        map(stale, DigestParam::Stale),
        map(algorithm, DigestParam::Algorithm),
        map(qop_options, DigestParam::Qop),
        map(auth_param, DigestParam::Param), // Must be last due to token overlap
    ))(input)
}

// other-challenge = auth-scheme LWS auth-param *(COMMA auth-param)
fn other_challenge(input: &[u8]) -> ParseResult<Challenge> {
    map(
        pair(
            auth_scheme,
            preceded(lws, comma_separated_list1(auth_param)), // Needs at least one param
        ),
        |(scheme, params)| Challenge::Other { scheme, params },
    )(input)
}

// challenge = ("Digest" LWS digest-cln *(COMMA digest-cln))
//             / other-challenge
pub(crate) fn challenge(input: &[u8]) -> ParseResult<Challenge> {
    alt((
        map(
            preceded(
                tag_no_case("Digest"),
                preceded(lws, comma_separated_list1(digest_cln)), // digest-cln + *(COMMA digest-cln)
            ),
            |params| Challenge::Digest { params },
        ),
        other_challenge,
    ))(input)
}

// other-response-auth = auth-param *(COMMA auth-param)
fn other_response_auth(input: &[u8]) -> ParseResult<Vec<Param>> {
    comma_separated_list1(auth_param)(input)
}

// quoted-string from RFC 2616, Section 2.2
// quoted-pair = "\\" CHAR
fn quoted_pair(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(tag(b"\\".as_slice()), take(1usize)))(input) // Use .as_slice() for b"\\"
}

// qdtext = <any TEXT except <"> >
fn qdtext(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(|c| c != b'"' && c != b'\r' && c != b'\n')(input) // Exclude " and CR/LF
}

// quoted-string = ( <"> *(qdtext / quoted-pair ) <"> )
fn quoted_string(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(
        ldquot, 
        recognize(many0(alt((qdtext, quoted_pair)))), 
        rdquot
    )(input)
}

// Convenience function for quoted string within auth params
fn auth_quoted_string(input: &[u8]) -> ParseResult<&[u8]> {
     delimited(ldquot, recognize(many0(alt((token, tag(b" ".as_slice()))))), rdquot)(input) // Use b" ".as_slice()
}

// auth-param = token EQUAL ( token / quoted-string )
// ... existing code ... 