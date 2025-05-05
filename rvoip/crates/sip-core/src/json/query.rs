//! # Query-based Access to SIP Values
//! 
//! This module provides a simplified JSONPath-like query system for extracting 
//! and searching for values within SIP message structures.
//!
//! ## Query Syntax
//!
//! The query system supports a subset of JSONPath syntax:
//!
//! - `$` - Root element
//! - `.` - Child operator
//! - `..` - Recursive descent (search at any depth)
//! - `*` - Wildcard
//! - `[n]` - Array index
//! - `[start:end]` - Array slice
//! - `[?(@.property == value)]` - Filter expression
//!
//! ## Use Cases
//!
//! The query interface is particularly useful for:
//!
//! - Finding all instances of a field regardless of location (e.g., all tags or branches)
//! - Exploring message structures when you don't know the exact path
//! - Extracting collections of related values
//! - Pattern matching across the message structure
//!
//! ## Examples
//!
//! ```
//! # use rvoip_sip_core::prelude::*;
//! # use rvoip_sip_core::json::SipJsonExt;
//! # fn example() -> Option<()> {
//! let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@example.com", Some("1928301774"))
//!     .to("Bob", "sip:bob@example.com", Some("4567"))
//!     .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
//!     .via("proxy.atlanta.com", "TCP", Some("z9hG4bK887jhd"))
//!     .build();
//!
//! // Find all display names (in From, To headers)
//! let display_names = request.query("$..display_name");
//! 
//! // Find all branch parameters (in Via headers)
//! let branches = request.query("$..Branch");
//!
//! // Find all tag parameters
//! let tags = request.query("$..Tag");
//! # Some(())
//! # }
//! ```

use crate::json::value::SipValue;
use crate::json::{SipJsonResult, SipJsonError};
use std::collections::HashSet;

/// Query a SipValue using a simplified JSONPath-like syntax.
///
/// This function allows for powerful searches through SIP message structures,
/// finding values that match specific patterns or criteria.
///
/// # Supported Syntax
///
/// - `$` - Root element
/// - `.` - Child operator
/// - `..` - Recursive descent (search at any depth)
/// - `*` - Wildcard
/// - `[n]` - Array index (zero-based)
/// - `[start:end]` - Array slice
/// - `[?(@.property == value)]` - Filter expression (basic)
///
/// # Parameters
///
/// - `value`: The SipValue to query
/// - `query_str`: The query string in JSONPath-like syntax
///
/// # Returns
///
/// A vector of references to SipValue objects that match the query
///
/// # Examples
///
/// Basic direct field access:
///
/// ```
/// # use rvoip_sip_core::json::{SipValue, query};
/// # use std::collections::HashMap;
/// # fn example() {
/// // Create a SipValue with a "method" field
/// let mut obj = HashMap::new();
/// obj.insert("method".to_string(), SipValue::String("INVITE".to_string()));
/// let message = SipValue::Object(obj);
///
/// // Query for the method field
/// let results = query::query(&message, "$.method");
/// assert_eq!(results.len(), 1);
/// assert_eq!(results[0].as_str(), Some("INVITE"));
/// # }
/// # example();
/// ```
///
/// Recursive descent to find all instances of a field:
///
/// ```
/// # use rvoip_sip_core::json::{SipValue, query};
/// # use std::collections::HashMap;
/// # fn example() {
/// // Create a nested structure with multiple display_name fields
/// let mut from = HashMap::new();
/// from.insert("display_name".to_string(), SipValue::String("Alice".to_string()));
///
/// let mut to = HashMap::new();
/// to.insert("display_name".to_string(), SipValue::String("Bob".to_string()));
///
/// let mut headers = HashMap::new();
/// headers.insert("From".to_string(), SipValue::Object(from));
/// headers.insert("To".to_string(), SipValue::Object(to));
///
/// let mut message = HashMap::new();
/// message.insert("headers".to_string(), SipValue::Object(headers));
///
/// let sip_msg = SipValue::Object(message);
///
/// // Find all display_name fields anywhere in the structure
/// let names = query::query(&sip_msg, "$..display_name");
/// assert_eq!(names.len(), 2);
/// assert!(names.iter().any(|v| v.as_str() == Some("Alice")));
/// assert!(names.iter().any(|v| v.as_str() == Some("Bob")));
/// # }
/// # example();
/// ```
///
/// Using array indices and wildcards:
///
/// ```
/// # use rvoip_sip_core::json::{SipValue, query};
/// # use std::collections::HashMap;
/// # fn example() {
/// // Create a structure with an array of Via headers
/// let mut via1 = HashMap::new();
/// via1.insert("branch".to_string(), SipValue::String("z9hG4bK776asdhds".to_string()));
/// via1.insert("transport".to_string(), SipValue::String("UDP".to_string()));
///
/// let mut via2 = HashMap::new();
/// via2.insert("branch".to_string(), SipValue::String("z9hG4bK887jhd".to_string()));
/// via2.insert("transport".to_string(), SipValue::String("TCP".to_string()));
///
/// let via_array = vec![SipValue::Object(via1), SipValue::Object(via2)];
///
/// let mut headers = HashMap::new();
/// headers.insert("Via".to_string(), SipValue::Array(via_array));
///
/// let message = SipValue::Object(headers);
///
/// // Get the first Via header's branch
/// let first_branch = query::query(&message, "$.Via[0].branch");
/// assert_eq!(first_branch[0].as_str(), Some("z9hG4bK776asdhds"));
///
/// // Get all Via branches using wildcard
/// let all_branches = query::query(&message, "$.Via[*].branch");
/// assert_eq!(all_branches.len(), 2);
///
/// // Get all branches using recursive descent (alternative approach)
/// let all_branches2 = query::query(&message, "$..branch");
/// assert_eq!(all_branches2.len(), 2);
/// # }
/// # example();
/// ```
///
/// Using filters:
///
/// ```
/// # use rvoip_sip_core::json::{SipValue, query};
/// # use std::collections::HashMap;
/// # fn example() {
/// // Create an array of header objects
/// let mut header1 = HashMap::new();
/// header1.insert("name".to_string(), SipValue::String("Via".to_string()));
/// header1.insert("transport".to_string(), SipValue::String("UDP".to_string()));
///
/// let mut header2 = HashMap::new();
/// header2.insert("name".to_string(), SipValue::String("Via".to_string()));
/// header2.insert("transport".to_string(), SipValue::String("TCP".to_string()));
///
/// let mut header3 = HashMap::new();
/// header3.insert("name".to_string(), SipValue::String("From".to_string()));
///
/// let headers = vec![
///    SipValue::Object(header1), 
///    SipValue::Object(header2),
///    SipValue::Object(header3)
/// ];
///
/// let message = SipValue::Array(headers);
///
/// // Find all Via headers
/// let via_headers = query::query(&message, "$[?(@.name == \"Via\")]");
/// assert_eq!(via_headers.len(), 2);
///
/// // Find UDP Via headers
/// let udp_headers = query::query(&message, "$[?(@.transport == \"UDP\")]");
/// assert_eq!(udp_headers.len(), 1);
/// # }
/// # example();
/// ```
pub fn query<'a>(value: &'a SipValue, query_str: &str) -> Vec<&'a SipValue> {
    if query_str.is_empty() {
        return vec![];
    }

    let query = parse_query(query_str);
    execute_query(value, &query)
}

/// Execute a parsed query against a value
fn execute_query<'a>(value: &'a SipValue, query: &[QueryPart]) -> Vec<&'a SipValue> {
    if query.is_empty() {
        return vec![value];
    }

    let mut results = Vec::new();
    let (head, tail) = (&query[0], &query[1..]);

    match head {
        QueryPart::Root => {
            // Continue with the rest of the query
            results.extend(execute_query(value, tail));
        }
        QueryPart::Child(name) => {
            if let Some(obj) = value.as_object() {
                if name == "*" {
                    // Wildcard - match all children
                    for child in obj.values() {
                        results.extend(execute_query(child, tail));
                    }
                } else if let Some(child) = obj.get(name) {
                    results.extend(execute_query(child, tail));
                }
            }
        }
        QueryPart::RecursiveDescent(name) => {
            // First check direct children
            if let Some(obj) = value.as_object() {
                if name == "*" {
                    // Wildcard - match all children
                    for child in obj.values() {
                        results.push(child);
                        // Continue recursion
                        results.extend(execute_recursive_descent(child, name, tail));
                    }
                } else if let Some(child) = obj.get(name) {
                    results.push(child);
                    results.extend(execute_query(child, tail));
                }
            }

            // Then recursively check all descendants
            results.extend(execute_recursive_descent(value, name, tail));
        }
        QueryPart::ArrayIndex(idx) => {
            if let Some(arr) = value.as_array() {
                let index = if *idx < 0 {
                    // Handle negative indices (counting from the end)
                    arr.len().checked_sub(idx.abs() as usize)
                } else {
                    Some(*idx as usize)
                };

                if let Some(i) = index {
                    if i < arr.len() {
                        results.extend(execute_query(&arr[i], tail));
                    }
                }
            }
        }
        QueryPart::ArrayWildcard => {
            if let Some(arr) = value.as_array() {
                for item in arr {
                    results.extend(execute_query(item, tail));
                }
            }
        }
        QueryPart::ArraySlice(start, end) => {
            if let Some(arr) = value.as_array() {
                let start_idx = if *start >= 0 {
                    *start as usize
                } else {
                    arr.len().saturating_sub(start.abs() as usize)
                };
                
                let end_idx = if *end >= 0 {
                    (*end as usize).min(arr.len())
                } else {
                    arr.len().saturating_sub(end.abs() as usize)
                };
                
                if start_idx < arr.len() && start_idx < end_idx {
                    for i in start_idx..end_idx {
                        results.extend(execute_query(&arr[i], tail));
                    }
                }
            }
        }
        QueryPart::Filter(filter) => {
            // Basic filtering support
            match value {
                SipValue::Array(arr) => {
                    for item in arr {
                        if evaluate_filter(item, filter) {
                            results.extend(execute_query(item, tail));
                        }
                    }
                }
                SipValue::Object(_) => {
                    if evaluate_filter(value, filter) {
                        results.extend(execute_query(value, tail));
                    }
                }
                _ => {}
            }
        }
    }

    results
}

/// Helper function for recursive descent
fn execute_recursive_descent<'a>(
    value: &'a SipValue,
    name: &str,
    remaining_query: &[QueryPart],
) -> Vec<&'a SipValue> {
    let mut results = Vec::new();
    
    match value {
        SipValue::Object(obj) => {
            // Check all children recursively
            for (key, child) in obj {
                // If this child matches the name, add its results
                if name == "*" || key == name {
                    results.extend(execute_query(child, remaining_query));
                }
                
                // Continue recursion for all children
                results.extend(execute_recursive_descent(child, name, remaining_query));
            }
        }
        SipValue::Array(arr) => {
            // Check all array elements recursively
            for item in arr {
                results.extend(execute_recursive_descent(item, name, remaining_query));
            }
        }
        _ => {}
    }
    
    results
}

/// Evaluate a filter against a value
fn evaluate_filter(value: &SipValue, filter: &FilterExpression) -> bool {
    match filter {
        FilterExpression::Equals(path, expected) => {
            // Navigate to the path
            if let Some(actual) = crate::json::path::get_path(value, path) {
                match (actual, expected) {
                    (SipValue::Null, SipValue::Null) => true,
                    (SipValue::Bool(a), SipValue::Bool(b)) => a == b,
                    (SipValue::Number(a), SipValue::Number(b)) => (a - b).abs() < f64::EPSILON,
                    (SipValue::String(a), SipValue::String(b)) => a == b,
                    _ => false,
                }
            } else {
                false
            }
        }
        FilterExpression::NotEquals(path, expected) => {
            !evaluate_filter(value, &FilterExpression::Equals(path.clone(), expected.clone()))
        }
        FilterExpression::Contains(path, substring) => {
            if let Some(actual) = crate::json::path::get_path(value, path) {
                if let SipValue::String(s) = actual {
                    if let SipValue::String(substr) = substring {
                        s.contains(substr)
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        }
        FilterExpression::Exists(path) => {
            crate::json::path::get_path(value, path).is_some()
        }
    }
}

/// Parse a query string into parts
fn parse_query(query: &str) -> Vec<QueryPart> {
    let mut parts = Vec::new();
    let mut chars = query.chars().peekable();
    
    // Add root if query starts with $
    if chars.peek() == Some(&'$') {
        parts.push(QueryPart::Root);
        chars.next();
    }
    
    let mut in_brackets = false;
    let mut current = String::new();
    
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                if !current.is_empty() {
                    parts.push(QueryPart::Child(current));
                    current = String::new();
                }
                
                // Check for recursive descent (..)
                if chars.peek() == Some(&'.') {
                    chars.next(); // Consume the second dot
                    
                    // For recursive descent, collect the entire field name after the double dots
                    let mut field_name = String::new();
                    
                    // Read until next . or [ or end of string
                    while let Some(&next_char) = chars.peek() {
                        if next_char == '.' || next_char == '[' {
                            break;
                        }
                        field_name.push(chars.next().unwrap());
                    }
                    
                    if !field_name.is_empty() {
                        parts.push(QueryPart::RecursiveDescent(field_name));
                    } else {
                        // If nothing follows .., prepare for a field name in the next iteration
                        // e.g., $..field would be [Root, RecursiveDescent("field")]
                        current = String::new();
                        if let Some(&next_char) = chars.peek() {
                            if next_char == '[' {
                                in_brackets = true;
                                chars.next(); // Consume the [
                                // Handle bracket parsing separately
                            }
                        }
                    }
                }
            }
            '[' => {
                if !current.is_empty() {
                    parts.push(QueryPart::Child(current));
                    current = String::new();
                }
                
                in_brackets = true;
                let mut bracket_content = String::new();
                
                // Parse bracket content
                while let Some(next_char) = chars.next() {
                    if next_char == ']' {
                        in_brackets = false;
                        break;
                    }
                    bracket_content.push(next_char);
                }
                
                // Parse the bracket content
                if bracket_content == "*" {
                    parts.push(QueryPart::ArrayWildcard);
                } else if let Ok(idx) = bracket_content.parse::<i32>() {
                    parts.push(QueryPart::ArrayIndex(idx));
                } else if bracket_content.contains(':') {
                    let slice_parts: Vec<&str> = bracket_content.split(':').collect();
                    if slice_parts.len() == 2 {
                        let start = slice_parts[0].parse::<i32>().unwrap_or(0);
                        let end = slice_parts[1].parse::<i32>().unwrap_or(0);
                        parts.push(QueryPart::ArraySlice(start, end));
                    }
                } else if bracket_content.starts_with("?") {
                    // Basic filter support
                    if let Some(filter) = parse_filter(&bracket_content[1..]) {
                        parts.push(QueryPart::Filter(filter));
                    }
                }
            }
            _ if !in_brackets => {
                current.push(c);
            }
            _ => {}
        }
    }
    
    // Add the last part if any
    if !current.is_empty() {
        parts.push(QueryPart::Child(current));
    }
    
    parts
}

/// Parse a filter expression
fn parse_filter(filter_str: &str) -> Option<FilterExpression> {
    // Very basic filter parsing for now
    // Expecting format like (@.path == value) or (@.path != value)
    let filter_str = filter_str.trim();
    
    if filter_str.starts_with("(@.") && filter_str.ends_with(")") {
        let content = &filter_str[3..filter_str.len()-1].trim();
        
        if content.contains(" == ") {
            let parts: Vec<&str> = content.split(" == ").collect();
            if parts.len() == 2 {
                let path = parts[0].trim();
                let value = parse_filter_value(parts[1].trim());
                return Some(FilterExpression::Equals(path.to_string(), value));
            }
        } else if content.contains(" != ") {
            let parts: Vec<&str> = content.split(" != ").collect();
            if parts.len() == 2 {
                let path = parts[0].trim();
                let value = parse_filter_value(parts[1].trim());
                return Some(FilterExpression::NotEquals(path.to_string(), value));
            }
        } else if content.contains(" contains ") {
            let parts: Vec<&str> = content.split(" contains ").collect();
            if parts.len() == 2 {
                let path = parts[0].trim();
                let value = parse_filter_value(parts[1].trim());
                return Some(FilterExpression::Contains(path.to_string(), value));
            }
        } else {
            // Just checking for existence
            return Some(FilterExpression::Exists(content.to_string()));
        }
    }
    
    None
}

/// Parse a value in a filter expression
fn parse_filter_value(value_str: &str) -> SipValue {
    if value_str == "null" {
        SipValue::Null
    } else if value_str == "true" {
        SipValue::Bool(true)
    } else if value_str == "false" {
        SipValue::Bool(false)
    } else if let Ok(n) = value_str.parse::<f64>() {
        SipValue::Number(n)
    } else if value_str.starts_with('"') && value_str.ends_with('"') {
        SipValue::String(value_str[1..value_str.len()-1].to_string())
    } else {
        SipValue::String(value_str.to_string())
    }
}

/// A part of a query
#[derive(Debug, Clone)]
enum QueryPart {
    /// Root element ($)
    Root,
    /// Child element (.)
    Child(String),
    /// Recursive descent (..)
    RecursiveDescent(String),
    /// Array index ([n])
    ArrayIndex(i32),
    /// Array wildcard ([*])
    ArrayWildcard,
    /// Array slice ([start:end])
    ArraySlice(i32, i32),
    /// Filter expression ([?(...)])
    Filter(FilterExpression),
}

/// A filter expression
#[derive(Debug, Clone)]
enum FilterExpression {
    /// Path equals value
    Equals(String, SipValue),
    /// Path not equals value
    NotEquals(String, SipValue),
    /// String contains substring
    Contains(String, SipValue),
    /// Path exists
    Exists(String),
} 