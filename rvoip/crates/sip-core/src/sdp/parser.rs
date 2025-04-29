use crate::error::{Error, Result};
use crate::types::sdp::{SdpSession, MediaDescription, Origin, ConnectionData, TimeDescription, ParsedAttribute, RtpMapAttribute, FmtpAttribute, CandidateAttribute, SsrcAttribute};
use bytes::Bytes;
use nom::{
    bytes::complete::{tag, take_till1, take_until},
    branch::alt,  // Added alt from branch module
    character::complete::{char, line_ending, not_line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1},
    sequence::{pair, preceded, terminated, tuple},
    IResult,
};
use std::collections::HashMap;
use std::str::{self, FromStr};
use crate::sdp::attributes::{self, MediaDirection}; // Import MediaDirection from attributes
use crate::parser::uri::host; // Import URI parsers
use crate::parser::uri::hostname::hostname; // Import hostname parser specifically
use crate::parser::uri::ipv4::ipv4_address; // Import ipv4 parser specifically  
use crate::parser::uri::ipv6::ipv6_reference; // Import ipv6 parser specifically

/// Parses a single SDP line into a key-value pair.
/// Example: "v=0" -> Ok(("", ('v', "0")))
fn parse_sdp_line(input: &str) -> IResult<&str, (char, &str)> {
    // SDP lines are key=value
    // key is a single character
    // value is the rest of the line until CRLF or LF
    let (input, key) = nom::character::complete::anychar(input)?;
    let (input, _) = terminated(char('='), space0)(input)?;
    
    // Split at the first line ending character, which could be \r, \n, or \r\n
    // This handles mixed line endings properly
    let mut value_end = input.len();
    for (i, c) in input.char_indices() {
        if c == '\r' || c == '\n' {
            value_end = i;
            break;
        }
    }
    
    let value = &input[..value_end];
    let remainder = if value_end < input.len() {
        let next_char = input.chars().nth(value_end).unwrap();
        if next_char == '\r' {
            // Check if we have \r\n
            if value_end + 1 < input.len() && input.chars().nth(value_end + 1) == Some('\n') {
                &input[value_end + 2..]
            } else {
                &input[value_end + 1..]
            }
        } else {
            // Must be \n
            &input[value_end + 1..]
        }
    } else {
        ""
    };
    
    Ok((remainder, (key, value.trim())))
}

fn validate_network_type(net_type: &str) -> Result<()> {
    // According to RFC 8866, only "IN" is defined
    if net_type != "IN" {
        return Err(Error::SdpParsingError(format!("Invalid network type: {}", net_type)));
    }
    Ok(())
}

fn validate_address_type(addr_type: &str) -> Result<()> {
    // According to RFC 8866, only "IP4" and "IP6" are defined
    match addr_type {
        "IP4" | "IP6" => Ok(()),
        _ => Err(Error::SdpParsingError(format!("Invalid address type: {}", addr_type))),
    }
}

fn validate_session_id(session_id: &str) -> Result<u64> {
    match session_id.parse::<u64>() {
        Ok(id) => Ok(id),
        Err(_) => Err(Error::SdpParsingError(format!("Invalid session ID: {}", session_id))),
    }
}

fn validate_session_version(session_version: &str) -> Result<u64> {
    match session_version.parse::<u64>() {
        Ok(ver) => Ok(ver),
        Err(_) => Err(Error::SdpParsingError(format!("Invalid session version: {}", session_version))),
    }
}

fn validate_ipv4_address(address: &str) -> Result<()> {
    // Use parser module's ipv4 validator if possible
    if !is_valid_ipv4(address) {
        return Err(Error::SdpParsingError(format!("Invalid IPv4 address format: {}", address)));
    }
    Ok(())
}

fn validate_ipv6_address(address: &str) -> Result<()> {
    // Use parser module's ipv6 validator if possible
    if !is_valid_ipv6(address) {
        return Err(Error::SdpParsingError(format!("Invalid IPv6 address format: {}", address)));
    }
    
    // Additional checks specific to SDP
    let double_colon_count = address.matches("::").count();
    if double_colon_count > 1 {
        return Err(Error::SdpParsingError(format!("Invalid IPv6 address: multiple double colons in {}", address)));
    }
    
    // Check for valid segments
    for segment in address.split(':') {
        // Skip empty segment (part of double colon)
        if segment.is_empty() {
            continue;
        }
        
        // Each segment must be a valid hex value up to 4 digits
        if segment.len() > 4 {
            return Err(Error::SdpParsingError(format!("Invalid IPv6 segment length: {}", segment)));
        }
        
        // Segment must be valid hexadecimal
        if !segment.chars().all(|c| c.is_digit(16)) {
            return Err(Error::SdpParsingError(format!("Invalid IPv6 segment (not hexadecimal): {}", segment)));
        }
    }
    
    Ok(())
}

fn validate_hostname(hostname: &str) -> Result<()> {
    // Use parser module's hostname validator if possible
    if !is_valid_hostname(hostname) {
        return Err(Error::SdpParsingError(format!("Invalid hostname: {}", hostname)));
    }
    
    // Validate each label
    let labels: Vec<&str> = hostname.split('.').collect();
    
    for label in labels {
        // Labels must not be empty and must be at most 63 characters
        if label.is_empty() || label.len() > 63 {
            return Err(Error::SdpParsingError(format!("Invalid hostname label: {}", label)));
        }
        
        // First character must be alphanumeric
        if !label.chars().next().unwrap().is_alphanumeric() {
            return Err(Error::SdpParsingError(format!("Invalid hostname label: {} (must start with alphanumeric character)", label)));
        }
        
        // All characters must be alphanumeric or hyphens
        // Last character cannot be a hyphen
        let chars: Vec<char> = label.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if !c.is_alphanumeric() && c != '-' {
                return Err(Error::SdpParsingError(format!("Invalid character in hostname label: {}", c)));
            }
            
            if i == chars.len() - 1 && c == '-' {
                return Err(Error::SdpParsingError(format!("Hostname label cannot end with hyphen: {}", label)));
            }
        }
    }
    
    Ok(())
}

// Add new function to parse time description line
pub fn parse_time_description_line(value: &str) -> Result<TimeDescription> {
    // Extract value part if input has t= prefix
    let value_to_parse = if value.starts_with("t=") {
        &value[2..]
    } else {
        value
    };
    
    let parts: Vec<&str> = value_to_parse.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(Error::SdpParsingError(format!("Invalid t= line format: {}", value)));
    }
    
    // Validate start and stop times per RFC 8866
    // t=<start-time> <stop-time>
    // Times are 10-digit NTP timestamps in seconds since 1900, or 0 for indefinite
    
    // Parse start time
    let start_time = match parts[0].parse::<u64>() {
        Ok(val) => val,
        Err(_) => return Err(Error::SdpParsingError(format!("Invalid start time (not numeric): {}", parts[0])))
    };
    
    // Parse stop time
    let stop_time = match parts[1].parse::<u64>() {
        Ok(val) => val,
        Err(_) => return Err(Error::SdpParsingError(format!("Invalid stop time (not numeric): {}", parts[1])))
    };
    
    // Additional validation beyond numeric: check if the stop time is after start time
    // Exception: 0 is special (start: session doesn't start until signaled, stop: session is unbounded)
    if start_time != 0 && stop_time != 0 && stop_time < start_time {
        return Err(Error::SdpParsingError(
            format!("Invalid time description: stop time ({}) is before start time ({})", stop_time, start_time)
        ));
    }
    
    Ok(TimeDescription {
        start_time: start_time.to_string(),
        stop_time: stop_time.to_string(),
    })
}

/// Helper function to validate IP address or hostname per RFC 8866
fn is_valid_address(addr: &str, addr_type: &str) -> bool {
    if addr_type == "IP4" {
        // If it looks like an IPv4 address (has 4 parts separated by dots), 
        // validate it strictly as an IPv4 address
        if addr.split('.').count() == 4 {
            // Check if all parts are valid octets
            let octets: Vec<&str> = addr.split('.').collect();
            let all_octets_valid = octets.iter().all(|octet| {
                if let Ok(num) = octet.parse::<u8>() {
                    true
                } else {
                    false
                }
            });
            
            if all_octets_valid {
                return is_valid_ipv4(addr);
            }
        }
        // Otherwise validate as a hostname
        return is_valid_hostname(addr);
    } else if addr_type == "IP6" {
        // If it contains colons, validate as IPv6
        if addr.contains(':') {
            let addr_to_validate = if addr.starts_with('[') && addr.ends_with(']') {
                &addr[1..addr.len()-1]
            } else {
                addr
            };
            return is_valid_ipv6(addr_to_validate);
        }
        // Otherwise validate as a hostname
        return is_valid_hostname(addr);
    }
    
    false
}

// Improve parse_origin_line to use the new validators
pub fn parse_origin_line(value: &str) -> Result<Origin> {
    // Handle both prefixed and non-prefixed input
    let value_to_parse = if value.starts_with("o=") {
        &value[2..]
    } else {
        value
    };
    
    let parts: Vec<&str> = value_to_parse.split_whitespace().collect();
    if parts.len() != 6 {
        return Err(Error::SdpParsingError(format!("Invalid origin line format: {}", value)));
    }

    let username = parts[0].to_string();
    let sess_id = validate_session_id(parts[1])?;
    let sess_version = validate_session_version(parts[2])?;
    
    validate_network_type(parts[3])?;
    validate_address_type(parts[4])?;

    // Special handling for the specific test case in test_strict_abnf_grammar_validation
    // which expects 10.47.16.256 to be rejected (since 256 is not a valid octet)
    if parts[4] == "IP4" && parts[5].split('.').count() == 4 {
        // If it looks like an IPv4 address, validate each octet strictly
        let octets: Vec<&str> = parts[5].split('.').collect();
        for octet in octets {
            match octet.parse::<u8>() {
                Ok(_) => {}, // Valid octet (0-255)
                Err(_) => return Err(Error::SdpParsingError(format!("Invalid IPv4 address format: {}", parts[5]))),
            }
        }
    }

    // General validation for IP or hostname
    if !is_valid_address(parts[5], parts[4]) {
        return Err(Error::SdpParsingError(format!("Invalid address format: {}", parts[5])));
    }

    Ok(Origin {
        username,
        sess_id: sess_id.to_string(),
        sess_version: sess_version.to_string(),
        net_type: parts[3].to_string(),
        addr_type: parts[4].to_string(),
        unicast_address: parts[5].to_string(),
    })
}

/// Parse SDP connection line (c=)
/// Per RFC 4566 and 8866, the format is:
/// c=<nettype> <addrtype> <connection-address>
pub fn parse_connection_line(line: &str) -> Result<ConnectionData> {
    let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
    if parts.len() < 3 {
        return Err(Error::SdpParsingError(format!("Invalid connection line: {}", line)));
    }

    let net_type = parts[0];
    if net_type != "IN" {
        return Err(Error::SdpParsingError(format!("Unsupported network type: {}", net_type)));
    }

    let addr_type = parts[1];
    if addr_type != "IP4" && addr_type != "IP6" {
        return Err(Error::SdpParsingError(format!("Unsupported address type: {}", addr_type)));
    }

    // Connection address
    let connection_address = parts[2];
    
    // Parse IP address and optional TTL/multicast count for IP4
    // Format: <base-unicast-address>[/<ttl>]/<number of addresses>
    let mut addr_parts = connection_address.split('/');
    let base_addr = addr_parts.next().unwrap(); // This can't fail since there's at least one part
    
    // Keep the original connection address string for later
    let original_connection_address = connection_address.to_string();
    
    // Validate base address based on addr_type
    if addr_type == "IP4" {
        if !is_valid_ipv4(base_addr) && !is_valid_hostname(base_addr) {
            return Err(Error::SdpParsingError(format!("Invalid IPv4 address or hostname: {}", base_addr)));
        }
    } else if addr_type == "IP6" {
        // For IPv6, the address might be within [] brackets
        let addr_to_validate = if base_addr.starts_with('[') && base_addr.ends_with(']') {
            &base_addr[1..base_addr.len()-1]
        } else {
            base_addr
        };
        
        if !is_valid_ipv6(addr_to_validate) && !is_valid_hostname(addr_to_validate) {
            return Err(Error::SdpParsingError(format!("Invalid IPv6 address or hostname: {}", base_addr)));
        }
    }
    
    // Now handle TTL and multicast for IPv4
    let mut ttl = None;
    let mut multicast_count = None;
    
    // Second part could be TTL or multicast count
    if let Some(second) = addr_parts.next() {
        // Parse as u8/u32 and validate
        match second.parse::<u32>() {
            Ok(val) => {
                // For IPv4, this is TTL
                if addr_type == "IP4" {
                    if val > 255 {
                        return Err(Error::SdpParsingError(format!("Invalid TTL (must be <= 255): {}", val)));
                    }
                    ttl = Some(val as u8);
                } else {
                    // For IPv6, this is multicast count
                    multicast_count = Some(val);
                }
            }
            Err(_) => {
                return Err(Error::SdpParsingError(format!("Invalid numeric value in connection address: {}", second)));
            }
        }
    }
    
    // Third part, if present, is always multicast count
    if let Some(third) = addr_parts.next() {
        // This should only happen for IPv4 with both TTL and multicast count
        if addr_type == "IP4" && ttl.is_some() {
            match third.parse::<u32>() {
                Ok(val) => {
                    multicast_count = Some(val);
                }
                Err(_) => {
                    return Err(Error::SdpParsingError(format!("Invalid multicast count: {}", third)));
                }
            }
        } else {
            // Too many parts in the address
            return Err(Error::SdpParsingError(format!("Too many parts in connection address: {}", connection_address)));
        }
    }
    
    // Ensure no more parts remain
    if addr_parts.next().is_some() {
        return Err(Error::SdpParsingError(format!("Invalid connection address format: {}", connection_address)));
    }
    
    // Validate TTL for IPv4 multicast addresses
    if addr_type == "IP4" {
        // Check if this looks like an IPv4 address (not a hostname)
        if base_addr.split('.').count() == 4 {
            let ip_parts: Vec<&str> = base_addr.split('.').collect();
            // Check if first octet is parseable
            if let Ok(first_octet) = ip_parts[0].parse::<u8>() {
                let is_multicast = first_octet >= 224 && first_octet <= 239;
                
                // TTL should be present for multicast addresses
                if is_multicast && ttl.is_none() && multicast_count.is_none() {
                    // RFC says TTL should be provided for IPv4 multicast
                    // But many implementations don't follow this strictly, so we'll just warn
                    log::warn!("IPv4 multicast address without TTL: {}", base_addr);
                }
            }
        }
    }

    Ok(ConnectionData {
        net_type: net_type.to_string(),
        addr_type: addr_type.to_string(),
        // Use the original connection address with all TTL and multicast parts
        connection_address: original_connection_address,
        ttl,
        multicast_count,
    })
}

/// Helper function to validate IPv4 addresses
pub fn is_valid_ipv4(addr: &str) -> bool {
    // Basic format check: must have 4 parts separated by dots
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() != 4 {
        return false;
    }

    // Each part must be a valid octet (0-255)
    for part in parts {
        match part.parse::<u8>() {
            Ok(_) => {}, // Valid octet (0-255)
            Err(_) => return false, // Outside 0-255 range or not a number
        }
    }

    // If we also want to use the parser module validation
    // This is a stronger check that ensures complete RFC compliance
    let input = addr.as_bytes();
    match ipv4_address(input) {
        Ok((remaining, _)) => remaining.is_empty(), // Must consume all input
        Err(_) => false,
    }
}

/// Helper function to validate IPv6 addresses
pub fn is_valid_ipv6(addr: &str) -> bool {
    // Use the parser module's ipv6_reference function
    // Need to add brackets if not already present
    let input = if addr.starts_with('[') {
        addr.as_bytes().to_vec()
    } else {
        let mut with_brackets = Vec::with_capacity(addr.len() + 2);
        with_brackets.push(b'[');
        with_brackets.extend_from_slice(addr.as_bytes());
        with_brackets.push(b']');
        with_brackets
    };
    
    match ipv6_reference(&input) {
        Ok((remaining, _)) => remaining.is_empty(), // Must consume all input
        Err(_) => false,
    }
}

/// Helper function to validate hostnames
pub fn is_valid_hostname(hostname_str: &str) -> bool {
    // Use the hostname parser from hostname.rs
    let input = hostname_str.as_bytes();
    match hostname(input) {
        Ok((remaining, _)) => remaining.is_empty() || remaining == b".", // Must consume all input (allow trailing dot)
        Err(_) => false,
    }
}

/// Parses an r= line for repeat times
fn parse_repeat_time_line(value: &str) -> Result<Vec<String>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(Error::SdpParsingError(format!("Invalid r= line format: {}", value)));
    }
    
    // Simple validation to check if the format conforms to the standard
    // r=<repeat interval> <active duration> <offsets from start-time>
    
    // Validate repeat interval and active duration are time values
    validate_time_field(parts[0], "repeat interval")?;
    validate_time_field(parts[1], "active duration")?;
    
    // Validate that at least one offset is present
    if parts.len() < 3 {
        return Err(Error::SdpParsingError("r= line must have at least one offset".to_string()));
    }
    
    // Validate all offsets
    for (i, offset) in parts[2..].iter().enumerate() {
        validate_time_field(offset, &format!("offset {}", i + 1))?;
    }
    
    // Return all parts as strings for the repeat_times field
    Ok(parts.iter().map(|s| s.to_string()).collect())
}

/// Helper to validate time fields in SDP (used for repeat times)
fn validate_time_field(time_str: &str, field_name: &str) -> Result<()> {
    // Time values can include unit suffixes: d (days), h (hours), m (minutes), s (seconds)
    // Format is a number followed by an optional unit
    
    let mut numeric_part = String::new();
    let mut unit_part = String::new();
    
    // Split into number and unit
    for c in time_str.chars() {
        if c.is_ascii_digit() {
            numeric_part.push(c);
        } else {
            unit_part.push(c);
        }
    }
    
    // Ensure numeric part is valid
    if numeric_part.is_empty() {
        return Err(Error::SdpParsingError(
            format!("Invalid {} time value '{}': missing numeric part", field_name, time_str)
        ));
    }
    
    let _num = numeric_part.parse::<u64>()
        .map_err(|_| Error::SdpParsingError(
            format!("Invalid {} time value '{}': numeric part not a valid integer", field_name, time_str)
        ))?;
    
    // If unit part exists, validate it
    if !unit_part.is_empty() {
        match unit_part.as_str() {
            "d" | "h" | "m" | "s" => (), // Valid units
            _ => return Err(Error::SdpParsingError(
                format!("Invalid {} time unit '{}': must be d, h, m, or s", field_name, unit_part)
            )),
        }
    }
    
    Ok(())
}

/// Parses the entire SDP content from bytes into an SdpSession struct.
pub fn parse_sdp(content: &Bytes) -> Result<SdpSession> {
    // Convert bytes to str - SDP is text based
    let sdp_str = str::from_utf8(content)
        .map_err(|e| Error::SdpParsingError(format!("Invalid UTF-8 in SDP: {}", e)))?;

    // Pre-process the string to remove comment lines
    let filtered_sdp = sdp_str.lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with('#') || trimmed.starts_with("//"))
        })
        .collect::<Vec<&str>>()
        .join("\r\n");

    // Parse all lines into key-value pairs
    let parse_result = many1(parse_sdp_line)(&filtered_sdp);

    match parse_result {
        Ok((remaining_input, lines)) => {
            if !remaining_input.trim().is_empty() {
                // Return error for trailing data, consistent with other parsers
                return Err(Error::SdpParsingError(format!("Trailing data after parsing lines: {:?}", remaining_input)));
            }

            // Check if first line is 'v=' (required by RFC 4566)
            if lines.is_empty() || lines[0].0 != 'v' {
                return Err(Error::SdpParsingError("SDP must start with a v= line".to_string()));
            }

            // Track seen mandatory fields to detect duplicates and ensure all are present
            let mut seen_v = false;
            let mut seen_o = false;
            let mut seen_s = false;
            let mut seen_t = false;

            // Need temporary Option fields for mandatory o, s, t during build
            let mut temp_origin: Option<Origin> = None;
            let mut temp_s_line: Option<String> = None;
            let mut temp_t_lines: Vec<TimeDescription> = Vec::new();
            
            let mut session = SdpSession {
                version: "".to_string(),
                origin: Origin { username: "-".into(), sess_id: "0".into(), sess_version: "0".into(), net_type: "IN".into(), addr_type: "IP4".into(), unicast_address: "0.0.0.0".into() }, // Temp default
                session_name: "".to_string(), // Temp default
                connection_info: None, 
                time_descriptions: Vec::new(), // Temp default
                media_descriptions: Vec::new(),
                direction: None,
                generic_attributes: Vec::new(),
            };

            let mut current_media: Option<MediaDescription> = None;

            // Add state for order checking
            #[derive(PartialEq, PartialOrd)]
            enum SdpParseSection { SessionHeader, MediaDescription }
            let mut current_section = SdpParseSection::SessionHeader;

            // Keep track of media-level connection info to enforce having it at session
            // level or in all media descriptions
            let mut all_media_have_connection = true;

            for (key, value) in lines {
                // Enforce basic order: session headers before media descriptions
                if key == 'm' && current_section < SdpParseSection::MediaDescription {
                    current_section = SdpParseSection::MediaDescription;
                } else if key != 'm' && current_section == SdpParseSection::MediaDescription && !matches!(key, 'a' | 'c' | 'b' | 'k' | 'i') {
                    // Allow only specific keys after m= line starts media section (a=, c=, b=, k=, i= according to RFC 4566)
                    if matches!(key, 'v' | 'o' | 's' | 't' | 'p' | 'u' | 'e' | 'r' | 'z') {
                         return Err(Error::SdpParsingError(format!("Invalid SDP order: '{}=' line found after 'm=' line", key)));
                    }
                }
                
                match key {
                    'v' => {
                        // Check for duplicate v= line
                        if seen_v {
                            return Err(Error::SdpParsingError("Duplicate v= line".to_string()));
                        }
                        seen_v = true;
                        
                        if value != "0" { return Err(Error::SdpParsingError("Unsupported SDP version".to_string())); }
                        session.version = value.to_string();
                    }
                    'o' => {
                        // Check for duplicate o= line
                        if seen_o {
                            return Err(Error::SdpParsingError("Duplicate o= line".to_string()));
                        }
                        seen_o = true;
                        
                        temp_origin = Some(parse_origin_line(value)?);
                    }
                    's' => {
                        // Check for duplicate s= line
                        if seen_s {
                            return Err(Error::SdpParsingError("Duplicate s= line".to_string()));
                        }
                        seen_s = true;
                        
                        if value.is_empty() { return Err(Error::SdpParsingError("Empty s= line".to_string())); } 
                        temp_s_line = Some(value.to_string());
                    }
                    'i' => { // Session Information
                         if current_media.is_none() {
                            session.generic_attributes.push(ParsedAttribute::Value("i".to_string(), value.to_string()));
                         } else {
                            // i= line is allowed at media level according to RFC 4566 section 5.4
                            // Store it in the media's generic attributes
                            current_media.as_mut().unwrap().generic_attributes.push(
                                ParsedAttribute::Value("i".to_string(), value.to_string())
                            );
                         }
                    }
                    'u' => { // URI
                         if current_media.is_none() {
                            session.generic_attributes.push(ParsedAttribute::Value("u".to_string(), value.to_string()));
                         } else {
                            return Err(Error::SdpParsingError("u= line found at media level (invalid)".to_string()));
                         }
                    }
                    'e' => { // Email
                         if current_media.is_none() {
                            session.generic_attributes.push(ParsedAttribute::Value("e".to_string(), value.to_string()));
                         } else {
                            return Err(Error::SdpParsingError("e= line found at media level (invalid)".to_string()));
                         }
                    }
                    'p' => { // Phone
                         if current_media.is_none() {
                            session.generic_attributes.push(ParsedAttribute::Value("p".to_string(), value.to_string()));
                         } else {
                            return Err(Error::SdpParsingError("p= line found at media level (invalid)".to_string()));
                         }
                    }
                    'c' => { 
                        let conn_data = parse_connection_line(value)?;
                        if let Some(media) = current_media.as_mut() {
                           if media.connection_info.is_some() { 
                               return Err(Error::SdpParsingError("Duplicate c= line for media".to_string())); 
                           }
                           media.connection_info = Some(conn_data);
                        } else {
                            if session.connection_info.is_some() { 
                                return Err(Error::SdpParsingError("Duplicate session-level c= line".to_string())); 
                            }
                            session.connection_info = Some(conn_data);
                        }
                    }
                    't' => { 
                        // Check for t= in media section (invalid)
                        if current_section == SdpParseSection::MediaDescription {
                            return Err(Error::SdpParsingError("Invalid SDP order: 't=' line found after 'm=' line".to_string()));
                        }
                        
                        // Record that we've seen at least one t= line
                        seen_t = true;
                        
                        // Parse t= line and add it to time descriptions
                        let time_desc = parse_time_description_line(value)?;
                        temp_t_lines.push(time_desc);
                    }
                    'r' => {
                        // r= lines must follow a t= line
                        if temp_t_lines.is_empty() {
                            return Err(Error::SdpParsingError("r= line without preceding t= line".to_string()));
                        }
                        
                        // Store r= lines as generic attributes
                        session.generic_attributes.push(ParsedAttribute::Value("r".to_string(), value.to_string()));
                    }
                    'a' => { // Attribute
                         let parsed_attr = parse_attribute(value)?;
                         if let Some(media) = current_media.as_mut() {
                             // Store in media description
                             match parsed_attr {
                                 ParsedAttribute::Ptime(v) => {
                                     if media.ptime.is_some() { 
                                        return Err(Error::SdpParsingError(format!("Duplicate ptime attribute for media {}", media.media)));
                                     }
                                     media.ptime = Some(v as u32);
                                 }
                                 ParsedAttribute::Direction(d) => {
                                      if media.direction.is_some() {
                                        return Err(Error::SdpParsingError(format!("Duplicate direction attribute for media {}", media.media)));
                                      }
                                     media.direction = Some(d);
                                 }
                                 // Other attribute types go into the generic vec
                                 _ => media.generic_attributes.push(parsed_attr),
                             }
                         } else {
                            // Store in session description
                             match parsed_attr {
                                 ParsedAttribute::Direction(d) => {
                                     if session.direction.is_some() {
                                        return Err(Error::SdpParsingError("Duplicate session-level direction attribute".to_string()));
                                     }
                                     session.direction = Some(d);
                                 }
                                  // Ptime is typically media-level, but treat as error when found at session level
                                 ParsedAttribute::Ptime(_) => {
                                     return Err(Error::SdpParsingError("ptime attribute found at session level (should be media level)".to_string()));
                                 }
                                 // Other attribute types go into the generic vec
                                 _ => session.generic_attributes.push(parsed_attr),
                             }
                         }
                    }
                    'm' => { // Media Description
                        // Set section state
                        current_section = SdpParseSection::MediaDescription;
                        // Add previous media description if exists
                        if let Some(media) = current_media.take() {
                            // Check if this media has connection info
                            if media.connection_info.is_none() {
                                all_media_have_connection = false;
                            }
                            session.media_descriptions.push(media);
                        }
                        current_media = Some(parse_media_description_line(value)?);
                    }
                    'b' => { // Bandwidth
                        // Parse the bandwidth line and add it as a dedicated attribute
                        let (bwtype, bandwidth) = parse_bandwidth(value)?;
                        if let Some(media) = current_media.as_mut() {
                            // Media-level bandwidth
                            media.generic_attributes.push(ParsedAttribute::Bandwidth(bwtype, bandwidth));
                        } else {
                            // Session-level bandwidth
                            session.generic_attributes.push(ParsedAttribute::Bandwidth(bwtype, bandwidth));
                        }
                    }
                    'z' | 'k' => { 
                        // Store as generic attributes
                        session.generic_attributes.push(ParsedAttribute::Value(key.to_string(), value.to_string()));
                    }
                    _ => { 
                        return Err(Error::SdpParsingError(format!("Unknown line type: '{}'", key)));
                    }
                }
            }

            // Add the last media description if it exists
            if let Some(media) = current_media.take() {
                // Check if this media has connection info
                if media.connection_info.is_none() {
                    all_media_have_connection = false;
                }
                session.media_descriptions.push(media);
            }

            // Verify all mandatory fields are present
            if !seen_v {
                return Err(Error::SdpParsingError("Missing mandatory v= field".to_string()));
            }
            if !seen_o {
                return Err(Error::SdpParsingError("Missing mandatory o= field".to_string()));
            }
            if !seen_s {
                return Err(Error::SdpParsingError("Missing mandatory s= field".to_string()));
            }
            if !seen_t {
                return Err(Error::SdpParsingError("Missing mandatory t= field".to_string()));
            }

            // Assign mandatory fields from temps
            session.origin = temp_origin.ok_or_else(|| Error::SdpParsingError("Missing mandatory o= field".to_string()))?;
            session.session_name = temp_s_line.ok_or_else(|| Error::SdpParsingError("Missing mandatory s= field".to_string()))?;
            if temp_t_lines.is_empty() {
                 return Err(Error::SdpParsingError("Missing mandatory t= field".to_string()));
            }
            session.time_descriptions = temp_t_lines;
            
            // Final validation (connection info)
            // A c= line MUST be present either at session level OR in ALL media descriptions
            let session_c_present = session.connection_info.is_some();
            
            if !session_c_present && !all_media_have_connection && !session.media_descriptions.is_empty() {
                 return Err(Error::SdpParsingError("Missing mandatory c= field (must be session level or in all media)".to_string()));
            }

            Ok(session)
        }
        Err(e) => Err(Error::SdpParsingError(format!("Failed parsing SDP lines: {:?}", e))),
    }
}

/// Parses an attribute line (a=key:value or a=key) into a ParsedAttribute enum variant.
pub fn parse_attribute(line: &str) -> Result<ParsedAttribute> {
    // Format is "a=<attribute>:<value>" or "a=<flag>"
    // Get the actual attribute part (strip "a=" prefix if present)
    let line_to_parse = if line.starts_with("a=") {
        &line[2..]
    } else {
        // If "a=" is not present, assume the line is already the attribute part
        line
    };

    let (attribute, value) = match line_to_parse.split_once(':') {
        Some((name, value)) => (name, Some(value)),
        None => (line_to_parse, None),
    };

    match attribute {
        "rtpmap" => {
            attributes::parse_rtpmap(value.unwrap_or_default())
        }
        "fmtp" => {
            attributes::parse_fmtp(value.unwrap_or_default())
        }
        "ptime" => {
            let ptime = attributes::parse_ptime(value.unwrap_or_default())?;
            Ok(ParsedAttribute::Ptime(ptime as u64))
        }
        "maxptime" => {
            let maxptime = attributes::parse_maxptime(value.unwrap_or_default())?;
            Ok(ParsedAttribute::MaxPtime(maxptime as u64))
        }
        "candidate" => {
            attributes::parse_candidate(value.unwrap_or_default())
        }
        "ssrc" => {
            attributes::parse_ssrc(value.unwrap_or_default())
        }
        "ice-ufrag" => {
            let ufrag = attributes::parse_ice_ufrag(value.unwrap_or_default())?;
            Ok(ParsedAttribute::IceUfrag(ufrag))
        }
        "ice-pwd" => {
            let pwd = attributes::parse_ice_pwd(value.unwrap_or_default())?;
            Ok(ParsedAttribute::IcePwd(pwd))
        }
        "fingerprint" => {
            let (hash, fingerprint) = attributes::parse_fingerprint(value.unwrap_or_default())?;
            Ok(ParsedAttribute::Fingerprint(hash, fingerprint))
        }
        "setup" => {
            let role = attributes::parse_setup(value.unwrap_or_default())?;
            Ok(ParsedAttribute::Setup(role))
        }
        "mid" => {
            let id = attributes::parse_mid(value.unwrap_or_default())?;
            Ok(ParsedAttribute::Mid(id))
        }
        "group" => {
            let (semantics, ids) = attributes::parse_group(value.unwrap_or_default())?;
            Ok(ParsedAttribute::Group(semantics, ids))
        }
        "rtcp-fb" => {
            let (pt, feedback_type, param) = attributes::parse_rtcp_fb(value.unwrap_or_default())?;
            Ok(ParsedAttribute::RtcpFb(pt, feedback_type, param))
        }
        "extmap" => {
            let (id, direction, uri, params) = attributes::parse_extmap(value.unwrap_or_default())?;
            // Convert u16 to u8 safely
            let id_u8 = u8::try_from(id).map_err(|_| Error::SdpParsingError(format!("ExtMap id too large: {}", id)))?;
            Ok(ParsedAttribute::ExtMap(id_u8, direction, uri, params))
        }
        "msid" => {
            let (stream_id, track_id) = attributes::parse_msid(value.unwrap_or_default())?;
            Ok(ParsedAttribute::Msid(stream_id, track_id))
        }
        "rid" => {
            let (id, direction, restrictions) = attributes::parse_rid(value.unwrap_or_default())?;
            Ok(ParsedAttribute::Rid(id, direction, restrictions))
        }
        "simulcast" => {
            let (send, recv) = attributes::parse_simulcast(value.unwrap_or_default())?;
            Ok(ParsedAttribute::Simulcast(send, recv))
        }
        "ice-options" => {
            let options = attributes::parse_ice_options(value.unwrap_or_default())?;
            Ok(ParsedAttribute::IceOptions(options))
        }
        "end-of-candidates" => Ok(ParsedAttribute::EndOfCandidates),
        "sctp-port" => {
            let port = attributes::parse_sctp_port(value.unwrap_or_default())?;
            Ok(ParsedAttribute::SctpPort(port))
        }
        "max-message-size" => {
            let size = attributes::parse_max_message_size(value.unwrap_or_default())?;
            Ok(ParsedAttribute::MaxMessageSize(size))
        }
        "sctpmap" => {
            let (number, app, streams) = attributes::parse_sctpmap(value.unwrap_or_default())?;
            // Convert u32 to u16 safely
            let streams_u16 = u16::try_from(streams).map_err(|_| Error::SdpParsingError(format!("SctpMap streams too large: {}", streams)))?;
            Ok(ParsedAttribute::SctpMap(number, app, streams_u16))
        }
        "sendrecv" => Ok(ParsedAttribute::Direction(MediaDirection::SendRecv)),
        "sendonly" => Ok(ParsedAttribute::Direction(MediaDirection::SendOnly)),
        "recvonly" => Ok(ParsedAttribute::Direction(MediaDirection::RecvOnly)),
        "inactive" => Ok(ParsedAttribute::Direction(MediaDirection::Inactive)),
        "rtcp-mux" => Ok(ParsedAttribute::RtcpMux),
        _ => {
            if let Some(val) = value {
                Ok(ParsedAttribute::Other(attribute.to_string(), Some(val.to_string())))
            } else {
                Ok(ParsedAttribute::Other(attribute.to_string(), None))
            }
        }
    }
}

/// Parses the media description line (m=...)
fn parse_media_description_line(value: &str) -> Result<MediaDescription> {
     // Format: m=<media> <port>[/<num_ports>] <proto> <fmt> ...
     let parts: Vec<&str> = value.split_whitespace().collect();
     if parts.len() < 3 {
         return Err(Error::SdpParsingError(format!("Invalid m= line format: {}", value)));
     }

     // Media type must be one of audio, video, text, application, message, or a non-standard token
     let media = parts[0].to_string();
     let valid_media_types = ["audio", "video", "text", "application", "message"];
     if !valid_media_types.contains(&media.as_str()) && !is_valid_token(&media) {
         return Err(Error::SdpParsingError(format!("Invalid media type: {}", media)));
     }
     
     // Port and optional port count
     let port_part = parts[1];
     let (port, _num_ports) = if port_part.contains('/') {
         let port_parts: Vec<&str> = port_part.split('/').collect();
         if port_parts.len() != 2 {
             return Err(Error::SdpParsingError(format!("Invalid port/num_ports format: {}", port_part)));
         }
         
         let base_port = port_parts[0].parse::<u16>()
             .map_err(|_| Error::SdpParsingError(format!("Invalid port in m= line: {}", port_parts[0])))?;
         
         let num_ports = port_parts[1].parse::<u16>()
             .map_err(|_| Error::SdpParsingError(format!("Invalid num_ports in m= line: {}", port_parts[1])))?;
         
         // Spec says num_ports should be positive
         if num_ports == 0 {
             return Err(Error::SdpParsingError("num_ports cannot be zero".to_string()));
         }
         
         (base_port, Some(num_ports))
     } else {
         let port = port_part.parse::<u16>()
             .map_err(|_| Error::SdpParsingError(format!("Invalid port in m= line: {}", port_part)))?;
         (port, None)
     };
     
     // Protocol must be a valid token or registered protocol
     let protocol = parts[2].to_string();
     if !is_valid_token(&protocol) {
         return Err(Error::SdpParsingError(format!("Invalid protocol: {}", protocol)));
     }
     
     // Handle formats, which are optional (RFC 8866 allows empty format list)
     let formats: Vec<String> = if parts.len() > 3 {
         // Validate each format is a valid token
         for fmt in &parts[3..] {
             if !is_valid_token(fmt) {
                 return Err(Error::SdpParsingError(format!("Invalid format value: {}", fmt)));
             }
         }
         parts[3..].iter().map(|s| s.to_string()).collect()
     } else {
         Vec::new()
     };

     Ok(MediaDescription {
         media,
         port,
         protocol,
         formats,
         connection_info: None, // Will be filled if c= line appears after m=
         ptime: None, // Initialize new field
         direction: None, // Initialize new field
         generic_attributes: Vec::new(), // Initialize new Vec
     })
}

/// Helper function to validate token format (per RFC 4566 ABNF)
fn is_valid_token(s: &str) -> bool {
    // Validate common predefined protocols without parsing
    if s == "RTP/AVP" || s == "RTP/SAVP" || s == "UDP/TLS/RTP/SAVPF" ||
       s == "UDP/DTLS/SCTP" || s == "webrtc-datachannel" {
        return true;
    }
    
    // Standard token validation per RFC 4566
    !s.is_empty() && s.chars().all(|c| 
        c.is_ascii_alphanumeric() || 
        c == '-' || c == '.' || c == '!' || 
        c == '%' || c == '*' || c == '_' || 
        c == '+' || c == '`' || c == '\'' || 
        c == '~' || c == '/'  // Add slash for compound protocol names
    )
}

// Fix Bandwidth type conversion
pub fn parse_bandwidth(line: &str) -> Result<(String, u64)> {
    // Example: b=AS:128
    let parts: Vec<&str> = line.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(Error::SdpParsingError(format!("Invalid bandwidth line format: {}", line)));
    }
    
    let bwtype = parts[0].to_string();
    let bandwidth = parts[1].parse::<u64>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid bandwidth value: {}", parts[1])))?;
    
    Ok((bwtype, bandwidth))
}