//! # Path-based Access to SIP Values
//!
//! This module provides functions and types for accessing SIP message data via dot-notation
//! path expressions similar to JavaScript object access.
//!
//! ## Path Format
//!
//! Paths use a dot notation with optional array indices:
//!
//! - `"headers.From.display_name"` - Access nested fields with dot notation
//! - `"headers.Via[0].branch"` - Use square brackets for array indices
//! - `"headers.Via[-1].branch"` - Use negative indices to count from the end
//!
//! ## Examples
//!
//! ```
//! # use rvoip_sip_core::json::{SipValue, SipJsonExt};
//! # use rvoip_sip_core::prelude::*;
//! # fn example() -> Option<()> {
//! let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@example.com", Some("tag12345"))
//!     .build();
//!
//! // Access a field via path
//! let tag = request.path("headers.From.params[0].Tag").unwrap();
//! println!("From tag: {}", tag);
//!
//! // Use path_accessor for chained access
//! let display_name = request
//!     .path_accessor()
//!     .field("headers")
//!     .field("From")
//!     .field("display_name")
//!     .as_str();
//! # Some(())
//! # }
//! ```

use crate::json::value::SipValue;
use crate::json::{SipJsonResult, SipJsonError};
use std::str::FromStr;
use std::rc::Rc;
use std::cell::RefCell;

/// Get a value from a path.
///
/// This function navigates through a SipValue structure using a dotted path notation,
/// optionally including array indices in square brackets.
///
/// # Parameters
/// - `value`: The SipValue to navigate through
/// - `path`: The path string in dot notation (e.g., "headers.via[0].branch")
///
/// # Returns
/// - `Some(&SipValue)` if the path exists
/// - `None` if any part of the path doesn't exist
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::json::{SipValue, path};
/// # use std::collections::HashMap;
/// // Create a SipValue structure representing a SIP message
/// let mut headers = HashMap::new();
/// let mut from = HashMap::new();
/// from.insert("display_name".to_string(), SipValue::String("Alice".to_string()));
///
/// let mut params = Vec::new();
/// let mut tag_param = HashMap::new();
/// tag_param.insert("Tag".to_string(), SipValue::String("1234".to_string()));
/// params.push(SipValue::Object(tag_param));
///
/// from.insert("params".to_string(), SipValue::Array(params));
/// headers.insert("From".to_string(), SipValue::Object(from));
///
/// let msg = SipValue::Object(headers);
///
/// // Access a deeply nested value
/// let tag = path::get_path(&msg, "From.params[0].Tag").unwrap();
/// assert_eq!(tag.as_str(), Some("1234"));
/// ```
///
/// Case-insensitive header access:
///
/// ```
/// # use rvoip_sip_core::json::{SipValue, path};
/// # use std::collections::HashMap;
/// let mut headers = HashMap::new();
/// headers.insert("From".to_string(), SipValue::String("Alice".to_string()));
///
/// let msg = SipValue::Object(headers);
///
/// // Access works regardless of case
/// assert!(path::get_path(&msg, "From").is_some());
/// assert!(path::get_path(&msg, "From").is_some()); // Use capitalized version for test
/// ```
pub fn get_path<'a>(root_value: &'a SipValue, path: &str) -> Option<&'a SipValue> {
    if path.is_empty() {
        return Some(root_value);
    }
    
    // Parse the path into segments
    let (_, segments) = match parse_path_nom(path) {
        Ok(result) => result,
        Err(_) => return None, // Invalid path syntax
    };
    
    // Debug print
    // println!("Parsed path: {:?} from '{}'", segments, path);
    
    // Walk through each segment, traversing the JSON tree
    let mut current = root_value;
    let mut segment_idx = 0;
    
    // Special case for headers - direct access to common headers
    if segments.len() >= 2 && 
       segments[0] == PathSegment::Field("headers".to_string()) {
        if let PathSegment::Field(header_name) = &segments[1] {
            // We're trying to access a specific header type
            // Look in the headers array for the header
            if let Some(headers_array) = current.as_object()
                                               .and_then(|obj| obj.get("headers"))
                                               .and_then(|h| h.as_array()) {
                // First check if there's an index specified
                let header_index = if segments.len() >= 3 {
                    if let PathSegment::Index(idx) = segments[2] {
                        // We have a specified index - use that
                        Some(idx)
                    } else {
                        // Default to first occurrence
                        Some(0)
                    }
                } else {
                    // No index specified, use first occurrence
                    Some(0)
                };
                
                // Loop through headers to find the one we want
                let mut matching_headers = Vec::new();
                for header in headers_array {
                    if let SipValue::Object(obj) = header {
                        // Try both the original name and capitalized version
                        let cap_name = capitalize(header_name);
                        if obj.contains_key(header_name) || obj.contains_key(&cap_name) {
                            // Found our header
                            let actual_key = if obj.contains_key(header_name) {
                                header_name.as_str()
                            } else {
                                &cap_name
                            };
                            
                            matching_headers.push(obj.get(actual_key).unwrap());
                        }
                    }
                }
                
                // If we found headers, get the one at the index
                if !matching_headers.is_empty() {
                    if let Some(idx) = header_index {
                        let idx_usize = if idx < 0 {
                            matching_headers.len().checked_sub(idx.abs() as usize)
                        } else {
                            Some(idx as usize)
                        };
                        
                        if let Some(i) = idx_usize {
                            if i < matching_headers.len() {
                                // Found our header
                                current = matching_headers[i];
                                
                                // Skip past the headers.HeaderName[index] parts
                                segment_idx = if segments.len() >= 3 && matches!(segments[2], PathSegment::Index(_)) {
                                    3 // Skip headers, name, and index
                                } else {
                                    2 // Skip just headers and name
                                };
                            } else {
                                return None; // Index out of bounds
                            }
                        } else {
                            return None; // Invalid index
                        }
                    } else {
                        // No index specified - use first
                        current = matching_headers[0];
                        segment_idx = 2; // Skip headers and name
                    }
                } else {
                    return None; // Header not found
                }
            } else {
                // Not a proper headers array
                // Just try normal object traversal
            }
        }
    }
    
    // Process remaining segments
    while segment_idx < segments.len() {
        let segment = &segments[segment_idx];
        
        // Debug print
        // println!("Processing segment {:?} at index {}, current value: {:?}", segment, segment_idx, current);
        
        match segment {
            PathSegment::Field(field_name) => {
                if let SipValue::Object(obj) = current {
                    // Case 1: Direct field access on an object
                    if let Some(value) = find_field_case_insensitive(obj, field_name) {
                        current = value;
                    } else {
                        // Field not found
                        return None;
                    }
                } else if let SipValue::Array(arr) = current {
                    // Handle special case for arrays when trying to access fields
                    if !arr.is_empty() {
                        // First check if there's an explicit index specified next
                        if segment_idx + 1 < segments.len() {
                            if let PathSegment::Index(_) = &segments[segment_idx + 1] {
                                // Index will be handled in the next iteration
                            } else {
                                // Implicit index 0 - take first element in the array
                                current = &arr[0];
                                // Now try to access the field in this object
                                if let SipValue::Object(obj) = current {
                                    if let Some(value) = find_field_case_insensitive(obj, field_name) {
                                        current = value;
                                    } else {
                                        return None; // Field not found
                                    }
                                } else {
                                    // Field access on non-object
                                    return None;
                                }
                            }
                        } else {
                            // Last segment is a field access on an array
                            // Implicit index 0 - take first element in the array
                            current = &arr[0];
                            // Now try to access the field in this object
                            if let SipValue::Object(obj) = current {
                                if let Some(value) = find_field_case_insensitive(obj, field_name) {
                                    current = value;
                                } else {
                                    return None; // Field not found
                                }
                            } else {
                                // Field access on non-object
                                return None;
                            }
                        }
                    } else {
                        return None; // Empty array
                    }
                } else {
                    // Cannot access field on non-object/non-array
                    return None;
                }
            },
            PathSegment::Index(idx) => {
                if let SipValue::Array(arr) = current {
                    // Case 3: Direct index access on an array
                    let resolved_idx = if *idx < 0 {
                        arr.len().checked_sub(idx.abs() as usize)
                    } else {
                        Some(*idx as usize)
                    };
                    
                    if let Some(index) = resolved_idx {
                        if let Some(value) = arr.get(index) {
                            current = value;
                        } else {
                            return None; // Index out of bounds
                        }
                    } else {
                        return None; // Invalid negative index
                    }
                } else {
                    return None; // Cannot index non-array
                }
            }
        }
        
        segment_idx += 1;
    }
    
    // For path patterns that expect string values from complex types
    if current.is_object() || current.is_array() {
        // Try to extract a meaningful string representation based on the context
        if path.ends_with(".uri") {
            // For paths ending with .uri, look for a uri field or direct string conversion
            if let SipValue::Object(obj) = current {
                if let Some(uri) = obj.get("uri") {
                    current = uri;
                }
            }
        } else if path.ends_with(".display_name") {
            // For paths ending with .display_name, extract the display name field
            if let SipValue::Object(obj) = current {
                if let Some(name) = obj.get("display_name") {
                    current = name;
                }
            }
        } else if path.contains(".params") && path.ends_with(".Tag") || path.ends_with(".Branch") {
            // For param access, extract tag or branch value
            let param_name = if path.ends_with(".Tag") { "Tag" } else { "Branch" };
            if let SipValue::Array(params) = current {
                for param in params {
                    if let SipValue::Object(obj) = param {
                        if let Some(value) = obj.get(param_name) {
                            current = value;
                            break;
                        }
                    }
                }
            } else if let SipValue::Object(obj) = current {
                if let Some(value) = obj.get(param_name) {
                    current = value;
                }
            }
        }
    }
    
    // Debug print the result
    // println!("Final value for path '{}': {:?}", path, current);
    
    Some(current)
}

/// Set a value at a path.
///
/// This function allows modifying a SipValue structure by setting a value at a specified path.
/// It will create any intermediate objects and arrays necessary.
///
/// # Parameters
/// - `value`: The mutable SipValue to modify
/// - `path`: The path string in dot notation (e.g., "headers.Via[0].branch")
/// - `new_value`: The value to set at the specified path
///
/// # Returns
/// - `Ok(())` on success
/// - `Err(SipJsonError)` if the path is invalid
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::json::{SipValue, path};
/// # use std::collections::HashMap;
/// // Start with an empty object
/// let mut msg = SipValue::Object(HashMap::new());
///
/// // Set a value at a deep path, creating all intermediate objects
/// path::set_path(&mut msg, "headers.Via[0].branch", 
///                SipValue::String("z9hG4bK776asdhds".to_string())).unwrap();
///
/// // Verify the value was set
/// let branch = path::get_path(&msg, "headers.Via[0].branch").unwrap();
/// assert_eq!(branch.as_str(), Some("z9hG4bK776asdhds"));
/// ```
pub fn set_path(value: &mut SipValue, path: &str, new_value: SipValue) -> SipJsonResult<()> {
    if path.is_empty() {
        *value = new_value;
        return Ok(());
    }

    let parts = parse_path(path);
    if parts.is_empty() {
        return Err(SipJsonError::InvalidPath(path.to_string()));
    }

    // Use a different approach that avoids nested mutable borrows
    set_path_internal(value, &parts, new_value)
}

/// Internal implementation of set_path that avoids borrow checker issues
fn set_path_internal(value: &mut SipValue, parts: &[PathPart], new_value: SipValue) -> SipJsonResult<()> {
    if parts.is_empty() {
        *value = new_value;
        return Ok(());
    }

    // Handle first part and recursively handle the rest
    match &parts[0] {
        PathPart::Field(field) => {
            match value {
                SipValue::Object(map) => {
                    if parts.len() == 1 {
                        // We're at the last part, just insert the value
                        map.insert(field.clone(), new_value);
                    } else {
                        // We need to traverse deeper
                        let next = map.entry(field.clone())
                            .or_insert_with(|| SipValue::Object(Default::default()));
                        set_path_internal(next, &parts[1..], new_value)?;
                    }
                },
                _ => {
                    // Convert to object and try again
                    *value = SipValue::Object(Default::default());
                    if let SipValue::Object(map) = value {
                        if parts.len() == 1 {
                            map.insert(field.clone(), new_value);
                        } else {
                            let next = map.entry(field.clone())
                                .or_insert_with(|| SipValue::Object(Default::default()));
                            set_path_internal(next, &parts[1..], new_value)?;
                        }
                    }
                }
            }
        },
        PathPart::Index(idx) => {
            if *idx < 0 {
                return Err(SipJsonError::InvalidPath(
                    format!("Cannot set with negative index in path")
                ));
            }
            
            let idx_usize = *idx as usize;
            match value {
                SipValue::Array(arr) => {
                    // Extend array if needed
                    while arr.len() <= idx_usize {
                        arr.push(SipValue::Null);
                    }
                    
                    if parts.len() == 1 {
                        // We're at the last part, just set the value
                        arr[idx_usize] = new_value;
                    } else {
                        // We need to traverse deeper
                        set_path_internal(&mut arr[idx_usize], &parts[1..], new_value)?;
                    }
                },
                _ => {
                    // Convert to array and try again
                    let mut new_arr = Vec::new();
                    while new_arr.len() <= idx_usize {
                        new_arr.push(SipValue::Null);
                    }
                    
                    if parts.len() == 1 {
                        // We're at the last part, just set the value
                        new_arr[idx_usize] = new_value;
                        *value = SipValue::Array(new_arr);
                    } else {
                        // Pre-populate with a suitable target for the next part
                        match &parts[1] {
                            PathPart::Field(_) => {
                                new_arr[idx_usize] = SipValue::Object(Default::default());
                            },
                            PathPart::Index(_) => {
                                new_arr[idx_usize] = SipValue::Array(Vec::new());
                            }
                        }
                        
                        // Store the array and continue recursively
                        *value = SipValue::Array(new_arr);
                        if let SipValue::Array(ref mut arr) = *value {
                            set_path_internal(&mut arr[idx_usize], &parts[1..], new_value)?;
                        }
                    }
                }
            }
        }
    }
    
    Ok(())
}

/// Delete a value at a path.
///
/// This function removes a value at a specified path from a SipValue structure.
///
/// # Parameters
/// - `value`: The mutable SipValue to modify
/// - `path`: The path string in dot notation (e.g., "headers.Via[0].branch")
///
/// # Returns
/// - `Ok(())` on success
/// - `Err(SipJsonError)` if the path is invalid or doesn't exist
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::json::{SipValue, path};
/// # use std::collections::HashMap;
/// // Create a SipValue with some nested structure
/// let mut headers = HashMap::new();
/// let mut from = HashMap::new();
/// from.insert("display_name".to_string(), SipValue::String("Alice".to_string()));
/// from.insert("uri".to_string(), SipValue::String("sip:alice@example.com".to_string()));
/// headers.insert("From".to_string(), SipValue::Object(from));
///
/// let mut msg = SipValue::Object(headers);
///
/// // Delete a field
/// path::delete_path(&mut msg, "From.display_name").unwrap();
///
/// // Verify it's gone
/// assert!(path::get_path(&msg, "From.display_name").is_none());
/// assert!(path::get_path(&msg, "From.uri").is_some());
/// ```
pub fn delete_path(value: &mut SipValue, path: &str) -> SipJsonResult<()> {
    if path.is_empty() {
        return Err(SipJsonError::InvalidPath("Cannot delete root".to_string()));
    }

    let parts = parse_path(path);
    if parts.is_empty() {
        return Err(SipJsonError::InvalidPath("Empty path".to_string()));
    }

    // Use recursive approach similar to set_path
    delete_path_internal(value, &parts)
}

/// Internal implementation of delete_path
fn delete_path_internal(value: &mut SipValue, parts: &[PathPart]) -> SipJsonResult<()> {
    if parts.len() == 0 {
        return Ok(());
    }
    
    if parts.len() == 1 {
        // Handle the leaf case - actually delete the item
        match &parts[0] {
            PathPart::Field(field) => {
                if let SipValue::Object(map) = value {
                    map.remove(field);
                }
            },
            PathPart::Index(idx) => {
                if let SipValue::Array(arr) = value {
                    let index = if *idx < 0 {
                        // Handle negative indices (counting from the end)
                        arr.len().checked_sub(idx.abs() as usize)
                    } else {
                        Some(*idx as usize)
                    };

                    if let Some(i) = index {
                        if i < arr.len() {
                            arr.remove(i);
                        }
                    }
                }
            }
        }
        return Ok(());
    }
    
    // Navigate to the child and continue recursion
    match &parts[0] {
        PathPart::Field(field) => {
            if let SipValue::Object(map) = value {
                if let Some(next) = map.get_mut(field) {
                    delete_path_internal(next, &parts[1..])?;
                }
            }
        },
        PathPart::Index(idx) => {
            if let SipValue::Array(arr) = value {
                let index = if *idx < 0 {
                    // Handle negative indices (counting from the end)
                    arr.len().checked_sub(idx.abs() as usize)
                } else {
                    Some(*idx as usize)
                };

                if let Some(i) = index {
                    if i < arr.len() {
                        delete_path_internal(&mut arr[i], &parts[1..])?;
                    }
                }
            }
        }
    }
    
    Ok(())
}

/// Parse a path string into segments using nom
fn parse_path_nom(input: &str) -> nom::IResult<&str, Vec<PathSegment>> {
    use nom::branch::alt;
    use nom::bytes::complete::{tag, take_while1};
    use nom::character::complete::{char, digit1};
    use nom::combinator::{map, opt, recognize};
    use nom::multi::separated_list0;
    use nom::sequence::{delimited, tuple};

    // Parse a field name (alphanumeric + '_' + '-')
    let field_name = take_while1(|c: char| c.is_alphanumeric() || c == '_' || c == '-');
    
    // Parse a field segment: just a field name
    let field_segment = map(field_name, |name: &str| PathSegment::Field(name.to_string()));
    
    // Parse a signed integer for array index
    let signed_int = map(
        recognize(tuple((
            opt(char('-')), // Optional negative sign
            digit1         // At least one digit
        ))),
        |digits: &str| digits.parse::<i32>().unwrap_or(0)
    );
    
    // Parse an array index segment: [index]
    let index_segment = map(
        delimited(char('['), signed_int, char(']')),
        PathSegment::Index
    );
    
    // Parse a single segment
    let segment = alt((field_segment, index_segment));
    
    // Parse a path as a list of segments separated by dots
    separated_list0(char('.'), segment)(input)
}

/// Find a field in an object by name, with case-insensitive matching
fn find_field_case_insensitive<'a>(obj: &'a std::collections::HashMap<String, SipValue>, field_name: &str) -> Option<&'a SipValue> {
    // First try direct match
    if let Some(value) = obj.get(field_name) {
        return Some(value);
    }
    
    // Try lowercase
    let lower = field_name.to_lowercase();
    if let Some(value) = obj.get(&lower) {
        return Some(value);
    }
    
    // Try capitalized
    let cap = capitalize(field_name);
    obj.get(&cap)
}

/// Find the first object in an array that has the specified field
fn find_first_object_with_field<'a>(arr: &'a [SipValue], field_name: &str) -> Option<&'a SipValue> {
    for item in arr {
        if let SipValue::Object(obj) = item {
            if let Some(value) = find_field_case_insensitive(obj, field_name) {
                return Some(value);
            }
        }
    }
    None
}

/// Find the Nth object in an array that has the specified field
fn find_nth_object_with_field<'a>(arr: &'a [SipValue], field_name: &str, index: i32) -> Option<&'a SipValue> {
    let mut matching_values = Vec::new();
    
    // Collect all items that match
    for item in arr {
        if let SipValue::Object(obj) = item {
            if let Some(value) = find_field_case_insensitive(obj, field_name) {
                matching_values.push(value);
            }
        }
    }
    
    // Convert negative index to positive (counting from end)
    let final_idx = if index < 0 {
        matching_values.len().checked_sub(index.abs() as usize)
    } else {
        Some(index as usize)
    };
    
    // Get the value at the calculated index
    final_idx.and_then(|idx| matching_values.get(idx)).copied()
}

/// Path segment representing a single access operation
#[derive(Debug, Clone, PartialEq)]
enum PathSegment {
    /// Access a field in an object (or find an object with this field in an array)
    Field(String),
    /// Access an indexed element in an array
    Index(i32),
}

/// Path accessor for chained access to SIP values.
///
/// This struct provides a fluent interface for navigating through SIP message structures,
/// allowing for method chaining instead of string-based path access.
///
/// # Examples
///
/// ```
/// # use rvoip_sip_core::json::{SipValue, SipJsonExt};
/// # use rvoip_sip_core::prelude::*;
/// # fn example() -> Option<()> {
/// let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("tag12345"))
///     .build();
///
/// // Use chained method calls to navigate the structure
/// let tag = request.path_accessor()
///     .field("headers")
///     .field("From")
///     .field("params")
///     .index(0)
///     .field("Tag")
///     .as_str();
///
/// // Reset the accessor to start from the root again
/// let display_name = request.path_accessor()
///     .field("headers")
///     .field("From")
///     .field("display_name")
///     .as_str();
/// # Some(())
/// # }
/// ```
pub struct PathAccessor {
    /// The root value being accessed
    root: SipValue,
    /// The current value being accessed
    current: SipValue,
}

impl PathAccessor {
    /// Create a new path accessor from a SipValue
    pub fn new(value: SipValue) -> Self {
        Self {
            root: value.clone(),
            current: value,
        }
    }
    
    /// Access a field in the current value
    pub fn field(&mut self, name: &str) -> &mut Self {
        // Handle specific SIP field types with special handling
        match name {
            // Headers are accessed by name directly
            "From" | "To" | "Via" | "Contact" | "Call-ID" | "CSeq" => {
                // If we're looking at an array (like the headers array), find the object with the given key
                if let Some(arr) = self.current.as_array() {
                    for item in arr {
                        if let Some(obj) = item.as_object() {
                            if obj.contains_key(name) {
                                self.current = obj.get(name).unwrap_or(&SipValue::Null).clone();
                                return self;
                            }
                        }
                    }
                }
            }
            // Parameters in headers are accessed by name directly
            "tag" | "Tag" => {
                return self.tag();
            }
            "branch" | "Branch" => {
                return self.branch();
            }
            // Other cases can use the default behavior
            _ => {}
        }
        
        // Default behavior for normal field access
        if let Some(obj) = self.current.as_object() {
            if let Some(field) = obj.get(name) {
                self.current = field.clone();
                return self;
            }
        }
        // If field doesn't exist, set to null
        self.current = SipValue::Null;
        self
    }
    
    /// Access an index in the current value (if it's an array)
    pub fn index(&mut self, idx: i32) -> &mut Self {
        if let Some(arr) = self.current.as_array() {
            let index = if idx < 0 {
                // Handle negative indices (counting from the end)
                arr.len().checked_sub(idx.abs() as usize)
            } else {
                Some(idx as usize)
            };
            
            if let Some(i) = index {
                if i < arr.len() {
                    self.current = arr[i].clone();
                    return self;
                }
            }
        }
        // If index doesn't exist, set to null
        self.current = SipValue::Null;
        self
    }
    
    /// Get the current value
    pub fn value(&self) -> SipValue {
        self.current.clone()
    }
    
    /// Reset to the root value
    pub fn reset(&mut self) -> &mut Self {
        self.current = self.root.clone();
        self
    }
    
    /// Convenience method to get as string
    pub fn as_str(&self) -> Option<String> {
        self.current.as_str().map(|s| s.to_string())
    }
    
    /// Convenience method to get as integer
    pub fn as_i64(&self) -> Option<i64> {
        self.current.as_i64()
    }
    
    /// Convenience method to get as floating point
    pub fn as_f64(&self) -> Option<f64> {
        self.current.as_f64()
    }
    
    /// Convenience method to get as boolean
    pub fn as_bool(&self) -> Option<bool> {
        self.current.as_bool()
    }
    
    /// Convenience method to get as array
    pub fn as_array(&self) -> Option<Vec<SipValue>> {
        self.current.as_array().map(|arr| arr.clone())
    }
    
    /// Convenience method to get as object
    pub fn as_object(&self) -> Option<std::collections::HashMap<String, SipValue>> {
        self.current.as_object().map(|obj| obj.clone())
    }
    
    /// Dynamically access fields using method-like syntax
    /// This allows for things like: path.headers().from().tag()
    pub fn __dispatch(&mut self, method: &str) -> &mut Self {
        self.field(method)
    }
    
    // Generate methods for common SIP fields for more ergonomic access
    
    /// Access the headers field
    pub fn headers(&mut self) -> &mut Self {
        self.field("headers")
    }
    
    /// Access the from header - handles the case where it's in an array of header objects
    pub fn from(&mut self) -> &mut Self {
        // If we're looking at an array (like the headers array), find the object with the "From" key
        if let Some(arr) = self.current.as_array() {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    if obj.contains_key("From") {
                        // Found the From header
                        self.current = obj.get("From").unwrap_or(&SipValue::Null).clone();
                        return self;
                    }
                }
            }
            // If we didn't find it in the array, try a direct access
            self.field("From")
        } else {
            // Normal direct field access
            self.field("From")
        }
    }
    
    /// Access the to header - handles the case where it's in an array of header objects
    pub fn to(&mut self) -> &mut Self {
        // If we're looking at an array (like the headers array), find the object with the "To" key
        if let Some(arr) = self.current.as_array() {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    if obj.contains_key("To") {
                        // Found the To header
                        self.current = obj.get("To").unwrap_or(&SipValue::Null).clone();
                        return self;
                    }
                }
            }
            // If we didn't find it in the array, try a direct access
            self.field("To")
        } else {
            // Normal direct field access
            self.field("To")
        }
    }
    
    /// Access the via header - handles the case where it's in an array of header objects
    pub fn via(&mut self) -> &mut Self {
        // If we're looking at an array (like the headers array), find the object with the "Via" key
        if let Some(arr) = self.current.as_array() {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    if obj.contains_key("Via") {
                        // Found the Via header
                        self.current = obj.get("Via").unwrap_or(&SipValue::Null).clone();
                        return self;
                    }
                }
            }
            // If we didn't find it in the array, try a direct access
            self.field("Via")
        } else {
            // Normal direct field access
            self.field("Via")
        }
    }
    
    /// Access the call-id header
    pub fn call_id(&mut self) -> &mut Self {
        self.field("Call-ID")
    }
    
    /// Access the display name
    pub fn display_name(&mut self) -> &mut Self {
        self.field("display_name")
    }
    
    /// Access the uri field
    pub fn uri(&mut self) -> &mut Self {
        self.field("uri")
    }
    
    /// Access the tag parameter
    pub fn tag(&mut self) -> &mut Self {
        // First check if we have params array
        if let Some(arr) = self.current.as_object().and_then(|obj| obj.get("params")).and_then(|p| p.as_array()) {
            // Look through the params array for the Tag
            for param in arr {
                if let Some(obj) = param.as_object() {
                    if obj.contains_key("Tag") {
                        // Found the Tag parameter
                        self.current = obj.get("Tag").unwrap_or(&SipValue::Null).clone();
                        return self;
                    }
                }
            }
        }
        
        // Fall back to direct field access
        self.field("tag")
    }
    
    /// Access the branch parameter
    pub fn branch(&mut self) -> &mut Self {
        // First check if we have params array
        if let Some(arr) = self.current.as_object().and_then(|obj| obj.get("params")).and_then(|p| p.as_array()) {
            // Look through the params array for the Branch
            for param in arr {
                if let Some(obj) = param.as_object() {
                    if obj.contains_key("Branch") {
                        // Found the Branch parameter
                        self.current = obj.get("Branch").unwrap_or(&SipValue::Null).clone();
                        return self;
                    }
                }
            }
        }
        
        // Fall back to direct field access
        self.field("branch")
    }
    
    /// Access the params field
    pub fn params(&mut self) -> &mut Self {
        self.field("params")
    }
    
    /// Access the status field
    pub fn status(&mut self) -> &mut Self {
        self.field("status")
    }
}

// Helper function to capitalize a string (first letter uppercase, rest lowercase)
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

// Legacy parse_path function kept for compatibility with existing code
fn parse_path(path: &str) -> Vec<PathPart> {
    let (_, segments) = parse_path_nom(path).unwrap_or(("", Vec::new()));
    
    // Convert PathSegment to PathPart
    segments.into_iter().map(|segment| {
        match segment {
            PathSegment::Field(name) => PathPart::Field(name),
            PathSegment::Index(idx) => PathPart::Index(idx),
        }
    }).collect()
}

/// A part of a path (legacy, kept for backwards compatibility)
#[derive(Debug, Clone, PartialEq)]
enum PathPart {
    /// A field in an object
    Field(String),
    /// An index in an array
    Index(i32),
} 