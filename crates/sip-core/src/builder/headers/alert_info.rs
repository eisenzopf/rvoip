use crate::error::{Error, Result};
use crate::types::{
    alert_info::{AlertInfo, AlertInfoHeader, AlertInfoList},
    uri::Uri,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;
use std::str::FromStr;

/// Alert-Info header builder
///
/// This module provides builder methods for the Alert-Info header in SIP messages.
///
/// ## SIP Alert-Info Header Overview
///
/// The Alert-Info header is defined in [RFC 3261 Section 20.4](https://datatracker.ietf.org/doc/html/rfc3261#section-20.4)
/// as part of the core SIP protocol. It allows a server to provide information about alternative
/// ring tones to be used by the user agent.
///
/// ## Format
///
/// ```text
/// Alert-Info: <http://www.example.com/sounds/moo.wav>
/// Alert-Info: <http://www.example.com/sounds/moo.wav>;appearance=2
/// ```
///
/// ## Purpose of Alert-Info Header
///
/// The Alert-Info header serves several specific purposes in SIP:
///
/// 1. In INVITE requests, it specifies an alternative ring tone to be used by the UAS
/// 2. In 180 (Ringing) responses, it specifies an alternative ringback tone to be used by the UAC
/// 3. It allows for rich caller experience customization with audio cues
/// 4. It enables branding of calls with organization-specific audio
///
/// ## Common Parameters
///
/// - **appearance**: Specifies the appearance index for multi-line phones
/// - **info**: Provides additional information about the alert
/// - **delay**: Specifies a delay before playing the alert tone
///
/// ## Examples
///
/// ## INVITE with Custom Ring Tone
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AlertInfoBuilderExt};
///
/// // Scenario: Calling a contact with a custom ring tone
///
/// // Create an INVITE request with a custom ring tone
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.com", None)
///     .contact("<sip:alice@192.168.1.2:5060>", None)
///     // Specify a custom ringtone URL
///     .alert_info_uri("http://www.example.com/sounds/moo.wav")
///     .build();
///
/// // Bob's phone will play the custom ringtone instead of the default one
/// ```
///
/// ## Ringing Response with Custom Ringback Tone
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AlertInfoBuilderExt};
///
/// // Scenario: Providing a custom ringback tone
///
/// // Create a 180 Ringing response with a custom ringback tone
/// let ringing = SimpleResponseBuilder::new(StatusCode::Ringing, Some("Ringing"))
///     .from("Bob", "sip:bob@example.com", Some("b5qt9xl3"))
///     .to("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     // Specify a custom ringback tone URL with an appearance parameter
///     .alert_info_uri_with_param("http://www.example.com/sounds/ringback.wav", "appearance", "2")
///     .build();
///
/// // Alice's phone will play this ringback tone while waiting for Bob to answer
/// ```
pub trait AlertInfoBuilderExt {
    /// Add an Alert-Info header with the specified URI
    ///
    /// This method adds an Alert-Info header with the given URI pointing to an alert resource,
    /// such as a custom ring tone or ringback tone.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI pointing to the alert resource
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Alert-Info header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AlertInfoBuilderExt};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// // Create a request with a custom ringtone
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("tag123"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .alert_info(uri)
    ///     .build();
    ///
    /// // The SIP message now includes the Alert-Info header with the specified URI
    /// ```
    fn alert_info(self, uri: Uri) -> Self;
    
    /// Add an Alert-Info header with a URI parsed from a string
    ///
    /// This method adds an Alert-Info header by parsing the provided URI string.
    ///
    /// # Parameters
    ///
    /// * `uri_str` - The URI string pointing to the alert resource
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Alert-Info header added
    /// 
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AlertInfoBuilderExt};
    ///
    /// // Create a request with a custom ringtone using a string URI
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("tag123"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .alert_info_uri("http://www.example.com/sounds/moo.wav")
    ///     .build();
    ///
    /// // The SIP message now includes the Alert-Info header with the specified URI
    /// ```
    fn alert_info_uri(self, uri_str: &str) -> Self;
    
    /// Add an Alert-Info header with a URI and a single parameter
    ///
    /// This method adds an Alert-Info header with the given URI and a single parameter,
    /// such as an appearance index or other metadata about the alert.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI pointing to the alert resource
    /// * `param_name` - The parameter name
    /// * `param_value` - The parameter value
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Alert-Info header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AlertInfoBuilderExt};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// // Create a request with a custom ringtone and appearance parameter
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("tag123"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .alert_info_with_param(uri, "appearance", "2")
    ///     .build();
    ///
    /// // The SIP message now includes the Alert-Info header with the specified URI and parameter
    /// ```
    fn alert_info_with_param(self, uri: Uri, param_name: &str, param_value: &str) -> Self;
    
    /// Add an Alert-Info header with a URI string and a single parameter
    ///
    /// This method adds an Alert-Info header by parsing the provided URI string and
    /// adding a single parameter, such as an appearance index or other metadata.
    ///
    /// # Parameters
    ///
    /// * `uri_str` - The URI string pointing to the alert resource
    /// * `param_name` - The parameter name
    /// * `param_value` - The parameter value
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Alert-Info header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AlertInfoBuilderExt};
    ///
    /// // Create a request with a custom ringtone and appearance parameter using a string URI
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("tag123"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .alert_info_uri_with_param("http://www.example.com/sounds/moo.wav", "appearance", "2")
    ///     .build();
    ///
    /// // The SIP message now includes the Alert-Info header with the specified URI and parameter
    /// ```
    fn alert_info_uri_with_param(self, uri_str: &str, param_name: &str, param_value: &str) -> Self;
    
    /// Add an Alert-Info header with a URI and multiple parameters
    ///
    /// This method adds an Alert-Info header with the given URI and multiple parameters
    /// specified as key-value pairs.
    ///
    /// # Parameters
    ///
    /// * `uri` - The URI pointing to the alert resource
    /// * `params` - A vector of parameter (name, value) tuples
    ///
    /// # Returns
    ///
    /// * `Self` - The builder with the Alert-Info header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AlertInfoBuilderExt};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// // Create a request with a custom ringtone and multiple parameters
    /// let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("tag123"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .alert_info_with_params(
    ///         uri, 
    ///         vec![
    ///             ("appearance", "2"),
    ///             ("intensity", "high")
    ///         ]
    ///     )
    ///     .build();
    ///
    /// // The SIP message now includes the Alert-Info header with the specified URI and parameters
    /// ```
    fn alert_info_with_params(self, uri: Uri, params: Vec<(&str, &str)>) -> Self;
}

impl<T> AlertInfoBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn alert_info(self, uri: Uri) -> Self {
        let alert_info = AlertInfo::new(uri);
        let header = AlertInfoHeader::new().with_alert_info(alert_info);
        self.set_header(header)
    }
    
    fn alert_info_uri(self, uri_str: &str) -> Self {
        match Uri::from_str(uri_str) {
            Ok(uri) => self.alert_info(uri),
            Err(_) => self // Silently handle parsing errors for builder fluency
        }
    }
    
    fn alert_info_with_param(self, uri: Uri, param_name: &str, param_value: &str) -> Self {
        let alert_info = AlertInfo::new(uri).with_param(param_name, param_value);
        let header = AlertInfoHeader::new().with_alert_info(alert_info);
        self.set_header(header)
    }
    
    fn alert_info_uri_with_param(self, uri_str: &str, param_name: &str, param_value: &str) -> Self {
        match Uri::from_str(uri_str) {
            Ok(uri) => self.alert_info_with_param(uri, param_name, param_value),
            Err(_) => self // Silently handle parsing errors for builder fluency
        }
    }
    
    fn alert_info_with_params(self, uri: Uri, params: Vec<(&str, &str)>) -> Self {
        let mut alert_info = AlertInfo::new(uri);
        for (name, value) in params {
            alert_info = alert_info.with_param(name, value);
        }
        let header = AlertInfoHeader::new().with_alert_info(alert_info);
        self.set_header(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_alert_info_basic() {
        let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
        let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .alert_info(uri.clone())
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::AlertInfo(header)) = request.header(&HeaderName::AlertInfo) {
            assert_eq!(header.alert_info_list.len(), 1);
            assert_eq!(header.alert_info_list.items[0].uri(), &uri);
        } else {
            panic!("Alert-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_alert_info_uri() {
        let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .alert_info_uri("http://www.example.com/sounds/moo.wav")
            .build();
            
        if let Some(TypedHeader::AlertInfo(header)) = request.header(&HeaderName::AlertInfo) {
            assert_eq!(header.alert_info_list.len(), 1);
            assert_eq!(
                header.alert_info_list.items[0].uri().to_string(),
                "http://www.example.com/sounds/moo.wav"
            );
        } else {
            panic!("Alert-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_alert_info_with_param() {
        let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
        let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .alert_info_with_param(uri.clone(), "appearance", "2")
            .build();
            
        if let Some(TypedHeader::AlertInfo(header)) = request.header(&HeaderName::AlertInfo) {
            assert_eq!(header.alert_info_list.len(), 1);
            assert_eq!(header.alert_info_list.items[0].uri(), &uri);
            assert_eq!(
                header.alert_info_list.items[0].get_param("appearance"),
                Some("2")
            );
        } else {
            panic!("Alert-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_alert_info_uri_with_param() {
        let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .alert_info_uri_with_param("http://www.example.com/sounds/moo.wav", "appearance", "2")
            .build();
            
        if let Some(TypedHeader::AlertInfo(header)) = request.header(&HeaderName::AlertInfo) {
            assert_eq!(header.alert_info_list.len(), 1);
            assert_eq!(
                header.alert_info_list.items[0].uri().to_string(),
                "http://www.example.com/sounds/moo.wav"
            );
            assert_eq!(
                header.alert_info_list.items[0].get_param("appearance"),
                Some("2")
            );
        } else {
            panic!("Alert-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_alert_info_with_params() {
        let uri = Uri::from_str("http://www.example.com/sounds/moo.wav").unwrap();
        let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .alert_info_with_params(
                uri.clone(),
                vec![
                    ("appearance", "2"),
                    ("intensity", "high")
                ]
            )
            .build();
            
        if let Some(TypedHeader::AlertInfo(header)) = request.header(&HeaderName::AlertInfo) {
            assert_eq!(header.alert_info_list.len(), 1);
            assert_eq!(header.alert_info_list.items[0].uri(), &uri);
            assert_eq!(
                header.alert_info_list.items[0].get_param("appearance"),
                Some("2")
            );
            assert_eq!(
                header.alert_info_list.items[0].get_param("intensity"),
                Some("high")
            );
        } else {
            panic!("Alert-Info header not found or has wrong type");
        }
    }

    #[test]
    fn test_invalid_uri() {
        // This should not add a header at all
        let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .alert_info_uri("not a valid uri")
            .build();
            
        assert_eq!(request.headers.len(), 0);
    }

    #[test]
    fn test_response_alert_info() {
        let uri = Uri::from_str("http://www.example.com/sounds/ringback.wav").unwrap();
        let response = ResponseBuilder::new(StatusCode::Ringing, Some("Ringing"))
            .alert_info_with_param(uri.clone(), "appearance", "2")
            .build();
            
        if let Some(TypedHeader::AlertInfo(header)) = response.header(&HeaderName::AlertInfo) {
            assert_eq!(header.alert_info_list.len(), 1);
            assert_eq!(header.alert_info_list.items[0].uri(), &uri);
            assert_eq!(
                header.alert_info_list.items[0].get_param("appearance"),
                Some("2")
            );
        } else {
            panic!("Alert-Info header not found or has wrong type");
        }
    }
} 