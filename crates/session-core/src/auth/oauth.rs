//! OAuth 2.0 Bearer Token validation for SIP (RFC 8898)

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use dashmap::DashMap;
use tracing::{debug, warn, info};
use serde::{Deserialize, Serialize};

use super::types::{TokenInfo, AuthError, AuthResult, IntrospectionResponse};

/// OAuth 2.0 configuration
#[derive(Debug, Clone)]
pub struct OAuth2Config {
    /// JWKS URI for JWT validation (e.g., https://auth.example.com/.well-known/jwks.json)
    pub jwks_uri: Option<String>,
    
    /// Token introspection endpoint (e.g., https://auth.example.com/oauth2/introspect)
    pub introspect_uri: Option<String>,
    
    /// Client credentials for introspection (if required)
    pub introspect_client_id: Option<String>,
    pub introspect_client_secret: Option<String>,
    
    /// Required scopes for different operations
    pub required_scopes: OAuth2Scopes,
    
    /// Cache validated tokens for performance
    pub cache_ttl: Duration,
    
    /// OAuth realm for WWW-Authenticate headers
    pub realm: String,
    
    /// Allow insecure connections (for testing only)
    pub allow_insecure: bool,
}

impl Default for OAuth2Config {
    fn default() -> Self {
        Self {
            jwks_uri: None,
            introspect_uri: None,
            introspect_client_id: None,
            introspect_client_secret: None,
            required_scopes: OAuth2Scopes::default(),
            cache_ttl: Duration::from_secs(300), // 5 minutes
            realm: "sip".to_string(),
            allow_insecure: false,
        }
    }
}

/// Required OAuth scopes for different SIP operations
#[derive(Debug, Clone)]
pub struct OAuth2Scopes {
    /// Scopes required for REGISTER
    pub register: Vec<String>,
    
    /// Scopes required for INVITE (making calls)
    pub call: Vec<String>,
    
    /// Scopes required for presence (PUBLISH, SUBSCRIBE)
    pub presence: Vec<String>,
    
    /// Scopes required for messaging
    pub message: Vec<String>,
}

impl Default for OAuth2Scopes {
    fn default() -> Self {
        Self {
            register: vec!["sip:register".to_string()],
            call: vec!["sip:call".to_string()],
            presence: vec!["sip:presence".to_string()],
            message: vec!["sip:message".to_string()],
        }
    }
}

/// Refresh configuration for presence
#[derive(Debug, Clone)]
pub struct RefreshConfig {
    /// How often to send PUBLISH to refresh our own presence
    pub publish_interval: Duration,
    
    /// How often to refresh SUBSCRIBE dialogs
    pub subscribe_refresh: Duration,
    
    /// Heartbeat interval for P2P presence
    pub p2p_heartbeat: Duration,
    
    /// Grace period before considering peer offline
    pub offline_threshold: Duration,
}

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            publish_interval: Duration::from_secs(3600),  // 1 hour
            subscribe_refresh: Duration::from_secs(3300), // 55 minutes
            p2p_heartbeat: Duration::from_secs(30),       // 30 seconds
            offline_threshold: Duration::from_secs(90),   // 90 seconds
        }
    }
}

/// JWT Key Set for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwkSet {
    pub keys: Vec<Jwk>,
}

/// JSON Web Key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Jwk {
    pub kty: String,
    pub use_: Option<String>,
    pub key_ops: Option<Vec<String>>,
    pub alg: Option<String>,
    pub kid: Option<String>,
    pub x5c: Option<Vec<String>>,
    pub x5t: Option<String>,
    pub n: Option<String>,
    pub e: Option<String>,
}

/// OAuth 2.0 Bearer Token Validator
pub struct OAuth2Validator {
    config: OAuth2Config,
    http_client: reqwest::Client,
    jwks_cache: Arc<RwLock<Option<(JwkSet, Instant)>>>,
    token_cache: Arc<DashMap<String, (TokenInfo, Instant)>>,
}

impl OAuth2Validator {
    /// Create a new OAuth validator
    pub async fn new(config: OAuth2Config) -> AuthResult<Self> {
        let http_client = if config.allow_insecure {
            reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .build()
                .map_err(|e| AuthError::ConfigError(e.to_string()))?
        } else {
            reqwest::Client::new()
        };
        
        let validator = Self {
            config,
            http_client,
            jwks_cache: Arc::new(RwLock::new(None)),
            token_cache: Arc::new(DashMap::new()),
        };
        
        // Pre-fetch JWKS if configured
        if validator.config.jwks_uri.is_some() {
            validator.fetch_jwks().await?;
        }
        
        Ok(validator)
    }
    
    /// Validate a Bearer token from Authorization header
    pub async fn validate_bearer_token(&self, token: &str) -> AuthResult<TokenInfo> {
        // Remove "Bearer " prefix if present
        let token = token.strip_prefix("Bearer ")
            .unwrap_or(token);
        
        // Check cache first
        if let Some(cached) = self.get_cached_token(token).await {
            debug!("Token found in cache");
            return Ok(cached);
        }
        
        // Try JWT validation first (faster, no network)
        if self.config.jwks_uri.is_some() {
            match self.validate_jwt(token).await {
                Ok(info) => {
                    self.cache_token(token, &info).await;
                    return Ok(info);
                }
                Err(e) => {
                    debug!("JWT validation failed: {}", e);
                }
            }
        }
        
        // Fall back to introspection (network call)
        if let Some(uri) = &self.config.introspect_uri {
            let info = self.introspect_token(token, uri).await?;
            self.cache_token(token, &info).await;
            return Ok(info);
        }
        
        Err(AuthError::InvalidToken("No validation method configured".to_string()))
    }
    
    /// Check if token has required scopes for operation
    pub fn check_scopes(&self, token_info: &TokenInfo, operation: &str) -> bool {
        let required = match operation {
            "REGISTER" => &self.config.required_scopes.register,
            "INVITE" | "ACK" | "BYE" | "CANCEL" => &self.config.required_scopes.call,
            "PUBLISH" | "SUBSCRIBE" | "NOTIFY" => &self.config.required_scopes.presence,
            "MESSAGE" => &self.config.required_scopes.message,
            _ => return true, // No scope requirement for other methods
        };
        
        token_info.has_all_scopes(required)
    }
    
    /// Generate WWW-Authenticate header for 401 response
    pub fn www_authenticate_header(&self, error: Option<&str>, description: Option<&str>) -> String {
        let mut header = format!("Bearer realm=\"{}\"", self.config.realm);
        
        if let Some(err) = error {
            header.push_str(&format!(", error=\"{}\"", err));
        }
        
        if let Some(desc) = description {
            header.push_str(&format!(", error_description=\"{}\"", desc));
        }
        
        header
    }
    
    /// Validate JWT token using JWKS
    async fn validate_jwt(&self, token: &str) -> AuthResult<TokenInfo> {
        // Get cached JWKS or fetch if needed
        let jwks = {
            let cache = self.jwks_cache.read().await;
            if let Some((jwks, cached_at)) = cache.as_ref() {
                // Use cached JWKS if it's less than 1 hour old
                if cached_at.elapsed() < Duration::from_secs(3600) {
                    jwks.clone()
                } else {
                    drop(cache);
                    self.fetch_jwks().await?
                }
            } else {
                drop(cache);
                self.fetch_jwks().await?
            }
        };
        
        // Use the proper JWT validator
        let mut token_info = super::jwt::validate_jwt_with_jwks(
            token,
            &jwks,
            None, // TODO: Configure expected issuer
            None, // TODO: Configure expected audience
        ).await?;
        
        // Add realm to token info
        token_info.realm = Some(self.config.realm.clone());
        
        Ok(token_info)
    }
    
    /// Introspect token using OAuth introspection endpoint
    async fn introspect_token(&self, token: &str, uri: &str) -> AuthResult<TokenInfo> {
        let mut form = vec![("token", token)];
        
        // Add client credentials if configured
        if let (Some(id), Some(secret)) = (&self.config.introspect_client_id, &self.config.introspect_client_secret) {
            form.push(("client_id", id));
            form.push(("client_secret", secret));
        }
        
        let response = self.http_client
            .post(uri)
            .form(&form)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(e.to_string()))?;
        
        if !response.status().is_success() {
            return Err(AuthError::IntrospectionError(
                format!("Introspection failed with status: {}", response.status())
            ));
        }
        
        let introspection: IntrospectionResponse = response
            .json()
            .await
            .map_err(|e| AuthError::IntrospectionError(e.to_string()))?;
        
        introspection.to_token_info()
    }
    
    /// Fetch JWKS from configured endpoint
    async fn fetch_jwks(&self) -> AuthResult<JwkSet> {
        let uri = self.config.jwks_uri.as_ref()
            .ok_or_else(|| AuthError::ConfigError("No JWKS URI configured".to_string()))?;
        
        let response = self.http_client
            .get(uri)
            .send()
            .await
            .map_err(|e| AuthError::NetworkError(e.to_string()))?;
        
        if !response.status().is_success() {
            return Err(AuthError::JwksFetchError(
                format!("JWKS fetch failed with status: {}", response.status())
            ));
        }
        
        let jwks: JwkSet = response
            .json()
            .await
            .map_err(|e| AuthError::JwksFetchError(e.to_string()))?;
        
        // Cache the JWKS
        let mut cache = self.jwks_cache.write().await;
        *cache = Some((jwks.clone(), Instant::now()));
        
        info!("JWKS fetched and cached");
        Ok(jwks)
    }
    
    /// Get token from cache if valid
    async fn get_cached_token(&self, token: &str) -> Option<TokenInfo> {
        if let Some(entry) = self.token_cache.get(token) {
            let (info, cached_at) = entry.clone();
            
            // Check if cache entry is still valid
            if cached_at.elapsed() < self.config.cache_ttl && !info.is_expired() {
                return Some(info);
            } else {
                // Remove expired entry
                drop(entry);
                self.token_cache.remove(token);
            }
        }
        None
    }
    
    /// Cache validated token
    async fn cache_token(&self, token: &str, info: &TokenInfo) {
        self.token_cache.insert(token.to_string(), (info.clone(), Instant::now()));
        debug!("Token cached for user: {}", info.subject);
    }
    
    /// Clear token cache
    pub async fn clear_cache(&self) {
        self.token_cache.clear();
        info!("Token cache cleared");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_scope_checking() {
        let config = OAuth2Config::default();
        let validator = OAuth2Validator::new(config).await.unwrap();
        
        let token_info = TokenInfo {
            subject: "alice@example.com".to_string(),
            scopes: vec!["sip:register".to_string(), "sip:call".to_string()],
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            client_id: "test-client".to_string(),
            realm: None,
            extra_claims: std::collections::HashMap::new(),
        };
        
        assert!(validator.check_scopes(&token_info, "REGISTER"));
        assert!(validator.check_scopes(&token_info, "INVITE"));
        assert!(!validator.check_scopes(&token_info, "PUBLISH")); // No presence scope
    }
    
    #[test]
    fn test_www_authenticate_header() {
        let config = OAuth2Config {
            realm: "example.com".to_string(),
            ..Default::default()
        };
        
        let validator = OAuth2Validator {
            config,
            http_client: reqwest::Client::new(),
            jwks_cache: Arc::new(RwLock::new(None)),
            token_cache: Arc::new(DashMap::new()),
        };
        
        let header = validator.www_authenticate_header(None, None);
        assert_eq!(header, "Bearer realm=\"example.com\"");
        
        let header = validator.www_authenticate_header(Some("invalid_token"), Some("The token expired"));
        assert_eq!(header, "Bearer realm=\"example.com\", error=\"invalid_token\", error_description=\"The token expired\"");
    }
}