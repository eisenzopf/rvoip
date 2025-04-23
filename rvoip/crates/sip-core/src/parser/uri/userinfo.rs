use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{char},
    combinator::{map_res, opt, recognize, value},
    multi::{many0, many1},
    sequence::{pair, preceded, terminated, delimited},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::common_chars::{unreserved, escaped, alphanum, hex_digit};
use crate::parser::ParseResult;
use crate::parser::utils::unescape_uri_component; // Import unescape helper
use crate::error::Error; // For error type
use crate::parser::uri::ipv6; // Import ipv6 parser

// user-unreserved = "&" / "=" / "+" / "$" / "," / ";" / "?" / "/"
fn is_user_unreserved(c: u8) -> bool {
    matches!(c, b'&' | b'=' | b'+' | b'$' | b',' | b';' | b'?' | b'/')
}

// For IPv6 references in userinfo (RFC 5118 Section 4.1)
// This checks if a character is valid in IPv6 references: hex digits, colons, and brackets
fn is_ipv6_char(c: u8) -> bool {
    c.is_ascii_hexdigit() || matches!(c, b':' | b'[' | b']' | b'.')
}

// Specialized version of IPv6 reference parser that returns the raw bytes
// This adapts the existing ipv6_reference parser for use in userinfo
fn ipv6_reference_raw(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(
        delimited(
            tag(b"["),
            take_while1(|c: u8| c.is_ascii_hexdigit() || c == b':' || c == b'.' || c == b'%'),
            tag(b"]"),
        )
    )(input)
}

// user = 1*( unreserved / escaped / user-unreserved )
// Returns raw bytes, unescaping happens in userinfo
// Extended to handle IPv6 references (RFC 5118)
pub fn user(input: &[u8]) -> ParseResult<&[u8]> {
    alt((
        // Handle IPv6 reference as a special case
        ipv6_reference_raw,
        // Standard user parsing
        recognize(many1(alt((
            unreserved,
            escaped,
            take_while1(is_user_unreserved),
        ))))
    ))(input)
}

// password = *( unreserved / escaped / "&" / "=" / "+" / "$" / "," )
fn is_password_char(c: u8) -> bool {
    unreserved(&[c]).is_ok() || // Check if unreserved
    matches!(c, b'&' | b'=' | b'+' | b'$' | b',')
}

fn password_char(input: &[u8]) -> ParseResult<&[u8]> {
    alt((escaped, take_while1(is_password_char)))(input)
}

// password = *( unreserved / escaped / "&" / "=" / "+" / "$" / "," )
// Returns raw bytes, unescaping happens in userinfo
pub fn password(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(password_char))(input)
}

// userinfo = user [ ":" password ] "@"
// Returns (unescaped_user_string, Option<unescaped_password_string>)
// Corrected structure: Parses user and optional password, terminated by '@'
// Extended to support IPv6 references in userinfo (RFC 5118)
pub fn userinfo(input: &[u8]) -> ParseResult<(String, Option<String>)> {
    map_res(
        terminated(
            pair(user, opt(preceded(tag(b":"), password))),
            tag(b"@")
        ),
        |(user_bytes, pass_opt_bytes)| -> Result<(String, Option<String>), Error> {
            let user_str = unescape_uri_component(user_bytes)?;
            let pass_str_opt = pass_opt_bytes
                .map(|p| unescape_uri_component(p))
                .transpose()?;
            Ok((user_str, pass_str_opt))
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_userinfo_unescaped() {
        let (rem, (user, pass)) = userinfo(b"user%40example.com:p%40ssword@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user@example.com");
        assert_eq!(pass, Some("p@ssword".to_string()));
    }

    #[test]
    fn test_userinfo_rfc3261_examples() {
        // Examples from RFC 3261, Section 19.1.1 (SIP-URI Components)
        
        // alice@atlanta.com
        let (rem, (user, pass)) = userinfo(b"alice@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "alice");
        assert_eq!(pass, None);
        
        // jdrosen@example.com
        let (rem, (user, pass)) = userinfo(b"jdrosen@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "jdrosen");
        assert_eq!(pass, None);
        
        // j.doe@example.com
        let (rem, (user, pass)) = userinfo(b"j.doe@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "j.doe");
        assert_eq!(pass, None);
    }

    #[test]
    fn test_userinfo_with_password() {
        // RFC 3261 defines the password format but recommends against use (Section 19.1.1)
        let (rem, (user, pass)) = userinfo(b"user:pass@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user");
        assert_eq!(pass, Some("pass".to_string()));
        
        // Empty password is valid
        let (rem, (user, pass)) = userinfo(b"user:@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user");
        assert_eq!(pass, Some("".to_string()));
    }

    #[test]
    fn test_userinfo_escaped_sequences() {
        // Test escape sequences in user
        let (rem, (user, pass)) = userinfo(b"user%20name@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user name");
        assert_eq!(pass, None);
        
        // Test escape sequences in password
        let (rem, (user, pass)) = userinfo(b"user:pass%20word@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user");
        assert_eq!(pass, Some("pass word".to_string()));
        
        // Multiple escaped characters
        let (rem, (user, pass)) = userinfo(b"user%3A%40%25:pass%26%3D%2B@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user:@%");
        assert_eq!(pass, Some("pass&=+".to_string()));
    }

    #[test]
    fn test_userinfo_special_chars() {
        // According to RFC 3261, user part can contain:
        // unreserved / escaped / user-unreserved
        // where user-unreserved = "&" / "=" / "+" / "$" / "," / ";" / "?" / "/"
        
        // Special chars in user
        let (rem, (user, pass)) = userinfo(b"user&=+$,;?/@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user&=+$,;?/");
        assert_eq!(pass, None);
        
        // Password can contain:
        // unreserved / escaped / "&" / "=" / "+" / "$" / ","
        let (rem, (user, pass)) = userinfo(b"user:pass&=+$,@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user");
        assert_eq!(pass, Some("pass&=+$,".to_string()));
    }

    #[test]
    fn test_userinfo_from_rfc4475() {
        // Examples from RFC 4475 (SIP Torture Test Messages)
        
        // The "wazup" message - basic user without password
        let (rem, (user, pass)) = userinfo(b"caller@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "caller");
        assert_eq!(pass, None);
        
        // The "escruri" message - URI with many escapes
        let (rem, (user, pass)) = userinfo(b"sip%3Auser%40example.com@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "sip:user@example.com");
        assert_eq!(pass, None);
        
        // The "unreason" message - unusual characters
        let (rem, (user, pass)) = userinfo(b"sip:user@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "sip");
        assert_eq!(pass, Some("user".to_string()));
    }

    #[test]
    fn test_utf8_handling() {
        // RFC 5198 introduces UTF-8 usage guidelines for Internet protocols
        // While not part of RFC 3261, testing UTF-8 handling is important
        
        // UTF-8 sequences should be properly escaped in SIP URIs
        let (rem, (user, pass)) = userinfo(b"user%C3%A4%C3%B6%C3%BC@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "useräöü");
        assert_eq!(pass, None);
        
        let (rem, (user, pass)) = userinfo(b"user:pass%E2%82%AC@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user");
        assert_eq!(pass, Some("pass€".to_string()));
    }

    #[test]
    fn test_invalid_userinfo() {
        // Missing user part (empty user)
        assert!(userinfo(b"@").is_err());
        
        // Incomplete escaped sequence
        assert!(userinfo(b"user%2@").is_err());
        
        // Invalid hex in escaped sequence
        assert!(userinfo(b"user%ZZ@").is_err());
        
        // Incomplete userinfo (missing @)
        assert!(userinfo(b"user").is_err());
        
        // Password with invalid characters for password
        // Password can only contain: unreserved / escaped / "&" / "=" / "+" / "$" / ","
        assert!(userinfo(b"user:pass;@").is_err());
    }

    #[test]
    fn test_strict_rfc3261_user_unreserved() {
        // RFC 3261 Section 25.1 defines user-unreserved characters exactly:
        // user-unreserved  =  "&" / "=" / "+" / "$" / "," / ";" / "?" / "/"
        
        // Each character should be individually testable per the ABNF
        let test_chars = [b'&', b'=', b'+', b'$', b',', b';', b'?', b'/'];
        
        for &c in &test_chars {
            let input = vec![c, b'@'];
            let (rem, (user, pass)) = userinfo(&input).unwrap_or_else(|_| panic!("Failed to parse char: {}", c as char));
            assert!(rem.is_empty());
            assert_eq!(user, String::from_utf8(vec![c]).unwrap());
            assert_eq!(pass, None);
        }
        
        // Combined test with all special chars
        let (rem, (user, pass)) = userinfo(b"&=+$,;?/@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "&=+$,;?/");
        assert_eq!(pass, None);
    }
    
    #[test]
    fn test_strict_rfc3261_password_special_chars() {
        // RFC 3261 Section 25.1 defines password characters exactly:
        // password = *( unreserved / escaped / "&" / "=" / "+" / "$" / "," )
        
        // Each allowed special character should be individually testable
        let allowed_chars = [b'&', b'=', b'+', b'$', b','];
        
        for &c in &allowed_chars {
            let input = vec![b'u', b':', c, b'@'];
            let (rem, (user, pass)) = userinfo(&input).unwrap_or_else(|_| panic!("Failed to parse allowed password char: {}", c as char));
            assert!(rem.is_empty());
            assert_eq!(user, "u");
            assert_eq!(pass, Some(String::from_utf8(vec![c]).unwrap()));
        }
        
        // Disallowed chars in password (that are allowed in user)
        let disallowed_chars = [b';', b'?', b'/'];
        
        for &c in &disallowed_chars {
            let input = vec![b'u', b':', c, b'@'];
            assert!(userinfo(&input).is_err(), "Parser incorrectly accepted disallowed password char: {}", c as char);
        }
    }
    
    #[test]
    fn test_rfc4475_torture_tests() {
        // From RFC 4475 Section 3.1.1.1 - "Valid, non-included (new) token forms"
        // Tokens in SIP can have multiple forms described in the torture test document
        
        // Example with !
        let (rem, (user, pass)) = userinfo(b"a!b@").unwrap_or_else(|_| panic!("Failed to parse user with ! character"));
        assert!(rem.is_empty());
        assert_eq!(user, "a!b");
        assert_eq!(pass, None);
        
        // Example with '
        let (rem, (user, pass)) = userinfo(b"a'b@").unwrap_or_else(|_| panic!("Failed to parse user with ' character"));
        assert!(rem.is_empty());
        assert_eq!(user, "a'b");
        assert_eq!(pass, None);
        
        // Example with (
        let (rem, (user, pass)) = userinfo(b"a(b@").unwrap_or_else(|_| panic!("Failed to parse user with ( character"));
        assert!(rem.is_empty());
        assert_eq!(user, "a(b");
        assert_eq!(pass, None);
    }
    
    #[test]
    fn test_rfc3261_escaping_rules() {
        // RFC 3261 states escaped characters represent their UTF-8 counterparts
        
        // Test escaped user-unreserved (shouldn't need escaping but should work if escaped)
        let (rem, (user, pass)) = userinfo(b"user%3B%3F%2F@").unwrap_or_else(|_| panic!("Failed to parse escaped user-unreserved chars"));
        assert!(rem.is_empty());
        assert_eq!(user, "user;?/");
        assert_eq!(pass, None);
        
        // Test regular invalid characters that must be escaped
        // Space and quotes must be escaped in SIP URIs
        let (rem, (user, pass)) = userinfo(b"first%20last:pass%22word%22@").unwrap_or_else(|_| panic!("Failed to parse escaped space and quotes"));
        assert!(rem.is_empty());
        assert_eq!(user, "first last");
        assert_eq!(pass, Some("pass\"word\"".to_string()));
    }
    
    #[test]
    fn test_rfc5118_ipv6_ref_in_userinfo() {
        // RFC 5118 Section 4.1 - Example with IPv6 reference in userinfo 
        // SIP URI with IPv6 reference in userinfo part: sip:[2001:db8::10:5070]@example.com
        // The userinfo would be just the bracketed IPv6 address as a string
        
        // Let's debug what's happening with the user part parsing
        let ipv6_user = b"[2001:db8::10:5070]";
        let result = user(ipv6_user);
        
        match result {
            Ok((rem, parsed)) => {
                println!("Successfully parsed user part:");
                println!("Remaining: {:?}", rem);
                println!("Parsed: {:?}", parsed);
            },
            Err(e) => {
                println!("Failed to parse user part:");
                println!("Error: {:?}", e);
            }
        }
        
        // Try parsing each character to see if we can identify specific problematic characters
        println!("\nTesting character handling:");
        println!("[ is_user_unreserved: {}", is_user_unreserved(b'['));
        println!("] is_user_unreserved: {}", is_user_unreserved(b']'));
        println!(": is_user_unreserved: {}", is_user_unreserved(b':'));
        println!("0 is_user_unreserved: {}", is_user_unreserved(b'0'));
        
        // Try the full userinfo parser
        let ipv6_userinfo = b"[2001:db8::10:5070]@";
        let userinfo_result = userinfo(ipv6_userinfo);
        
        match userinfo_result {
            Ok((rem, (user, pass))) => {
                println!("\nSuccessfully parsed full userinfo:");
                println!("Remaining: {:?}", rem);
                println!("User: {:?}", user);
                println!("Password: {:?}", pass);
            },
            Err(e) => {
                println!("\nFailed to parse full userinfo:");
                println!("Error: {:?}", e);
            }
        }
        
        // Now verify the result
        let (rem, (user, pass)) = userinfo(ipv6_userinfo).unwrap_or_else(|_| panic!("Failed to parse IPv6 reference in userinfo"));
        assert!(rem.is_empty());
        assert_eq!(user, "[2001:db8::10:5070]");
        assert_eq!(pass, None);
    }
    
    #[test]
    fn test_rfc3261_tel_subscriber() {
        // According to RFC 3261 ABNF, userinfo can contain telephone-subscriber
        // While the implementation might handle telephone syntax elsewhere,
        // the ABNF allows for telephone-subscriber in the userinfo position
        
        // Example based on RFC 3966 (The tel URI)
        let (rem, (user, pass)) = userinfo(b"+1-212-555-0123@").unwrap_or_else(|_| panic!("Failed to parse telephone-subscriber in userinfo"));
        assert!(rem.is_empty());
        assert_eq!(user, "+1-212-555-0123");
        assert_eq!(pass, None);
    }
    
    #[test]
    fn test_escaped_separator() {
        // Test that an escaped colon in user part is not treated as a separator
        let (rem, (user, pass)) = userinfo(b"user%3Aname@").unwrap_or_else(|_| panic!("Failed to parse escaped colon in userinfo"));
        assert!(rem.is_empty());
        assert_eq!(user, "user:name");
        assert_eq!(pass, None);
        
        // Test that an escaped @ in user or password is not treated as end of userinfo
        let (rem, (user, pass)) = userinfo(b"user%40example.com@").unwrap_or_else(|_| panic!("Failed to parse escaped @ in user"));
        assert!(rem.is_empty());
        assert_eq!(user, "user@example.com");
        assert_eq!(pass, None);
        
        let (rem, (user, pass)) = userinfo(b"user:pass%40word@").unwrap_or_else(|_| panic!("Failed to parse escaped @ in password"));
        assert!(rem.is_empty());
        assert_eq!(user, "user");
        assert_eq!(pass, Some("pass@word".to_string()));
    }
} 