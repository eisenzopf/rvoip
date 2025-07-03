// RFC 3261 Section 22 & 25.1
// Parser for the credentials part of Authorization headers

// Use the new digest_param parser from common
use super::common::{auth_scheme, digest_param, auth_param, digest_credential}; 
use crate::parser::common::comma_separated_list1;
use crate::parser::token::token;
use crate::parser::whitespace::lws;
use crate::parser::ParseResult;
// Import the necessary types from types::auth
use crate::types::auth::{AuthParam, Credentials, DigestParam, AuthScheme, Algorithm, Qop};
use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_till1},
    character::complete::{char, digit1},
    combinator::{map, map_res, opt, recognize},
    sequence::{preceded, pair},
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
    // First check if the input is empty
    if input.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::TakeWhile1)));
    }
    
    // Check if the input starts with "Digest " (Digest followed by a space)
    if input.len() >= 7 && (&input[0..7] == b"Digest " || &input[0..7] == b"DIGEST " || &input[0..7] == b"digest ") {
        let (rem, params) = preceded(
            pair(
                tag_no_case("Digest"),
                lws
            ),
            digest_credential
        )(input)?;
        return Ok((rem, Credentials::Digest { params }));
    }
    
    // Check if the input starts with "Basic " (Basic followed by a space)
    if input.len() >= 6 && (&input[0..6] == b"Basic " || &input[0..6] == b"BASIC " || &input[0..6] == b"basic ") {
        let (rem, token_bytes) = preceded(
            pair(
                tag_no_case("Basic"),
                lws
            ),
            basic_credentials_token
        )(input)?;
        
        let token = std::str::from_utf8(token_bytes)
                        .map_err(|_| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Char)))? // Basic error conversion
                        .to_string();
        return Ok((rem, Credentials::Basic { token }));
    }
    
    // Check if the input starts with "Bearer " (Bearer followed by a space)
    if input.len() >= 7 && (&input[0..7] == b"Bearer " || &input[0..7] == b"BEARER " || &input[0..7] == b"bearer ") {
        let (rem, token_bytes) = preceded(
            pair(
                tag_no_case("Bearer"),
                lws
            ),
            basic_credentials_token // We can reuse this for Bearer tokens
        )(input)?;
        
        let token = std::str::from_utf8(token_bytes)
                        .map_err(|_| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Char)))? // Basic error conversion
                        .to_string();
        
        // Create a parameter with token68 value
        let param = AuthParam {
            name: "token68".to_string(),
            value: token,
        };
        
        return Ok((rem, Credentials::Other { 
            scheme: "Bearer".to_string(), 
            params: vec![param] 
        }));
    }
    
    // If neither of the special cases, use the general approach
    let (rem, scheme_str) = auth_scheme(input)?;
    let (rem, _) = lws(rem)?;

    match AuthScheme::from_str(&scheme_str) {
        Ok(AuthScheme::Digest) => {
            // We shouldn't get here because we already handled Digest above
            let (rem, params) = digest_credential(rem)?;
            Ok((rem, Credentials::Digest { params }))
        }
        Ok(AuthScheme::Basic) => {
            // We shouldn't get here because we already handled Basic above
            let (rem, token_bytes) = basic_credentials_token(rem)?;
            let token = std::str::from_utf8(token_bytes)
                            .map_err(|_| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Char)))? // Basic error conversion
                            .to_string();
            Ok((rem, Credentials::Basic { token }))
        }
        Ok(AuthScheme::Other(scheme)) => {
            // Parse comma-separated list of generic auth params
            let (rem, params) = comma_separated_list1(auth_param)(rem)?;
            Ok((rem, Credentials::Other { scheme, params }))
        }
        Err(_) => {
            // If AuthScheme::from_str fails, it's likely an invalid scheme token
             Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Fail))) // Or some other appropriate error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_digest_credentials() {
        // Test a complete Digest Authorization header value
        let input = b"Digest username=\"alice\",\
                     realm=\"example.com\",\
                     nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\",\
                     uri=\"sip:bob@example.com\",\
                     response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\",\
                     algorithm=MD5,\
                     cnonce=\"0a1b2c3d4e5f\",\
                     qop=auth,\
                     nc=00000001";
        
        let (rem, creds) = credentials(input).unwrap();
        assert!(rem.is_empty());
        
        match creds {
            Credentials::Digest { params } => {
                assert_eq!(params.len(), 9);
                
                // Check specific parameters
                assert!(params.iter().any(|p| match p {
                    DigestParam::Username(u) => u == "alice",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Realm(r) => r == "example.com",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Nonce(n) => n == "dcd98b7102dd2f0e8b11d0f600bfb0c093",
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
                    DigestParam::Cnonce(c) => c == "0a1b2c3d4e5f",
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
            },
            _ => panic!("Expected Digest credentials"),
        }
    }

    #[test]
    fn test_digest_credentials_minimal() {
        // Test minimal Digest credentials (fewer than all possible parameters)
        let input = b"Digest username=\"alice\",realm=\"example.com\",nonce=\"1234567890\",uri=\"sip:bob@example.com\",response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\"";
        let (rem, creds) = credentials(input).unwrap();
        assert!(rem.is_empty());
        
        match creds {
            Credentials::Digest { params } => {
                assert_eq!(params.len(), 5);
                
                // Check required parameters
                assert!(params.iter().any(|p| match p {
                    DigestParam::Username(u) => u == "alice",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Realm(r) => r == "example.com",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Nonce(n) => n == "1234567890",
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
            },
            _ => panic!("Expected Digest credentials"),
        }
    }

    #[test]
    fn test_digest_credentials_with_opaque() {
        // Test Digest credentials with opaque parameter (mirrored from challenge)
        let input = b"Digest username=\"alice\",realm=\"example.com\",nonce=\"1234567890\",uri=\"sip:bob@example.com\",response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\",opaque=\"5ccc069c403ebaf9f0171e9517f40e41\"";
        let (rem, creds) = credentials(input).unwrap();
        assert!(rem.is_empty());
        
        match creds {
            Credentials::Digest { params } => {
                assert_eq!(params.len(), 6);
                
                // Check opaque parameter
                assert!(params.iter().any(|p| match p {
                    DigestParam::Opaque(o) => o == "5ccc069c403ebaf9f0171e9517f40e41",
                    _ => false
                }));
            },
            _ => panic!("Expected Digest credentials"),
        }
    }

    #[test]
    fn test_digest_credentials_auth_int() {
        // Test Digest credentials with auth-int qop
        let input = b"Digest username=\"alice\",realm=\"example.com\",nonce=\"1234567890\",uri=\"sip:bob@example.com\",response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\",qop=auth-int,cnonce=\"0a1b2c3d4e5f\",nc=00000001";
        let (rem, creds) = credentials(input).unwrap();
        assert!(rem.is_empty());
        
        match creds {
            Credentials::Digest { params } => {
                // Check qop parameter
                assert!(params.iter().any(|p| match p {
                    DigestParam::MsgQop(q) => *q == Qop::AuthInt,
                    _ => false
                }));
            },
            _ => panic!("Expected Digest credentials"),
        }
    }

    #[test]
    fn test_basic_credentials() {
        // Test Basic authentication credentials
        let input = b"Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="; // Base64 of "Aladdin:open sesame"
        let (rem, creds) = credentials(input).unwrap();
        assert!(rem.is_empty());
        
        match creds {
            Credentials::Basic { token } => {
                assert_eq!(token, "QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
            },
            _ => panic!("Expected Basic credentials"),
        }
    }

    #[test]
    fn test_other_credentials() {
        // Test some other authentication scheme
        let input = b"Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let (rem, creds) = credentials(input).unwrap();
        assert!(rem.is_empty());
        
        match creds {
            Credentials::Other { scheme, params } => {
                assert_eq!(scheme, "Bearer");
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].name, "token68");
                assert_eq!(params[0].value, "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9");
            },
            _ => panic!("Expected Other credentials"),
        }
    }

    #[test]
    fn test_invalid_scheme() {
        // Test with an invalid character in the scheme
        let input = b"Digest@Invalid username=\"alice\"";
        assert!(credentials(input).is_err());
    }

    #[test]
    fn test_missing_parameters() {
        // Test with missing parameters after scheme
        let input = b"Digest ";
        assert!(credentials(input).is_err());

        // Test with empty string
        let input = b"";
        assert!(credentials(input).is_err());
    }

    #[test]
    fn test_incorrect_parameter_format() {
        // Test with incorrectly formatted parameters
        let input = b"Digest username=\"alice\" realm=\"example.com\""; // Missing comma
        // The parser should correctly parse the first parameter and return the remainder
        let result = credentials(input);
        match result {
            Ok((rem, Credentials::Digest { params })) => {
                // It should successfully parse the scheme and first parameter
                assert_eq!(params.len(), 1);
                assert!(params.iter().any(|p| match p {
                    DigestParam::Username(u) => u == "alice",
                    _ => false
                }));
                // It should return the remainder for further parsing
                assert_eq!(rem, b" realm=\"example.com\"");
            },
            _ => panic!("Credentials parser should parse the scheme and first parameter"),
        }
    }

    #[test]
    fn test_trailing_content() {
        // Test with trailing content
        let input = b"Digest username=\"alice\",realm=\"example.com\",nonce=\"1234567890\",uri=\"sip:bob@example.com\",response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\";Content-Type: application/sdp";
        let (rem, _) = credentials(input).unwrap();
        assert_eq!(rem, b";Content-Type: application/sdp");
    }

    #[test]
    fn test_basic_credentials_with_multiple_tokens() {
        // Basic credentials should handle multiple tokens
        let input = b"Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==,param=value";
        let (rem, creds) = credentials(input).unwrap();
        assert_eq!(rem, b",param=value");
        
        match creds {
            Credentials::Basic { token } => {
                assert_eq!(token, "QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
            },
            _ => panic!("Expected Basic credentials"),
        }
    }

    #[test]
    fn test_rfc_example() {
        // Test example from RFC 3261 section 22.4
        let input = b"Digest username=\"bob\",\
                     realm=\"atlanta.example.com\",\
                     nonce=\"ea9c8e88df84f1cec4341ae6cbe5a359\",\
                     opaque=\"\",\
                     uri=\"sips:ss2.example.com\",\
                     response=\"dfe56131d1958046689d83306477ecc\"";
        
        let (rem, creds) = credentials(input).unwrap();
        assert!(rem.is_empty());
        
        match creds {
            Credentials::Digest { params } => {
                assert_eq!(params.len(), 6);
                
                // Print all parameters for debugging
                println!("All parameters in RFC example:");
                for (i, param) in params.iter().enumerate() {
                    println!("Param {}: {:?}", i, param);
                }
                
                // Check specific parameters from the RFC example
                assert!(params.iter().any(|p| match p {
                    DigestParam::Username(u) => u == "bob",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Realm(r) => r == "atlanta.example.com",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Nonce(n) => n == "ea9c8e88df84f1cec4341ae6cbe5a359",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Opaque(o) => o == "",
                    _ => false
                }));
                assert!(params.iter().any(|p| match p {
                    DigestParam::Uri(u) => u.to_string() == "sips:ss2.example.com",
                    _ => false
                }));
                
                // The response value is now parsed and available through the test
                let response_param = params.iter().find_map(|p| match p {
                    DigestParam::Response(r) => {
                        println!("Found response: {}", r);
                        Some(r.as_str())
                    },
                    _ => None
                });
                
                if response_param.is_none() {
                    println!("Response parameter not found in params!");
                }
                
                assert!(response_param.is_some());
                assert_eq!(response_param.unwrap(), "dfe56131d1958046689d83306477ecc");
            },
            _ => panic!("Expected Digest credentials"),
        }
    }
} 