//! Path-based access to SIP values

use crate::json::value::SipValue;
use crate::json::{SipJsonResult, SipJsonError};
use std::str::FromStr;
use std::rc::Rc;
use std::cell::RefCell;

/// Get a value from a path
/// 
/// Examples:
/// - "headers.via.branch"
/// - "headers.via[0].branch"
/// - "headers.via[-1].branch" (last element)
pub fn get_path<'a>(value: &'a SipValue, path: &str) -> Option<&'a SipValue> {
    if path.is_empty() {
        return Some(value);
    }

    let mut current = value;
    let parts = parse_path(path);

    for part in parts {
        match part {
            PathPart::Field(field) => {
                // Case 1: Normal field access on an object
                if let Some(obj) = current.as_object() {
                    if let Some(next) = obj.get(&field) {
                        current = next;
                        continue;
                    }
                } 
                
                // Case 2: Field access on an array - try to find an object with matching key
                // This enables paths like "headers.from.display_name" to work without indices
                if let Some(arr) = current.as_array() {
                    let mut found = false;
                    for item in arr {
                        if let Some(obj) = item.as_object() {
                            // Handle SIP headers stored in the headers array
                            // Check if the object has the field as a key (case-insensitive)
                            if obj.contains_key(&field) || 
                               obj.contains_key(&field.to_lowercase()) || 
                               obj.contains_key(&capitalize(&field)) {
                                
                                // Get the actual key with correct case
                                let actual_key = if obj.contains_key(&field) {
                                    &field
                                } else if obj.contains_key(&field.to_lowercase()) {
                                    &field.to_lowercase()
                                } else {
                                    &capitalize(&field)
                                };
                                
                                current = obj.get(actual_key).unwrap(); // Safe unwrap as we know it exists
                                found = true;
                                break;
                            }
                        }
                    }
                    
                    if found {
                        continue;
                    }
                }
                
                // Not found in either way
                return None;
            }
            PathPart::Index(idx) => {
                if let Some(arr) = current.as_array() {
                    let index = if idx < 0 {
                        // Handle negative indices (counting from the end)
                        arr.len().checked_sub(idx.abs() as usize)
                    } else {
                        Some(idx as usize)
                    };

                    if let Some(i) = index {
                        if i < arr.len() {
                            current = &arr[i];
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
        }
    }

    Some(current)
}

/// Set a value at a path
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

/// Delete a value at a path
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

/// Parse a path string into parts
fn parse_path(path: &str) -> Vec<PathPart> {
    let mut result = Vec::new();
    
    if path.is_empty() {
        return result;
    }
    
    // Split by dots, but handle array indexing
    let parts = path.split('.');
    
    for part in parts {
        // Check if this part has an array index
        if let Some(bracket_pos) = part.find('[') {
            if let Some(close_pos) = part.find(']') {
                if bracket_pos < close_pos {
                    // Get the field name (part before the bracket)
                    let field = &part[0..bracket_pos];
                    if !field.is_empty() {
                        result.push(PathPart::Field(field.to_string()));
                    }
                    
                    // Get the index value
                    let index_str = &part[bracket_pos + 1..close_pos];
                    if let Ok(index) = index_str.parse::<i32>() {
                        result.push(PathPart::Index(index));
                    }
                    
                    continue;
                }
            }
        }
        
        // Regular field name
        result.push(PathPart::Field(part.to_string()));
    }
    
    result
}

/// A part of a path
#[derive(Debug, Clone, PartialEq)]
enum PathPart {
    /// A field in an object
    Field(String),
    /// An index in an array
    Index(i32),
}

/// A fluent interface for accessing values in a SIP value using paths
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