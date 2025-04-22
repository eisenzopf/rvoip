// RFC 3261 Section 22 & 25.1
// Parser for the challenge part of Authenticate headers

// Use the new digest_param parser from common
use super::common::{auth_scheme, digest_param, auth_param}; 
use crate::parser::common::comma_separated_list1;
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;
// Import the necessary types from types::auth
use crate::types::auth::{AuthParam, Challenge, DigestParam, Scheme};
use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{map, map_res},
    sequence::{pair, preceded},
    IResult,
};
use std::str::FromStr;

// challenge = ("Digest" LWS digest-challenge-params)
//             / ("Basic" LWS basic-challenge-params) // Typically just realm
//             / other-challenge
// digest-challenge-params = digest-param *(COMMA digest-param)
// basic-challenge-params = auth-param *(COMMA auth-param) ; Usually just realm
// other-challenge = auth-scheme LWS auth-param *(COMMA auth-param)
pub(crate) fn challenge(input: &[u8]) -> ParseResult<Challenge> {
    let (rem, scheme_str) = auth_scheme(input)?;
    let (rem, _) = lws(rem)?;

    match Scheme::from_str(&scheme_str) {
        Ok(Scheme::Digest) => {
            // Parse comma-separated list of digest params
            let (rem, params) = comma_separated_list1(digest_param)(rem)?;
            Ok((rem, Challenge::Digest { params }))
        }
        Ok(Scheme::Basic) => {
             // Basic challenge usually just has realm, maybe others?
             // Parse as generic auth params for now.
            let (rem, params) = comma_separated_list1(auth_param)(rem)?;
            Ok((rem, Challenge::Basic { params }))
        }
        Ok(Scheme::Other(scheme)) => {
            // Parse comma-separated list of generic auth params
            let (rem, params) = comma_separated_list1(auth_param)(rem)?;
            Ok((rem, Challenge::Other { scheme, params }))
        }
        Err(_) => {
            // If Scheme::from_str fails, it's likely an invalid scheme token
             Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Fail))) // Or some other appropriate error
        }
    }
}

// Remove old internal parsers, they are handled by common.rs or the main challenge parser now 