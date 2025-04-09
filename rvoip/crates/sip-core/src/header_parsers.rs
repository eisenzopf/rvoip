use std::collections::HashMap;
use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_till, take_until, take_while, take_while1},
    character::complete::{char, digit1, space0, space1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use crate::error::{Error, Result};
use crate::uri::Uri;

/// Parses a comma-separated list of values, respecting quotes
pub fn parse_comma_separated_list(input: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    
    for c in input.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            },
            ',' if !in_quotes => {
                if !current.trim().is_empty() {
                    result.push(current.trim().to_string());
                    current.clear();
                }
            },
            _ => current.push(c),
        }
    }
    
    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }
    
    result
}

/// Parses a quoted string value
fn quoted_string(input: &str) -> IResult<&str, String> {
    let (input, value) = delimited(
        char('"'),
        take_till(|c| c == '"'),
        char('"')
    )(input)?;
    
    Ok((input, value.to_string()))
}

/// Parses an unquoted token
fn token(input: &str) -> IResult<&str, String> {
    let (input, value) = take_while1(|c: char| {
        c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '!' || c == '%' || 
        c == '*' || c == '_' || c == '+' || c == '`' || c == '\'' || c == '~'
    })(input)?;
    
    Ok((input, value.to_string()))
}

/// Parses a parameter value (either quoted string or token)
fn param_value(input: &str) -> IResult<&str, String> {
    alt((quoted_string, token))(input)
}

/// Parses a name=value pair
fn name_value_pair(input: &str) -> IResult<&str, (String, String)> {
    let (input, _) = space0(input)?;
    let (input, name) = token(input)?;
    let (input, _) = tuple((space0, char('='), space0))(input)?;
    let (input, value) = param_value(input)?;
    
    Ok((input, (name, value)))
}

/// Parses authentication parameters in WWW-Authenticate and Authorization headers
pub fn parse_auth_params(input: &str) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    
    // Extract scheme first (e.g., "Digest")
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return Err(Error::InvalidHeader(format!("Invalid auth header format: {}", input)));
    }
    
    result.insert("scheme".to_string(), parts[0].to_string());
    
    // Parse each parameter
    let params = parts[1];
    for part in parse_comma_separated_list(params) {
        if let Ok((_, (name, value))) = name_value_pair(&part) {
            result.insert(name, value);
        }
    }
    
    Ok(result)
}

/// Parse a Via header value
pub fn parse_via(input: &str) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    
    // Via header format: SIP/2.0/UDP host:port;branch=xxx;other=params
    let parts: Vec<&str> = input.split(';').collect();
    
    if parts.is_empty() {
        return Err(Error::InvalidHeader(format!("Invalid Via header: {}", input)));
    }
    
    // Parse protocol part: SIP/2.0/UDP
    let protocol_parts: Vec<&str> = parts[0].trim().split('/').collect();
    if protocol_parts.len() < 3 {
        return Err(Error::InvalidHeader(format!("Invalid Via protocol: {}", parts[0])));
    }
    
    result.insert("protocol".to_string(), protocol_parts[0].to_string());
    result.insert("version".to_string(), protocol_parts[1].to_string());
    
    // Extract transport and host:port
    let transport_and_host = protocol_parts[2].trim().split_whitespace().collect::<Vec<&str>>();
    
    if transport_and_host.is_empty() {
        return Err(Error::InvalidHeader(format!("Missing transport in Via: {}", parts[0])));
    }
    
    result.insert("transport".to_string(), transport_and_host[0].to_string());
    
    // Extract sent-by (host:port)
    if transport_and_host.len() > 1 {
        let sent_by = transport_and_host[1];
        if sent_by.contains(':') {
            let host_port: Vec<&str> = sent_by.split(':').collect();
            result.insert("host".to_string(), host_port[0].to_string());
            if host_port.len() > 1 {
                result.insert("port".to_string(), host_port[1].to_string());
            }
        } else {
            result.insert("host".to_string(), sent_by.to_string());
        }
    }
    
    // Parse parameters
    for i in 1..parts.len() {
        let param = parts[i].trim();
        if param.contains('=') {
            let param_parts: Vec<&str> = param.split('=').collect();
            if param_parts.len() >= 2 {
                result.insert(param_parts[0].to_string(), param_parts[1].to_string());
            }
        } else {
            result.insert(param.to_string(), "".to_string());
        }
    }
    
    Ok(result)
}

/// Parse multiple Via headers
pub fn parse_multiple_vias(input: &str) -> Result<Vec<HashMap<String, String>>> {
    let via_parts = parse_comma_separated_list(input);
    
    let mut result = Vec::new();
    for part in via_parts {
        result.push(parse_via(&part)?);
    }
    
    Ok(result)
}

/// Parse a Contact header value
pub fn parse_contact(input: &str) -> Result<Vec<HashMap<String, String>>> {
    let mut contacts = Vec::new();
    
    // Split by commas for multiple contacts, but respect < > pairs
    let contact_parts = parse_comma_separated_list(input);
    
    // Parse each contact
    for contact in contact_parts {
        let mut components = HashMap::new();
        
        // Check if we have a display name
        if contact.contains('<') && contact.contains('>') {
            let display_name = contact[..contact.find('<').unwrap()].trim();
            if !display_name.is_empty() {
                components.insert("display_name".to_string(), 
                                  display_name.trim_matches('"').to_string());
            }
            
            // Extract URI and parameters
            let rest = &contact[contact.find('<').unwrap()..];
            if let Some(uri_end) = rest.find('>') {
                // Extract URI
                let uri = &rest[1..uri_end];
                components.insert("uri".to_string(), uri.to_string());
                
                // Extract parameters after >
                if uri_end + 1 < rest.len() {
                    let params = &rest[uri_end+1..];
                    parse_parameters(params, &mut components);
                }
            }
        } else {
            // Just a URI, possibly with parameters
            let parts: Vec<&str> = contact.split(';').collect();
            components.insert("uri".to_string(), parts[0].to_string());
            
            // Parse parameters
            for i in 1..parts.len() {
                let param = parts[i].trim();
                parse_parameter(param, &mut components);
            }
        }
        
        contacts.push(components);
    }
    
    Ok(contacts)
}

/// Parse a From or To header value
pub fn parse_address(input: &str) -> Result<HashMap<String, String>> {
    // Similar to contact but only one address
    let mut result = HashMap::new();
    
    // Check if we have a display name
    if input.contains('<') && input.contains('>') {
        let display_name = input[..input.find('<').unwrap()].trim();
        if !display_name.is_empty() {
            result.insert("display_name".to_string(), 
                          display_name.trim_matches('"').to_string());
        }
        
        // Extract URI and parameters
        let rest = &input[input.find('<').unwrap()..];
        if let Some(uri_end) = rest.find('>') {
            // Extract URI
            let uri = &rest[1..uri_end];
            result.insert("uri".to_string(), uri.to_string());
            
            // Try to parse the URI
            if let Ok(parsed_uri) = Uri::from_str(uri) {
                result.insert("uri_scheme".to_string(), parsed_uri.scheme.to_string());
                if let Some(user) = parsed_uri.user {
                    result.insert("uri_user".to_string(), user);
                }
                result.insert("uri_host".to_string(), parsed_uri.host.to_string());
                if let Some(port) = parsed_uri.port {
                    result.insert("uri_port".to_string(), port.to_string());
                }
            }
            
            // Extract parameters after >
            if uri_end + 1 < rest.len() {
                let params = &rest[uri_end+1..];
                parse_parameters(params, &mut result);
            }
        }
    } else {
        // Just a URI, possibly with parameters
        let parts: Vec<&str> = input.split(';').collect();
        result.insert("uri".to_string(), parts[0].to_string());
        
        // Try to parse the URI
        if let Ok(parsed_uri) = Uri::from_str(parts[0]) {
            result.insert("uri_scheme".to_string(), parsed_uri.scheme.to_string());
            if let Some(user) = parsed_uri.user {
                result.insert("uri_user".to_string(), user);
            }
            result.insert("uri_host".to_string(), parsed_uri.host.to_string());
            if let Some(port) = parsed_uri.port {
                result.insert("uri_port".to_string(), port.to_string());
            }
        }
        
        // Parse parameters
        for i in 1..parts.len() {
            let param = parts[i].trim();
            parse_parameter(param, &mut result);
        }
    }
    
    Ok(result)
}

/// Parse a CSeq header value
pub fn parse_cseq(input: &str) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return Err(Error::InvalidHeader(format!("Invalid CSeq format: {}", input)));
    }
    
    // Parse sequence number
    match parts[0].parse::<u32>() {
        Ok(num) => {
            result.insert("sequence".to_string(), num.to_string());
        },
        Err(_) => {
            return Err(Error::InvalidHeader(format!("Invalid CSeq number: {}", parts[0])));
        }
    }
    
    // Parse method
    result.insert("method".to_string(), parts[1].to_string());
    
    Ok(result)
}

/// Parse Content-Type header
pub fn parse_content_type(input: &str) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    
    // Split at semicolon to separate media type from parameters
    let parts: Vec<&str> = input.split(';').collect();
    
    // Parse media type/subtype
    let media_parts: Vec<&str> = parts[0].trim().split('/').collect();
    if media_parts.len() < 2 {
        result.insert("media_type".to_string(), parts[0].trim().to_string());
    } else {
        result.insert("media_type".to_string(), media_parts[0].trim().to_string());
        result.insert("media_subtype".to_string(), media_parts[1].trim().to_string());
    }
    
    // Parse parameters
    for i in 1..parts.len() {
        let param = parts[i].trim();
        parse_parameter(param, &mut result);
    }
    
    Ok(result)
}

/// Parse a parameter (name=value or flag) and add to map
pub fn parse_parameter(param: &str, map: &mut HashMap<String, String>) {
    if param.contains('=') {
        let param_parts: Vec<&str> = param.split('=').collect();
        if param_parts.len() >= 2 {
            map.insert(
                param_parts[0].trim().to_string(), 
                param_parts[1].trim().trim_matches('"').to_string()
            );
        }
    } else if !param.is_empty() {
        map.insert(param.to_string(), "".to_string());
    }
}

/// Parse parameters (multiple name=value pairs separated by semicolons)
pub fn parse_parameters(params: &str, map: &mut HashMap<String, String>) {
    for param in params.split(';') {
        let param = param.trim();
        if !param.is_empty() {
            parse_parameter(param, map);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_via() {
        let via = "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds";
        let result = parse_via(via).unwrap();
        
        assert_eq!(result.get("protocol").unwrap(), "SIP");
        assert_eq!(result.get("version").unwrap(), "2.0");
        assert_eq!(result.get("transport").unwrap(), "UDP");
        assert_eq!(result.get("host").unwrap(), "pc33.example.com");
        assert_eq!(result.get("port").unwrap(), "5060");
        assert_eq!(result.get("branch").unwrap(), "z9hG4bK776asdhds");
    }
    
    #[test]
    fn test_parse_multiple_vias() {
        let vias = "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds, SIP/2.0/TCP proxy.example.com;branch=z9hG4bK123456";
        let result = parse_multiple_vias(vias).unwrap();
        
        assert_eq!(result.len(), 2);
        
        assert_eq!(result[0].get("transport").unwrap(), "UDP");
        assert_eq!(result[0].get("host").unwrap(), "pc33.example.com");
        
        assert_eq!(result[1].get("transport").unwrap(), "TCP");
        assert_eq!(result[1].get("host").unwrap(), "proxy.example.com");
    }
    
    #[test]
    fn test_parse_contact() {
        let contact = "\"Alice\" <sip:alice@example.com>;expires=3600";
        let result = parse_contact(contact).unwrap();
        
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].get("display_name").unwrap(), "Alice");
        assert_eq!(result[0].get("uri").unwrap(), "sip:alice@example.com");
        assert_eq!(result[0].get("expires").unwrap(), "3600");
        
        // Test multiple contacts
        let contact = "\"Alice\" <sip:alice@example.com>;expires=3600, <sip:bob@example.com>;q=0.8";
        let result = parse_contact(contact).unwrap();
        
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].get("display_name").unwrap(), "Alice");
        assert_eq!(result[0].get("uri").unwrap(), "sip:alice@example.com");
        assert_eq!(result[0].get("expires").unwrap(), "3600");
        
        assert_eq!(result[1].get("uri").unwrap(), "sip:bob@example.com");
        assert_eq!(result[1].get("q").unwrap(), "0.8");
    }
    
    #[test]
    fn test_parse_address() {
        let address = "\"Bob\" <sip:bob@example.com>;tag=a6c85cf";
        let result = parse_address(address).unwrap();
        
        assert_eq!(result.get("display_name").unwrap(), "Bob");
        assert_eq!(result.get("uri").unwrap(), "sip:bob@example.com");
        assert_eq!(result.get("uri_scheme").unwrap(), "sip");
        assert_eq!(result.get("uri_user").unwrap(), "bob");
        assert_eq!(result.get("tag").unwrap(), "a6c85cf");
    }
    
    #[test]
    fn test_parse_auth_params() {
        let auth = "Digest realm=\"example.com\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", algorithm=MD5";
        let result = parse_auth_params(auth).unwrap();
        
        assert_eq!(result.get("scheme").unwrap(), "Digest");
        assert_eq!(result.get("realm").unwrap(), "example.com");
        assert_eq!(result.get("nonce").unwrap(), "dcd98b7102dd2f0e8b11d0f600bfb0c093");
        assert_eq!(result.get("algorithm").unwrap(), "MD5");
    }
    
    #[test]
    fn test_parse_cseq() {
        let cseq = "314159 INVITE";
        let result = parse_cseq(cseq).unwrap();
        
        assert_eq!(result.get("sequence").unwrap(), "314159");
        assert_eq!(result.get("method").unwrap(), "INVITE");
    }
    
    #[test]
    fn test_parse_content_type() {
        let content_type = "application/sdp";
        let result = parse_content_type(content_type).unwrap();
        
        assert_eq!(result.get("media_type").unwrap(), "application");
        assert_eq!(result.get("media_subtype").unwrap(), "sdp");
        
        // With parameters
        let content_type = "multipart/mixed; boundary=boundary1";
        let result = parse_content_type(content_type).unwrap();
        
        assert_eq!(result.get("media_type").unwrap(), "multipart");
        assert_eq!(result.get("media_subtype").unwrap(), "mixed");
        assert_eq!(result.get("boundary").unwrap(), "boundary1");
    }
} 