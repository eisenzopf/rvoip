// RFC 3261 Section 22 & 25.1
// Parser for the credentials part of Authorization headers

use super::common::*; // Import all common auth parsers
use crate::parser::common::comma_separated_list1;
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;
use crate::types::auth::{AuthParam, Credentials, DigestCredential, DigestParam as DigestRespParam}; // Reuse DigestParam enum?
use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{map},
    sequence::{pair, preceded},
    IResult,
};

// dig-resp = username / realm / nonce / digest-uri
//          / dresponse / algorithm / cnonce
//          / opaque / message-qop / nonce-count / auth-param
// Assuming DigestRespParam enum covers these
fn dig_resp(input: &[u8]) -> ParseResult<DigestRespParam> {
    alt((
        map(username, DigestRespParam::Username),
        map(realm, DigestRespParam::Realm),
        map(nonce, DigestRespParam::Nonce),
        map(digest_uri, DigestRespParam::Uri),
        map(dresponse, DigestRespParam::Response),
        map(algorithm, DigestRespParam::Algorithm),
        map(cnonce, DigestRespParam::Cnonce),
        map(opaque, DigestRespParam::Opaque),
        map(message_qop, DigestRespParam::MsgQop),
        map(nonce_count, DigestRespParam::NonceCount),
        map(auth_param, DigestRespParam::Param), // Must be last
    ))(input)
}

// other-response = auth-scheme LWS auth-param *(COMMA auth-param)
fn other_response(input: &[u8]) -> ParseResult<Credentials> {
    map(
        pair(
            auth_scheme,
            preceded(lws, comma_separated_list1(auth_param)), // Needs at least one param
        ),
        |(scheme, params)| Credentials::Other { scheme, params },
    )(input)
}

// credentials = ("Digest" LWS digest-response) / other-response
// digest-response = dig-resp *(COMMA dig-resp)
pub(crate) fn credentials(input: &[u8]) -> ParseResult<Credentials> {
    alt((
        map(
            preceded(
                tag_no_case("Digest"),
                preceded(lws, comma_separated_list1(dig_resp)), // dig-resp + *(COMMA dig-resp)
            ),
            |params| Credentials::Digest { params },
        ),
        other_response,
    ))(input)
} 