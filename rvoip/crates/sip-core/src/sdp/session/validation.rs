// Session validation utilities
//
// Functions for validating hostnames, usernames, IP addresses, and other SDP components.

use std::net::{Ipv4Addr, Ipv6Addr};
use nom::{
    IResult,
    bytes::complete::take_while1,
    character::complete::char,
    combinator::{recognize, verify},
    multi::separated_list1,
};

/// Validates if a string is a valid username per SDP rules
pub fn is_valid_username(username: &str) -> bool {
    if username.is_empty() {
        return false;
    }
    
    username.chars().all(|c| {
        matches!(c,
            'a'..='z' | 'A'..='Z' | '0'..='9' |
            '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '.' |
            '^' | '_' | '`' | '{' | '|' | '}' | '~' | ' ' | '/' | ':' | '='
        )
    })
}

/// Validates if a string is a valid hostname per SDP rules using nom
fn parse_hostname(input: &str) -> IResult<&str, &str> {
    // A hostname is a sequence of labels separated by dots
    let parse_label = verify(
        take_while1(|c: char| c.is_alphanumeric() || c == '-'),
        |s: &str| {
            !s.is_empty() && 
            s.len() <= 63 && 
            s.chars().next().unwrap().is_alphanumeric() && 
            s.chars().last().unwrap().is_alphanumeric()
        }
    );
    
    let (input, hostname) = recognize(
        separated_list1(
            char('.'),
            parse_label
        )
    )(input)?;
    
    // Check overall hostname length
    if hostname.len() > 255 {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::TooLarge
        )));
    }
    
    Ok((input, hostname))
}

/// Helper to validate hostname without nom (useful for direct string checks)
pub fn is_valid_hostname(hostname: &str) -> bool {
    if hostname.is_empty() || hostname.len() > 255 {
        return false;
    }

    let labels: Vec<&str> = hostname.split('.').collect();
    
    if labels.is_empty() {
        return false;
    }
    
    for label in labels {
        // Each DNS label must be between 1 and 63 characters long
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        
        // Labels must start and end with alphanumeric characters
        if !label.chars().next().unwrap().is_alphanumeric() 
           || !label.chars().last().unwrap().is_alphanumeric() {
            return false;
        }
        
        // Labels can contain alphanumeric characters and hyphens
        if !label.chars().all(|c| c.is_alphanumeric() || c == '-') {
            return false;
        }
    }
    
    true
}

/// Check if a string is a valid IPv4 address or hostname
pub fn is_valid_ipv4_or_hostname(s: &str) -> bool {
    s.parse::<Ipv4Addr>().is_ok() || is_valid_hostname(s)
}

/// Check if a string is a valid IPv6 address or hostname
pub fn is_valid_ipv6_or_hostname(s: &str) -> bool {
    s.parse::<Ipv6Addr>().is_ok() || is_valid_hostname(s)
} 