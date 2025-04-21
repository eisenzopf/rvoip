use std::collections::HashMap;
use std::str::FromStr;
use std::net::IpAddr;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_while, take_while1},
    character::complete::{char, digit1},
    combinator::{map, map_res, opt, verify},
    multi::{many0, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use crate::error::{Error, Result};
use crate::uri::{Uri, Scheme, Host};
use crate::types::param::Param;
use super::utils::{clone_str, parse_token};
use ordered_float::NotNan;

// Parse the scheme of a URI (sip, sips, tel)
pub fn scheme_parser(input: &str) -> IResult<&str, Scheme> {
    alt((
        map(tag("sips"), |_| Scheme::Sips),
        map(tag("SIPS"), |_| Scheme::Sips),
        map(tag("sip"), |_| Scheme::Sip),
        map(tag("SIP"), |_| Scheme::Sip),
        map(tag("tel"), |_| Scheme::Tel),
        map(tag("TEL"), |_| Scheme::Tel),
    ))(input)
}

// Helper: Check if char is allowed in user/password part (unreserved or escaped)
fn is_userinfo_char(c: char) -> bool {
    c.is_alphanumeric() || 
    matches!(c, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')' | 
               '&' | '=' | '+' | '$' | ',' | 
               '%')
    // Percent escapes are handled separately by unescaper
}

// Parse the userinfo part (user:password@)
pub fn userinfo_parser(input: &str) -> IResult<&str, (Option<String>, Option<String>)> {
    match opt(terminated(
        pair(
            map(
                // Use take_while1 with allowed chars, not take_till
                take_while1(is_userinfo_char), 
                |s: &str| unescape_user_info(s).unwrap_or_else(|_| s.to_string())
            ),
            opt(preceded(
                char(':'),
                map(
                    // Use take_while1 with allowed chars for password too
                    take_while1(is_userinfo_char), 
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
pub fn host_parser(input: &str) -> IResult<&str, Host> {
    alt((
        ipv6_parser,
        ipv4_parser,
        domain_parser
    ))(input)
}

// Parse the port part
pub fn port_parser(input: &str) -> IResult<&str, u16> {
    map_res(
        preceded(char(':'), digit1),
        |s: &str| s.parse::<u16>()
    )(input)
}

// Parse a single parameter
fn parameter_parser(input: &str) -> IResult<&str, Param> {
    let (input, _) = char(';')(input)?; // Consume semicolon first
    let (input, key_str) = map(
                take_till(|c| c == '=' || c == ';' || c == '?' || c == '\r' || c == '\n'),
                |s: &str| s.trim() // Trim whitespace from key
            )(input)?;
    
    // Fail if key is empty after trimming
    if key_str.is_empty() {
        return Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)));
    }

    let (input, opt_val_str) = opt(preceded(
                char('='),
                map(
                    // Value stops at semicolon, comma, ?, or EOL
                    take_till(|c| c == ';' || c == ',' || c == '?' || c == '\r' || c == '\n'),
                    |s: &str| s.trim() // Trim whitespace from value
                )
            ))(input)?;

    // Attempt to unescape key and value - ignore errors for now, just use original
    let key = unescape_param(key_str).unwrap_or_else(|_| key_str.to_string());
    let opt_val = opt_val_str.map(|v| unescape_param(v).unwrap_or_else(|_| v.to_string()));

    // Match known parameters (case-insensitive)
    let param = match key.to_ascii_lowercase().as_str() {
        "branch" => Param::Branch(opt_val.unwrap_or_default()),
        "tag" => Param::Tag(opt_val.unwrap_or_default()),
        "expires" => {
            opt_val
                .as_ref()
                .and_then(|v| v.parse::<u32>().ok())
                .map(Param::Expires)
                .unwrap_or_else(|| Param::Other(key, opt_val.clone()))
        }
        "received" => {
            opt_val
                .as_ref()
                .and_then(|v| IpAddr::from_str(v).ok())
                .map(Param::Received)
                .unwrap_or_else(|| Param::Other(key, opt_val.clone()))
        }
        "maddr" => Param::Maddr(opt_val.unwrap_or_default()),
        "ttl" => {
            opt_val
                .as_ref()
                .and_then(|v| v.parse::<u8>().ok())
                .map(Param::Ttl)
                .unwrap_or_else(|| Param::Other(key, opt_val.clone()))
        }
        "lr" => Param::Lr, // Flag parameter
        "q" => {
             opt_val
                 .as_ref()
                 .and_then(|v| v.parse::<f32>().ok())
                 .and_then(|f| NotNan::try_from(f).ok())
                 .map(Param::Q)
                 .unwrap_or_else(|| Param::Other(key, opt_val.clone()))
        }
        "transport" => Param::Transport(opt_val.unwrap_or_default()),
        "user" => Param::User(opt_val.unwrap_or_default()),
        "method" => Param::Method(opt_val.unwrap_or_default()),
        // Unknown parameter or parameter without value treated as Other
        _ => Param::Other(key, opt_val),
    };

    Ok((input, param))
}

/// Parser for URI parameters (e.g., ;param1=value1;param2)
pub fn parameters_parser(input: &str) -> IResult<&str, Vec<Param>> {
    // Revert to many0(parameter_parser) - parameter_parser now handles required ;
    many0(parameter_parser)(input)
}

// Parse a single header (key=value)
fn header_parser(input: &str) -> IResult<&str, (String, String)> {
    // Refactor using separated_pair for clarity
    separated_pair(
        // Key: Take characters until '=' or '&'. Check non-empty. Unescape.
        map_res(
            take_till(|c| c == '=' || c == '&'), 
            |key_part: &str| {
                let trimmed_key = key_part.trim();
                if trimmed_key.is_empty() {
                    // Return an error that map_res understands
                    Err(Error::MalformedUriComponent { 
                        component: "header key".to_string(), 
                        message: "Empty header key found".to_string() 
                    }) 
                } else {
                    unescape_param(trimmed_key) // Unescape non-empty key
                }
            }
        ),
        char('='), // Separator
        // Value: Take characters until '&' or end of input. Unescape.
        map_res(
            take_till(|c| c == '&'), // Consume value until next separator or end
            |s: &str| unescape_param(s.trim()) // Trim and unescape value
        )
    )(input)
}

// Parse all headers (?key=value&key2=value2)
fn headers_parser(input: &str) -> IResult<&str, HashMap<String, String>> {
    preceded(
        char('?'),
        map(
            // Ensure header_parser can handle empty values correctly if needed
            separated_list1(char('&'), header_parser), 
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
    
    // Parse parameters first, consumes input up to the '?' or end
    let (input_after_params, parameters_vec) = opt(parameters_parser)(input)?; 
    // Then parse headers from the remaining input *after* parameters
    // This assumes parameters end before headers start
    let (final_input, headers_map) = opt(headers_parser)(input_after_params)?;

    let mut uri = Uri::new(scheme, host);
    uri.user = user;
    uri.password = password;
    uri.port = port;
    uri.parameters = parameters_vec.unwrap_or_default();
    uri.headers = headers_map.unwrap_or_default();

    // Return the final remaining input after parsing everything
    Ok((final_input, uri)) 
}

/// Parse a URI string into a Uri object
pub fn parse_uri(input: &str) -> Result<Uri> {
    let trimmed_input = input.trim();
    if trimmed_input.is_empty() {
        return Err(Error::InvalidUri("Empty URI string".to_string()));
    }
    match uri_parser(trimmed_input) {
        // Ensure the entire input was consumed by the uri_parser
        Ok((rest, uri)) if rest.is_empty() => Ok(uri), 
        Ok((rest, _)) => Err(Error::InvalidUri(format!("URI parser finished but did not consume entire input. Remaining: '{}'", rest))),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => {
             // Provide more context on parsing failure
             Err(Error::InvalidUri(format!("Failed to parse URI near '{}': {:?}", &trimmed_input[..trimmed_input.len() - e.input.len()], e.code)))
        },
        Err(nom::Err::Incomplete(_)) => Err(Error::InvalidUri("Incomplete URI input".to_string())),
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

impl FromStr for Uri {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
         let trimmed = s.trim();
         if trimmed == "*" {
              return Err(Error::InvalidUri("Parsing '*' URI is not currently supported by this structure".to_string()));
         }
         // Call the main public parse function which handles trimming and full consumption check
         parse_uri(trimmed)
    }
} 