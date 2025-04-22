// RFC 3261 Section 25.1 & Authentication Sections
// Common components for challenge/credentials parsing

use crate::parser::common_chars::{lhex, token, digit};
use crate::parser::quoted::quoted_string;
use crate::parser::separators::{comma, equal, ldquot, rdquot};
use crate::parser::uri::parse_uri;
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;
use crate::types::auth::{Algorithm, Qop, AuthParam, DigestResponseValue, AuthenticationInfoParam};
use crate::types::Uri;
use nom::{
    branch::alt,
    bytes::complete as bytes,
    character::complete::char,
    combinator::{map, map_res, opt, value},
    multi::{many0, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair},
    IResult,
};
use std::str;

// auth-scheme = token
// Returns the scheme as String (case-insensitive comparison happens elsewhere)
pub(crate) fn auth_scheme(input: &[u8]) -> ParseResult<String> {
    map_res(token, |bytes| str::from_utf8(bytes).map(String::from))(input)
}

// auth-param-name = token
fn auth_param_name(input: &[u8]) -> ParseResult<&[u8]> {
    token(input)
}

// auth-value = token / quoted-string
fn auth_value(input: &[u8]) -> ParseResult<&[u8]> {
    alt((token, quoted_string))(input)
}

// auth-param = auth-param-name EQUAL auth-value
pub(crate) fn auth_param(input: &[u8]) -> ParseResult<AuthParam> {
    map_res(
        separated_pair(auth_param_name, equal, auth_value),
        |(name_bytes, value_bytes)| {
            let name = str::from_utf8(name_bytes)?.to_string();
            // Value requires unquoting if it was a quoted_string
            // TODO: Need an unquote helper or store raw bytes?
            // For now, store as String, assuming quoted_string gives content.
            let value = str::from_utf8(value_bytes)?.to_string();
            Ok(AuthParam { name, value })
        },
    )(input)
}

// realm-value = quoted-string
fn realm_value(input: &[u8]) -> ParseResult<&[u8]> {
    quoted_string(input)
}

// realm = "realm" EQUAL realm-value
pub(crate) fn realm(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("realm"), preceded(equal, realm_value)),
        |bytes| str::from_utf8(bytes).map(String::from), // Assuming realm_value returns content
    )(input)
}

// nonce-value = quoted-string
pub(crate) fn nonce_value(input: &[u8]) -> ParseResult<&[u8]> {
    quoted_string(input)
}

// nonce = "nonce" EQUAL nonce-value
pub(crate) fn nonce(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("nonce"), preceded(equal, nonce_value)),
        |bytes| str::from_utf8(bytes).map(String::from),
    )(input)
}

// opaque = "opaque" EQUAL quoted-string
pub(crate) fn opaque(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("opaque"), preceded(equal, quoted_string)),
        |bytes| str::from_utf8(bytes).map(String::from),
    )(input)
}

// stale = "stale" EQUAL ( "true" / "false" )
pub(crate) fn stale(input: &[u8]) -> ParseResult<bool> {
    preceded(
        bytes::tag_no_case("stale"),
        preceded(
            equal,
            alt((
                value(true, bytes::tag_no_case("true")),
                value(false, bytes::tag_no_case("false")),
            )),
        ),
    )(input)
}

// algorithm = "algorithm" EQUAL ( "MD5" / "MD5-sess" / token )
pub(crate) fn algorithm(input: &[u8]) -> ParseResult<Algorithm> {
    map_res(
        preceded(
            pair(bytes::tag_no_case(b"algorithm"), equal),
            alt((bytes::tag_no_case(b"MD5-sess"), bytes::tag_no_case(b"MD5"), token)),
        ),
        |bytes| {
            let s = str::from_utf8(bytes)?;
            Ok(match s.to_ascii_uppercase().as_str() {
                "MD5" => Algorithm::Md5,
                "MD5-SESS" => Algorithm::Md5Sess,
                other => Algorithm::Other(other.to_string()),
            })
        },
    )(input)
}

// qop-value = "auth" / "auth-int" / token
fn qop_value(input: &[u8]) -> ParseResult<Qop> {
    map_res(
        alt((bytes::tag_no_case(b"auth-int"), bytes::tag_no_case(b"auth"), token)),
        |bytes| {
            let s = str::from_utf8(bytes)?;
            Ok(match s.to_ascii_lowercase().as_str() {
                "auth" => Qop::Auth,
                "auth-int" => Qop::AuthInt,
                other => Qop::Other(other.to_string()),
            })
        },
    )(input)
}

// qop-options = "qop" EQUAL LDQUOT qop-value *("," qop-value) RDQUOT
// Returns Vec<Qop>
pub(crate) fn qop_options(input: &[u8]) -> ParseResult<Vec<Qop>> {
    preceded(
        pair(bytes::tag_no_case(b"qop"), equal),
        delimited(
            ldquot,
            separated_list1(bytes::tag(b","), qop_value),
            rdquot,
        ),
    )(input)
}

// username-value = quoted-string
fn username_value(input: &[u8]) -> ParseResult<&[u8]> {
    quoted_string(input)
}

// username = "username" EQUAL username-value
pub(crate) fn username(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("username"), preceded(equal, username_value)),
        |bytes| str::from_utf8(bytes).map(String::from),
    )(input)
}

// digest-uri-value = Request-URI (SIP-URI / SIPS-URI / absoluteURI)
// Uses the main parse_uri parser
fn digest_uri_value(input: &[u8]) -> ParseResult<Uri> {
    parse_uri(input)
}

// digest-uri = "uri" EQUAL LDQUOT digest-uri-value RDQUOT
pub(crate) fn digest_uri(input: &[u8]) -> ParseResult<Uri> {
    preceded(
        bytes::tag_no_case("uri"),
        preceded(
            equal,
            delimited(ldquot, digest_uri_value, rdquot)
        )
    )(input)
}

// request-digest = LDQUOT 32LHEX RDQUOT
fn request_digest(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(ldquot, recognize(many_m_n(32, 32, lhex)), rdquot)(input) 
}

// dresponse = "response" EQUAL request-digest
pub(crate) fn dresponse(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("response"), preceded(equal, request_digest)),
        |bytes| str::from_utf8(bytes).map(String::from)
    )(input)
}

// cnonce-value = nonce-value (quoted-string)
fn cnonce_value(input: &[u8]) -> ParseResult<&[u8]> {
    nonce_value(input)
}

// cnonce = "cnonce" EQUAL cnonce-value
pub(crate) fn cnonce(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("cnonce"), preceded(equal, cnonce_value)),
        |bytes| str::from_utf8(bytes).map(String::from)
    )(input)
}

// message-qop = "qop" EQUAL qop-value
// Note: This is different from qop-options in challenge
pub(crate) fn message_qop(input: &[u8]) -> ParseResult<Qop> {
    preceded(
        bytes::tag_no_case("qop"),
        preceded(equal, qop_value)
    )(input)
}

// nc-value = 8LHEX
fn nc_value(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many_m_n(8, 8, lhex))(input)
}

// nonce-count = "nc" EQUAL nc-value
pub(crate) fn nonce_count(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("nc"), preceded(equal, nc_value)),
        |bytes| str::from_utf8(bytes).map(String::from)
    )(input)
}

// nextnonce = "nextnonce" EQUAL nonce-value
pub(crate) fn nextnonce(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("nextnonce"), preceded(equal, nonce_value)),
        |bytes| str::from_utf8(bytes).map(String::from)
    )(input)
}

// response-digest = LDQUOT *LHEX RDQUOT
fn response_digest(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(ldquot, recognize(many0(lhex)), rdquot)(input)
}

// response-auth = "rspauth" EQUAL response-digest
pub(crate) fn response_auth(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("rspauth"), preceded(equal, response_digest)),
        |bytes| str::from_utf8(bytes).map(String::from)
    )(input)
}

// ainfo = nextnonce / message-qop / response-auth / cnonce / nonce-count
// Used by Authentication-Info parser
pub(crate) fn ainfo(input: &[u8]) -> ParseResult<AuthenticationInfoParam> {
    alt((
        map(nextnonce, AuthenticationInfoParam::NextNonce),
        map(message_qop, AuthenticationInfoParam::Qop),
        map(response_auth, AuthenticationInfoParam::ResponseAuth),
        map(cnonce, AuthenticationInfoParam::Cnonce),
        map(nonce_count, AuthenticationInfoParam::NonceCount),
    ))(input)
} 