use std::collections::HashMap;
use std::str::FromStr;
use std::net::IpAddr;

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
use crate::types::param::Param;
use super::utils::clone_str;
use ordered_float::NotNan;

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
fn parameter_parser(input: &str) -> IResult<&str, Param> {
    let (input, (key_str, opt_val_str)) = preceded(
        char(';'),
        pair(
            map(
                take_till(|c| c == '=' || c == ';' || c == '?' || c == '\r' || c == '\n'),
                |s: &str| s.trim() // Trim whitespace from key
            ),
            opt(preceded(
                char('='),
                map(
                    take_till(|c| c == ';' || c == '?' || c == '\r' || c == '\n'),
                    |s: &str| s.trim() // Trim whitespace from value
                )
            ))
        )
    )(input)?;

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
    many0(parameter_parser)(input)
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

    let (input, parameters_vec) = opt(parameters_parser)(input)?;
    let (input, headers_map) = opt(headers_parser)(input)?;

    let mut uri = Uri::new(scheme, host);

    uri.user = user;
    uri.password = password;
    uri.port = port;

    uri.parameters = parameters_vec.unwrap_or_default();

    if let Some(hdrs) = headers_map {
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