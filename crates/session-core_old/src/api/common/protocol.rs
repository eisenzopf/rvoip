//! Protocol parsing utilities for SIP URIs

#[derive(Debug, Clone, PartialEq)]
pub enum Protocol {
    Sip,
    Sips,  // Secure SIP
    Tel,
}

/// Parse a call target and determine the protocol
/// 
/// # Examples
/// ```rust
/// use rvoip_session_core::api::common::protocol::{parse_call_target, Protocol};
/// 
/// let (uri, proto) = parse_call_target("bob@example.com", 5060);
/// // When port is default (5060), it may or may not be included in the URI
/// assert!(uri == "sip:bob@example.com:5060" || uri == "sip:bob@example.com");
/// assert_eq!(proto, Protocol::Sip);
/// 
/// let (uri, proto) = parse_call_target("+14155551234", 5060);
/// assert_eq!(uri, "tel:+14155551234");
/// assert_eq!(proto, Protocol::Tel);
/// ```
pub fn parse_call_target(target: &str, default_port: u16) -> (String, Protocol) {
    // Check for explicit protocol
    if target.starts_with("sip:") {
        let uri = add_port_if_needed(target, default_port, Protocol::Sip);
        return (uri, Protocol::Sip);
    }
    
    if target.starts_with("sips:") {
        let uri = add_port_if_needed(target, default_port, Protocol::Sips);
        return (uri, Protocol::Sips);
    }
    
    if target.starts_with("tel:") {
        // Tel URIs don't use ports
        return (target.to_string(), Protocol::Tel);
    }
    
    // Auto-detect protocol based on format
    if target.contains('@') {
        // Looks like a SIP address (user@host)
        let uri = format!("sip:{}", target);
        let uri = add_port_if_needed(&uri, default_port, Protocol::Sip);
        (uri, Protocol::Sip)
    } else if is_phone_number(target) {
        // Looks like a phone number
        (format!("tel:{}", target), Protocol::Tel)
    } else {
        // Default to SIP with just a username
        let uri = format!("sip:{}", target);
        let uri = add_port_if_needed(&uri, default_port, Protocol::Sip);
        (uri, Protocol::Sip)
    }
}

/// Add port to SIP URI if not present
fn add_port_if_needed(uri: &str, port: u16, protocol: Protocol) -> String {
    if protocol == Protocol::Tel {
        // Tel URIs don't use ports
        return uri.to_string();
    }
    
    // Check if URI already has a port
    if let Some(host_part) = uri.split('@').nth(1) {
        if host_part.contains(':') {
            // Already has a port
            uri.to_string()
        } else if port != 5060 {
            // Add non-default port
            format!("{}:{}", uri, port)
        } else {
            // Default port, don't add
            uri.to_string()
        }
    } else {
        // No @ sign, can't add port meaningfully
        uri.to_string()
    }
}

/// Check if a string looks like a phone number
fn is_phone_number(s: &str) -> bool {
    // Allow digits, +, -, (, ), and spaces
    let cleaned: String = s.chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    
    if cleaned.is_empty() {
        return false;
    }
    
    // Check for common phone number patterns
    // International: starts with +
    if cleaned.starts_with('+') {
        return cleaned.len() >= 8 && cleaned.len() <= 15;
    }
    
    // Emergency numbers (short)
    if cleaned.len() <= 3 {
        return cleaned.chars().all(|c| c.is_ascii_digit());
    }
    
    // Regular phone numbers (7-15 digits)
    cleaned.len() >= 7 && cleaned.len() <= 15 && cleaned.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_sip_targets() {
        assert_eq!(
            parse_call_target("bob@example.com", 5060),
            ("sip:bob@example.com".to_string(), Protocol::Sip)
        );
        
        assert_eq!(
            parse_call_target("bob@example.com", 5070),
            ("sip:bob@example.com:5070".to_string(), Protocol::Sip)
        );
        
        assert_eq!(
            parse_call_target("sip:alice@example.com", 5060),
            ("sip:alice@example.com".to_string(), Protocol::Sip)
        );
        
        assert_eq!(
            parse_call_target("sip:alice@example.com:5080", 5060),
            ("sip:alice@example.com:5080".to_string(), Protocol::Sip)
        );
    }
    
    #[test]
    fn test_parse_tel_targets() {
        assert_eq!(
            parse_call_target("+14155551234", 5060),
            ("tel:+14155551234".to_string(), Protocol::Tel)
        );
        
        assert_eq!(
            parse_call_target("tel:911", 5060),
            ("tel:911".to_string(), Protocol::Tel)
        );
        
        assert_eq!(
            parse_call_target("4155551234", 5060),
            ("tel:4155551234".to_string(), Protocol::Tel)
        );
    }
}