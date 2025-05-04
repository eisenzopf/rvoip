//! Path-based access to SIP values

use crate::json::value::SipValue;
use crate::json::{SipJsonResult, SipJsonError};
use std::str::FromStr;

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
                if let Some(obj) = current.as_object() {
                    if let Some(next) = obj.get(&field) {
                        current = next;
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
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
                    
                    // Debug output for parsed path parts
                    println!("Parsed path part: field={}, index={}", field, &part[bracket_pos+1..close_pos]);
                    
                    continue;
                }
            }
        }
        
        // Regular field name
        result.push(PathPart::Field(part.to_string()));
        println!("Parsed path part: field={}", part);
    }
    
    println!("Complete path parsing for '{}' resulted in {} parts", path, result.len());
    
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