// RFC 3261 Section 22.3 Proxy-Authenticate

use super::auth::challenge::challenge; // Use the challenge parser
use crate::parser::ParseResult;
use crate::types::auth::Challenge;
use nom::IResult;

// Proxy-Authenticate = "Proxy-Authenticate" HCOLON challenge
// Note: HCOLON is handled by the top-level message_header parser.
// This parser receives the value *after* HCOLON.
pub fn parse_proxy_authenticate(input: &[u8]) -> ParseResult<Challenge> {
    challenge(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{DigestParam, Qop, Algorithm};

    #[test]
    fn test_parse_proxy_authenticate_digest() {
        let input = br#"Digest realm="atlanta.com", nonce="1234abcd", opaque="5ccc069c403ebaf9f0171e9517f40e41""#;
        let result = parse_proxy_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenge) = result.unwrap();
        assert!(rem.is_empty());
        if let Challenge::Digest { params } = challenge {
            assert!(params.contains(&DigestParam::Realm("atlanta.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("1234abcd".to_string())));
            assert!(params.contains(&DigestParam::Opaque("5ccc069c403ebaf9f0171e9517f40e41".to_string())));
        } else {
            panic!("Expected Digest challenge");
        }
    }
} 