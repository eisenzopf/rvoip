// RFC 3261 Section 22.4 WWW-Authenticate

use super::auth::challenge::challenge; // Use the challenge parser
use crate::parser::ParseResult;
use crate::types::auth::Challenge;
use nom::IResult;

// WWW-Authenticate = "WWW-Authenticate" HCOLON challenge
// Note: HCOLON is handled by the top-level message_header parser.
// This parser receives the value *after* HCOLON.
pub(crate) fn parse_www_authenticate(input: &[u8]) -> ParseResult<Challenge> {
    // The input here is the value part after "WWW-Authenticate: "
    challenge(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{DigestParam, Qop, Algorithm};

    #[test]
    fn test_parse_www_authenticate_digest() {
        let input = br#"Digest realm="atlanta.com", nonce="8452cd5a", qop="auth,auth-int""#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenge) = result.unwrap();
        assert!(rem.is_empty());
        if let Challenge::Digest { params } = challenge {
            assert!(params.contains(&DigestParam::Realm("atlanta.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("8452cd5a".to_string())));
            assert!(params.contains(&DigestParam::Qop(vec![Qop::Auth, Qop::AuthInt])));
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_parse_www_authenticate_other() {
        let input = br#"NewScheme realm="apps.example.com", type=1, title="Login Required""#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenge) = result.unwrap();
        assert!(rem.is_empty());
         if let Challenge::Other { scheme, params } = challenge {
            assert_eq!(scheme, "NewScheme");
            assert_eq!(params.len(), 3);
            // Basic check, assumes AuthParam implements PartialEq
            // assert!(params.contains(&AuthParam { name: "realm".to_string(), value: "apps.example.com".to_string() }));
        } else {
            panic!("Expected Other challenge");
        }
    }
} 