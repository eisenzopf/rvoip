use crate::error::{Error, Result};
use std::collections::HashMap;
use ordered_float::NotNan;
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::accept::Accept;
use crate::parser::headers::accept::AcceptValue;
use super::HeaderSetter;

/// Extension trait for adding Accept header building capabilities
pub trait AcceptExt {
    /// Add an Accept header with a single media type
    ///
    /// # Arguments
    ///
    /// * `media_type` - The media type (e.g., "application/sdp")
    /// * `q` - Optional quality value (0.0 to 1.0, where 1.0 is highest priority)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AcceptExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .accept("application/sdp", Some(0.9))
    ///     .build();
    /// ```
    fn accept(
        self, 
        media_type: &str, 
        q: Option<f32>
    ) -> Self;

    /// Add an Accept header with multiple media types
    ///
    /// # Arguments
    ///
    /// * `media_types` - A vector of tuples containing (media_type, q_value)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AcceptExt};
    /// 
    /// let media_types = vec![
    ///     ("application/sdp", Some(1.0)),
    ///     ("application/json", Some(0.5)),
    /// ];
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .accepts(media_types)
    ///     .build();
    /// ```
    fn accepts(
        self, 
        media_types: Vec<(&str, Option<f32>)>
    ) -> Self;
}

impl<T> AcceptExt for T 
where 
    T: HeaderSetter,
{
    fn accept(
        self, 
        media_type: &str, 
        q: Option<f32>
    ) -> Self {
        // Parse the media type (format: type/subtype)
        let parts: Vec<&str> = media_type.split('/').collect();
        if parts.len() != 2 {
            return self; // Return self unchanged if format is invalid
        }

        let m_type = parts[0].to_string();
        let m_subtype = parts[1].to_string();

        // Create q value if provided
        let q_value = match q {
            Some(v) => match NotNan::new(v) {
                Ok(nn) => Some(nn),
                Err(_) => None,
            },
            None => None,
        };

        // Create the Accept header with the single media type
        let accept_value = AcceptValue {
            m_type,
            m_subtype,
            q: q_value,
            params: HashMap::new(),
        };

        let header_value = Accept::from_media_types(vec![accept_value]);
        self.set_header(header_value)
    }

    fn accepts(
        self, 
        media_types: Vec<(&str, Option<f32>)>
    ) -> Self {
        // Convert the media types input to the required format
        let mut accept_values = Vec::with_capacity(media_types.len());

        for (media_type, q) in media_types {
            // Parse the media type (format: type/subtype)
            let parts: Vec<&str> = media_type.split('/').collect();
            if parts.len() != 2 {
                continue; // Skip invalid media types
            }

            let m_type = parts[0].to_string();
            let m_subtype = parts[1].to_string();

            // Create q value if provided
            let q_value = match q {
                Some(v) => match NotNan::new(v) {
                    Ok(nn) => Some(nn),
                    Err(_) => None,
                },
                None => None,
            };

            // Create the Accept value
            let accept_value = AcceptValue {
                m_type,
                m_subtype,
                q: q_value,
                params: HashMap::new(),
            };

            accept_values.push(accept_value);
        }

        // If we have no valid media types, just return self
        if accept_values.is_empty() {
            return self;
        }

        // Create the Accept header with all media types
        let header_value = Accept::from_media_types(accept_values);
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::header::HeaderName;
    use crate::types::Accept;
    
    #[test]
    fn test_accept_single() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .accept("application/sdp", Some(0.8))
            .build();
            
        // Check if Accept header exists with the correct value
        let header = request.header(&HeaderName::Accept);
        assert!(header.is_some(), "Accept header not found");
        
        if let Some(TypedHeader::Accept(accept)) = header {
            // Check if the accept includes "application/sdp"
            assert!(accept.accepts_type("application", "sdp"), "application/sdp not found in Accept header");
            
            // Check the q value
            let media_types = accept.media_types();
            assert_eq!(media_types.len(), 1);
            
            let media_type = &media_types[0];
            assert_eq!(media_type.m_type, "application");
            assert_eq!(media_type.m_subtype, "sdp");
            
            // Check if the q param is present
            let has_q = media_type.q.map(|q| (q.into_inner() - 0.8).abs() < 0.001).unwrap_or(false);
            assert!(has_q, "q parameter with value 0.8 not found");
        } else {
            panic!("Expected Accept header");
        }
    }
    
    #[test]
    fn test_accepts_multiple() {
        let media_types = vec![
            ("application/sdp", Some(1.0)),
            ("application/json", Some(0.5)),
        ];
        
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .accepts(media_types)
            .build();
            
        // Check if Accept header exists with the correct values
        let header = request.header(&HeaderName::Accept);
        assert!(header.is_some(), "Accept header not found");
        
        if let Some(TypedHeader::Accept(accept)) = header {
            // Check if the accept includes both media types
            assert!(accept.accepts_type("application", "sdp"), "application/sdp not found in Accept header");
            assert!(accept.accepts_type("application", "json"), "application/json not found in Accept header");
            
            // Check the q values
            let media_types = accept.media_types();
            assert_eq!(media_types.len(), 2);
            
            // Find the application/sdp media type and check its q value
            let sdp_type = media_types.iter().find(|m| m.m_type == "application" && m.m_subtype == "sdp");
            assert!(sdp_type.is_some(), "application/sdp not found in Accept header");
            let has_q = sdp_type.unwrap().q.map(|q| (q.into_inner() - 1.0).abs() < 0.001).unwrap_or(false);
            assert!(has_q, "q parameter with value 1.0 not found for application/sdp");
            
            // Find the application/json media type and check its q value
            let json_type = media_types.iter().find(|m| m.m_type == "application" && m.m_subtype == "json");
            assert!(json_type.is_some(), "application/json not found in Accept header");
            let has_q = json_type.unwrap().q.map(|q| (q.into_inner() - 0.5).abs() < 0.001).unwrap_or(false);
            assert!(has_q, "q parameter with value 0.5 not found for application/json");
        } else {
            panic!("Expected Accept header");
        }
    }
} 