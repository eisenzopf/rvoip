// RFC 3261 Section 22.2 Authorization

use super::auth::common::auth_scheme;
use super::auth::credentials::credentials;
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;
use crate::types::auth::{Credentials, Authorization as AuthorizationHeader};
use nom::IResult;
use nom::sequence::{pair, preceded};
use nom::combinator::map;

// Authorization = "Authorization" HCOLON credentials
// Note: HCOLON is handled by the top-level message_header parser.
// This parser receives the value *after* HCOLON.
// Make this function public
pub fn parse_authorization(input: &[u8]) -> ParseResult<AuthorizationHeader> {
    // Input is the value after "Authorization: "
    map(credentials, AuthorizationHeader)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{DigestParam as DigestRespParam, Qop, Algorithm};
    use crate::types::Uri;

    #[test]
    fn test_parse_authorization_digest() {
        let input = br#"Digest username="Alice", realm="atlanta.com", nonce="xyz", uri="sip:ss1.example.com", response="abc""#;
        let result = parse_authorization(input);
        assert!(result.is_ok());
        let (rem, creds) = result.unwrap();
        assert!(rem.is_empty());
        if let Credentials::Digest { params } = creds {
            assert!(params.contains(&DigestRespParam::Username("Alice".to_string())));
            assert!(params.contains(&DigestRespParam::Realm("atlanta.com".to_string())));
            assert!(params.contains(&DigestRespParam::Nonce("xyz".to_string())));
            assert!(params.contains(&DigestRespParam::Response("abc".to_string())));
            assert!(params.iter().any(|p| matches!(p, DigestRespParam::Uri(_))));
        } else {
            panic!("Expected Digest credentials");
        }
    }
} 