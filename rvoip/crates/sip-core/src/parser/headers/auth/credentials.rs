// RFC 3261 Section 22 & 25.1
// Parser for the credentials part of Authorization headers

// Use the new digest_param parser from common
use super::common::{auth_scheme, digest_param, auth_param}; 
use crate::parser::common::comma_separated_list1;
use crate::parser::common_chars::token; // Need token for Basic scheme
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;
// Import the necessary types from types::auth
use crate::types::auth::{AuthParam, Credentials, DigestParam, Scheme};
use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_till1}, // Need take_till1 for Basic token
    combinator::{map, map_res, recognize},
    sequence::{pair, preceded},
    IResult,
};
use std::str::FromStr;


// Basic credentials token (base64 encoded part after "Basic ")
// RFC 7617: #auth-param BWS token68
// token68 = 1*( ALPHA / DIGIT / "-" / "." / "_" / "~" / "+" / "/" ) *"="
// Simplified: Take everything until EOL or comma (as it's usually the only thing)
fn basic_credentials_token(input: &[u8]) -> ParseResult<&[u8]> {
    // This might be too simple; a robust parser would check Base64 chars.
    recognize(take_till1(|c| c == b'\r' || c == b'\n' || c == b','))(input)
}

// credentials = ("Digest" LWS digest-response)
//             / ("Basic" LWS basic-credentials)
//             / other-response
// digest-response = digest-param *(COMMA digest-param)
// basic-credentials = base64-user-pass (token68)
// other-response = auth-scheme LWS auth-param *(COMMA auth-param)
pub fn credentials(input: &[u8]) -> ParseResult<Credentials> {
    let (rem, scheme_str) = auth_scheme(input)?;
    let (rem, _) = lws(rem)?;

    match Scheme::from_str(&scheme_str) {
        Ok(Scheme::Digest) => {
            // Parse comma-separated list of digest params
            let (rem, params) = comma_separated_list1(digest_param)(rem)?;
            Ok((rem, Credentials::Digest { params }))
        }
        Ok(Scheme::Basic) => {
            // Parse the Base64 token
            let (rem, token_bytes) = basic_credentials_token(rem)?;
            let token = std::str::from_utf8(token_bytes)
                            .map_err(|_| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Char)))? // Basic error conversion
                            .to_string();
            Ok((rem, Credentials::Basic { token }))
        }
        Ok(Scheme::Other(scheme)) => {
            // Parse comma-separated list of generic auth params
            let (rem, params) = comma_separated_list1(auth_param)(rem)?;
            Ok((rem, Credentials::Other { scheme, params }))
        }
        Err(_) => {
            // If Scheme::from_str fails, it's likely an invalid scheme token
             Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Fail))) // Or some other appropriate error
        }
    }
} 