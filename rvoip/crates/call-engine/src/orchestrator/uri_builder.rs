//! SIP URI builder for the call center
//!
//! This module provides centralized URI generation to eliminate hardcoded
//! IP addresses and domains throughout the codebase.

use crate::config::GeneralConfig;

/// SIP URI builder that uses configuration to generate URIs
pub struct SipUriBuilder<'a> {
    config: &'a GeneralConfig,
}

impl<'a> SipUriBuilder<'a> {
    /// Create a new URI builder with the given configuration
    pub fn new(config: &'a GeneralConfig) -> Self {
        Self { config }
    }

    /// Generate agent SIP URI from username
    pub fn agent_uri(&self, username: &str) -> String {
        self.config.agent_sip_uri(username)
    }

    /// Generate call center SIP URI
    pub fn call_center_uri(&self) -> String {
        self.config.call_center_uri()
    }

    /// Generate registrar URI
    pub fn registrar_uri(&self) -> String {
        self.config.registrar_uri()
    }

    /// Generate contact URI for an agent with optional port
    pub fn contact_uri(&self, username: &str, port: Option<u16>) -> String {
        self.config.agent_contact_uri(username, port)
    }

    /// Generate agent URI with fallback to contact URI if available
    pub fn agent_uri_with_fallback(&self, username: &str, contact_uri: Option<&str>) -> String {
        contact_uri
            .map(|uri| uri.to_string())
            .unwrap_or_else(|| self.agent_uri(username))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GeneralConfig;

    #[test]
    fn test_uri_builder_basic() {
        let config = GeneralConfig {
            local_ip: "192.168.1.100".to_string(),
            domain: "example.com".to_string(),
            registrar_domain: "registrar.example.com".to_string(),
            call_center_service: "cc".to_string(),
            ..Default::default()
        };

        let builder = SipUriBuilder::new(&config);

        assert_eq!(builder.agent_uri("alice"), "sip:alice@192.168.1.100");
        assert_eq!(builder.call_center_uri(), "sip:cc@example.com");
        assert_eq!(builder.registrar_uri(), "sip:registrar@registrar.example.com");
        assert_eq!(builder.contact_uri("alice", Some(5071)), "sip:alice@192.168.1.100:5071");
        assert_eq!(builder.contact_uri("alice", None), "sip:alice@192.168.1.100");
    }

    #[test]
    fn test_uri_builder_with_fallback() {
        let config = GeneralConfig::default();
        let builder = SipUriBuilder::new(&config);

        // Test with contact URI provided
        let contact_uri = Some("sip:alice@192.168.1.100:5071");
        assert_eq!(
            builder.agent_uri_with_fallback("alice", contact_uri),
            "sip:alice@192.168.1.100:5071"
        );

        // Test with no contact URI (fallback to generated)
        assert_eq!(
            builder.agent_uri_with_fallback("alice", None),
            "sip:alice@127.0.0.1"
        );
    }
} 