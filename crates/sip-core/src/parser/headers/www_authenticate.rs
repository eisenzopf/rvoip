// RFC 3261 Section 20.44 WWW-Authenticate
//
// The WWW-Authenticate header field consists of at least one challenge that
// indicates the authentication scheme(s) and parameters applicable.
//
// WWW-Authenticate  =  "WWW-Authenticate" HCOLON challenge
//                      *(COMMA challenge)
//
// See Section 22.4 for further details on the WWW-Authenticate header field.

use crate::parser::headers::auth::challenge::challenge; // Use the challenge parser from headers::auth
use crate::parser::ParseResult;
use crate::parser::separators::comma;
use crate::parser::whitespace::owsp;
use crate::parser::utils::unfold_lws;
use crate::types::auth::Challenge;
use nom::bytes::complete::{tag, tag_no_case, take_while};
use nom::combinator::{all_consuming, opt, not, recognize, verify};
use nom::sequence::{preceded, terminated, delimited, tuple};
use nom::multi::{many0, many1};
use nom::IResult;
use nom::error::{ParseError, ErrorKind, make_error};
use nom::Err;

/// Parse a WWW-Authenticate header value (everything after the header name and HCOLON)
///
/// The input for this function should be the raw bytes after "WWW-Authenticate: "
/// 
/// # Examples
///
/// ```
/// # use rvoip_sip_core::parser::headers::www_authenticate::parse_www_authenticate;
/// let input = b"Digest realm=\"example.com\", nonce=\"1234567890\"";
/// let (_, challenge) = parse_www_authenticate(input).unwrap();
/// ```
pub fn parse_www_authenticate(input: &[u8]) -> ParseResult<Vec<Challenge>> {
    // Handle an empty input case
    if input.is_empty() {
        return Err(Err::Error(make_error(input, ErrorKind::TakeWhile1)));
    }
    
    // Check if the input contains line folding (CRLF followed by whitespace)
    let has_line_folding = input.windows(3).any(|w| 
        (w[0] == b'\r' && w[1] == b'\n' && (w[2] == b' ' || w[2] == b'\t')) ||
        (w[0] == b'\n' && (w[1] == b' ' || w[1] == b'\t'))
    );
    
    let processed_input: &[u8] = if has_line_folding {
        // If line folding is present, pre-process the input to unfold it
        let unfolded = unfold_lws(input);
        
        // This is a bit of a hack, but we need to store the unfolded bytes somewhere
        // so they survive for the duration of this function
        let bytes_box = Box::new(unfolded);
        let bytes_ref = Box::leak(bytes_box);
        
        bytes_ref.as_slice()
    } else {
        // No line folding, use input directly
        input
    };
    
    // Trim any leading whitespace
    let (mut input, _) = opt(take_while(|c| c == b' ' || c == b'\t' || c == b'\r' || c == b'\n'))(processed_input)?;
    
    // Parse the first challenge
    match challenge(input) {
        Ok((mut rest, first_challenge)) => {
            // Store the challenges in a vector
            let mut challenges = vec![first_challenge];
            
            // While there's more input and it starts with a comma, parse additional challenges
            while !rest.is_empty() {
                // Skip any whitespace
                let (r, _) = opt(take_while(|c| c == b' ' || c == b'\t' || c == b'\r' || c == b'\n'))(rest)?;
                
                // If there's no more content or it doesn't start with a comma, we're done
                if r.is_empty() || r[0] != b',' {
                    rest = r;
                    break;
                }
                
                // Skip the comma and any whitespace
                let (r, _) = preceded(comma, opt(take_while(|c| c == b' ' || c == b'\t' || c == b'\r' || c == b'\n')))(r)?;
                
                // If there's no more content after the comma and whitespace, that's an error
                if r.is_empty() {
                    return Err(Err::Error(make_error(r, ErrorKind::Tag)));
                }
                
                // Parse the next challenge
                match challenge(r) {
                    Ok((r, next_challenge)) => {
                        challenges.push(next_challenge);
                        rest = r;
                    },
                    Err(_) => {
                        // If we can't parse the next challenge, just return what we've got so far
                        break;
                    }
                }
            }
            
            // Trim any trailing whitespace
            if !rest.is_empty() {
                let is_only_whitespace = rest.iter().all(|&c| c == b' ' || c == b'\t' || c == b'\r' || c == b'\n');
                if is_only_whitespace {
                    rest = &[];
                }
            }
            
            Ok((rest, challenges))
        },
        Err(e) => Err(e)
    }
}

/// Parse a complete WWW-Authenticate header, including the header name
/// 
/// Format: "WWW-Authenticate" HCOLON challenge *(COMMA challenge)
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::parser::headers::www_authenticate::www_authenticate_header;
/// let input = b"WWW-Authenticate: Digest realm=\"example.com\", nonce=\"1234567890\"";
/// let (_, challenges) = www_authenticate_header(input).unwrap();
/// ```
pub fn www_authenticate_header(input: &[u8]) -> ParseResult<Vec<Challenge>> {
    preceded(
        terminated(
            tag_no_case(b"WWW-Authenticate"),
            crate::parser::separators::hcolon
        ),
        parse_www_authenticate
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{Algorithm, Qop, DigestParam, AuthParam};
    use nom::combinator::all_consuming;

    #[test]
    fn test_parse_www_authenticate_digest() {
        let input = br#"Digest realm="atlanta.com", nonce="8452cd5a", qop="auth,auth-int""#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Digest { params } = &challenges[0] {
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
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Other { scheme, params } = &challenges[0] {
            assert_eq!(scheme, "NewScheme");
            assert_eq!(params.len(), 3);
            // Check for specific AuthParams
            assert!(params.contains(&AuthParam { name: "realm".to_string(), value: "apps.example.com".to_string() }));
            assert!(params.contains(&AuthParam { name: "type".to_string(), value: "1".to_string() })); // Values are strings
            assert!(params.contains(&AuthParam { name: "title".to_string(), value: "Login Required".to_string() }));
        } else {
            panic!("Expected Other challenge");
        }
    }
    
    #[test]
    fn test_parse_www_authenticate_multiple_challenges() {
        let input = br#"Digest realm="atlanta.com", nonce="8452cd5a", Basic realm="example.com""#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 2);
        
        // First challenge - Digest
        if let Challenge::Digest { params } = &challenges[0] {
            assert!(params.contains(&DigestParam::Realm("atlanta.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("8452cd5a".to_string())));
        } else {
            panic!("Expected Digest challenge");
        }
        
        // Second challenge - Basic
        if let Challenge::Basic { params } = &challenges[1] {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "realm");
            assert_eq!(params[0].value, "example.com");
        } else {
            panic!("Expected Basic challenge");
        }
    }
    
    #[test]
    fn test_parse_www_authenticate_with_algorithm() {
        let input = br#"Digest realm="atlanta.com", nonce="8452cd5a", algorithm=MD5"#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Digest { params } = &challenges[0] {
            assert!(params.contains(&DigestParam::Algorithm(Algorithm::Md5)));
        } else {
            panic!("Expected Digest challenge");
        }
    }
    
    #[test]
    fn test_parse_www_authenticate_with_quoted_algorithm() {
        // Some implementations incorrectly quote the algorithm parameter
        let input = br#"Digest realm="atlanta.com", nonce="8452cd5a", algorithm="MD5""#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Digest { params } = &challenges[0] {
            assert!(params.contains(&DigestParam::Algorithm(Algorithm::Md5)));
        } else {
            panic!("Expected Digest challenge");
        }
    }
    
    #[test]
    fn test_parse_www_authenticate_with_stale() {
        let input = br#"Digest realm="atlanta.com", nonce="8452cd5a", stale=true"#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Digest { params } = &challenges[0] {
            assert!(params.contains(&DigestParam::Stale(true)));
        } else {
            panic!("Expected Digest challenge");
        }
    }
    
    #[test]
    fn test_parse_www_authenticate_with_opaque() {
        let input = br#"Digest realm="atlanta.com", nonce="8452cd5a", opaque="5ccc069c403ebaf9f0171e9517f40e41""#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Digest { params } = &challenges[0] {
            assert!(params.contains(&DigestParam::Opaque("5ccc069c403ebaf9f0171e9517f40e41".to_string())));
        } else {
            panic!("Expected Digest challenge");
        }
    }
    
    #[test]
    fn test_parse_www_authenticate_with_domain() {
        let input = br#"Digest realm="atlanta.com", nonce="8452cd5a", domain="sip:ss1.example.com""#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Digest { params } = &challenges[0] {
            let domain_param = params.iter().find(|p| match p {
                DigestParam::Domain(_) => true,
                _ => false
            });
            assert!(domain_param.is_some());
        } else {
            panic!("Expected Digest challenge");
        }
    }
    
    #[test]
    fn test_parse_www_authenticate_with_whitespace() {
        let input = br#"Digest    realm="atlanta.com",   nonce="8452cd5a"  "#;
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Digest { params } = &challenges[0] {
            assert!(params.contains(&DigestParam::Realm("atlanta.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("8452cd5a".to_string())));
        } else {
            panic!("Expected Digest challenge");
        }
    }
    
    #[test]
    fn test_parse_www_authenticate_line_folding() {
        let input = b"Digest realm=\"atlanta.com\",\r\n nonce=\"8452cd5a\"";
        let result = parse_www_authenticate(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Digest { params } = &challenges[0] {
            assert!(params.contains(&DigestParam::Realm("atlanta.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("8452cd5a".to_string())));
        } else {
            panic!("Expected Digest challenge");
        }
    }
    
    #[test]
    fn test_empty_input() {
        let input = b"";
        let result = parse_www_authenticate(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_invalid_scheme() {
        let input = b"Invalid@Scheme realm=\"example.com\"";
        let result = parse_www_authenticate(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_missing_parameters() {
        let input = b"Digest ";
        let result = parse_www_authenticate(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_malformed_parameter() {
        let input = b"Digest realm=atlanta.com"; // Missing quotes
        let result = parse_www_authenticate(input);
        
        // The parser should be more lenient and parse the Digest scheme
        assert!(result.is_ok(), "Parser should be lenient with malformed parameters");
        
        // If successful, it should have parsed at least the scheme
        if let Ok((_, challenges)) = result {
            assert_eq!(challenges.len(), 1);
            match &challenges[0] {
                Challenge::Digest { .. } => {},
                _ => panic!("Expected a Digest challenge")
            }
        }
    }
    
    #[test]
    fn test_trailing_content() {
        // We should reject content after the parsed challenge
        let input = b"Digest realm=\"atlanta.com\" INVALID";
        let result = all_consuming(parse_www_authenticate)(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_www_authenticate_header() {
        let input = b"WWW-Authenticate: Digest realm=\"atlanta.com\", nonce=\"8452cd5a\"";
        let result = www_authenticate_header(input);
        assert!(result.is_ok());
        let (rem, challenges) = result.unwrap();
        assert!(rem.is_empty());
        
        assert_eq!(challenges.len(), 1);
        if let Challenge::Digest { params } = &challenges[0] {
            assert!(params.contains(&DigestParam::Realm("atlanta.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("8452cd5a".to_string())));
        } else {
            panic!("Expected Digest challenge");
        }
    }
    
    #[test]
    fn test_www_authenticate_header_case_insensitive() {
        let input = b"www-authenticate: Digest realm=\"atlanta.com\", nonce=\"8452cd5a\"";
        let result = www_authenticate_header(input);
        assert!(result.is_ok());
        
        let input = b"WWW-AUTHENTICATE: Digest realm=\"atlanta.com\", nonce=\"8452cd5a\"";
        let result = www_authenticate_header(input);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_www_authenticate_header_with_multiple_challenges() {
        let input = b"WWW-Authenticate: Digest realm=\"atlanta.com\", Basic realm=\"example.com\"";
        let result = www_authenticate_header(input);
        assert!(result.is_ok());
        let (_, challenges) = result.unwrap();
        
        assert_eq!(challenges.len(), 2);
    }
    
    #[test]
    fn test_www_authenticate_header_with_invalid_scheme() {
        let input = b"WWW-Authenticate: Invalid@Scheme realm=\"example.com\"";
        let result = www_authenticate_header(input);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_rfc_example() {
        // Example from RFC 3261 Section 22.4
        let input = b"WWW-Authenticate: Digest realm=\"atlanta.com\", domain=\"sip:boxesbybob.com\", qop=\"auth\", nonce=\"f84f1cec41e6cbe5aea9c8e88d359\", opaque=\"\", stale=FALSE, algorithm=MD5";
        
        let result = www_authenticate_header(input);
        assert!(result.is_ok(), "RFC example should be valid");
        
        if let Ok((rem, challenges)) = result {
            // Print remaining input for debugging
            if !rem.is_empty() {
                println!("Remaining input: {:?}", rem);
                println!("As string: {:?}", std::str::from_utf8(rem));
            }
            
            // There should be no remaining input
            assert!(rem.is_empty(), "Should have no remaining input");
            
            assert_eq!(challenges.len(), 1, "Expected one challenge");
            if let Challenge::Digest { params } = &challenges[0] {
                // Check that essential parameters were parsed
                let has_realm = params.iter().any(|p| matches!(p, DigestParam::Realm(_)));
                let has_nonce = params.iter().any(|p| matches!(p, DigestParam::Nonce(_)));
                
                assert!(has_realm, "realm parameter should be present");
                assert!(has_nonce, "nonce parameter should be present");
                
                // Check for domain parameter
                let domain_param = params.iter().find(|p| matches!(p, DigestParam::Domain(_)));
                assert!(domain_param.is_some(), "domain parameter should be present");
            } else {
                panic!("Expected Digest challenge");
            }
        }
    }
} 