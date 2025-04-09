use std::collections::HashMap;
use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_while, take_while1},
    character::complete::{char, digit1},
    combinator::{map, map_res, opt, verify},
    multi::{many0, separated_list0},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use crate::error::{Error, Result};
use crate::uri::{Uri, Scheme, Host};
use super::utils::clone_str;

// Parse the scheme of a URI (sip, sips, tel)
fn scheme_parser(input: &str) -> IResult<&str, Scheme> {
    map_res(
        alt((
            tag("sip"),
            tag("sips"),
            tag("tel"),
        )),
        |s: &str| Scheme::from_str(s)
    )(input)
}

// Parse the userinfo part (user:password@)
fn userinfo_parser(input: &str) -> IResult<&str, (Option<String>, Option<String>)> {
    match opt(terminated(
        pair(
            map(
                take_till(|c| c == ':' || c == '@'),
                |s: &str| unescape_user_info(s).unwrap_or_else(|_| s.to_string())
            ),
            opt(preceded(
                char(':'),
                map(
                    take_till(|c| c == '@'),
                    |s: &str| unescape_user_info(s).unwrap_or_else(|_| s.to_string())
                )
            ))
        ),
        char('@')
    ))(input) {
        Ok((remaining, Some((user, password)))) => Ok((remaining, (Some(user), password))),
        Ok((remaining, None)) => Ok((remaining, (None, None))),
        Err(e) => Err(e),
    }
}

// Parse IPv4 address
fn ipv4_parser(input: &str) -> IResult<&str, Host> {
    let ip_parser = verify(
        take_while1(|c: char| c.is_ascii_digit() || c == '.'),
        |s: &str| is_valid_ipv4(s)
    );
    
    map(ip_parser, |s: &str| Host::IPv4(s.to_string()))(input)
}

// Parse IPv6 address
fn ipv6_parser(input: &str) -> IResult<&str, Host> {
    let ip_parser = delimited(
        char('['),
        take_while1(|c: char| c.is_ascii_hexdigit() || c == ':' || c == '.'),
        char(']')
    );
    
    map(ip_parser, |s: &str| Host::IPv6(s.to_string()))(input)
}

// Parse domain name
fn domain_parser(input: &str) -> IResult<&str, Host> {
    let domain_parser = take_while1(|c: char| c.is_alphanumeric() || c == '.' || c == '-' || c == '+');
    
    map(domain_parser, |s: &str| Host::Domain(s.to_string()))(input)
}

// Parse the host part (either IPv4, IPv6, or domain)
fn host_parser(input: &str) -> IResult<&str, Host> {
    alt((
        ipv6_parser,
        ipv4_parser,
        domain_parser
    ))(input)
}

// Parse the port part
fn port_parser(input: &str) -> IResult<&str, u16> {
    map_res(
        preceded(char(':'), digit1),
        |s: &str| s.parse::<u16>()
    )(input)
}

// Parse a single parameter
fn parameter_parser(input: &str) -> IResult<&str, (String, Option<String>)> {
    preceded(
        char(';'),
        pair(
            map(
                take_till(|c| c == '=' || c == ';' || c == '?'),
                |s: &str| unescape_param(s).unwrap_or_else(|_| s.to_string())
            ),
            opt(preceded(
                char('='),
                map(
                    take_till(|c| c == ';' || c == '?'),
                    |s: &str| unescape_param(s).unwrap_or_else(|_| s.to_string())
                )
            ))
        )
    )(input)
}

// Parse all parameters
fn parameters_parser(input: &str) -> IResult<&str, HashMap<String, Option<String>>> {
    map(
        many0(parameter_parser),
        |params| params.into_iter().collect()
    )(input)
}

// Parse a single header
fn header_parser(input: &str) -> IResult<&str, (String, String)> {
    separated_pair(
        map(
            take_till(|c| c == '=' || c == '&'),
            |s: &str| unescape_param(s).unwrap_or_else(|_| s.to_string())
        ),
        char('='),
        map(
            take_till(|c| c == '&'),
            |s: &str| unescape_param(s).unwrap_or_else(|_| s.to_string())
        )
    )(input)
}

// Parse all headers
fn headers_parser(input: &str) -> IResult<&str, HashMap<String, String>> {
    preceded(
        char('?'),
        map(
            separated_list0(char('&'), header_parser),
            |headers| headers.into_iter().collect()
        )
    )(input)
}

// Parser for a complete URI
fn uri_parser(input: &str) -> IResult<&str, Uri> {
    let (input, scheme) = terminated(scheme_parser, char(':'))(input)?;
    let (input, (user, password)) = userinfo_parser(input)?;
    let (input, host) = host_parser(input)?;
    let (input, port) = opt(port_parser)(input)?;
    
    let (input, parameters) = opt(parameters_parser)(input)?;
    let (input, headers) = opt(headers_parser)(input)?;
    
    let mut uri = Uri::new(scheme, host);
    
    uri.user = user;
    uri.password = password;
    uri.port = port;
    
    if let Some(params) = parameters {
        uri.parameters = params;
    }
    
    if let Some(hdrs) = headers {
        uri.headers = hdrs;
    }
    
    Ok((input, uri))
}

/// Parse a URI string into a Uri object
pub fn parse_uri(input: &str) -> Result<Uri> {
    match uri_parser(input) {
        Ok((_, uri)) => Ok(uri),
        Err(e) => Err(Error::InvalidUri(format!("Failed to parse URI: {input} - Error: {e:?}"))),
    }
}

/// Unescape URI user info component
fn unescape_user_info(s: &str) -> Result<String> {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    
    while let Some(c) = chars.next() {
        if c == '%' {
            let mut hex = String::with_capacity(2);
            
            if let Some(h1) = chars.next() {
                hex.push(h1);
            } else {
                return Err(Error::MalformedUriComponent {
                    component: "user info".to_string(),
                    message: "Incomplete percent encoding".to_string()
                });
            }
            
            if let Some(h2) = chars.next() {
                hex.push(h2);
            } else {
                return Err(Error::MalformedUriComponent {
                    component: "user info".to_string(),
                    message: "Incomplete percent encoding".to_string()
                });
            }
            
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                return Err(Error::MalformedUriComponent {
                    component: "user info".to_string(),
                    message: format!("Invalid percent encoding: %{}", hex)
                });
            }
        } else {
            result.push(c);
        }
    }
    
    Ok(result)
}

/// Unescape URI parameters and headers
fn unescape_param(s: &str) -> Result<String> {
    unescape_user_info(s) // Same algorithm
}

/// Check if a string is a valid IPv4 address
fn is_valid_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    
    if parts.len() != 4 {
        return false;
    }
    
    for part in parts {
        match part.parse::<u8>() {
            Ok(_) => continue,
            Err(_) => return false,
        }
    }
    
    true
}

/// Check if a string is a valid IPv6 address (simplified validation)
fn is_valid_ipv6(s: &str) -> bool {
    // Check for basic IPv6 format
    let parts = s.split(':').collect::<Vec<&str>>();
    
    // IPv6 has 8 parts max, or fewer if contains ::
    if parts.len() > 8 {
        return false;
    }
    
    // Check for empty parts (::)
    let empty_parts = parts.iter().filter(|p| p.is_empty()).count();
    
    // Handle :: (consecutive colons)
    if empty_parts > 0 {
        if empty_parts > 2 || (empty_parts == 2 && !s.contains("::")) {
            return false;
        }
    }
    
    // Validate each part
    for part in parts {
        if part.is_empty() {
            continue; // Empty part due to ::
        }
        
        // Check if it's an IPv4 address in the last part (IPv4-mapped IPv6)
        if part.contains('.') {
            return is_valid_ipv4(part);
        }
        
        // Each part should be a valid hex number with at most 4 digits
        if part.len() > 4 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_uri() {
        let uri = parse_uri("sip:example.com").unwrap();
        assert_eq!(uri.scheme, Scheme::Sip);
        assert!(matches!(uri.host, Host::Domain(ref domain) if domain == "example.com"));
        assert!(uri.user.is_none());
        
        let uri = parse_uri("sips:secure.example.com:5061").unwrap();
        assert_eq!(uri.scheme, Scheme::Sips);
        assert!(matches!(uri.host, Host::Domain(ref domain) if domain == "secure.example.com"));
        assert_eq!(uri.port, Some(5061));
    }

    #[test]
    fn test_parse_complex_uri() {
        let uri = parse_uri("sip:alice@example.com;transport=tcp?subject=Meeting").unwrap();
        assert_eq!(uri.scheme, Scheme::Sip);
        assert_eq!(uri.user, Some("alice".to_string()));
        assert!(matches!(uri.host, Host::Domain(ref domain) if domain == "example.com"));
        assert_eq!(uri.parameters.get("transport").unwrap(), &Some("tcp".to_string()));
        assert_eq!(uri.headers.get("subject").unwrap(), "Meeting");
    }

    #[test]
    fn test_tel_uri() {
        let uri = parse_uri("tel:+1-212-555-0123").unwrap();
        assert_eq!(uri.scheme, Scheme::Tel);
        assert!(matches!(uri.host, Host::Domain(ref domain) if domain == "+1-212-555-0123"));
    }

    #[test]
    fn test_escaped_uri() {
        let input = "sip:user%20with%20spaces@example.com;param=value%20with%20spaces";
        let parsed = parse_uri(input).unwrap();
        assert_eq!(parsed.user, Some("user with spaces".to_string()));
        assert_eq!(parsed.parameters.get("param").unwrap(), &Some("value with spaces".to_string()));
    }

    #[test]
    fn test_is_valid_ipv4() {
        assert!(is_valid_ipv4("192.168.1.1"));
        assert!(is_valid_ipv4("127.0.0.1"));
        assert!(is_valid_ipv4("255.255.255.255"));
        assert!(is_valid_ipv4("0.0.0.0"));
        
        assert!(!is_valid_ipv4("192.168.1"));
        assert!(!is_valid_ipv4("192.168.1.256"));
        assert!(!is_valid_ipv4("192.168.1.1.1"));
        assert!(!is_valid_ipv4("192.168.1.abc"));
    }

    #[test]
    fn test_is_valid_ipv6() {
        assert!(is_valid_ipv6("2001:db8::1"));
        assert!(is_valid_ipv6("::1"));
        assert!(is_valid_ipv6("2001:db8:0:0:0:0:0:1"));
        assert!(is_valid_ipv6("2001:db8::192.168.1.1")); // IPv4-mapped
        
        assert!(!is_valid_ipv6("2001:db8:::1")); // too many colons
        assert!(!is_valid_ipv6("2001:db8::1::1")); // multiple ::
        assert!(!is_valid_ipv6("2001:db8:gggg::1")); // invalid hex
    }
} 