use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use async_trait::async_trait;
use anyhow::Result;
use serde::{Serialize, Deserialize};
use tracing::{debug, info, warn, error};

use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};
use rvoip_session_core::Session;

use crate::errors::Error;

/// Policy decision
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Allow the operation
    Allow,
    
    /// Deny the operation
    Deny,
    
    /// Challenge the operation (e.g., require authentication)
    Challenge,
}

/// Policy enforcement point trait
#[async_trait]
pub trait PolicyEnforcer: Send + Sync {
    /// Decide whether to allow an incoming request
    async fn decide_incoming_request(&self, request: &Request) -> Result<PolicyDecision, Error>;
    
    /// Decide whether to allow an outgoing request
    async fn decide_outgoing_request(&self, request: &Request) -> Result<PolicyDecision, Error>;
    
    /// Decide whether to allow a new session
    async fn decide_new_session(&self, session: &Session) -> Result<PolicyDecision, Error>;
    
    /// Generate a challenge response if needed
    async fn challenge_request(&self, request: &Request) -> Result<Response, Error>;
}

/// Rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum number of requests per time window
    pub max_requests: u32,
    
    /// Time window in seconds
    pub window_seconds: u32,
}

/// IP-based policy rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpPolicyRule {
    /// IP or CIDR range
    pub ip_range: String,
    
    /// Whether to allow traffic from this range
    pub allow: bool,
    
    /// Rate limit for this range (if allowed)
    pub rate_limit: Option<RateLimitConfig>,
}

/// URI-based policy rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UriPolicyRule {
    /// URI pattern to match
    pub pattern: String,
    
    /// Whether to allow traffic to/from this URI
    pub allow: bool,
}

/// A simple policy engine
pub struct PolicyEngine {
    /// IP-based rules
    ip_rules: Vec<IpPolicyRule>,
    
    /// Source URI rules
    source_uri_rules: Vec<UriPolicyRule>,
    
    /// Destination URI rules
    destination_uri_rules: Vec<UriPolicyRule>,
    
    /// Authentication required for methods
    auth_required_methods: HashMap<Method, bool>,
    
    /// Rate limiting by IP
    rate_limits: HashMap<IpAddr, RateLimitConfig>,
}

impl PolicyEngine {
    /// Create a new policy engine
    pub fn new() -> Self {
        let mut auth_required_methods = HashMap::new();
        
        // Default authentication requirements
        auth_required_methods.insert(Method::Register, true);
        auth_required_methods.insert(Method::Invite, true);
        auth_required_methods.insert(Method::Subscribe, true);
        
        Self {
            ip_rules: Vec::new(),
            source_uri_rules: Vec::new(),
            destination_uri_rules: Vec::new(),
            auth_required_methods,
            rate_limits: HashMap::new(),
        }
    }
    
    /// Add an IP-based policy rule
    pub fn add_ip_rule(&mut self, rule: IpPolicyRule) {
        self.ip_rules.push(rule);
    }
    
    /// Add a source URI policy rule
    pub fn add_source_uri_rule(&mut self, rule: UriPolicyRule) {
        self.source_uri_rules.push(rule);
    }
    
    /// Add a destination URI policy rule
    pub fn add_destination_uri_rule(&mut self, rule: UriPolicyRule) {
        self.destination_uri_rules.push(rule);
    }
    
    /// Set authentication requirement for a method
    pub fn set_auth_required(&mut self, method: Method, required: bool) {
        self.auth_required_methods.insert(method, required);
    }
    
    /// Check if a request is from an allowed IP
    fn is_allowed_ip(&self, ip: &IpAddr) -> bool {
        // Default deny if no rules match
        if self.ip_rules.is_empty() {
            return true; // Allow if no rules configured
        }
        
        for rule in &self.ip_rules {
            // TODO: Implement proper CIDR range checking
            // For now, we just do simple string comparison
            if rule.ip_range == ip.to_string() {
                return rule.allow;
            }
        }
        
        false
    }
    
    /// Check if a URI is allowed by the rules
    fn is_allowed_uri(&self, uri: &Uri, rules: &[UriPolicyRule]) -> bool {
        if rules.is_empty() {
            return true; // Allow if no rules configured
        }
        
        let uri_str = uri.to_string();
        for rule in rules {
            if uri_str.contains(&rule.pattern) {
                return rule.allow;
            }
        }
        
        false
    }
    
    /// Check if authentication is required for a method
    fn is_auth_required(&self, method: &Method) -> bool {
        self.auth_required_methods.get(method).copied().unwrap_or(false)
    }
    
    /// Check if a request is authenticated
    fn is_authenticated(&self, request: &Request) -> bool {
        // Check for Authorization header
        if let Some(auth_header) = request.header(&rvoip_sip_core::HeaderName::Authorization) {
            // TODO: Implement proper authentication checking
            // For now, we just check if the header exists
            return true;
        }
        
        false
    }
}

#[async_trait]
impl PolicyEnforcer for PolicyEngine {
    async fn decide_incoming_request(&self, request: &Request) -> Result<PolicyDecision, Error> {
        // Check if method is allowed
        if request.method == Method::Cancel || request.method == Method::Ack {
            // Always allow CANCEL and ACK
            return Ok(PolicyDecision::Allow);
        }
        
        // Check if authentication is required
        if self.is_auth_required(&request.method) && !self.is_authenticated(request) {
            return Ok(PolicyDecision::Challenge);
        }
        
        // TODO: Check source IP (need access to transport layer information)
        
        // Check From URI
        if let Some(from_header) = request.header(&rvoip_sip_core::HeaderName::From) {
            if let Some(from_text) = from_header.value.as_text() {
                // Simple URI extraction
                let uri_start = from_text.find('<').map(|pos| pos + 1).unwrap_or(0);
                let uri_end = from_text[uri_start..].find('>').map(|pos| uri_start + pos).unwrap_or(from_text.len());
                let uri_str = &from_text[uri_start..uri_end];
                
                if let Ok(uri) = uri_str.parse::<Uri>() {
                    if !self.is_allowed_uri(&uri, &self.source_uri_rules) {
                        warn!("Blocked request from disallowed URI: {}", uri);
                        return Ok(PolicyDecision::Deny);
                    }
                }
            }
        }
        
        // Check destination URI
        if !self.is_allowed_uri(&request.uri, &self.destination_uri_rules) {
            warn!("Blocked request to disallowed URI: {}", request.uri);
            return Ok(PolicyDecision::Deny);
        }
        
        // All checks passed
        Ok(PolicyDecision::Allow)
    }
    
    async fn decide_outgoing_request(&self, request: &Request) -> Result<PolicyDecision, Error> {
        // Most outgoing requests are allowed, but check destination URI
        if !self.is_allowed_uri(&request.uri, &self.destination_uri_rules) {
            warn!("Blocked outgoing request to disallowed URI: {}", request.uri);
            return Ok(PolicyDecision::Deny);
        }
        
        Ok(PolicyDecision::Allow)
    }
    
    async fn decide_new_session(&self, session: &Session) -> Result<PolicyDecision, Error> {
        // Most session policy would be enforced at the INVITE level
        Ok(PolicyDecision::Allow)
    }
    
    async fn challenge_request(&self, request: &Request) -> Result<Response, Error> {
        let mut response = Response::new(StatusCode::Unauthorized);
        
        // Add WWW-Authenticate header
        let realm = "rvoip";
        let nonce = uuid::Uuid::new_v4().to_string();
        let auth_value = format!("Digest realm=\"{}\", nonce=\"{}\"", realm, nonce);
        response.headers.push(rvoip_sip_core::Header::text(
            rvoip_sip_core::HeaderName::WwwAuthenticate,
            auth_value
        ));
        
        Ok(response)
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
} 