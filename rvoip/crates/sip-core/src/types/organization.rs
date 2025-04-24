use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::header::{HeaderValue, HeaderName, Header, TypedHeaderTrait};
use crate::parser::headers::organization::parse_organization;

/// Represents the Organization header field as defined in RFC 3261 Section 20.27
/// 
/// The Organization header field indicates the identity of the organizational entity 
/// associated with the user agent (for example, "Rudeless Ventures").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Organization(pub String);

impl Organization {
    /// Creates a new Organization header value
    pub fn new<S: Into<String>>(org: S) -> Self {
        Organization(org.into())
    }

    /// Returns the organization name as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Organization {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse the input as bytes
        let bytes = s.as_bytes();
        let (_, org) = parse_organization(bytes)
            .map_err(|_| crate::error::Error::ParseError("Failed to parse Organization header".into()))?;
        
        // Return the parsed Organization
        Ok(org)
    }
}

impl fmt::Display for Organization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TypedHeaderTrait for Organization {
    type Name = HeaderName;
    
    fn header_name() -> Self::Name {
        HeaderName::Organization
    }
    
    fn to_header(&self) -> Header {
        Header::text(HeaderName::Organization, &self.0)
    }
    
    fn from_header(header: &Header) -> Result<Self, crate::error::Error> {
        if header.name != HeaderName::Organization {
            return Err(crate::error::Error::ParseError(
                format!("Expected Organization header, got {}", header.name)
            ));
        }
        
        // Get the value as a string
        if let Some(text) = header.value.as_text() {
            Ok(Organization(text.to_string()))
        } else {
            Err(crate::error::Error::ParseError("Invalid Organization header value".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_organization_parsing() {
        let test_cases = [
            "Rudeless Ventures",
            "Example Corp.",
            "IETF SIP Working Group",
            "",
            "Acme ÜÖ GmbH", // UTF-8 characters
        ];

        for test_case in test_cases {
            let org = Organization::from_str(test_case).unwrap();
            assert_eq!(org.as_str(), test_case);
            
            // Test TypedHeaderTrait implementation
            let header = Header::text(HeaderName::Organization, test_case);
            let parsed_org = Organization::from_header(&header).unwrap();
            assert_eq!(parsed_org, org);
            
            // Test conversion back to Header
            let converted_header = org.to_header();
            assert_eq!(converted_header.name, HeaderName::Organization);
            assert_eq!(converted_header.value.as_text().unwrap(), test_case);
        }
    }

    #[test]
    fn test_organization_display() {
        let org = Organization::new("Rudeless Ventures");
        assert_eq!(format!("{}", org), "Rudeless Ventures");
    }
} 