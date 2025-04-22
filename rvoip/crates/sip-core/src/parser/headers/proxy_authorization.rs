// RFC 3261 Section 22.3 Proxy-Authorization

use super::auth::credentials::credentials;
use crate::parser::ParseResult;
use crate::types::auth::Credentials;
use nom::IResult;
use nom::combinator::map;

// Proxy-Authorization = "Proxy-Authorization" HCOLON credentials
// Note: HCOLON is handled by the top-level message_header parser.
// This parser receives the value *after* HCOLON.
pub fn parse_proxy_authorization(input: &[u8]) -> ParseResult<Credentials> {
    credentials(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{DigestParam, Qop, Algorithm, Uri};

    #[test]
    fn test_parse_proxy_authorization_digest() {
        let input = br#"Digest username="Bob", realm="proxy.com", nonce="qwert", uri="sip:resource.com", response="12345""#;
        let result = parse_proxy_authorization(input);
        assert!(result.is_ok());
        let (rem, creds) = result.unwrap();
        assert!(rem.is_empty());
        if let Credentials::Digest { params } = creds {
            assert!(params.contains(&DigestParam::Username("Bob".to_string())));
            assert!(params.contains(&DigestParam::Realm("proxy.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("qwert".to_string())));
            assert!(params.contains(&DigestParam::Response("12345".to_string())));
            assert!(params.iter().any(|p| matches!(p, DigestParam::Uri(_))));
        } else {
            panic!("Expected Digest credentials");
        }
    }
} 