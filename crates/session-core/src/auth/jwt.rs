//! JWT validation using jsonwebtoken crate
//!
//! Provides proper JWT validation with signature verification
//! using JWKS (JSON Web Key Set) for OAuth 2.0 Bearer tokens.

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::types::{AuthError, AuthResult, TokenInfo};
use super::oauth::{Jwk, JwkSet};

/// JWT Claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    /// Subject (user identifier)
    pub sub: String,
    
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    
    /// Issued at (Unix timestamp)
    pub iat: Option<i64>,
    
    /// Not before (Unix timestamp)
    pub nbf: Option<i64>,
    
    /// Issuer
    pub iss: Option<String>,
    
    /// Audience
    pub aud: Option<serde_json::Value>,
    
    /// JWT ID
    pub jti: Option<String>,
    
    /// Client ID
    pub client_id: Option<String>,
    
    /// Scopes (space-separated or array)
    pub scope: Option<String>,
    
    /// Scopes as array (alternative format)
    pub scopes: Option<Vec<String>>,
    
    /// Additional claims
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl JwtClaims {
    /// Convert to TokenInfo
    pub fn to_token_info(&self, realm: Option<String>) -> AuthResult<TokenInfo> {
        let expires_at = DateTime::from_timestamp(self.exp, 0)
            .ok_or_else(|| AuthError::JwtValidationError("Invalid expiration".to_string()))?;
        
        if expires_at < Utc::now() {
            return Err(AuthError::TokenExpired);
        }
        
        // Parse scopes from either format
        let scopes = if let Some(scope) = &self.scope {
            scope.split_whitespace().map(String::from).collect()
        } else if let Some(scopes) = &self.scopes {
            scopes.clone()
        } else {
            Vec::new()
        };
        
        Ok(TokenInfo {
            subject: self.sub.clone(),
            scopes,
            expires_at,
            client_id: self.client_id.clone().unwrap_or_else(|| "unknown".to_string()),
            realm,
            extra_claims: self.extra.clone(),
        })
    }
}

/// JWT Validator with JWKS support
pub struct JwtValidator {
    /// Cached JWKS
    jwks: Option<JwkSet>,
    
    /// Allowed algorithms
    algorithms: Vec<Algorithm>,
    
    /// Expected issuer
    expected_issuer: Option<String>,
    
    /// Expected audience
    expected_audience: Option<String>,
    
    /// Validate expiration
    validate_exp: bool,
    
    /// Validate not-before
    validate_nbf: bool,
}

impl JwtValidator {
    /// Create a new JWT validator
    pub fn new() -> Self {
        Self {
            jwks: None,
            algorithms: vec![Algorithm::RS256, Algorithm::RS384, Algorithm::RS512],
            expected_issuer: None,
            expected_audience: None,
            validate_exp: true,
            validate_nbf: true,
        }
    }
    
    /// Set JWKS for validation
    pub fn with_jwks(mut self, jwks: JwkSet) -> Self {
        self.jwks = Some(jwks);
        self
    }
    
    /// Set expected issuer
    pub fn with_issuer(mut self, issuer: String) -> Self {
        self.expected_issuer = Some(issuer);
        self
    }
    
    /// Set expected audience
    pub fn with_audience(mut self, audience: String) -> Self {
        self.expected_audience = Some(audience);
        self
    }
    
    /// Validate a JWT token
    pub fn validate(&self, token: &str) -> AuthResult<JwtClaims> {
        // Decode header to get key ID
        let header = decode_header(token)
            .map_err(|e| AuthError::JwtValidationError(format!("Failed to decode header: {}", e)))?;
        
        // Get the appropriate key from JWKS
        let key = self.get_decoding_key(&header.kid)?;
        
        // Set up validation
        let mut validation = Validation::new(header.alg);
        validation.validate_exp = self.validate_exp;
        validation.validate_nbf = self.validate_nbf;
        
        if let Some(iss) = &self.expected_issuer {
            validation.set_issuer(&[iss]);
        }
        
        if let Some(aud) = &self.expected_audience {
            validation.set_audience(&[aud]);
        }
        
        // Decode and validate the token
        let token_data = decode::<JwtClaims>(token, &key, &validation)
            .map_err(|e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::ExpiredSignature => AuthError::TokenExpired,
                _ => AuthError::JwtValidationError(format!("Validation failed: {}", e)),
            })?;
        
        Ok(token_data.claims)
    }
    
    /// Get decoding key from JWKS
    fn get_decoding_key(&self, kid: &Option<String>) -> AuthResult<DecodingKey> {
        let jwks = self.jwks.as_ref()
            .ok_or_else(|| AuthError::ConfigError("No JWKS configured".to_string()))?;
        
        // Find the key by kid
        let jwk = if let Some(kid) = kid {
            jwks.keys.iter()
                .find(|k| k.kid.as_ref() == Some(kid))
                .ok_or_else(|| AuthError::JwtValidationError(format!("Key {} not found in JWKS", kid)))?
        } else if jwks.keys.len() == 1 {
            // If there's only one key and no kid specified, use it
            &jwks.keys[0]
        } else {
            return Err(AuthError::JwtValidationError("No key ID specified and multiple keys available".to_string()));
        };
        
        // Convert JWK to DecodingKey
        jwk_to_decoding_key(jwk)
    }
}

/// Convert JWK to DecodingKey
fn jwk_to_decoding_key(jwk: &Jwk) -> AuthResult<DecodingKey> {
    match jwk.kty.as_str() {
        "RSA" => {
            // For RSA keys, we need n (modulus) and e (exponent)
            let n = jwk.n.as_ref()
                .ok_or_else(|| AuthError::JwtValidationError("Missing RSA modulus".to_string()))?;
            let e = jwk.e.as_ref()
                .ok_or_else(|| AuthError::JwtValidationError("Missing RSA exponent".to_string()))?;
            
            // Create RSA components for DecodingKey
            DecodingKey::from_rsa_components(n, e)
                .map_err(|e| AuthError::JwtValidationError(format!("Failed to create RSA key: {}", e)))
        }
        "EC" => {
            // For EC keys, we would need x and y coordinates
            // This is a simplified implementation
            Err(AuthError::JwtValidationError("EC keys not yet supported".to_string()))
        }
        "oct" => {
            // For symmetric keys (HMAC)
            let k = jwk.n.as_ref()  // Sometimes symmetric keys use 'k' field
                .ok_or_else(|| AuthError::JwtValidationError("Missing symmetric key".to_string()))?;
            
            use base64::{Engine as _, engine::general_purpose};
            let key_bytes = general_purpose::URL_SAFE_NO_PAD.decode(k)
                .map_err(|e| AuthError::JwtValidationError(format!("Failed to decode key: {}", e)))?;
            
            Ok(DecodingKey::from_secret(&key_bytes))
        }
        _ => Err(AuthError::JwtValidationError(format!("Unsupported key type: {}", jwk.kty)))
    }
}

/// Validate a JWT token with JWKS
pub async fn validate_jwt_with_jwks(
    token: &str,
    jwks: &JwkSet,
    expected_issuer: Option<String>,
    expected_audience: Option<String>,
) -> AuthResult<TokenInfo> {
    let mut validator = JwtValidator::new()
        .with_jwks(jwks.clone());
    
    if let Some(iss) = expected_issuer {
        validator = validator.with_issuer(iss);
    }
    
    if let Some(aud) = expected_audience {
        validator = validator.with_audience(aud);
    }
    
    let claims = validator.validate(token)?;
    claims.to_token_info(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_jwt_claims_to_token_info() {
        let claims = JwtClaims {
            sub: "alice@example.com".to_string(),
            exp: (Utc::now() + chrono::Duration::hours(1)).timestamp(),
            iat: Some(Utc::now().timestamp()),
            nbf: None,
            iss: Some("https://auth.example.com".to_string()),
            aud: None,
            jti: None,
            client_id: Some("test-client".to_string()),
            scope: Some("sip:register sip:call".to_string()),
            scopes: None,
            extra: HashMap::new(),
        };
        
        let token_info = claims.to_token_info(Some("example.com".to_string())).unwrap();
        assert_eq!(token_info.subject, "alice@example.com");
        assert_eq!(token_info.scopes.len(), 2);
        assert!(token_info.has_scope("sip:register"));
        assert!(token_info.has_scope("sip:call"));
        assert!(!token_info.is_expired());
    }
    
    #[test]
    fn test_expired_token() {
        let claims = JwtClaims {
            sub: "alice@example.com".to_string(),
            exp: (Utc::now() - chrono::Duration::hours(1)).timestamp(),
            iat: None,
            nbf: None,
            iss: None,
            aud: None,
            jti: None,
            client_id: None,
            scope: None,
            scopes: None,
            extra: HashMap::new(),
        };
        
        let result = claims.to_token_info(None);
        assert!(matches!(result, Err(AuthError::TokenExpired)));
    }
}