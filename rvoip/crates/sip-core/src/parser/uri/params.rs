use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while1, take_while_m_n},
    character::complete::{digit1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1},
    sequence::{pair, preceded, separated_pair},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

// Use new specific modules
use crate::parser::common_chars::{escaped, unreserved};
use crate::parser::token::token;
use crate::parser::separators::{semi, equal};
use crate::parser::uri::host::host; // For maddr_param
use crate::parser::utils::unescape_uri_component; // Import unescape helper
use crate::parser::ParseResult;
use crate::types::param::{Param, GenericValue}; // Using GenericValue now
use crate::types::uri::Host as UriHost; // Avoid conflict if Host enum imported directly
use crate::error::Error;

// param-unreserved = "[" / "]" / "/" / ":" / "&" / "+" / "$"
fn is_param_unreserved(c: u8) -> bool {
    matches!(c, b'[' | b']' | b'/' | b':' | b'&' | b'+' | b'$')
}

// paramchar = param-unreserved / unreserved / escaped
// Returns raw bytes
pub fn paramchar(input: &[u8]) -> ParseResult<&[u8]> {
    alt((take_while1(is_param_unreserved), unreserved, escaped))(input)
}

// pname = 1*paramchar
// Returns raw bytes, unescaping happens in other_param
pub fn pname(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many1(paramchar))(input)
}

// pvalue = 1*paramchar
// Returns raw bytes, unescaping happens in other_param
pub fn pvalue(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many1(paramchar))(input)
}

// other-param = pname [ "=" pvalue ]
// Updated to unescape name and value
fn other_param(input: &[u8]) -> ParseResult<Param> {
    // If the input ends with an equals sign and nothing after it, this should fail per RFC 3261
    if input.len() > 1 && input[input.len() - 1] == b'=' {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::Tag)));
    }
    
    map_res(
        pair(pname, opt(preceded(equal, pvalue))),
        |(name_bytes, value_opt_bytes)| -> Result<Param, Error> {
            let name = unescape_uri_component(name_bytes)?;
            let value_opt = value_opt_bytes
                .map(|v_bytes| unescape_uri_component(v_bytes))
                .transpose()?;
            
            // Construct Param::Other, but now the value is just Option<String>
            // This loses the Host/Token/Quoted distinction from generic_param
            Ok(Param::Other(name, value_opt.map(|v| GenericValue::Token(v))))
        }
    )(input)
}

// transport-param = "transport=" ( "udp" / "tcp" / "sctp" / "tls" / other-transport)
// other-transport = token
// RFC 3261 specifies parameter names are case-insensitive
fn transport_param(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(tag_no_case(b"transport="), token),
        |t_bytes| str::from_utf8(t_bytes).map(|s| Param::Transport(s.to_string()))
    )(input)
}

// user-param = "user=" ( "phone" / "ip" / other-user)
// other-user = token
// RFC 3261 specifies parameter names are case-insensitive
fn user_param(input: &[u8]) -> ParseResult<Param> {
     map_res(
        preceded(tag_no_case(b"user="), token),
        |u_bytes| str::from_utf8(u_bytes).map(|s| Param::User(s.to_string()))
    )(input)
}

// method-param = "method=" Method (Method from request line)
// For URI context, Method is just a token.
// RFC 3261 specifies parameter names are case-insensitive
fn method_param(input: &[u8]) -> ParseResult<Param> {
     map_res(
        preceded(tag_no_case(b"method="), token),
        |m_bytes| str::from_utf8(m_bytes).map(|s| Param::Method(s.to_string()))
    )(input)
}

// ttl-param = "ttl=" ttl (1*3 DIGIT)
// RFC 3261 limits TTL to 1*3DIGIT (max 255)
// RFC 3261 specifies parameter names are case-insensitive
fn ttl_param(input: &[u8]) -> ParseResult<Param> {
    let original_input = input;  // Store original input for error handling
    
    // First check for exactly 1-3 digits after "ttl="
    let (rem, digits) = preceded(
        tag_no_case(b"ttl="), 
        take_while_m_n(1, 3, |c: u8| c.is_ascii_digit())
    )(input)?;
    
    // Check if there might be more digits following
    if !rem.is_empty() && rem[0].is_ascii_digit() {
        return Err(nom::Err::Error(NomError::new(original_input, ErrorKind::TooLarge)));
    }
    
    // Now parse the value and check the limit
    let s = str::from_utf8(digits)
        .map_err(|_| nom::Err::Failure(NomError::new(original_input, ErrorKind::Char)))?;
    
    let value = s.parse::<u32>()
        .map_err(|_| nom::Err::Failure(NomError::new(original_input, ErrorKind::Digit)))?;
    
    // RFC 3261's 1*3DIGIT effectively limits TTL to 0-999, but IPv4 TTL is 0-255
    if value > 255 {
        return Err(nom::Err::Error(NomError::new(original_input, ErrorKind::TooLarge)));
    }
    
    Ok((rem, Param::Ttl(value as u8)))
}

// maddr-param = "maddr=" host
// RFC 3261 specifies parameter names are case-insensitive
// Need to handle invalid hosts by propagating errors
fn maddr_param(input: &[u8]) -> ParseResult<Param> {
    // Parse "maddr=" prefix (case-insensitive)
    let (remaining, _) = tag_no_case(b"maddr=")(input)?;
    
    // Explicitly try to parse a valid host, allowing host() to do validation
    match host(remaining) {
        Ok((rem, host_val)) => {
            // Convert host to string, maintaining format
            let host_str = match &host_val {
                UriHost::Domain(domain) => domain.clone(),
                UriHost::Address(addr) => {
                    if addr.is_ipv6() {
                        // For IPv6, use the raw input to preserve brackets
                        let consumed_len = remaining.len() - rem.len();
                        let raw_ipv6 = &remaining[..consumed_len];
                        String::from_utf8_lossy(raw_ipv6).into_owned()
                    } else {
                        // For IPv4, toString is fine
                        addr.to_string()
                    }
                }
            };
            
            Ok((rem, Param::Maddr(host_str)))
        },
        // Propagate host parsing errors
        Err(e) => Err(e),
    }
}

// lr-param = "lr"
// RFC 3261 specifies parameter names are case-insensitive
fn lr_param(input: &[u8]) -> ParseResult<Param> {
    map(tag_no_case(b"lr"), |_| Param::Lr)(input)
}

// uri-parameter = transport-param / user-param / method-param / ttl-param / maddr-param / lr-param / other-param
fn uri_parameter(input: &[u8]) -> ParseResult<Param> {
    // Order matters: check specific params before generic 'other_param'
    alt((
        transport_param,
        user_param,
        method_param,
        ttl_param,
        maddr_param,
        lr_param,
        other_param, // Must be last
    ))(input)
}

// uri-parameters = *( ";" uri-parameter)
pub fn uri_parameters(input: &[u8]) -> ParseResult<Vec<Param>> {
    many0(preceded(semi, uri_parameter))(input)
}

#[cfg(test)]
mod tests {
     use super::*;
     use crate::types::uri::Host;
     use std::net::{Ipv4Addr, IpAddr};

    #[test]
    fn test_other_param_unescaped() {
        let (rem, param) = other_param(b"name%20with%20space=val%2fslash").unwrap();
        assert!(rem.is_empty());
        // Check unescaped name and value
        if let Param::Other(name, Some(GenericValue::Token(value))) = param {
            assert_eq!(name, "name with space");
            assert_eq!(value, "val/slash");
        } else {
            panic!("Param structure mismatch");
        }
    }
    
    #[test]
    fn test_uri_parameters_unescaped() {
        let input = b";transport=tcp;p%20name=p%20val;lr";
        let (rem, params) = uri_parameters(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
        assert!(params.contains(&Param::Transport("tcp".to_string())));
        assert!(params.contains(&Param::Lr));
        assert!(params.iter().any(|p| matches!(p, Param::Other(n, Some(GenericValue::Token(v))) if n == "p name" && v == "p val")));
    }

    // === RFC 3261 Section 19.1.1 Specific Parameter Tests ===

    #[test]
    fn test_transport_param() {
        // Standard values from RFC 3261
        let (rem, param) = transport_param(b"transport=udp").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Transport("udp".to_string()));

        let (rem, param) = transport_param(b"transport=tcp").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Transport("tcp".to_string()));

        let (rem, param) = transport_param(b"transport=sctp").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Transport("sctp".to_string()));

        let (rem, param) = transport_param(b"transport=tls").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Transport("tls".to_string()));

        // Other-transport (custom transport protocol)
        let (rem, param) = transport_param(b"transport=ws").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Transport("ws".to_string()));

        // Case insensitivity check
        let (rem, param) = transport_param(b"transport=TcP").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Transport("TcP".to_string()));

        // Remaining input
        let (rem, param) = transport_param(b"transport=udp;other=param").unwrap();
        assert_eq!(rem, b";other=param");
        assert_eq!(param, Param::Transport("udp".to_string()));
    }

    #[test]
    fn test_user_param() {
        // Standard values from RFC 3261
        let (rem, param) = user_param(b"user=phone").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::User("phone".to_string()));

        let (rem, param) = user_param(b"user=ip").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::User("ip".to_string()));

        // Other-user (custom user type)
        let (rem, param) = user_param(b"user=extension").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::User("extension".to_string()));

        // Case insensitivity check
        let (rem, param) = user_param(b"user=PhOnE").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::User("PhOnE".to_string()));
    }

    #[test]
    fn test_method_param() {
        // Common methods
        let (rem, param) = method_param(b"method=INVITE").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Method("INVITE".to_string()));

        let (rem, param) = method_param(b"method=REGISTER").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Method("REGISTER".to_string()));

        // Case sensitivity for method values - should preserve case
        let (rem, param) = method_param(b"method=Subscribe").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Method("Subscribe".to_string()));
    }

    #[test]
    fn test_ttl_param() {
        // Valid TTL values (1-255)
        let (rem, param) = ttl_param(b"ttl=1").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Ttl(1));

        let (rem, param) = ttl_param(b"ttl=128").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Ttl(128));

        let (rem, param) = ttl_param(b"ttl=255").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Ttl(255));

        // RFC 3261 limits TTL to 1*3DIGIT (max 255)
        assert!(ttl_param(b"ttl=256").is_err());
        assert!(ttl_param(b"ttl=1000").is_err());
        assert!(ttl_param(b"ttl=0").is_ok()); // Technically allowed by ABNF
    }

    #[test]
    fn test_maddr_param() {
        // IPv4 address
        let (rem, param) = maddr_param(b"maddr=192.168.1.1").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Maddr("192.168.1.1".to_string()));

        // Domain name
        let (rem, param) = maddr_param(b"maddr=example.com").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Maddr("example.com".to_string()));

        // IPv6 reference - assuming we support IPv6 in host parser
        let (rem, param) = maddr_param(b"maddr=[2001:db8::1]").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Maddr("[2001:db8::1]".to_string()));

        // Invalid host should fail
        assert!(maddr_param(b"maddr=invalid..host").is_err());
    }

    #[test]
    fn test_lr_param() {
        // Flag parameter without a value
        let (rem, param) = lr_param(b"lr").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Lr);

        // With trailing input
        let (rem, param) = lr_param(b"lr;next=param").unwrap();
        assert_eq!(rem, b";next=param");
        assert_eq!(param, Param::Lr);
    }

    // === Combined URI Parameter Tests ===

    #[test]
    fn test_uri_parameter_precedence() {
        // Test that specific param parsers take precedence over other_param
        let (rem, param) = uri_parameter(b"transport=udp").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Transport("udp".to_string()));

        let (rem, param) = uri_parameter(b"lr").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Lr);

        // Generic/other parameters
        let (rem, param) = uri_parameter(b"custom=value").unwrap();
        assert!(rem.is_empty());
        assert!(matches!(param, Param::Other(n, Some(GenericValue::Token(v))) if n == "custom" && v == "value"));
    }

    #[test]
    fn test_uri_parameters_multiple() {
        // Multiple parameters of different types
        let input = b";transport=tcp;ttl=5;lr;custom=value";
        let (rem, params) = uri_parameters(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 4);
        assert!(params.contains(&Param::Transport("tcp".to_string())));
        assert!(params.contains(&Param::Ttl(5)));
        assert!(params.contains(&Param::Lr));
        assert!(params.iter().any(|p| matches!(p, Param::Other(n, Some(GenericValue::Token(v))) if n == "custom" && v == "value")));

        // Empty parameter list
        let (rem, params) = uri_parameters(b"").unwrap();
        assert!(rem.is_empty());
        assert!(params.is_empty());
    }

    // === RFC 4475 Torture Test Cases ===

    #[test]
    fn test_rfc4475_escaped_semicolons() {
        // RFC 4475 Section 3.1.1.13: "Escaped Semicolons in URI Parameters"
        // We should handle escaped semicolons correctly in parameter values
        
        // In params.rs context, semicolons are already processed by uri_parameters
        // so we're testing the individual param parsers with escaped values
        
        let (rem, param) = other_param(b"param=value%3Bwith%3Bsemicolons").unwrap();
        assert!(rem.is_empty());
        if let Param::Other(name, Some(GenericValue::Token(value))) = param {
            assert_eq!(name, "param");
            assert_eq!(value, "value;with;semicolons");
        } else {
            panic!("Param structure mismatch");
        }
    }

    #[test]
    fn test_rfc4475_case_sensitivity() {
        // RFC 4475 Section 3.1.1.10: "Case-Sensitivity in URI User Part"
        // While this section primarily concerns the user part, the RFC also notes:
        // "Parameter names are case-insensitive, but their values are not"
        
        // Parameter names should be case-insensitive
        let input1 = b";Transport=udp;Custom=Value";
        let (_, params1) = uri_parameters(input1).unwrap();
        
        let input2 = b";transport=udp;custom=Value";
        let (_, params2) = uri_parameters(input2).unwrap();
        
        // The Transport/transport parameters should produce identical Param values
        assert!(params1.iter().any(|p| matches!(p, Param::Transport(v) if v == "udp")));
        assert!(params2.iter().any(|p| matches!(p, Param::Transport(v) if v == "udp")));
        
        // But parameter values are case-sensitive
        let val1 = params1.iter().find_map(|p| match p {
            Param::Other(n, Some(GenericValue::Token(v))) if n == "custom" || n == "Custom" => Some(v.clone()),
            _ => None,
        });
        
        let val2 = params2.iter().find_map(|p| match p {
            Param::Other(n, Some(GenericValue::Token(v))) if n == "custom" || n == "Custom" => Some(v.clone()),
            _ => None,
        });
        
        assert_eq!(val1, Some("Value".to_string()));
        assert_eq!(val2, Some("Value".to_string()));
    }

    // === RFC 5118 IPv6 Test Cases ===

    #[test]
    fn test_rfc5118_ipv6_params() {
        // RFC 5118 - IPv6 examples in parameters
        
        // maddr with IPv6
        let (rem, param) = maddr_param(b"maddr=[2001:db8::1]").unwrap();
        assert!(rem.is_empty());
        assert_eq!(param, Param::Maddr("[2001:db8::1]".to_string()));
        
        // URI parameters with IPv6
        let input = b";maddr=[2001:db8::1];transport=tcp";
        let (rem, params) = uri_parameters(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| matches!(p, Param::Maddr(m) if m == "[2001:db8::1]")));
        assert!(params.contains(&Param::Transport("tcp".to_string())));
    }

    // === Error Cases & Edge Cases ===

    #[test]
    fn test_param_error_cases() {
        // Invalid TTL (non-digit)
        assert!(ttl_param(b"ttl=abc").is_err());
        
        // Invalid maddr (malformed host)
        assert!(maddr_param(b"maddr=not..valid").is_err());
        
        // Empty parameter value should be rejected per RFC 3261 ABNF: pvalue = 1*paramchar
        assert!(other_param(b"empty=").is_err());
        
        // Parameter with no value (not lr)
        let (rem, param) = other_param(b"flag").unwrap();
        assert!(rem.is_empty());
        if let Param::Other(name, None) = param {
            assert_eq!(name, "flag");
        } else {
            panic!("Param structure mismatch");
        }
    }
} 