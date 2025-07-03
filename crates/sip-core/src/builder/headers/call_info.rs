use crate::types::{
    call_info::{CallInfo, CallInfoValue, InfoPurpose},
    TypedHeader,
    uri::Uri,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;
use std::str::FromStr;
/// Call-Info header builder
///
/// This module provides builder methods for the Call-Info header in SIP messages.
///
/// ## SIP Call-Info Header Overview
///
/// The Call-Info header is defined in [RFC 3261 Section 20.9](https://datatracker.ietf.org/doc/html/rfc3261#section-20.9)
/// as part of the core SIP protocol. It provides additional information about the caller,
/// callee, or call context by referencing external URIs. Each Call-Info entry includes
/// a URI and an optional "purpose" parameter that describes the type of information.
///
/// ## Purpose of Call-Info Header
///
/// The Call-Info header serves several important purposes in SIP:
///
/// 1. It provides caller identification information (photos, business cards)
/// 2. It enables enhanced call screening with visual information about callers
/// 3. It allows for sharing supplementary information during a call
/// 4. It enables integration with external systems and resources
/// 5. It supports multimedia call context in enterprise and customer service scenarios
///
/// ## Common Purpose Values
///
/// - **icon**: A small image of the caller (e.g., avatar or photo)
/// - **info**: General information about the caller or call
/// - **card**: A business card or contact information (e.g., vCard)
/// - **answer**: Specifies a URI to redirect an incoming call to auto-answer
/// - **language**: Indicates preferred languages for the call
///
/// ## Real-world Applications
///
/// - **Contact Centers**: Displaying customer information to agents
/// - **Enterprise PBX**: Showing caller photos and department information
/// - **Telehealth**: Providing patient records during consultations
/// - **Emergency Services**: Including location or contextual information
/// - **Financial Services**: Displaying account information during calls
///
/// ## Security Considerations
///
/// When implementing Call-Info headers, consider:
///
/// - Call-Info URIs may be fetched automatically by receiving UAs
/// - External resources should be properly secured (HTTPS)
/// - Privacy implications of sharing caller/callee information
/// - Potential information disclosure to unauthorized parties
///
/// # Examples
///
/// ## Enterprise Calling with Caller Information
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::CallInfoBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Enterprise PBX call with enhanced caller ID information
///
/// // Create an INVITE with caller's photo and business card
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice Smith", "sip:alice@company.com", Some("a73kszlfl"))
///     .to("Bob Jones", "sip:bob@example.com", None)
///     .contact("<sip:alice@192.0.2.1:5060>", None)
///     // Add caller's photo for display on receiving device
///     .call_info_uri("https://company.com/employees/alice/photo.jpg", Some("icon"))
///     // Add caller's business card information
///     .call_info_uri("https://company.com/employees/alice/vcard.vcf", Some("card"))
///     // Add caller's department information
///     .call_info_uri("https://company.com/departments/marketing.html", Some("info"))
///     .build();
///
/// // Bob's phone can display Alice's photo and offer access to her
/// // business card and department information before answering
/// ```
///
/// ## Contact Center with Customer Information
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::CallInfoBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Contact center routing call with customer context
///
/// // Create an INVITE with customer information
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:agent@contact-center.example.com").unwrap()
///     .from("Customer", "sip:+15551234567@example.com", Some("call123"))
///     .to("Support", "sip:agent@contact-center.example.com", None)
///     .contact("<sip:gateway@203.0.113.1:5060>", None)
///     // Add link to customer profile in CRM system
///     .call_info_uri("https://crm.example.com/customer/1234567", Some("info"))
///     // Add customer service history
///     .call_info_uri("https://crm.example.com/customer/1234567/history", Some("info"))
///     // Add customer's account status
///     .call_info_uri("https://crm.example.com/customer/1234567/status", Some("info"))
///     .build();
///
/// // The agent's screen can automatically display the customer's
/// // profile and history when the call arrives
/// ```
///
/// ## Healthcare Appointment with Patient Information
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::CallInfoBuilderExt;
/// use rvoip_sip_core::builder::headers::cseq::CSeqBuilderExt;
/// use rvoip_sip_core::types::{StatusCode, Method};
///
/// // Scenario: Telehealth system connecting doctor with patient records
///
/// // Create a 200 OK response with patient information
/// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .from("Dr. Smith", "sip:doctor@hospital.example.org", Some("xyz123"))
///     .to("Patient", "sip:patient@example.com", Some("abc456"))
///     .cseq_with_method(42, Method::Invite)
///     // Add link to patient's electronic health record
///     .call_info_uri("https://ehr.hospital.example.org/patient/987654", Some("info"))
///     // Add patient's recent lab results
///     .call_info_uri("https://ehr.hospital.example.org/patient/987654/labs", Some("info"))
///     // Add patient's appointment details
///     .call_info_uri("https://ehr.hospital.example.org/appointments/12345", Some("info"))
///     .build();
///
/// // The doctor's telehealth application can display the patient's
/// // medical information during the video consultation
/// ```
pub trait CallInfoBuilderExt {
    /// Add a Call-Info header using a URI string
    ///
    /// Creates and adds a Call-Info header with a URI pointing to additional information
    /// about the caller, callee, or call context. The optional purpose parameter indicates
    /// the type of information provided by the URI.
    ///
    /// # Parameters
    /// - `uri`: The URI string pointing to the additional information (should use HTTPS for security)
    /// - `purpose`: Optional purpose parameter indicating the type of information:
    ///   - "icon": A small image or avatar of the caller/callee
    ///   - "info": General information about the caller/callee or call
    ///   - "card": Business card or contact information (vCard)
    ///   - Any other string will be treated as a custom purpose
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Note
    /// If the URI cannot be parsed, this method will silently fail.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::CallInfoBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Add a caller's photo to an INVITE request
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     // Add caller's photo for display during incoming call
    ///     .call_info_uri("https://example.com/users/alice/photo.jpg", Some("icon"))
    ///     .build();
    ///
    /// // Bob's phone can display Alice's photo when the call arrives
    /// ```
    fn call_info_uri(self, uri: &str, purpose: Option<&str>) -> Self;

    /// Add a Call-Info header with a pre-constructed CallInfoValue
    ///
    /// This method allows for more control when adding a Call-Info header by using
    /// a pre-constructed CallInfoValue object. This is useful when advanced parameters
    /// or custom purposes are needed.
    ///
    /// # Parameters
    /// - `value`: A pre-constructed CallInfoValue object
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::CallInfoBuilderExt;
    /// use rvoip_sip_core::types::{Method, call_info::{CallInfoValue, InfoPurpose}, uri::Uri};
    /// use std::str::FromStr;
    ///
    /// // Create an INVITE with custom Call-Info
    /// let uri = Uri::from_str("https://example.com/meeting/12345").unwrap();
    /// let call_info = CallInfoValue::new(uri)
    ///     .with_purpose(InfoPurpose::Other("meeting-context".to_string()));
    ///
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:conference@example.com").unwrap()
    ///     .from("Organizer", "sip:organizer@example.com", None)
    ///     .to("Conference", "sip:conference@example.com", None)
    ///     // Add meeting context information
    ///     .call_info_value(call_info)
    ///     .build();
    ///
    /// // The conference system can use the meeting context URL
    /// // to load relevant documents and information
    /// ```
    fn call_info_value(self, value: CallInfoValue) -> Self;

    /// Add a Call-Info header with multiple values
    ///
    /// This method adds a single Call-Info header with multiple values. In SIP,
    /// this is represented as a comma-separated list of values in a single header.
    /// This is more efficient than adding multiple separate Call-Info headers.
    ///
    /// # Parameters
    /// - `values`: A vector of CallInfoValue objects
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::CallInfoBuilderExt;
    /// use rvoip_sip_core::types::{Method, call_info::{CallInfoValue, InfoPurpose}, uri::Uri};
    /// use std::str::FromStr;
    ///
    /// // Create an INVITE with multiple Call-Info values in a single header
    /// let photo_uri = Uri::from_str("https://example.com/alice/photo.jpg").unwrap();
    /// let card_uri = Uri::from_str("https://example.com/alice/card.vcf").unwrap();
    /// let profile_uri = Uri::from_str("https://example.com/alice/profile.html").unwrap();
    ///
    /// let photo_info = CallInfoValue::new(photo_uri).with_purpose(InfoPurpose::Icon);
    /// let card_info = CallInfoValue::new(card_uri).with_purpose(InfoPurpose::Card);
    /// let profile_info = CallInfoValue::new(profile_uri).with_purpose(InfoPurpose::Info);
    ///
    /// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     // Add all caller information in a single header
    ///     .call_info_values(vec![photo_info, card_info, profile_info])
    ///     .build();
    ///
    /// // Results in a single Call-Info header with comma-separated values
    /// // instead of three separate Call-Info headers
    /// ```
    fn call_info_values(self, values: Vec<CallInfoValue>) -> Self;
}

impl CallInfoBuilderExt for SimpleRequestBuilder {
    fn call_info_uri(self, uri: &str, purpose: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(parsed_uri) => {
                let mut value = CallInfoValue::new(parsed_uri);
                
                if let Some(purpose_str) = purpose {
                    let purpose_enum = match purpose_str {
                        "icon" => InfoPurpose::Icon,
                        "info" => InfoPurpose::Info,
                        "card" => InfoPurpose::Card,
                        other => InfoPurpose::Other(other.to_string()),
                    };
                    value = value.with_purpose(purpose_enum);
                }
                
                self.call_info_value(value)
            },
            Err(_) => self // Silently fail if URI is invalid
        }
    }
    
    fn call_info_value(self, value: CallInfoValue) -> Self {
        self.header(TypedHeader::CallInfo(CallInfo::with_value(value)))
    }
    
    fn call_info_values(self, values: Vec<CallInfoValue>) -> Self {
        if values.is_empty() {
            return self;
        }
        self.header(TypedHeader::CallInfo(CallInfo::new(values)))
    }
}

impl CallInfoBuilderExt for SimpleResponseBuilder {
    fn call_info_uri(self, uri: &str, purpose: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(parsed_uri) => {
                let mut value = CallInfoValue::new(parsed_uri);
                
                if let Some(purpose_str) = purpose {
                    let purpose_enum = match purpose_str {
                        "icon" => InfoPurpose::Icon,
                        "info" => InfoPurpose::Info,
                        "card" => InfoPurpose::Card,
                        other => InfoPurpose::Other(other.to_string()),
                    };
                    value = value.with_purpose(purpose_enum);
                }
                
                self.call_info_value(value)
            },
            Err(_) => self // Silently fail if URI is invalid
        }
    }
    
    fn call_info_value(self, value: CallInfoValue) -> Self {
        self.header(TypedHeader::CallInfo(CallInfo::with_value(value)))
    }
    
    fn call_info_values(self, values: Vec<CallInfoValue>) -> Self {
        if values.is_empty() {
            return self;
        }
        self.header(TypedHeader::CallInfo(CallInfo::new(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    use crate::types::headers::HeaderAccess;
    use crate::builder::headers::cseq::CSeqBuilderExt;
    use crate::builder::headers::from::FromBuilderExt;
    use crate::builder::headers::to::ToBuilderExt;

    #[test]
    fn test_request_call_info_uri() {
        let uri = "http://example.com/alice/photo.jpg";
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .call_info_uri(uri, Some("icon"))
            .build();
            
        let call_info_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallInfo(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_info_headers.len(), 1);
        assert_eq!(call_info_headers[0].0.len(), 1);
        assert_eq!(call_info_headers[0].0[0].uri.to_string(), uri);
        assert_eq!(call_info_headers[0].0[0].purpose(), Some(InfoPurpose::Icon));
    }
    
    #[test]
    fn test_response_call_info_uri() {
        let uri = "http://example.com/alice/photo.jpg";
        
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .cseq_with_method(101, Method::Invite)
            .call_info_uri(uri, Some("icon"))
            .build();
            
        let call_info_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallInfo(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_info_headers.len(), 1);
        assert_eq!(call_info_headers[0].0.len(), 1);
        assert_eq!(call_info_headers[0].0[0].uri.to_string(), uri);
        assert_eq!(call_info_headers[0].0[0].purpose(), Some(InfoPurpose::Icon));
    }
    
    #[test]
    fn test_request_call_info_multiple_purposes() {
        let icon_uri = "http://example.com/alice/photo.jpg";
        let card_uri = "http://example.com/alice/card.html";
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .call_info_uri(icon_uri, Some("icon"))
            .call_info_uri(card_uri, Some("card"))
            .build();
            
        let call_info_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallInfo(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        // Note: The implementation adds a new Call-Info header for each call to call_info_uri,
        // rather than appending values to an existing header. This is consistent with SIP
        // which allows multiple instances of the same header.
        assert_eq!(call_info_headers.len(), 2);
        
        // Find the icon header
        let icon_header = call_info_headers.iter()
            .find(|h| h.0[0].purpose() == Some(InfoPurpose::Icon))
            .unwrap();
        assert_eq!(icon_header.0[0].uri.to_string(), icon_uri);
        
        // Find the card header
        let card_header = call_info_headers.iter()
            .find(|h| h.0[0].purpose() == Some(InfoPurpose::Card))
            .unwrap();
        assert_eq!(card_header.0[0].uri.to_string(), card_uri);
    }
    
    #[test]
    fn test_request_call_info_values() {
        let icon_uri = Uri::from_str("http://example.com/alice/photo.jpg").unwrap();
        let card_uri = Uri::from_str("http://example.com/alice/card.html").unwrap();
        
        let icon_value = CallInfoValue::new(icon_uri).with_purpose(InfoPurpose::Icon);
        let card_value = CallInfoValue::new(card_uri).with_purpose(InfoPurpose::Card);
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .call_info_values(vec![icon_value, card_value])
            .build();
            
        let call_info_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallInfo(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_info_headers.len(), 1);
        assert_eq!(call_info_headers[0].0.len(), 2);
        
        // Check first value (icon)
        assert_eq!(call_info_headers[0].0[0].purpose(), Some(InfoPurpose::Icon));
        assert_eq!(call_info_headers[0].0[0].uri.to_string(), "http://example.com/alice/photo.jpg");
        
        // Check second value (card)
        assert_eq!(call_info_headers[0].0[1].purpose(), Some(InfoPurpose::Card));
        assert_eq!(call_info_headers[0].0[1].uri.to_string(), "http://example.com/alice/card.html");
    }
} 