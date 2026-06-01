// Session validation utilities
//
// Functions for validating hostnames, usernames, IP addresses, and other SDP components.

use std::net::{Ipv4Addr, Ipv6Addr};

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
            || !label.chars().last().unwrap().is_alphanumeric()
        {
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
