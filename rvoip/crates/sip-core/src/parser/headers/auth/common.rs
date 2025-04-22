// RFC 3261 Section 25.1 & Authentication Sections
// Common components for challenge/credentials parsing

use crate::parser::common_chars::{lhex, digit};
use crate::parser::token::token;
use crate::parser::quoted::quoted_string;
use crate::parser::separators::{comma, equal, ldquot, rdquot};
use crate::parser::uri::parse_uri;
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;
// Keep types used internally or returned by base parsers
use crate::types::auth::{Algorithm, Qop, AuthParam, DigestParam, AuthenticationInfoParam};
use crate::types::Uri;
use nom::{
    branch::alt,
    bytes::complete as bytes,
    character::complete::char,
    combinator::{map, map_res, opt, value, recognize},
    multi::{many0, separated_list0, separated_list1, many_m_n},
    sequence::{delimited, pair, preceded, separated_pair},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError}, // Import error handling types
};
use std::str::{self, FromStr, Utf8Error}; // Add Utf8Error

// auth-scheme = token
// Returns the scheme as String (case-insensitive comparison happens elsewhere)
pub fn auth_scheme(input: &[u8]) -> ParseResult<String> {
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
// Returns AuthParam struct { name: String, value: String }
pub fn auth_param(input: &[u8]) -> ParseResult<AuthParam> {
    map_res(
        separated_pair(auth_param_name, equal, auth_value),
        |(name_bytes, value_bytes)| {
            // Specify error type for map_res
            let name = str::from_utf8(name_bytes).map_err(|e| NomError::new(name_bytes, ErrorKind::Char))?.to_string();
            let value = str::from_utf8(value_bytes).map_err(|e| NomError::new(value_bytes, ErrorKind::Char))?.to_string();
            Ok::<_, NomError<&[u8]>>(AuthParam { name, value }) // Explicit Ok type needed by map_res
        },
    )(input)
}

// realm-value = quoted-string
fn realm_value(input: &[u8]) -> ParseResult<&[u8]> {
    quoted_string(input)
}

// realm = "realm" EQUAL realm-value
// Returns String
pub fn realm(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("realm"), preceded(equal, realm_value)),
        |bytes| str::from_utf8(bytes).map(String::from), // Assuming realm_value returns content
    )(input)
}

// nonce-value = quoted-string
pub fn nonce_value(input: &[u8]) -> ParseResult<&[u8]> {
    quoted_string(input)
}

// nonce = "nonce" EQUAL nonce-value
// Returns String
pub fn nonce(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("nonce"), preceded(equal, nonce_value)),
        |bytes| str::from_utf8(bytes).map(String::from),
    )(input)
}

// opaque = "opaque" EQUAL quoted-string
// Returns String
pub fn opaque(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("opaque"), preceded(equal, quoted_string)),
        |bytes| str::from_utf8(bytes).map(String::from),
    )(input)
}

// domain = "domain" EQUAL LDQUOT URI *( 1*SP URI ) RDQUOT ; Not quite right per RFC
// RFC 3261 says domain is a quoted string containing a space-separated list of URIs.
// Let's simplify for now and parse as a single quoted string.
// Returns String
pub fn domain(input: &[u8]) -> ParseResult<String> {
     map_res(
        preceded(bytes::tag_no_case("domain"), preceded(equal, quoted_string)),
        |bytes| str::from_utf8(bytes).map(String::from),
    )(input)
    // TODO: Parse the actual list of URIs inside the string if needed later
}

// stale = "stale" EQUAL ( "true" / "false" )
// Returns bool
pub fn stale(input: &[u8]) -> ParseResult<bool> {
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

// algorithm_tag = "MD5" / "MD5-sess" / token ; RFC uses just `token` but specifies known values
// algorithm = "algorithm" EQUAL algorithm_tag
// Returns Algorithm enum
pub fn algorithm(input: &[u8]) -> ParseResult<Algorithm> {
    map_res(
        preceded(bytes::tag_no_case("algorithm"), preceded(equal, token)),
        |bytes| Algorithm::from_str(str::from_utf8(bytes)?),
    )(input)
}


// qop-value = "auth" / "auth-int" / token
// Returns Qop enum
fn qop_value(input: &[u8]) -> ParseResult<Qop> {
    map_res(token, |bytes| Qop::from_str(str::from_utf8(bytes)?))(input)
}

// qop-options = "qop" EQUAL LDQUOT qop-value *("," qop-value) RDQUOT
// Returns Vec<Qop>
pub fn qop_options(input: &[u8]) -> ParseResult<Vec<Qop>> {
    preceded(
        pair(bytes::tag_no_case(b"qop"), equal),
        delimited(
            ldquot,
            separated_list1(comma, qop_value), // Use comma separator
            rdquot,
        ),
    )(input)
}

// username-value = quoted-string
fn username_value(input: &[u8]) -> ParseResult<&[u8]> {
    quoted_string(input)
}

// username = "username" EQUAL username-value
// Returns String
pub fn username(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("username"), preceded(equal, username_value)),
        |bytes| str::from_utf8(bytes).map(String::from),
    )(input)
}

// digest-uri-value = Request-URI (SIP-URI / SIPS-URI / absoluteURI)
// Parses the content of the quoted string.
fn digest_uri_value_content(input: &[u8]) -> ParseResult<&[u8]> {
    quoted_string(input)
}

// digest-uri = "uri" EQUAL LDQUOT digest-uri-value RDQUOT ; Corrected grammar
// Returns Uri struct
pub fn digest_uri<'a>(input: &'a [u8]) -> ParseResult<'a, Uri> {
    let (rest, _) = preceded(
        bytes::tag_no_case("uri"),
        equal
    )(input)?;
    
    let (rest, value) = quoted_string(rest)?;
    
    // Parse the URI from the quoted string
    let (_, uri) = parse_uri(value)?;
    
    Ok((rest, uri))
}


// request-digest = LDQUOT 32LHEX RDQUOT ; RFC 2617 specifies 32 hex digits
fn request_digest(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(ldquot, recognize(many_m_n(32, 32, lhex)), rdquot)(input)
}

// dresponse = "response" EQUAL request-digest
// Returns String (hex digest)
pub fn dresponse(input: &[u8]) -> ParseResult<String> {
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
// Returns String
pub fn cnonce(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("cnonce"), preceded(equal, cnonce_value)),
        |bytes| str::from_utf8(bytes).map(String::from)
    )(input)
}

// message-qop = "qop" EQUAL qop-value
// Note: This is different from qop-options in challenge, value is not quoted
// Returns Qop enum
pub fn message_qop(input: &[u8]) -> ParseResult<Qop> {
    preceded(
        bytes::tag_no_case("qop"),
        preceded(equal, qop_value)
    )(input)
}

// nc-value = 8LHEX
// Returns &[u8] hex digits
fn nc_value(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many_m_n(8, 8, lhex))(input)
}

// nonce-count = "nc" EQUAL nc-value
// Returns u32 (parsed hex)
pub fn nonce_count(input: &[u8]) -> ParseResult<u32> {
    map_res(
        preceded(bytes::tag_no_case("nc"), preceded(equal, nc_value)),
        |bytes| {
            // Convert Utf8Error and ParseIntError into nom::Err::Failure
            let s = str::from_utf8(bytes).map_err(|_| NomError::new(bytes, ErrorKind::Char))?;
            u32::from_str_radix(s, 16).map_err(|_| NomError::new(bytes, ErrorKind::Digit))
        }
    )(input)
}


// nextnonce = "nextnonce" EQUAL nonce-value
// Returns String
pub fn nextnonce(input: &[u8]) -> ParseResult<String> {
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
// Returns String (hex digest)
pub fn response_auth(input: &[u8]) -> ParseResult<String> {
    map_res(
        preceded(bytes::tag_no_case("rspauth"), preceded(equal, response_digest)),
        |bytes| str::from_utf8(bytes).map(String::from)
    )(input)
}

// ainfo = nextnonce / message-qop / response-auth / cnonce / nonce-count
// Used by Authentication-Info parser
// Returns AuthenticationInfoParam enum variant
pub fn ainfo(input: &[u8]) -> ParseResult<AuthenticationInfoParam> {
    alt((
        map(nextnonce, AuthenticationInfoParam::NextNonce),
        map(message_qop, AuthenticationInfoParam::Qop), // message_qop returns Qop
        map(response_auth, AuthenticationInfoParam::ResponseAuth),
        map(cnonce, AuthenticationInfoParam::Cnonce),
        map(nonce_count, AuthenticationInfoParam::NonceCount), // nonce_count returns u32
    ))(input)
}

// digest-param = realm / nonce / opaque / stale / algorithm / domain / qop-options ; Challenge params
//              / username / digest-uri / dresponse / cnonce / message-qop / nonce-count ; Credentials params
//              / auth-param ; Fallback for others
// Returns DigestParam enum variant
pub fn digest_param(input: &[u8]) -> ParseResult<DigestParam> {
    alt((
        // Challenge & Credentials params first (most specific tags)
        map(realm, DigestParam::Realm),
        map(nonce, DigestParam::Nonce),
        map(opaque, DigestParam::Opaque),
        map(algorithm, DigestParam::Algorithm), // algorithm returns Algorithm
        // Challenge Only
        // map(domain, |d| DigestParam::Domain(vec![d])), // Needs proper split later
        map(stale, DigestParam::Stale),
        map(qop_options, DigestParam::Qop), // qop_options returns Vec<Qop>
        // Credentials Only
        map(username, DigestParam::Username),
        map(digest_uri, DigestParam::Uri), // digest_uri returns Uri
        map(dresponse, DigestParam::Response),
        map(cnonce, DigestParam::Cnonce),
        map(message_qop, DigestParam::MsgQop), // message_qop returns Qop
        map(nonce_count, DigestParam::NonceCount), // nonce_count returns u32
        // Generic fallback MUST be last
        map(auth_param, DigestParam::Param),
    ))(input)
} 