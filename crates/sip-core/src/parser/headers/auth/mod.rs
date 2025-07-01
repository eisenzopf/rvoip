pub mod common;
pub mod challenge;
pub mod credentials;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{
        Algorithm, AuthParam, Challenge, Credentials, DigestParam, Qop, AuthScheme
    };

    #[test]
    fn test_rfc3261_digest_challenge() {
        // Example from RFC 3261 Section 22.4
        let input = b"Digest realm=\"atlanta.example.com\", domain=\"sip:boxesbybob.example.com\", qop=\"auth\", nonce=\"f84f1cec41e6cbe5aea9c8e88d359\", opaque=\"\", stale=FALSE, algorithm=MD5";
        
        let (rem, chal) = challenge::challenge(input).unwrap();
        assert!(rem.is_empty());
        
        if let Challenge::Digest { params } = chal {
            // Verify that required params are present
            let realm = params.iter().find_map(|p| {
                if let DigestParam::Realm(v) = p { Some(v) } else { None }
            });
            assert_eq!(realm, Some(&"atlanta.example.com".to_string()));
            
            let nonce = params.iter().find_map(|p| {
                if let DigestParam::Nonce(v) = p { Some(v) } else { None }
            });
            assert_eq!(nonce, Some(&"f84f1cec41e6cbe5aea9c8e88d359".to_string()));
            
            // Verify algorithm
            let algo = params.iter().find_map(|p| {
                if let DigestParam::Algorithm(v) = p { Some(v) } else { None }
            });
            assert_eq!(algo, Some(&Algorithm::Md5));
            
            // Verify stale
            let stale = params.iter().find_map(|p| {
                if let DigestParam::Stale(v) = p { Some(v) } else { None }
            });
            assert_eq!(stale, Some(&false));
        } else {
            panic!("Expected Digest challenge");
        }
    }
    
    #[test]
    fn test_rfc3261_digest_credentials() {
        // Example from RFC 3261 Section 22.4
        let input = b"Digest username=\"bob\", realm=\"atlanta.example.com\", nonce=\"ea9c8e88df84f1cec4341ae6cbe5a359\", opaque=\"\", uri=\"sips:ss2.example.com\", response=\"dfe56131d1958046689d83306477ecc\"";
        
        let (rem, creds) = credentials::credentials(input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Digest { params } = creds {
            // Verify required fields
            let username = params.iter().find_map(|p| {
                if let DigestParam::Username(v) = p { Some(v) } else { None }
            });
            assert_eq!(username, Some(&"bob".to_string()));
            
            let realm = params.iter().find_map(|p| {
                if let DigestParam::Realm(v) = p { Some(v) } else { None }
            });
            assert_eq!(realm, Some(&"atlanta.example.com".to_string()));
            
            let nonce = params.iter().find_map(|p| {
                if let DigestParam::Nonce(v) = p { Some(v) } else { None }
            });
            assert_eq!(nonce, Some(&"ea9c8e88df84f1cec4341ae6cbe5a359".to_string()));
        } else {
            panic!("Expected Digest credentials");
        }
    }
    
    #[test]
    fn test_basic_auth() {
        // Test Basic authentication parsing
        let challenge_input = b"Basic realm=\"sip.example.com\"";
        let (rem, chal) = challenge::challenge(challenge_input).unwrap();
        assert!(rem.is_empty());
        
        if let Challenge::Basic { params } = chal {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "realm");
            assert_eq!(params[0].value, "sip.example.com");
        } else {
            panic!("Expected Basic challenge");
        }
        
        // Test Basic credentials parsing
        let creds_input = b"Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="; // Base64 of "Aladdin:open sesame"
        let (rem, creds) = credentials::credentials(creds_input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Basic { token } = creds {
            assert_eq!(token, "QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        } else {
            panic!("Expected Basic credentials");
        }
    }
    
    #[test]
    fn test_digest_auth_with_qop() {
        // Test Digest with quality of protection
        let input = b"Digest username=\"alice\", realm=\"example.com\", nonce=\"12345\", uri=\"sip:bob@example.com\", response=\"1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d\", qop=auth, cnonce=\"abcdef\", nc=00000001";
        
        let (rem, creds) = credentials::credentials(input).unwrap();
        assert!(rem.is_empty());
        
        if let Credentials::Digest { params } = creds {
            // Check for qop
            let qop = params.iter().find_map(|p| {
                if let DigestParam::MsgQop(v) = p { Some(v) } else { None }
            });
            assert_eq!(qop, Some(&Qop::Auth));
            
            // Check for cnonce
            let cnonce = params.iter().find_map(|p| {
                if let DigestParam::Cnonce(v) = p { Some(v) } else { None }
            });
            assert_eq!(cnonce, Some(&"abcdef".to_string()));
            
            // Check for nonce count
            let nc = params.iter().find_map(|p| {
                if let DigestParam::NonceCount(v) = p { Some(v) } else { None }
            });
            assert_eq!(nc, Some(&1u32));
        } else {
            panic!("Expected Digest credentials");
        }
    }
} 