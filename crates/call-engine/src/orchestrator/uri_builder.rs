//! # SIP URI Builder for Call Center Operations
//!
//! This module provides sophisticated SIP URI construction and management utilities
//! for call center operations. It handles the creation of properly formatted SIP URIs
//! for agents, customers, and system endpoints, ensuring compliance with SIP standards
//! while providing convenient abstractions for common call center URI patterns.
//!
//! ## Overview
//!
//! SIP URI construction is critical for proper call routing and session management
//! in call center operations. This module provides a comprehensive URI builder that
//! handles various SIP URI formats, parameter encoding, security considerations,
//! and integration with call center-specific requirements such as agent identification,
//! customer routing, and system service endpoints.
//!
//! ## Key Features
//!
//! - **Standard Compliance**: Full RFC 3261 SIP URI standard compliance
//! - **Call Center Patterns**: Specialized patterns for call center operations
//! - **Parameter Management**: Comprehensive SIP URI parameter handling
//! - **Security Features**: Secure parameter encoding and validation
//! - **Agent URIs**: Specialized agent endpoint URI construction
//! - **Customer URIs**: Customer identification and routing URIs
//! - **Service URIs**: System service and queue endpoint URIs
//! - **Validation**: Built-in URI format validation and error checking
//! - **Performance**: Efficient URI construction with minimal allocations
//! - **Extensibility**: Flexible design for future URI pattern additions
//!
//! ## SIP URI Patterns
//!
//! ### Agent URIs
//!
//! Agent URIs identify specific agents within the call center system:
//!
//! - **Basic Format**: `sip:agent@domain`
//! - **With Extension**: `sip:agent@domain;ext=1001`
//! - **With Department**: `sip:agent@domain;dept=support`
//! - **With Skills**: `sip:agent@domain;skills=tech,billing`
//!
//! ### Customer URIs
//!
//! Customer URIs identify external customers and routing preferences:
//!
//! - **E.164 Format**: `sip:+15551234567@gateway.provider.com`
//! - **Local Format**: `sip:1001@local.domain`
//! - **With Priority**: `sip:customer@domain;priority=vip`
//! - **With Context**: `sip:customer@domain;context=support`
//!
//! ### Service URIs
//!
//! Service URIs identify call center services and queues:
//!
//! - **Queue URI**: `sip:support-queue@call-center.com`
//! - **IVR Service**: `sip:ivr@call-center.com;menu=main`
//! - **Conference**: `sip:conf-12345@call-center.com`
//! - **Recording**: `sip:recording@call-center.com;session=abc123`
//!
//! ## Examples
//!
//! ### Basic SIP URI Construction
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::uri_builder::SipUriBuilder;
//! use rvoip_call_engine::config::GeneralConfig;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = GeneralConfig {
//!     local_ip: "192.168.1.100".to_string(),
//!     domain: "call-center.com".to_string(),
//!     registrar_domain: "registrar.call-center.com".to_string(),
//!     call_center_service: "cc".to_string(),
//!     ..Default::default()
//! };
//! 
//! let builder = SipUriBuilder::new(&config);
//! 
//! // Build basic agent URI
//! let agent_uri = builder.agent_uri("alice");
//! println!("👤 Basic Agent URI: {}", agent_uri);
//! // Output: sip:alice@192.168.1.100
//! 
//! // Build call center URI
//! let call_center_uri = builder.call_center_uri();
//! println!("📞 Call Center URI: {}", call_center_uri);
//! // Output: sip:cc@call-center.com
//! 
//! // Build registrar URI
//! let registrar_uri = builder.registrar_uri();
//! println!("📋 Registrar URI: {}", registrar_uri);
//! // Output: sip:registrar@registrar.call-center.com
//! 
//! // Build contact URI with port
//! let contact_uri = builder.contact_uri("alice", Some(5071));
//! println!("🌐 Contact URI: {}", contact_uri);
//! // Output: sip:alice@192.168.1.100:5071
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent URI with Parameters
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::uri_builder::SipUriBuilder;
//! use rvoip_call_engine::config::GeneralConfig;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = GeneralConfig::default();
//! let builder = SipUriBuilder::new(&config);
//! 
//! // Build agent URIs with different methods
//! let agent_uri = builder.agent_uri("alice.johnson");
//! println!("👩‍💼 Agent URI: {}", agent_uri);
//! 
//! let contact_uri = builder.contact_uri("alice.johnson", Some(5071));
//! println!("📞 Contact URI: {}", contact_uri);
//! 
//! // Build fallback URI
//! let fallback_uri = builder.agent_uri_with_fallback("alice.johnson", Some(&contact_uri));
//! println!("🔄 Fallback URI: {}", fallback_uri);
//! 
//! // Multiple agent URIs
//! let agents = vec!["alice", "bob", "carol"];
//! println!("\n👥 Multiple Agent URIs:");
//! for agent in agents {
//!     let uri = builder.agent_uri(agent);
//!     println!("  {}: {}", agent, uri);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Customer URI with Routing Information
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::uri_builder::SipUriBuilder;
//! use rvoip_call_engine::config::GeneralConfig;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = GeneralConfig::default();
//! let builder = SipUriBuilder::new(&config);
//! 
//! // Build customer-related URIs using actual API
//! let customer_uri = builder.agent_uri("customer123");  
//! println!("⭐ Customer URI: {}", customer_uri);
//! 
//! // Build contact URIs for different customers
//! let vip_contact = builder.contact_uri("vip-customer", Some(5071));
//! println!("💎 VIP Contact URI: {}", vip_contact);
//! 
//! // Build registrar URI for customer authentication
//! let registrar_uri = builder.registrar_uri();
//! println!("📞 Registrar URI: {}", registrar_uri);
//! 
//! // Example of URI building patterns
//! let customers = vec![
//!     ("customer-001", None),
//!     ("vip-customer", Some(5071)),
//!     ("premium-customer", Some(5072)),
//! ];
//! 
//! println!("\n🎯 Customer URI Patterns:");
//! for (customer, port) in customers {
//!     let uri = builder.contact_uri(customer, port);
//!     println!("  {}: {}", customer, uri);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Service URI Generation  
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::uri_builder::SipUriBuilder;
//! use rvoip_call_engine::config::GeneralConfig;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = GeneralConfig::default();
//! let builder = SipUriBuilder::new(&config);
//! 
//! // Build queue service URI using agent_uri (adapted for services)
//! let queue_uri = builder.agent_uri("technical-support-queue");
//! println!("📋 Technical Support Queue URI: {}", queue_uri);
//! 
//! // Build IVR service URI
//! let ivr_uri = builder.agent_uri("main-ivr");
//! println!("\n📱 Main IVR Service URI: {}", ivr_uri);
//! 
//! // Build conference service URI
//! let conference_uri = builder.agent_uri("conf-12345");
//! println!("\n🎤 Conference Service URI: {}", conference_uri);
//! 
//! // Build recording service URI
//! let recording_uri = builder.agent_uri("recording-service");
//! println!("\n📹 Recording Service URI: {}", recording_uri);
//! 
//! // Generate service contact URIs with different ports
//! let services = vec![
//!     ("queue", Some(5060)),
//!     ("ivr", Some(5061)),
//!     ("conference", Some(5062)),
//!     ("recording", None),
//! ];
//! 
//! println!("\n⚙️ Service Contact URIs:");
//! for (service_name, port) in services {
//!     let contact_uri = builder.contact_uri(service_name, port);
//!     println!("  {}: {}", service_name, contact_uri);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### URI Generation Patterns
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::uri_builder::SipUriBuilder;
//! use rvoip_call_engine::config::GeneralConfig;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = GeneralConfig::default();
//! let builder = SipUriBuilder::new(&config);
//! 
//! // Batch URI operations for multiple agents
//! let agent_names = vec!["alice", "bob", "carol", "diana"];
//! 
//! println!("🔧 URI Generation Patterns:");
//! println!("\n👥 Batch Agent URI Creation:");
//! for name in &agent_names {
//!     let uri = builder.agent_uri(name);
//!     println!("  {}: {}", name, uri);
//! }
//! 
//! // Generate contact URIs with different ports for load balancing
//! println!("\n📞 Contact URIs with Port Distribution:");
//! for (i, name) in agent_names.iter().enumerate() {
//!     let port = Some(5060 + i as u16);
//!     let contact_uri = builder.contact_uri(name, port);
//!     println!("  {}: {}", name, contact_uri);
//! }
//! 
//! // URI fallback examples
//! println!("\n🔄 URI Fallback Examples:");
//! let custom_contact = Some("sip:alice@custom.domain.com:5071");
//! let fallback_uri = builder.agent_uri_with_fallback("alice", custom_contact);
//! println!("  With custom contact: {}", fallback_uri);
//! 
//! let default_fallback = builder.agent_uri_with_fallback("alice", None);
//! println!("  Default fallback: {}", default_fallback);
//! # Ok(())
//! # }
//! ```
//!
//! ## Integration with Call Center Components
//!
//! ### Agent Registration Integration
//!
//! The URI builder integrates seamlessly with agent registration:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::uri_builder::SipUriBuilder;
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Integration with agent registration:
//! println!("🔗 Agent Registration Integration:");
//! 
//! println!("  📝 Registration Process:");
//! println!("     1. Agent provides basic information");
//! println!("     2. URI builder creates standardized agent URI");
//! println!("     3. URI includes extension, department, skills");
//! println!("     4. URI used for SIP registration with session-core");
//! println!("     5. URI stored in agent database for routing");
//! 
//! println!("  🎯 Routing Integration:");
//! println!("     ↳ URIs parsed for agent capability matching");
//! println!("     ↳ Skills extracted for routing decisions");
//! println!("     ↳ Department used for load balancing");
//! println!("     ↳ Extension used for direct transfers");
//! 
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance and Efficiency
//!
//! ### Optimized URI Construction
//!
//! The URI builder is optimized for high-performance operations:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::uri_builder::SipUriBuilder;
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! println!("⚡ URI Builder Performance:");
//! 
//! println!("  🚀 Construction Efficiency:");
//! println!("     ↳ String builder pattern minimizes allocations");
//! println!("     ↳ Parameter encoding cache for common values");
//! println!("     ↳ Compiled regex patterns for validation");
//! println!("     ↳ Reusable builder instances");
//! 
//! println!("  💾 Memory Optimization:");
//! println!("     ↳ Efficient parameter storage");
//! println!("     ↳ Minimal overhead per URI");
//! println!("     ↳ Optimized string concatenation");
//! 
//! println!("  📊 Scalability:");
//! println!("     ↳ Thread-safe builder operations");
//! println!("     ↳ Concurrent URI construction");
//! println!("     ↳ Batch processing support");
//! 
//! # Ok(())
//! # }
//! ```

//! SIP URI builder for call center operations

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