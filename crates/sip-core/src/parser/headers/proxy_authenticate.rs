// RFC 3261 Section 22.3 Proxy-Authenticate
//
// The Proxy-Authenticate header field is used by a proxy server to challenge 
// the authorization of a client. It has the same grammar as WWW-Authenticate
// and follows the same challenge response model.
//
// ABNF:
// Proxy-Authenticate = "Proxy-Authenticate" HCOLON challenge
//                      *(COMMA challenge)
// challenge          = ("Digest" LWS digest-cln *(COMMA digest-cln)) / 
//                      ("Basic" LWS realm) / other-challenge
// other-challenge    = auth-scheme LWS auth-param *(COMMA auth-param)
//
// Example:
// Proxy-Authenticate: Digest realm="atlanta.example.com",
//                     domain="sip:ss1.example.com",
//                     qop="auth",
//                     nonce="f84f1cec41e6cbe5aea9c8e88d359",
//                     opaque="",
//                     stale=FALSE,
//                     algorithm=MD5

use nom::{
    IResult,
    bytes::complete::{tag, tag_no_case, take_while, take_while1},
    character::complete::{char, multispace0, multispace1},
    combinator::{opt, map, verify},
    sequence::{preceded, terminated, tuple, delimited},
    error::{Error, ErrorKind, ParseError},
    branch::alt,
    multi::separated_list0,
};

use crate::types::auth::{Challenge, DigestParam, Qop, Algorithm, AuthParam};
use crate::parser::utils::unfold_lws;

type ParseResult<'a, T> = IResult<&'a [u8], T, Error<&'a [u8]>>;

/// Parse linear whitespace (LWS) according to RFC 3261
/// LWS = [*WSP CRLF] 1*WSP
/// This function handles line folding as specified in RFC 3261 Section 7.3.1
fn parse_lws(input: &[u8]) -> ParseResult<()> {
    // Handle whitespace followed by optional CRLF followed by whitespace
    let (input, _) = multispace0::<&[u8], Error<&[u8]>>(input)?;
    
    // Handle line folding (CRLF + WSP)
    let (input, _) = opt(tuple((
        tag::<_, &[u8], Error<&[u8]>>("\r\n"),
        multispace1::<&[u8], Error<&[u8]>>
    )))(input)?;
    
    Ok((input, ()))
}

/// Parse a token according to RFC 3261 Section 25.1
/// token = 1*(alphanum / "-" / "." / "!" / "%" / "*" / "_" / "+" / "`" / "'" / "~" )
fn parse_token(input: &[u8]) -> ParseResult<&[u8]> {
    let (input, _) = parse_lws(input)?;
    take_while1(|c: u8| {
        c.is_ascii_alphanumeric() || 
        matches!(c, b'-' | b'.' | b'!' | b'%' | b'*' | b'_' | b'+' | b'`' | b'\'' | b'~')
    })(input)
}

/// Parse a quoted string according to RFC 3261 Section 25.1
/// quoted-string = SWS DQUOTE *(qdtext / quoted-pair) DQUOTE
fn parse_quoted_string(input: &[u8]) -> ParseResult<&[u8]> {
    let (input, _) = parse_lws(input)?;
    delimited(
        char::<&[u8], Error<&[u8]>>('"'),
        take_while(|c| c != b'"' && c != b'\\'),
        char::<&[u8], Error<&[u8]>>('"')
    )(input)
}

/// Parse an auth-param according to RFC 2617 Section 1.2
/// auth-param = token "=" (token / quoted-string)
fn parse_auth_param(input: &[u8]) -> ParseResult<AuthParam> {
    let (input, _) = parse_lws(input)?;
    let (input, name) = parse_token(input)?;
    let (input, _) = parse_lws(input)?;
    let (input, _) = char::<&[u8], Error<&[u8]>>('=')(input)?;
    let (input, _) = parse_lws(input)?;
    
    let name_str = std::str::from_utf8(name).unwrap_or("");
    
    // Some parameters can have unquoted values according to RFC 2617
    // algorithm, qop, and stale can be tokens rather than quoted-strings
    let (input, value) = match name_str.to_ascii_lowercase().as_str() {
        "algorithm" | "stale" | "qop" => alt((
            parse_quoted_string,
            parse_token
        ))(input)?,
        _ => parse_quoted_string(input)?,
    };
    
    Ok((input, AuthParam {
        name: name_str.to_string(),
        value: std::str::from_utf8(value).unwrap_or("").to_string(),
    }))
}

/// Parse a comma-separated list of auth-params according to RFC 2617
/// auth-param *(COMMA auth-param)
fn parse_auth_params(input: &[u8]) -> ParseResult<Vec<AuthParam>> {
    let (input, first) = parse_auth_param(input)?;
    let mut params = vec![first];
    let mut current_input = input;
    
    loop {
        // Handle commas with optional line folding
        let comma_parser = delimited(
            parse_lws,
            char::<&[u8], Error<&[u8]>>(','),
            parse_lws
        );
        
        match preceded(comma_parser, parse_auth_param)(current_input) {
            Ok((remaining, param)) => {
                params.push(param);
                current_input = remaining;
            },
            Err(_) => break,
        }
    }
    
    Ok((current_input, params))
}

/// Parse an authentication scheme according to RFC 3261
/// Only Digest and Basic schemes are supported according to RFC 3261
fn parse_scheme(input: &[u8]) -> ParseResult<&[u8]> {
    let (input, _) = parse_lws(input)?;
    let (input, scheme) = parse_token(input)?;
    let scheme_str = std::str::from_utf8(scheme).unwrap_or("");
    
    // Only allow Digest and Basic schemes as per RFC 3261
    if !scheme_str.eq_ignore_ascii_case("digest") && !scheme_str.eq_ignore_ascii_case("basic") {
        return Err(nom::Err::Error(Error::new(input, ErrorKind::Tag)));
    }
    
    Ok((input, scheme))
}

/// Parse a challenge according to RFC 3261 Section 22.3
/// challenge = ("Digest" LWS digest-cln *(COMMA digest-cln)) / 
///             ("Basic" LWS realm) / other-challenge
fn parse_challenge(input: &[u8]) -> ParseResult<Challenge> {
    let (input, scheme) = parse_scheme(input)?;
    let (input, _) = parse_lws(input)?;
    let (input, params) = parse_auth_params(input)?;

    let scheme_str = std::str::from_utf8(scheme).unwrap_or("");
    let challenge = match scheme_str.to_ascii_lowercase().as_str() {
        "digest" => {
            // Validate required parameters according to RFC 2617 Section 3.2.1
            let has_realm = params.iter().any(|p| p.name.eq_ignore_ascii_case("realm"));
            let has_nonce = params.iter().any(|p| p.name.eq_ignore_ascii_case("nonce"));
            if !has_realm || !has_nonce {
                return Err(nom::Err::Error(Error::new(input, ErrorKind::Verify)));
            }

            let mut digest_params = Vec::new();
            for param in params {
                match param.name.to_ascii_lowercase().as_str() {
                    "realm" => digest_params.push(DigestParam::Realm(param.value)),
                    "nonce" => digest_params.push(DigestParam::Nonce(param.value)),
                    "opaque" => digest_params.push(DigestParam::Opaque(param.value)),
                    "algorithm" => {
                        let algorithm = match param.value.to_ascii_uppercase().as_str() {
                            "MD5" => Algorithm::Md5,
                            "MD5-SESS" => Algorithm::Md5Sess,
                            "SHA-256" => Algorithm::Sha256,
                            "SHA-256-SESS" => Algorithm::Sha256Sess,
                            "SHA-512-256" => Algorithm::Sha512,
                            "SHA-512-256-SESS" => Algorithm::Sha512Sess,
                            _ => Algorithm::Other(param.value),
                        };
                        digest_params.push(DigestParam::Algorithm(algorithm));
                    },
                    "qop" => {
                        let qops = param.value.split(',')
                            .map(|s| match s.trim().to_ascii_lowercase().as_str() {
                                "auth" => Qop::Auth,
                                "auth-int" => Qop::AuthInt,
                                _ => Qop::Other(s.trim().to_string()),
                            })
                            .collect();
                        digest_params.push(DigestParam::Qop(qops));
                    },
                    "stale" => {
                        let stale = param.value.eq_ignore_ascii_case("true");
                        digest_params.push(DigestParam::Stale(stale));
                    },
                    "domain" => {
                        let domains = param.value.split(' ')
                            .map(|s| s.trim().to_string())
                            .collect();
                        digest_params.push(DigestParam::Domain(domains));
                    },
                    _ => (),
                }
            }
            Challenge::Digest { params: digest_params }
        },
        "basic" => {
            // Basic auth requires realm parameter according to RFC 2617 Section 2
            if !params.iter().any(|p| p.name.eq_ignore_ascii_case("realm")) {
                return Err(nom::Err::Error(Error::new(input, ErrorKind::Verify)));
            }
            Challenge::Basic { params }
        },
        _ => unreachable!(), // parse_scheme already validated the scheme
    };

    Ok((input, challenge))
}

/// Parse the value of a Proxy-Authenticate header according to RFC 3261
/// Proxy-Authenticate = "Proxy-Authenticate" HCOLON challenge *(COMMA challenge)
pub fn parse_proxy_authenticate<'a>(input: &'a [u8]) -> ParseResult<'a, Vec<Challenge>> {
    if input.is_empty() {
        return Err(nom::Err::Error(Error::new(input, ErrorKind::TakeWhile1)));
    }

    // First try to parse as a single challenge
    match parse_challenge(input) {
        Ok((remaining, challenge)) => {
            let mut challenges = vec![challenge];
            
            // Try to parse more challenges if there are any
            // Multiple challenges are separated by commas
            let mut current_input = remaining;
            
            while !current_input.is_empty() {
                match preceded(
                    delimited(
                        parse_lws,
                        char::<&[u8], Error<&[u8]>>(','),
                        parse_lws
                    ),
                    parse_challenge
                )(current_input) {
                    Ok((new_remaining, challenge)) => {
                        challenges.push(challenge);
                        current_input = new_remaining;
                    },
                    Err(_) => break,
                }
    }

            Ok((current_input, challenges))
        },
        Err(e) => Err(e),
    }
}

/// Parse a complete Proxy-Authenticate header according to RFC 3261
pub fn proxy_authenticate_header(input: &[u8]) -> ParseResult<Vec<Challenge>> {
    preceded(
        terminated(
            tag_no_case(b"Proxy-Authenticate"),
            crate::parser::separators::hcolon
        ),
        parse_proxy_authenticate
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_proxy_authenticate_basic() {
        let input = "Basic realm=\"proxy.example.com\"";
        let (_, challenges) = parse_proxy_authenticate(input.as_bytes()).unwrap();
        assert_eq!(challenges.len(), 1);
        
        if let Challenge::Basic { params } = &challenges[0] {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "realm");
            assert_eq!(params[0].value, "proxy.example.com");
        } else {
            panic!("Expected Basic challenge");
        }
    }

    #[test]
    fn test_parse_proxy_authenticate_digest() {
        let input = r#"Digest realm="proxy.example.com", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", algorithm=MD5, qop="auth,auth-int""#;
        let (_, challenges) = parse_proxy_authenticate(input.as_bytes()).unwrap();
        assert_eq!(challenges.len(), 1);
        
        if let Challenge::Digest { params } = &challenges[0] {
            assert_eq!(params.len(), 4);
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string())));
            assert!(params.contains(&DigestParam::Algorithm(Algorithm::Md5)));
            
            let qop = params.iter().find(|p| matches!(p, DigestParam::Qop(_)));
            assert!(qop.is_some());
            if let DigestParam::Qop(qops) = qop.unwrap() {
                assert_eq!(qops.len(), 2);
                assert!(qops.contains(&Qop::Auth));
                assert!(qops.contains(&Qop::AuthInt));
            }
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_parse_proxy_authenticate_multiple() {
        let input = r#"Digest realm="proxy.example.com", nonce="nonce123", Basic realm="proxy.example.com""#;
        let (_, challenges) = parse_proxy_authenticate(input.as_bytes()).unwrap();
        assert_eq!(challenges.len(), 2);
        
        // Check Digest challenge
        if let Challenge::Digest { params } = &challenges[0] {
            assert_eq!(params.len(), 2);
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("nonce123".to_string())));
        } else {
            panic!("Expected first challenge to be Digest");
        }
        
        // Check Basic challenge
        if let Challenge::Basic { params } = &challenges[1] {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "realm");
            assert_eq!(params[0].value, "proxy.example.com");
        } else {
            panic!("Expected second challenge to be Basic");
        }
    }

    #[test]
    fn test_parse_proxy_authenticate_multiple_separate() {
        let input = r#"Digest realm="proxy.example.com", nonce="nonce123", Basic realm="proxy.example.com""#;
        let (_, challenges) = parse_proxy_authenticate(input.as_bytes()).unwrap();
        assert_eq!(challenges.len(), 2);
        
        // Check Digest challenge
        if let Challenge::Digest { params } = &challenges[0] {
            assert_eq!(params.len(), 2);
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("nonce123".to_string())));
        } else {
            panic!("Expected first challenge to be Digest");
        }
        
        // Check Basic challenge
        if let Challenge::Basic { params } = &challenges[1] {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "realm");
            assert_eq!(params[0].value, "proxy.example.com");
        } else {
            panic!("Expected second challenge to be Basic");
        }
    }

    #[test]
    fn test_parse_proxy_authenticate_line_folding() {
        let input = "Digest realm=\"proxy.example.com\",\r\n nonce=\"nonce123\",\r\n algorithm=SHA-256";
        let (_, challenges) = parse_proxy_authenticate(input.as_bytes()).unwrap();
        assert_eq!(challenges.len(), 1);
        
        if let Challenge::Digest { params } = &challenges[0] {
            assert_eq!(params.len(), 3);
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("nonce123".to_string())));
            assert!(params.contains(&DigestParam::Algorithm(Algorithm::Sha256)));
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_parse_proxy_authenticate_errors() {
        // Test empty input
        assert!(parse_proxy_authenticate(b"").is_err());
        
        // Test invalid scheme
        assert!(parse_proxy_authenticate(b"Invalid realm=\"test\"").is_err());
        
        // Test missing realm
        assert!(parse_proxy_authenticate(b"Digest nonce=\"test\"").is_err());
        
        // Test missing nonce in Digest
        assert!(parse_proxy_authenticate(b"Digest realm=\"test\"").is_err());
        
        // Test invalid parameter format
        assert!(parse_proxy_authenticate(b"Digest realm=test").is_err());
    }

    #[test]
    fn test_parse_proxy_authenticate_edge_cases() {
        // Test whitespace handling
        let input = "  Digest  realm=\"test\"  ,  nonce=\"test\"  ";
        let (_, challenges) = parse_proxy_authenticate(input.as_bytes()).unwrap();
        assert_eq!(challenges.len(), 1);
        
        // Test empty parameter value
        let input = "Digest realm=\"test\", nonce=\"\"";
        let (_, challenges) = parse_proxy_authenticate(input.as_bytes()).unwrap();
        assert_eq!(challenges.len(), 1);
        
        // Test special characters in values
        let input = "Digest realm=\"test@example.com\", nonce=\"test!@#$%^&*()\"";
        let (_, challenges) = parse_proxy_authenticate(input.as_bytes()).unwrap();
        assert_eq!(challenges.len(), 1);
        
        // Test multiple spaces in parameter names
        let input = "Digest realm=\"test\", nonce=\"test\", algorithm = SHA-256";
        let (_, challenges) = parse_proxy_authenticate(input.as_bytes()).unwrap();
        assert_eq!(challenges.len(), 1);
    }
} 