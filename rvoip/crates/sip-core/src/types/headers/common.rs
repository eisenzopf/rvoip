// Common header functionality shared across header types

use crate::error::{Error, Result};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait};
use crate::types::param::Param;

use std::str::FromStr;

/// Utility function to extract string field from a Header value
pub fn header_value_to_string(header: &Header) -> Result<String> {
    match &header.value {
        HeaderValue::Raw(bytes) => {
            String::from_utf8(bytes.clone())
                .map_err(|e| Error::ParseError(format!("Invalid UTF-8 in header value: {}", e)))
        },
        _ => Err(Error::ParseError("Expected raw header value".to_string()))
    }
}

/// Utility function to extract string list from a Header value (comma-separated)
pub fn header_value_to_string_list(header: &Header) -> Result<Vec<String>> {
    let value_str = header_value_to_string(header)?;
    let parts = value_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(parts)
}

/// Utility function to parse parameters from a Header value
pub fn parse_header_params(value_str: &str) -> Result<Vec<Param>> {
    let params = value_str
        .split(';')
        .skip(1) // Skip the main value part
        .filter_map(|param_str| {
            let param_str = param_str.trim();
            if param_str.is_empty() {
                return None;
            }
            
            let parts: Vec<&str> = param_str.splitn(2, '=').collect();
            let name = parts[0].trim();
            let value = if parts.len() > 1 {
                Some(parts[1].trim().to_string())
            } else {
                None
            };
            
            Some(Param::new(name.to_string(), value))
        })
        .collect();
    
    Ok(params)
} 