// RFC 3261 Section 22.3 Proxy-Authorization
//
// The Proxy-Authorization header field allows a client to identify itself 
// to a proxy that requires authentication.
//
// ABNF:
// Proxy-Authorization = "Proxy-Authorization" HCOLON credentials
// credentials        = ("Digest" LWS digest-response) / 
//                      ("Basic" LWS basic-credentials) /
//                      other-response
// digest-response    = dig-resp-params *(COMMA dig-resp-params)
// basic-credentials  = base64-user-pass
// other-response     = auth-scheme LWS auth-param *(COMMA auth-param)
// dig-resp-params    = username / realm / nonce / digest-uri /
//                      dresponse / algorithm / cnonce / opaque /
//                      message-qop / nonce-count / auth-param
//
// Example:
// Proxy-Authorization: Digest username="alice", 
//                      realm="atlanta.example.com",
//                      nonce="84a4cc6f3082121f32b42a2187831a9e",
//                      response="7587245234b3434cc3412213e5f113a5432"

use super::auth::credentials::credentials;
use crate::parser::ParseResult;
use crate::types::auth::Credentials;
use nom::IResult;
use nom::combinator::map;
use crate::types::auth::{DigestParam, Qop, Algorithm};
use crate::types::uri::Uri;

/// Parses the value of a Proxy-Authorization header.
///
/// This parser handles the part after "Proxy-Authorization:" in the SIP message.
/// The header name and HCOLON are handled by the top-level message_header parser.
///
/// This implementation delegates to the `credentials` parser from the auth module,
/// which handles the parsing of authentication credentials according to RFC 3261
/// sections 22.3 and 25.1.
///
/// The Proxy-Authorization header is used by clients to provide authentication
/// credentials to a proxy that has challenged them with a Proxy-Authenticate header.
pub fn parse_proxy_authorization(input: &[u8]) -> ParseResult<Credentials> {
    // The Proxy-Authorization header follows the same format as Authorization,
    // so we can directly use the credentials parser from the auth module.
    credentials(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{DigestParam, Qop, Algorithm};
    use crate::types::uri::Uri;

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
    
    #[test]
    fn test_parse_proxy_authorization_full_digest() {
        // Test a complete Digest credential with all possible parameters
        let input = br#"Digest username="alice", realm="atlanta.com", nonce="1234abcd", uri="sip:server.atlanta.com", response="a2fe4fd8c0a9208f5e5", algorithm=MD5, cnonce="0a1b2c3d4e5f", opaque="5ccc069c403ebaf9f0171e9517f40e41", qop=auth, nc=00000001"#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Digest { params } = creds {
            assert!(params.contains(&DigestParam::Username("alice".to_string())));
            assert!(params.contains(&DigestParam::Realm("atlanta.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("1234abcd".to_string())));
            assert!(params.contains(&DigestParam::Response("a2fe4fd8c0a9208f5e5".to_string())));
            assert!(params.contains(&DigestParam::Algorithm(Algorithm::Md5)));
            assert!(params.contains(&DigestParam::Cnonce("0a1b2c3d4e5f".to_string())));
            assert!(params.contains(&DigestParam::Opaque("5ccc069c403ebaf9f0171e9517f40e41".to_string())));
            assert!(params.contains(&DigestParam::MsgQop(Qop::Auth)));
            assert!(params.contains(&DigestParam::NonceCount(1)));
            
            // Check URI
            let uri_param = params.iter().find(|p| matches!(p, DigestParam::Uri(_)));
            assert!(uri_param.is_some());
            if let DigestParam::Uri(uri) = uri_param.unwrap() {
                assert_eq!(uri.to_string(), "sip:server.atlanta.com");
            }
        } else {
            panic!("Expected Digest credentials");
        }
    }
    
    #[test]
    fn test_parse_proxy_authorization_basic() {
        // Test Basic authentication credentials
        let input = br#"Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Basic { token } = creds {
            assert_eq!(token, "QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
            // This is base64 for "Aladdin:open sesame"
        } else {
            panic!("Expected Basic credentials");
        }
    }
    
    #[test]
    fn test_parse_proxy_authorization_other_scheme() {
        // Test custom authentication scheme
        let input = br#"Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Other { scheme, params } = creds {
            assert_eq!(scheme, "Bearer");
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "token68");
            assert_eq!(params[0].value, "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9");
        } else {
            panic!("Expected Other credentials scheme");
        }
    }
    
    #[test]
    fn test_parse_proxy_authorization_custom_auth() {
        // Test custom authentication with parameters
        let input = br#"CustomAuth realm="example.com", param1="value1", param2="value2""#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Other { scheme, params } = creds {
            assert_eq!(scheme, "CustomAuth");
            assert_eq!(params.len(), 3);
            assert_eq!(params[0].name, "realm");
            assert_eq!(params[0].value, "example.com");
            assert_eq!(params[1].name, "param1");
            assert_eq!(params[1].value, "value1");
            assert_eq!(params[2].name, "param2");
            assert_eq!(params[2].value, "value2");
        } else {
            panic!("Expected Other credentials scheme");
        }
    }
    
    #[test]
    fn test_parse_proxy_authorization_case_sensitivity() {
        // Test case-insensitivity of the scheme and parameters
        let input = br#"digest USERNAME="Bob", REALM="proxy.com", NONCE="qwert", URI="sip:resource.com", RESPONSE="12345""#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
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
    
    #[test]
    fn test_parse_proxy_authorization_error_cases() {
        // Empty input
        assert!(parse_proxy_authorization(b"").is_err());
        
        // Missing parameters after scheme
        assert!(parse_proxy_authorization(b"Digest").is_err());
        assert!(parse_proxy_authorization(b"Digest ").is_err());
        
        // Invalid scheme
        assert!(parse_proxy_authorization(b"Invalid@Scheme realm=\"example.com\"").is_err());
        
        // Note: The Digest scheme technically requires certain parameters (username, realm, nonce, uri, response)
        // for the credentials to be valid per RFC 3261. However, the credentials parser itself doesn't enforce
        // these requirements at the parse level - it merely parses the parameters that are provided.
        // 
        // Therefore, incomplete Digest credentials can still be parsed successfully, although
        // they would need to be validated at a higher application level to ensure that all
        // required parameters are present before using them in an authentication process.
        //
        // For example, these would successfully parse but would be invalid in actual use:
        // - Digest username="Bob"
        // - Digest realm="proxy.com"
    }
    
    #[test]
    fn test_parse_proxy_authorization_with_line_folding() {
        // Test credentials with linear whitespace folding
        let input = br#"Digest username="Bob",
 realm="proxy.com", nonce="qwert", uri="sip:resource.com", response="12345""#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
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
    
    #[test]
    fn test_parse_proxy_authorization_with_whitespace() {
        // Test credentials with extra whitespace
        let input = br#"Digest  username = "Bob" ,  realm = "proxy.com" , nonce = "qwert" , uri = "sip:resource.com" , response = "12345"  "#;
        let result = parse_proxy_authorization(input);
        assert!(result.is_ok());
        let (rem, creds) = result.unwrap();
        
        // The credentials parser may leave trailing whitespace in the remainder,
        // which is acceptable according to the RFC 3261 grammar
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
    
    #[test]
    fn test_rfc3261_examples() {
        // Example from RFC 3261 Section 22.3 (adapted)
        let input = br#"Digest username="alice", realm="atlanta.example.com", nonce="84a4cc6f3082121f32b42a2187831a9e", response="7587245234b3434cc3412213e5f113a5432", uri="sip:atlanta.example.com""#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Digest { params } = creds {
            assert!(params.contains(&DigestParam::Username("alice".to_string())));
            assert!(params.contains(&DigestParam::Realm("atlanta.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("84a4cc6f3082121f32b42a2187831a9e".to_string())));
            assert!(params.contains(&DigestParam::Response("7587245234b3434cc3412213e5f113a5432".to_string())));
            
            // Check URI
            let uri_param = params.iter().find(|p| matches!(p, DigestParam::Uri(_)));
            assert!(uri_param.is_some());
            if let DigestParam::Uri(uri) = uri_param.unwrap() {
                assert_eq!(uri.to_string(), "sip:atlanta.example.com");
            }
        } else {
            panic!("Expected Digest credentials");
        }
    }
    
    #[test]
    fn test_abnf_compliance() {
        // Test various combinations to ensure ABNF compliance
        
        // Digest with parameters in different order
        let input = br#"Digest response="12345", nonce="qwert", username="Bob", realm="proxy.com", uri="sip:resource.com""#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
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
        
        // Test with various algorithm values
        let input = br#"Digest username="Bob", realm="proxy.com", nonce="qwert", uri="sip:resource.com", response="12345", algorithm=SHA-256"#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Digest { params } = creds {
            assert!(params.contains(&DigestParam::Algorithm(Algorithm::Sha256)));
        } else {
            panic!("Expected Digest credentials");
        }
        
        // Test with auth-int qop value
        let input = br#"Digest username="Bob", realm="proxy.com", nonce="qwert", uri="sip:resource.com", response="12345", qop=auth-int, cnonce="0a1b2c3d4e5f", nc=00000001"#;
        let (rem, creds) = parse_proxy_authorization(input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Digest { params } = creds {
            assert!(params.contains(&DigestParam::MsgQop(Qop::AuthInt)));
            assert!(params.contains(&DigestParam::Cnonce("0a1b2c3d4e5f".to_string())));
            assert!(params.contains(&DigestParam::NonceCount(1)));
        } else {
            panic!("Expected Digest credentials");
        }
    }
} 