use crate::error::{Error, Result};
use std::collections::HashMap;
use std::str::FromStr;
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::content_disposition::{ContentDisposition, DispositionType, Handling};
use super::HeaderSetter;

/// Extension trait for adding Content-Disposition header building capabilities
pub trait ContentDispositionExt {
    /// Add a Content-Disposition header with session disposition type
    ///
    /// # Arguments
    ///
    /// * `handling` - The handling parameter (optional or required)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .content_disposition_session("optional")
    ///     .build();
    /// ```
    fn content_disposition_session(self, handling: &str) -> Self;

    /// Add a Content-Disposition header with render disposition type
    ///
    /// # Arguments
    ///
    /// * `handling` - The handling parameter (optional or required)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .content_disposition_render("required")
    ///     .build();
    /// ```
    fn content_disposition_render(self, handling: &str) -> Self;

    /// Add a Content-Disposition header with icon disposition type
    ///
    /// # Arguments
    ///
    /// * `size` - The size parameter for the icon
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .content_disposition_icon("32")
    ///     .build();
    /// ```
    fn content_disposition_icon(self, size: &str) -> Self;

    /// Add a Content-Disposition header with alert disposition type
    ///
    /// # Arguments
    ///
    /// * `handling` - Optional handling parameter (optional or required)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .content_disposition_alert(Some("optional"))
    ///     .build();
    /// ```
    fn content_disposition_alert(self, handling: Option<&str>) -> Self;

    /// Add a Content-Disposition header with a custom disposition type
    ///
    /// # Arguments
    ///
    /// * `disposition_type` - The disposition type string
    /// * `params` - Map of parameter names to values
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentDispositionExt};
    /// use std::collections::HashMap;
    /// 
    /// let mut params = HashMap::new();
    /// params.insert("handling".to_string(), "optional".to_string());
    /// params.insert("custom".to_string(), "value".to_string());
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .content_disposition("custom-disp", params)
    ///     .build();
    /// ```
    fn content_disposition(self, disposition_type: &str, params: HashMap<String, String>) -> Self;
}

impl<T> ContentDispositionExt for T 
where 
    T: HeaderSetter,
{
    fn content_disposition_session(self, handling: &str) -> Self {
        let mut params = HashMap::new();
        params.insert("handling".to_string(), handling.to_string());

        // Create Content-Disposition with session type
        let header_value = ContentDisposition {
            disposition_type: DispositionType::Session,
            params,
        };
        
        // Debug print
        eprintln!("Setting ContentDisposition header: {:?}", header_value);
        
        // Try to convert it to a header and back to see if conversion is working
        let header = header_value.to_header();
        eprintln!("Created header: {:?}", header);
        
        match ContentDisposition::from_header(&header) {
            Ok(cd) => eprintln!("Converted back to ContentDisposition: {:?}", cd),
            Err(e) => eprintln!("Failed to convert back: {:?}", e),
        }
        
        self.set_header(header_value)
    }

    fn content_disposition_render(self, handling: &str) -> Self {
        let mut params = HashMap::new();
        params.insert("handling".to_string(), handling.to_string());

        // Create Content-Disposition with render type
        let header_value = ContentDisposition {
            disposition_type: DispositionType::Render,
            params,
        };
        self.set_header(header_value)
    }

    fn content_disposition_icon(self, size: &str) -> Self {
        let mut params = HashMap::new();
        params.insert("size".to_string(), size.to_string());

        // Create Content-Disposition with icon type
        let header_value = ContentDisposition {
            disposition_type: DispositionType::Icon,
            params,
        };
        self.set_header(header_value)
    }

    fn content_disposition_alert(self, handling: Option<&str>) -> Self {
        let mut params = HashMap::new();
        if let Some(h) = handling {
            params.insert("handling".to_string(), h.to_string());
        }

        // Create Content-Disposition with alert type
        let header_value = ContentDisposition {
            disposition_type: DispositionType::Alert,
            params,
        };
        self.set_header(header_value)
    }

    fn content_disposition(self, disposition_type: &str, params: HashMap<String, String>) -> Self {
        // Parse the disposition type
        let disp_type = match DispositionType::from_str(disposition_type) {
            Ok(dt) => dt,
            Err(_) => return self, // Return self unchanged if parsing fails
        };
        
        // Create Content-Disposition
        let header_value = ContentDisposition {
            disposition_type: disp_type,
            params,
        };
        
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::header::HeaderName;
    
    #[test]
    fn test_content_disposition_session() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_disposition_session("optional")
            .build();
            
        // Check if Content-Disposition header exists with the correct value
        let header = request.header(&HeaderName::ContentDisposition);
        eprintln!("DEBUG: Header type: {:?}", header.map(|h| h.name()));
        assert!(header.is_some(), "Content-Disposition header not found");
        
        // Try with typed_header instead
        let typed_header = request.typed_header::<ContentDisposition>();
        if let Some(content_disp) = typed_header {
            // Check for correct disposition type
            assert_eq!(content_disp.disposition_type, DispositionType::Session, 
                      "Expected disposition type 'session', got '{:?}'", content_disp.disposition_type);
            
            // Check for the handling parameter
            let handling = content_disp.params.get("handling");
            assert_eq!(handling, Some(&"optional".to_string()), 
                      "Expected handling parameter 'optional', got '{:?}'", handling);
        } else {
            panic!("Expected Content-Disposition header via typed_header but got None");
        }
    }
    
    #[test]
    fn test_content_disposition_render() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_disposition_render("required")
            .build();
            
        // Check if Content-Disposition header exists with the correct value
        let header = request.header(&HeaderName::ContentDisposition);
        assert!(header.is_some(), "Content-Disposition header not found");
        
        // Try with typed_header instead
        let typed_header = request.typed_header::<ContentDisposition>();
        if let Some(content_disp) = typed_header {
            // Check for correct disposition type
            assert_eq!(content_disp.disposition_type, DispositionType::Render, 
                      "Expected disposition type 'render', got '{:?}'", content_disp.disposition_type);
            
            // Check for the handling parameter
            let handling = content_disp.params.get("handling");
            assert_eq!(handling, Some(&"required".to_string()), 
                      "Expected handling parameter 'required', got '{:?}'", handling);
        } else {
            panic!("Expected Content-Disposition header via typed_header but got None");
        }
    }
    
    #[test]
    fn test_content_disposition_icon() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_disposition_icon("32")
            .build();
            
        // Check if Content-Disposition header exists with the correct value
        let header = request.header(&HeaderName::ContentDisposition);
        assert!(header.is_some(), "Content-Disposition header not found");
        
        // Try with typed_header instead
        let typed_header = request.typed_header::<ContentDisposition>();
        if let Some(content_disp) = typed_header {
            // Check for correct disposition type
            assert_eq!(content_disp.disposition_type, DispositionType::Icon, 
                      "Expected disposition type 'icon', got '{:?}'", content_disp.disposition_type);
            
            // Check for the size parameter
            let size = content_disp.params.get("size");
            assert_eq!(size, Some(&"32".to_string()), 
                      "Expected size parameter '32', got '{:?}'", size);
        } else {
            panic!("Expected Content-Disposition header via typed_header but got None");
        }
    }
    
    #[test]
    fn test_content_disposition_custom() {
        let mut params = HashMap::new();
        params.insert("handling".to_string(), "optional".to_string());
        params.insert("custom".to_string(), "value".to_string());
        
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_disposition("custom-disp", params)
            .build();
            
        // Check if Content-Disposition header exists with the correct value
        let header = request.header(&HeaderName::ContentDisposition);
        assert!(header.is_some(), "Content-Disposition header not found");
        
        // Try with typed_header instead
        let typed_header = request.typed_header::<ContentDisposition>();
        if let Some(content_disp) = typed_header {
            // Check for correct disposition type
            assert_eq!(content_disp.disposition_type, DispositionType::Other("custom-disp".to_string()), 
                      "Expected disposition type 'custom-disp', got '{:?}'", content_disp.disposition_type);
            
            // Check for the handling parameter
            let handling = content_disp.params.get("handling");
            assert_eq!(handling, Some(&"optional".to_string()), 
                      "Expected handling parameter 'optional', got '{:?}'", handling);
            
            // Check for the custom parameter
            let custom = content_disp.params.get("custom");
            assert_eq!(custom, Some(&"value".to_string()), 
                      "Expected custom parameter 'value', got '{:?}'", custom);
        } else {
            panic!("Expected Content-Disposition header via typed_header but got None");
        }
    }
} 