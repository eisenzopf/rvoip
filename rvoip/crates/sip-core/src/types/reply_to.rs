use crate::types::address::Address; // Or maybe UriWithParams?
use crate::parser::headers::reply_to::parse_reply_to; // Use the parser
use crate::error::{Error, Result};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize}; // Add import

/// Typed Reply-To header.
/// 
/// Defined in RFC 3261 Section 20.32:
/// Reply-To = "Reply-To" HCOLON rplyto-spec
/// rplyto-spec = ( name-addr / addr-spec ) *( SEMI rplyto-param )
/// rplyto-param = generic-param
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] // Added Serialize, Deserialize
pub struct ReplyTo(pub Address); // Or UriWithParams

impl ReplyTo {
    /// Creates a new ReplyTo header.
    pub fn new(address: Address) -> Self {
        Self(address)
    }
    
    /// Access the underlying Address
    pub fn address(&self) -> &Address {
        &self.0
    }

    /// Access the URI from the Address
    pub fn uri(&self) -> &crate::types::uri::Uri {
        &self.0.uri
    }

    /// Access parameters from the Address
    pub fn params(&self) -> &[crate::types::param::Param] {
        &self.0.params
    }

    /// Check if a parameter is present (case-insensitive key)
    pub fn has_param(&self, key: &str) -> bool {
        self.0.has_param(key)
    }

    /// Get a parameter value (case-insensitive key)
    pub fn get_param(&self, key: &str) -> Option<Option<&str>> {
        self.0.get_param(key)
    }
    
    /// Validates the Reply-To header according to RFC 3261
    /// 
    /// While RFC 3261 doesn't specify many restrictions on Reply-To,
    /// this method performs basic validation to ensure URI scheme is valid
    /// and header parameters are properly formed.
    pub fn validate(&self) -> Result<()> {
        // Validate URI scheme is supported
        match self.0.uri.scheme {
            crate::types::uri::Scheme::Sip | 
            crate::types::uri::Scheme::Sips | 
            crate::types::uri::Scheme::Tel => Ok(()),
            _ => Err(Error::InvalidUri(format!("Unsupported scheme in Reply-To: {}", self.0.uri.scheme)))
        }
    }
    
    /// Add a parameter to this Reply-To header
    pub fn with_param(mut self, param: crate::types::param::Param) -> Self {
        self.0.params.push(param);
        self
    }
    
    /// Creates a new ReplyTo with a SIP URI
    pub fn sip(host: impl Into<String>, user: Option<impl Into<String>>) -> Result<Self> {
        let mut uri = crate::types::uri::Uri::sip(host);
        if let Some(u) = user {
            uri = uri.with_user(u);
        }
        let address = Address::new(None::<String>, uri);
        Ok(Self(address))
    }
    
    /// Creates a new ReplyTo with a SIPS URI
    pub fn sips(host: impl Into<String>, user: Option<impl Into<String>>) -> Result<Self> {
        let mut uri = crate::types::uri::Uri::sips(host);
        if let Some(u) = user {
            uri = uri.with_user(u);
        }
        let address = Address::new(None::<String>, uri);
        Ok(Self(address))
    }
    
    /// Creates a new ReplyTo with a TEL URI
    pub fn tel(number: impl Into<String>) -> Result<Self> {
        let uri = crate::types::uri::Uri::tel(number);
        let address = Address::new(None::<String>, uri);
        Ok(Self(address))
    }
    
    /// Creates a new ReplyTo with a display name
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.0.display_name = Some(name.into());
        self
    }
}

impl fmt::Display for ReplyTo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Address display
    }
}

impl FromStr for ReplyTo {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Use all_consuming to ensure entire input is parsed
        let result = all_consuming(parse_reply_to)(s.as_bytes())
            .map(|(_rem, header)| header)
            .map_err(|e| Error::from(e.to_owned()));
        
        // If parsing succeeded, validate the result
        if let Ok(reply_to) = &result {
            reply_to.validate()?;
        }
        
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::uri::{Uri, Scheme, Host};
    use crate::types::address::Address;
    use crate::types::param::{Param, GenericValue};
    use std::str::FromStr;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_reply_to_from_str() {
        let s = "\"Support\" <sip:support@example.com>;dept=billing";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert_eq!(reply_to.address().display_name, Some("Support".to_string()));
        assert_eq!(reply_to.uri().scheme, Scheme::Sip);
        assert_eq!(reply_to.params().len(), 1);
        assert!(reply_to.has_param("dept"));
        assert_eq!(reply_to.get_param("dept"), Some(Some("billing")));
    }
    
    #[test]
    fn test_reply_to_display() {
        let uri = Uri::from_str("sip:user@example.com").unwrap();
        let addr = Address::new(None::<String>, uri);
        let reply_to = ReplyTo::new(addr);
        
        assert_eq!(reply_to.to_string(), "<sip:user@example.com>");
    }
    
    #[test]
    fn test_reply_to_with_params() {
        let uri = Uri::from_str("sip:support@example.com").unwrap();
        let mut addr = Address::new(Some("Support".to_string()), uri);
        addr.params.push(Param::Other("dept".to_string(), Some(GenericValue::Token("sales".to_string()))));
        let reply_to = ReplyTo::new(addr);
        
        assert!(reply_to.has_param("dept"));
        assert_eq!(reply_to.get_param("dept"), Some(Some("sales")));
    }
    
    #[test]
    fn test_reply_to_with_ipv6() {
        // Testing with IPv6 address in URI
        let s = "<sip:[2001:db8::1]>";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        if let Host::Address(addr) = &reply_to.uri().host {
            if let IpAddr::V6(ipv6) = addr {
                // Convert expected address to bytes for comparison
                let expected = Ipv6Addr::from_str("2001:db8::1").unwrap();
                assert_eq!(ipv6.octets(), expected.octets());
            } else {
                panic!("Expected IPv6 address");
            }
        } else {
            panic!("Expected address type host");
        }
    }
    
    #[test]
    fn test_reply_to_with_tel_uri() {
        // Testing with tel URI scheme - TEL URIs store the number in the host part
        let s = "tel:+1-212-555-1234";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert_eq!(reply_to.uri().scheme, Scheme::Tel);
        // TEL URIs in our implementation store the number in the host field
        if let Host::Domain(number) = &reply_to.uri().host {
            assert_eq!(number, "+1-212-555-1234");
        } else {
            panic!("Expected domain type host for TEL URI");
        }
    }
    
    #[test]
    fn test_reply_to_with_special_display_name() {
        // Display name with characters requiring escaping
        let s = "\"Support Team @ Example, Inc.\" <sip:support@example.com>";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert_eq!(reply_to.address().display_name, Some("Support Team @ Example, Inc.".to_string()));
        assert_eq!(reply_to.to_string(), s);
    }
    
    #[test]
    fn test_reply_to_with_quoted_param() {
        // Parameter with quoted string value
        let s = "<sip:support@example.com>;note=\"Call us at 24/7 service\"";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert!(reply_to.has_param("note"));
        assert_eq!(reply_to.get_param("note"), Some(Some("Call us at 24/7 service")));
    }
    
    #[test]
    fn test_reply_to_with_multiple_params_same_name() {
        // Parameters with the same name in URI and header
        let s = "<sip:support@example.com;priority=low>;priority=high";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        // URI parameters
        assert!(reply_to.uri().parameters.iter().any(|p| 
            matches!(p, Param::Other(k, Some(GenericValue::Token(v))) 
                if k == "priority" && v == "low")
        ));
        
        // Header parameters
        assert!(reply_to.has_param("priority"));
        assert_eq!(reply_to.get_param("priority"), Some(Some("high")));
    }
    
    #[test]
    fn test_reply_to_with_escaped_display_name() {
        // Display name with escaped quotes
        let s = "\"Support\\\"Team\" <sip:support@example.com>";
        let reply_to = ReplyTo::from_str(s).unwrap();
        
        assert_eq!(reply_to.address().display_name, Some("Support\"Team".to_string()));
    }
    
    #[test]
    fn test_reply_to_factory_methods() {
        // Test the factory methods for creating ReplyTo objects
        let reply_to = ReplyTo::sip("example.com", Some("support")).unwrap();
        assert_eq!(reply_to.uri().scheme, Scheme::Sip);
        assert_eq!(reply_to.uri().user, Some("support".to_string()));
        
        let reply_to = ReplyTo::sips("secure.example.com", Some("secure")).unwrap();
        assert_eq!(reply_to.uri().scheme, Scheme::Sips);
        
        let reply_to = ReplyTo::tel("+1-212-555-1234").unwrap();
        assert_eq!(reply_to.uri().scheme, Scheme::Tel);
        
        let reply_to = ReplyTo::sip("example.com", Some("support"))
            .unwrap()
            .with_display_name("Support Team")
            .with_param(Param::Other("department".to_string(), Some(GenericValue::Token("sales".to_string()))));
            
        assert_eq!(reply_to.address().display_name, Some("Support Team".to_string()));
        assert!(reply_to.has_param("department"));
    }
    
    #[test]
    fn test_reply_to_validation() {
        // Test validation of supported schemes
        let uri = Uri::from_str("http://example.com").unwrap();
        let addr = Address::new(None::<String>, uri);
        let reply_to = ReplyTo::new(addr);
        
        assert!(reply_to.validate().is_err());
    }
    
    #[test]
    fn test_malformed_reply_to() {
        // Test handling of malformed Reply-To headers
        assert!(ReplyTo::from_str("").is_err());
        assert!(ReplyTo::from_str("<>").is_err());
        assert!(ReplyTo::from_str("<sip:>").is_err());
        assert!(ReplyTo::from_str("<sip:@>").is_err());
        
        // Invalid scheme
        assert!(ReplyTo::from_str("<invalid:user@example.com>").is_err());
        
        // Malformed parameters
        assert!(ReplyTo::from_str("<sip:user@example.com>;=value").is_err());
        assert!(ReplyTo::from_str("<sip:user@example.com>;key=").is_err());
    }
}

// TODO: Implement methods if needed 