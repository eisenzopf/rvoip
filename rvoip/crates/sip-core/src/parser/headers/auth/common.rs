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
    bytes::complete::{self as bytes, take_while1},
    character::complete::char,
    combinator::{map, map_res, opt, value, recognize, verify},
    multi::{many0, separated_list0, separated_list1, many_m_n},
    sequence::{delimited, pair, preceded, separated_pair},
    IResult,
    error::{Error as NomError, ErrorKind},
};
use std::str::{self, FromStr, Utf8Error}; // Add Utf8Error

// Helper function to check if a byte is a valid token character
fn is_token_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || 
    c == b'-' || c == b'.' || c == b'!' || c == b'%' || c == b'*' || 
    c == b'_' || c == b'+' || c == b'`' || c == b'\'' || c == b'~'
}

// auth-scheme = token
// Returns the scheme as String (case-insensitive comparison happens elsewhere)
pub fn auth_scheme(input: &[u8]) -> ParseResult<String> {
    // First check if the input contains '@' which is not a valid token character
    if input.iter().any(|&c| c == b'@') {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::TakeWhile1)));
    }
    
    let mut i = 0;
    let len = input.len();
    
    // Check if we have at least one character
    if len == 0 {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::TakeWhile1)));
    }
    
    // Verify the first character is valid (not whitespace)
    if !is_token_char(input[0]) {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::TakeWhile1)));
    }
    
    // Find the end of the token
    while i < len && is_token_char(input[i]) {
        i += 1;
    }
    
    // Convert to string
    match str::from_utf8(&input[0..i]) {
        Ok(s) => Ok((&input[i..], s.to_string())),
        Err(_) => Err(nom::Err::Error(NomError::new(input, ErrorKind::AlphaNumeric)))
    }
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

// Helper function to check if a byte is a hex digit (0-9, a-f, A-F)
fn is_hex_digit(c: u8) -> bool {
    (c >= b'0' && c <= b'9') || (c >= b'A' && c <= b'F') || (c >= b'a' && c <= b'f')
}

// request-digest = LDQUOT 32LHEX RDQUOT ; RFC 2617 specifies 32 hex digits
// However, we'll allow other lengths as some implementations don't adhere strictly to 32
fn request_digest(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(
        ldquot,
        take_while1(is_hex_digit),
        rdquot
    )(input)
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
    // Ensure we get exactly 8 hex digits (uppercase or lowercase)
    if input.len() < 8 {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::TakeWhile1)));
    }
    
    // Check that the first 8 characters are all hex digits (0-9, a-f, A-F)
    for &c in &input[0..8] {
        if !(c.is_ascii_digit() || (c >= b'a' && c <= b'f') || (c >= b'A' && c <= b'F')) {
            return Err(nom::Err::Error(NomError::new(input, ErrorKind::TakeWhile1)));
        }
    }
    
    // Return the 8 characters
    Ok((&input[8..], &input[0..8]))
}

// nonce-count = "nc" EQUAL nc-value
// Returns u32 (parsed hex)
pub fn nonce_count(input: &[u8]) -> ParseResult<u32> {
    let (rem, nc_bytes) = preceded(
        bytes::tag_no_case("nc"),
        preceded(equal, nc_value)
    )(input)?;
    
    // Convert Utf8Error and ParseIntError into nom::Err::Failure
    let s = str::from_utf8(nc_bytes)
        .map_err(|_| nom::Err::Error(NomError::new(nc_bytes, ErrorKind::Char)))?;
    
    let value = u32::from_str_radix(s, 16)
        .map_err(|_| nom::Err::Error(NomError::new(nc_bytes, ErrorKind::Digit)))?;
    
    Ok((rem, value))
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
        map(domain, |d| DigestParam::Domain(vec![d])), // Simple domain handling, not splitting yet
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

// digest-credential = digest-param *(COMMA digest-param)
// Returns a list of DigestParam enum variants
pub fn digest_credential(input: &[u8]) -> ParseResult<Vec<DigestParam>> {
    crate::parser::common::comma_separated_list1(digest_param)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{DigestParam, Qop, Algorithm, AuthParam};

    #[test]
    fn test_auth_scheme() {
        let (rem, scheme) = auth_scheme(b"Digest").unwrap();
        assert!(rem.is_empty());
        assert_eq!(scheme, "Digest");

        let (rem, scheme) = auth_scheme(b"Basic").unwrap();
        assert!(rem.is_empty());
        assert_eq!(scheme, "Basic");

        let (rem, scheme) = auth_scheme(b"CustomAuth").unwrap();
        assert!(rem.is_empty());
        assert_eq!(scheme, "CustomAuth");

        // Test with trailing data
        let (rem, scheme) = auth_scheme(b"Digest ").unwrap();
        assert_eq!(rem, b" ");
        assert_eq!(scheme, "Digest");

        // Test with invalid token characters
        assert!(auth_scheme(b"").is_err());
        assert!(auth_scheme(b" Digest").is_err());
        assert!(auth_scheme(b"Digest@Invalid").is_err());
    }

    #[test]
    fn test_auth_param() {
        let (rem, param) = auth_param(b"realm=\"example.com\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param.name, "realm");
        assert_eq!(param.value, "example.com");

        // Test with quoted string containing special characters
        let (rem, param) = auth_param(b"opaque=\"AB\\\"CD\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param.name, "opaque");
        assert_eq!(param.value, "AB\\\"CD");

        // Test with token value
        let (rem, param) = auth_param(b"stale=false").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param.name, "stale");
        assert_eq!(param.value, "false");

        // Test with trailing content
        let (rem, param) = auth_param(b"realm=\"example.com\",").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(param.name, "realm");
        assert_eq!(param.value, "example.com");

        // Test invalid cases
        assert!(auth_param(b"=value").is_err()); // Missing name
        assert!(auth_param(b"name=").is_err()); // Missing value
        assert!(auth_param(b"name\"value").is_err()); // Missing equal sign
    }

    #[test]
    fn test_realm() {
        let (rem, val) = realm(b"realm=\"example.com\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "example.com");

        // Test with different casing
        let (rem, val) = realm(b"REALM=\"test.org\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "test.org");

        // Test with trailing content
        let (rem, val) = realm(b"realm=\"sip.example.org\",").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(val, "sip.example.org");

        // Test invalid cases
        assert!(realm(b"realm=token").is_err()); // Value must be quoted
        assert!(realm(b"real=\"example.com\"").is_err()); // Wrong parameter name
    }

    #[test]
    fn test_nonce() {
        let (rem, val) = nonce(b"nonce=\"1234567890abcdef\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "1234567890abcdef");

        // Test with different casing
        let (rem, val) = nonce(b"NONCE=\"ABCDEF1234567890\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "ABCDEF1234567890");

        // Test with trailing content
        let (rem, val) = nonce(b"nonce=\"1234567890abcdef\",").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(val, "1234567890abcdef");

        // Test invalid cases
        assert!(nonce(b"nonce=token").is_err()); // Value must be quoted
    }

    #[test]
    fn test_stale() {
        let (rem, val) = stale(b"stale=true").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, true);

        let (rem, val) = stale(b"stale=false").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, false);

        // Test with different casing
        let (rem, val) = stale(b"STALE=TRUE").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, true);

        // Test with trailing content
        let (rem, val) = stale(b"stale=false,").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(val, false);

        // Test invalid cases
        assert!(stale(b"stale=yes").is_err()); // Must be true or false
        assert!(stale(b"stale=\"true\"").is_err()); // Must not be quoted
    }

    #[test]
    fn test_algorithm() {
        let (rem, val) = algorithm(b"algorithm=MD5").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Algorithm::Md5);

        let (rem, val) = algorithm(b"algorithm=MD5-sess").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Algorithm::Md5Sess);

        // Test with custom algorithm token
        let (rem, val) = algorithm(b"algorithm=SHA-256").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Algorithm::Sha256);

        // Test with different casing
        let (rem, val) = algorithm(b"ALGORITHM=md5").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Algorithm::Md5);

        // Test with trailing content
        let (rem, val) = algorithm(b"algorithm=MD5,").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(val, Algorithm::Md5);

        // Test invalid cases
        assert!(algorithm(b"algorithm=\"MD5\"").is_err()); // Must not be quoted
    }

    #[test]
    fn test_qop_options() {
        let (rem, vals) = qop_options(b"qop=\"auth\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0], Qop::Auth);

        let (rem, vals) = qop_options(b"qop=\"auth,auth-int\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(vals.len(), 2);
        assert_eq!(vals[0], Qop::Auth);
        assert_eq!(vals[1], Qop::AuthInt);

        // Test with custom qop
        let (rem, vals) = qop_options(b"qop=\"auth,auth-int,custom\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(vals.len(), 3);
        assert_eq!(vals[2], Qop::Other("custom".to_string()));

        // Test with different casing
        let (rem, vals) = qop_options(b"QOP=\"AUTH\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(vals[0], Qop::Auth);

        // Test with trailing content
        let (rem, vals) = qop_options(b"qop=\"auth\",").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(vals[0], Qop::Auth);

        // Test invalid cases
        assert!(qop_options(b"qop=auth").is_err()); // Value must be quoted
        assert!(qop_options(b"qop=\"\"").is_err()); // Empty list not allowed
    }

    #[test]
    fn test_username() {
        let (rem, val) = username(b"username=\"alice\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "alice");

        // Test with special characters
        let (rem, val) = username(b"username=\"alice@example.com\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "alice@example.com");

        // Test with different casing
        let (rem, val) = username(b"USERNAME=\"bob\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "bob");

        // Test with trailing content
        let (rem, val) = username(b"username=\"carol\",").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(val, "carol");

        // Test invalid cases
        assert!(username(b"username=alice").is_err()); // Value must be quoted
    }

    #[test]
    fn test_dresponse() {
        // Valid hex response
        let (rem, val) = dresponse(b"response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d");

        // Test with shorter response (30 characters)
        let (rem, val) = dresponse(b"response=\"dfe56131d1958046689d83306477ecc\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "dfe56131d1958046689d83306477ecc");

        // Test with uppercase hex
        let (rem, val) = dresponse(b"response=\"1A2B3C4D5E6F7A8B9C0D1E2F3A4B5C6D\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "1A2B3C4D5E6F7A8B9C0D1E2F3A4B5C6D");

        // Test with different casing in parameter name
        let (rem, val) = dresponse(b"RESPONSE=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d");

        // Test with trailing content
        let (rem, val) = dresponse(b"response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\",").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(val, "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d");

        // Test invalid cases
        assert!(dresponse(b"response=\"xyz\"").is_err()); // Invalid hex chars
        assert!(dresponse(b"response=\"\"").is_err()); // Empty value
        assert!(dresponse(b"response=1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d").is_err()); // Not quoted
    }

    #[test]
    fn test_cnonce() {
        let (rem, val) = cnonce(b"cnonce=\"0a1b2c3d4e5f\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "0a1b2c3d4e5f");

        // Test with different casing
        let (rem, val) = cnonce(b"CNONCE=\"0A1B2C3D4E5F\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, "0A1B2C3D4E5F");

        // Test with trailing content
        let (rem, val) = cnonce(b"cnonce=\"0a1b2c3d4e5f\",").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(val, "0a1b2c3d4e5f");

        // Test invalid cases
        assert!(cnonce(b"cnonce=0a1b2c3d4e5f").is_err()); // Value must be quoted
    }

    #[test]
    fn test_message_qop() {
        let (rem, val) = message_qop(b"qop=auth").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Qop::Auth);

        let (rem, val) = message_qop(b"qop=auth-int").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Qop::AuthInt);

        // Test with custom qop
        let (rem, val) = message_qop(b"qop=custom").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Qop::Other("custom".to_string()));

        // Test with different casing
        let (rem, val) = message_qop(b"QOP=AUTH").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, Qop::Auth);

        // Test with trailing content
        let (rem, val) = message_qop(b"qop=auth,").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(val, Qop::Auth);

        // Test invalid cases
        assert!(message_qop(b"qop=\"auth\"").is_err()); // Value must not be quoted
    }

    #[test]
    fn test_nonce_count() {
        let (rem, val) = nonce_count(b"nc=00000001").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 1);

        // Test with higher value
        let (rem, val) = nonce_count(b"nc=000ABCDE").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 0xABCDE);

        // Test with different casing
        let (rem, val) = nonce_count(b"NC=00000001").unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, 1);

        // Test with trailing content
        let (rem, val) = nonce_count(b"nc=00000001,").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(val, 1);

        // Test invalid cases
        assert!(nonce_count(b"nc=1").is_err()); // Must be 8 hex chars
        assert!(nonce_count(b"nc=0000000G").is_err()); // Invalid hex char
        assert!(nonce_count(b"nc=\"00000001\"").is_err()); // Value must not be quoted
    }

    #[test]
    fn test_digest_param() {
        // Test realm parameter
        let (rem, param) = digest_param(b"realm=\"example.com\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, DigestParam::Realm("example.com".to_string()));

        // Test nonce parameter
        let (rem, param) = digest_param(b"nonce=\"1234567890abcdef\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, DigestParam::Nonce("1234567890abcdef".to_string()));

        // Test opaque parameter
        let (rem, param) = digest_param(b"opaque=\"someopaquedata\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, DigestParam::Opaque("someopaquedata".to_string()));

        // Test stale parameter
        let (rem, param) = digest_param(b"stale=false").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, DigestParam::Stale(false));

        // Test algorithm parameter
        let (rem, param) = digest_param(b"algorithm=MD5").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, DigestParam::Algorithm(Algorithm::Md5));

        // Test qop parameter
        let (rem, param) = digest_param(b"qop=\"auth,auth-int\"").unwrap();
        assert!(rem.is_empty());
        match param {
            DigestParam::Qop(qops) => {
                assert_eq!(qops.len(), 2);
                assert_eq!(qops[0], Qop::Auth);
                assert_eq!(qops[1], Qop::AuthInt);
            },
            _ => panic!("Expected Qop parameter"),
        }

        // Test unknown parameter
        let (rem, param) = digest_param(b"custom=\"value\"").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, DigestParam::Param(AuthParam { 
            name: "custom".to_string(), 
            value: "value".to_string() 
        }));

        // Test trailing content
        let (rem, param) = digest_param(b"realm=\"example.com\",").unwrap();
        assert_eq!(rem, b",");
        assert_eq!(param, DigestParam::Realm("example.com".to_string()));
    }

    #[test]
    fn test_digest_credential() {
        // Test a digest credential with multiple parameters
        let input = b"username=\"alice\",realm=\"example.com\",nonce=\"1234abcd\",uri=\"sip:bob@example.com\",response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\",algorithm=MD5,cnonce=\"0a1b2c3d\",qop=auth,nc=00000001";
        let (rem, params) = digest_credential(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 9);

        // Verify specific parameters
        assert!(params.iter().any(|p| match p {
            DigestParam::Username(u) => u == "alice",
            _ => false
        }));
        assert!(params.iter().any(|p| match p {
            DigestParam::Realm(r) => r == "example.com",
            _ => false
        }));
        assert!(params.iter().any(|p| match p {
            DigestParam::Nonce(n) => n == "1234abcd",
            _ => false
        }));
        assert!(params.iter().any(|p| match p {
            DigestParam::Uri(u) => u.to_string() == "sip:bob@example.com",
            _ => false
        }));
        assert!(params.iter().any(|p| match p {
            DigestParam::Response(r) => r == "1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d",
            _ => false
        }));
        assert!(params.iter().any(|p| match p {
            DigestParam::Algorithm(a) => *a == Algorithm::Md5,
            _ => false
        }));
        assert!(params.iter().any(|p| match p {
            DigestParam::Cnonce(c) => c == "0a1b2c3d",
            _ => false
        }));
        assert!(params.iter().any(|p| match p {
            DigestParam::MsgQop(q) => *q == Qop::Auth,
            _ => false
        }));
        assert!(params.iter().any(|p| match p {
            DigestParam::NonceCount(nc) => *nc == 1,
            _ => false
        }));

        // Test with a subset of parameters
        let input = b"realm=\"example.com\",nonce=\"1234abcd\",response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\"";
        let (rem, params) = digest_credential(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);

        // Test with trailing content
        let input = b"realm=\"example.com\",nonce=\"1234abcd\";more-content";
        let (rem, params) = digest_credential(input).unwrap();
        assert_eq!(rem, b";more-content");
        assert_eq!(params.len(), 2);

        // Test invalid cases (missing comma)
        // The parser should only match the first param and return the remainder
        let input = b"realm=\"example.com\" nonce=\"1234abcd\"";
        // When we parse this, we should only get the first parameter
        let result = digest_credential(input);
        match result {
            Ok((rem, params)) => {
                assert_eq!(params.len(), 1);
                assert_eq!(rem, b" nonce=\"1234abcd\"");
            },
            Err(_) => {
                panic!("Parser should parse the first parameter and return remainder");
            }
        }
    }
} 