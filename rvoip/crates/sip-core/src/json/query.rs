//! Query-based access to SIP values
//! 
//! This module provides a simplified JSONPath-like query system
//! to extract values from SIP structures.

use crate::json::value::SipValue;
use crate::json::{SipJsonResult, SipJsonError};
use std::collections::HashSet;

/// Query a SipValue using a simplified JSONPath-like syntax
///
/// Supported syntax:
/// - `$` - root
/// - `.` - child operator
/// - `..` - recursive descent
/// - `*` - wildcard
/// - `[n]` - array index
/// - `[start:end]` - array slice
/// - `[?(@.property == value)]` - filter (basic)
///
/// Examples:
/// - `$.headers.via.branch` - Get the branch parameter of the Via header
/// - `$.headers.via[*].branch` - Get all branch parameters from all Via headers
/// - `$..branch` - Get all branch parameters anywhere in the structure
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
                    chars.next();
                    if let Some(next_char) = chars.next() {
                        if next_char == '[' {
                            in_brackets = true;
                            current = String::new();
                        } else {
                            current.push(next_char);
                            parts.push(QueryPart::RecursiveDescent(current));
                            current = String::new();
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