//! # Header Access Utilities
//!
//! This module provides utilities for accessing headers in SIP messages.
//! It defines the `HeaderAccess` trait that provides a consistent API
//! for accessing headers across different message types.

use crate::error::{Error, Result};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait};
use std::collections::HashSet;
use std::str::FromStr;
use std::any::TypeId;

/// Trait for consistent header access across SIP message types.
///
/// Implementing this trait provides a uniform way to access headers
/// in SIP messages, regardless of whether they are requests, responses,
/// or other message types.
pub trait HeaderAccess {
    /// Returns all headers with the specified type.
    ///
    /// # Type Parameters
    ///
    /// - `T`: The expected header type that implements `TypedHeaderTrait`
    ///
    /// # Returns
    ///
    /// A vector of references to all headers of the specified type
    fn typed_headers<T: TypedHeaderTrait + 'static>(&self) -> Vec<&T>;

    /// Returns the first header with the specified type, if any.
    ///
    /// # Type Parameters
    ///
    /// - `T`: The expected header type that implements `TypedHeaderTrait`
    ///
    /// # Returns
    ///
    /// An optional reference to the first header of the specified type
    fn typed_header<T: TypedHeaderTrait + 'static>(&self) -> Option<&T>;

    /// Returns all headers with the specified name.
    ///
    /// # Parameters
    ///
    /// - `name`: The header name
    ///
    /// # Returns
    ///
    /// A vector of references to all typed headers with the specified name
    fn headers(&self, name: &HeaderName) -> Vec<&TypedHeader>;

    /// Returns the first header with the specified name, if any.
    ///
    /// # Parameters
    ///
    /// - `name`: The header name
    ///
    /// # Returns
    ///
    /// An optional reference to the first header with the specified name
    fn header(&self, name: &HeaderName) -> Option<&TypedHeader>;

    /// Returns all headers with the specified name as a string.
    ///
    /// # Parameters
    ///
    /// - `name`: The header name as a string
    ///
    /// # Returns
    ///
    /// A vector of references to all typed headers with the specified name,
    /// or an empty vector if the name is invalid
    fn headers_by_name(&self, name: &str) -> Vec<&TypedHeader>;

    /// Returns the raw value of the first header with the specified name, if any.
    ///
    /// # Parameters
    ///
    /// - `name`: The header name
    ///
    /// # Returns
    ///
    /// An optional string containing the raw value of the first header with the specified name
    fn raw_header_value(&self, name: &HeaderName) -> Option<String>;

    /// Returns all raw values for headers with the specified name.
    ///
    /// # Parameters
    ///
    /// - `name`: The header name
    ///
    /// # Returns
    ///
    /// A vector of byte vectors containing the raw values of all headers with the specified name
    fn raw_headers(&self, name: &HeaderName) -> Vec<Vec<u8>>;

    /// Returns all header names present in the message.
    ///
    /// # Returns
    ///
    /// A vector of all header names present in the message, without duplicates
    fn header_names(&self) -> Vec<HeaderName>;

    /// Checks if a header with the specified name is present.
    ///
    /// # Parameters
    ///
    /// - `name`: The header name
    ///
    /// # Returns
    ///
    /// `true` if a header with the specified name is present, `false` otherwise
    fn has_header(&self, name: &HeaderName) -> bool;
}

/// Helper function to try to extract a typed header of a specific type
/// from a `TypedHeader` enum.
pub fn try_as_typed_header<'a, T: TypedHeaderTrait + 'static>(header: &'a TypedHeader) -> Option<&'a T> {
    header.as_typed_ref::<T>()
}

/// Helper function to collect all headers of a specific type from a list of headers
/// 
/// This function uses the more efficient `as_typed_ref` method on TypedHeader
/// and handles special cases like Via headers that can contain multiple entries.
pub fn collect_typed_headers<'a, T: TypedHeaderTrait + 'static>(headers: &'a [TypedHeader]) -> Vec<&'a T> {
    // Filter headers by name first for efficiency
    let target_name: HeaderName = T::header_name().into();
    let type_id = std::any::TypeId::of::<T>();
    
    let mut result = Vec::new();
    
    // Special handling for Via headers - each Via can contain multiple entries
    if type_id == std::any::TypeId::of::<crate::types::via::Via>() {
        let vias = headers
            .iter()
            .filter(|h| h.name() == target_name)
            .filter_map(|h| h.as_typed_ref::<T>());
            
        // Via headers should always count as multiple headers since the struct contains a Vec of ViaHeader
        for via in vias {
            // For Via headers, we count each entry as separate header
            let via_obj = unsafe { &*(via as *const T as *const crate::types::via::Via) };
            for _ in via_obj.headers() {
                result.push(via);
            }
        }
        return result;
    }
    
    // Standard handling for all other header types
    headers
        .iter()
        .filter(|h| h.name() == target_name)
        .filter_map(|h| h.as_typed_ref::<T>())
        .collect()
} 