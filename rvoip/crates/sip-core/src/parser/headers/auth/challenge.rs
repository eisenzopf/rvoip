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
pub fn challenge(input: &[u8]) -> ParseResult<Challenge> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{DigestParam, Algorithm, Qop, Scheme};

    #[test]
    fn test_digest_challenge() {
        // Test a typical WWW-Authenticate Digest challenge
        let input = b"Digest realm=\"example.com\",nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\",opaque=\"5ccc069c403ebaf9f0171e9517f40e41\",algorithm=MD5,qop=\"auth,auth-int\"";
        let (rem, chal) = challenge(input).unwrap();
        assert!(rem.is_empty());
        
        match chal {
            Challenge::Digest { params } => {
                assert_eq!(params.len(), 5);
                
                // Check specific parameters
                assert!(params.iter().any(|p| match p {
                    DigestParam::Realm(r) => r == "example.com",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Nonce(n) => n == "dcd98b7102dd2f0e8b11d0f600bfb0c093",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Opaque(o) => o == "5ccc069c403ebaf9f0171e9517f40e41",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Algorithm(a) => *a == Algorithm::Md5,
                    _ => false
                }));
                
                // Check qop options
                let qop_param = params.iter().find(|p| match p {
                    DigestParam::Qop(_) => true,
                    _ => false
                });
                
                if let Some(DigestParam::Qop(qops)) = qop_param {
                    assert_eq!(qops.len(), 2);
                    assert_eq!(qops[0], Qop::Auth);
                    assert_eq!(qops[1], Qop::AuthInt);
                } else {
                    panic!("qop parameter not found");
                }
            },
            _ => panic!("Expected Digest challenge"),
        }
    }

    #[test]
    fn test_digest_challenge_minimal() {
        // Test minimal Digest challenge (just realm and nonce are required)
        let input = b"Digest realm=\"example.com\",nonce=\"1234567890\"";
        let (rem, chal) = challenge(input).unwrap();
        assert!(rem.is_empty());
        
        match chal {
            Challenge::Digest { params } => {
                assert_eq!(params.len(), 2);
                
                // Check required parameters
                assert!(params.iter().any(|p| match p {
                    DigestParam::Realm(r) => r == "example.com",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Nonce(n) => n == "1234567890",
                    _ => false
                }));
            },
            _ => panic!("Expected Digest challenge"),
        }
    }

    #[test]
    fn test_digest_challenge_with_domain() {
        // Test Digest challenge with domain parameter
        let input = b"Digest realm=\"example.com\",domain=\"sip:ss1.example.com\",nonce=\"1234567890\"";
        let (rem, chal) = challenge(input).unwrap();
        assert!(rem.is_empty());
        
        match chal {
            Challenge::Digest { params } => {
                assert_eq!(params.len(), 3);
                
                // Check domain parameter - it's a Vector<String> in the actual implementation
                assert!(params.iter().any(|p| match p {
                    DigestParam::Domain(d) => d.len() == 1 && d[0] == "sip:ss1.example.com",
                    _ => false
                }));
            },
            _ => panic!("Expected Digest challenge"),
        }
    }

    #[test]
    fn test_digest_challenge_stale() {
        // Test Digest challenge with stale parameter (used for nonce expiration)
        let input = b"Digest realm=\"example.com\",nonce=\"1234567890\",stale=true";
        let (rem, chal) = challenge(input).unwrap();
        assert!(rem.is_empty());
        
        match chal {
            Challenge::Digest { params } => {
                assert_eq!(params.len(), 3);
                
                // Check stale parameter
                assert!(params.iter().any(|p| match p {
                    DigestParam::Stale(s) => *s == true,
                    _ => false
                }));
            },
            _ => panic!("Expected Digest challenge"),
        }
    }

    #[test]
    fn test_basic_challenge() {
        // Test Basic authentication challenge
        let input = b"Basic realm=\"WallyWorld\"";
        let (rem, chal) = challenge(input).unwrap();
        assert!(rem.is_empty());
        
        match chal {
            Challenge::Basic { params } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].name, "realm");
                assert_eq!(params[0].value, "WallyWorld");
            },
            _ => panic!("Expected Basic challenge"),
        }
    }

    #[test]
    fn test_other_challenge() {
        // Test some other authentication scheme
        let input = b"OAuth realm=\"example.com\",oauth_version=\"1.0\"";
        let (rem, chal) = challenge(input).unwrap();
        assert!(rem.is_empty());
        
        match chal {
            Challenge::Other { scheme, params } => {
                assert_eq!(scheme, "OAuth");
                assert_eq!(params.len(), 2);
                assert_eq!(params[0].name, "realm");
                assert_eq!(params[0].value, "example.com");
                assert_eq!(params[1].name, "oauth_version");
                assert_eq!(params[1].value, "1.0");
            },
            _ => panic!("Expected Other challenge"),
        }
    }

    #[test]
    fn test_invalid_scheme() {
        // Test with an invalid character in the scheme
        let input = b"Digest@Invalid realm=\"example.com\"";
        assert!(challenge(input).is_err());
    }

    #[test]
    fn test_missing_parameters() {
        // Test with missing parameters after scheme
        let input = b"Digest ";
        assert!(challenge(input).is_err());

        // Test with empty string
        let input = b"";
        assert!(challenge(input).is_err());
    }

    #[test]
    fn test_incorrect_parameter_format() {
        // Test with incorrectly formatted parameters
        let input = b"Digest realm=\"example.com\" nonce=\"1234567890\""; // Missing comma
        // The parser should correctly parse the first parameter and return the remainder
        let result = challenge(input);
        match result {
            Ok((rem, Challenge::Digest { params })) => {
                // It should successfully parse the scheme and first parameter
                assert_eq!(params.len(), 1);
                assert!(params.iter().any(|p| match p {
                    DigestParam::Realm(r) => r == "example.com",
                    _ => false
                }));
                // It should return the remainder for further parsing
                assert_eq!(rem, b" nonce=\"1234567890\"");
            },
            _ => panic!("Challenge parser should parse the scheme and first parameter"),
        }
    }

    #[test]
    fn test_trailing_content() {
        // Test with trailing content
        let input = b"Digest realm=\"example.com\",nonce=\"1234567890\";Content-Type: application/sdp";
        let (rem, _) = challenge(input).unwrap();
        assert_eq!(rem, b";Content-Type: application/sdp");
    }

    #[test]
    fn test_rfc_example() {
        // Test example from RFC 3261 section 22.4
        let input = b"Digest realm=\"atlanta.example.com\",\
                     domain=\"sip:boxesbybob.example.com\",\
                     qop=\"auth\",\
                     nonce=\"f84f1cec41e6cbe5aea9c8e88d359\",\
                     opaque=\"\",\
                     stale=FALSE,\
                     algorithm=MD5";
        
        let (rem, chal) = challenge(input).unwrap();
        assert!(rem.is_empty());
        
        match chal {
            Challenge::Digest { params } => {
                assert_eq!(params.len(), 7);
                
                // Check specific parameters from the RFC example
                assert!(params.iter().any(|p| match p {
                    DigestParam::Realm(r) => r == "atlanta.example.com",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Domain(d) => d.len() == 1 && d[0] == "sip:boxesbybob.example.com",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Nonce(n) => n == "f84f1cec41e6cbe5aea9c8e88d359",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Opaque(o) => o == "",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Stale(s) => *s == false,
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Algorithm(a) => *a == Algorithm::Md5,
                    _ => false
                }));
                
                // Check qop option
                let qop_param = params.iter().find(|p| match p {
                    DigestParam::Qop(_) => true,
                    _ => false
                });
                
                if let Some(DigestParam::Qop(qops)) = qop_param {
                    assert_eq!(qops.len(), 1);
                    assert_eq!(qops[0], Qop::Auth);
                } else {
                    panic!("qop parameter not found");
                }
            },
            _ => panic!("Expected Digest challenge"),
        }
    }
} 