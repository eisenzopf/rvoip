//! JWT token issuance

use std::sync::Arc;
use jsonwebtoken::{encode, decode, Header, Algorithm, EncodingKey, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::{Result, Error, User};

/// JWT issuer
pub struct JwtIssuer {
    pub(crate) config: JwtConfig,
    encoding_key: Arc<EncodingKey>,
    decoding_key: Arc<DecodingKey>,
    header: Header,
}

/// JWT claims for user tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserClaims {
    // Standard claims
    pub iss: String,              // Issuer
    pub sub: String,              // Subject (user ID)
    pub aud: Vec<String>,         // Audience
    pub exp: u64,                 // Expiration
    pub iat: u64,                 // Issued at
    pub jti: String,              // JWT ID
    
    // Custom claims
    pub username: String,
    pub email: Option<String>,
    pub roles: Vec<String>,
    pub scope: String,
}

/// Refresh token claims (minimal)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshTokenClaims {
    pub iss: String,
    pub sub: String,              // User ID
    pub jti: String,              // Unique ID for revocation
    pub exp: u64,
    pub iat: u64,
}

/// JWT configuration
#[derive(Debug, Clone, Deserialize)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: Vec<String>,
    pub access_ttl_seconds: u64,
    pub refresh_ttl_seconds: u64,
    pub algorithm: String,
    #[serde(skip)]
    pub signing_key: Option<String>,  // Will be set programmatically
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            issuer: "https://users.rvoip.local".to_string(),
            audience: vec!["rvoip-api".to_string(), "rvoip-sip".to_string()],
            access_ttl_seconds: 900,  // 15 minutes
            refresh_ttl_seconds: 2592000,  // 30 days
            algorithm: "RS256".to_string(),
            signing_key: None,
        }
    }
}

impl JwtIssuer {
    pub fn new(mut config: JwtConfig) -> Result<Self> {
        // Generate RSA key pair if not provided
        if config.signing_key.is_none() {
            config.signing_key = Some(Self::generate_rsa_key_pair()?);
        }
        
        let signing_key = config.signing_key.as_ref().unwrap();
        
        // Create encoding key
        let encoding_key = match config.algorithm.as_str() {
            "RS256" => EncodingKey::from_rsa_pem(signing_key.as_bytes())
                .map_err(|e| Error::Config(format!("Invalid RSA key: {}", e)))?,
            "HS256" => EncodingKey::from_secret(signing_key.as_bytes()),
            _ => return Err(Error::Config(format!("Unsupported algorithm: {}", config.algorithm))),
        };
        
        // For RS256, we need to extract public key for decoding
        let decoding_key = match config.algorithm.as_str() {
            "RS256" => {
                let public_key = Self::extract_public_key_from_private(signing_key)?;
                DecodingKey::from_rsa_pem(public_key.as_bytes())
                    .map_err(|e| Error::Config(format!("Invalid public key: {}", e)))?
            },
            "HS256" => DecodingKey::from_secret(signing_key.as_bytes()),
            _ => unreachable!(),
        };
        
        let algorithm = match config.algorithm.as_str() {
            "RS256" => Algorithm::RS256,
            "HS256" => Algorithm::HS256,
            _ => unreachable!(),
        };
        
        let mut header = Header::new(algorithm);
        header.kid = Some("users-core-2024".to_string());
        
        Ok(Self {
            config,
            encoding_key: Arc::new(encoding_key),
            decoding_key: Arc::new(decoding_key),
            header,
        })
    }
    
    pub fn create_access_token(&self, user: &User) -> Result<String> {
        let now = chrono::Utc::now();
        let exp = now + chrono::Duration::seconds(self.config.access_ttl_seconds as i64);
        
        let claims = UserClaims {
            iss: self.config.issuer.clone(),
            sub: user.id.clone(),
            aud: self.config.audience.clone(),
            exp: exp.timestamp() as u64,
            iat: now.timestamp() as u64,
            jti: Uuid::new_v4().to_string(),
            username: user.username.clone(),
            email: user.email.clone(),
            roles: user.roles.clone(),
            scope: self.roles_to_scope(&user.roles),
        };
        
        encode(&self.header, &claims, &self.encoding_key)
            .map_err(|e| Error::Jwt(e))
    }
    
    pub fn create_refresh_token(&self, user_id: &str) -> Result<String> {
        let now = chrono::Utc::now();
        let exp = now + chrono::Duration::seconds(self.config.refresh_ttl_seconds as i64);
        
        let claims = RefreshTokenClaims {
            iss: self.config.issuer.clone(),
            sub: user_id.to_string(),
            jti: Uuid::new_v4().to_string(),
            exp: exp.timestamp() as u64,
            iat: now.timestamp() as u64,
        };
        
        encode(&self.header, &claims, &self.encoding_key)
            .map_err(|e| Error::Jwt(e))
    }
    
    pub fn validate_refresh_token(&self, token: &str) -> Result<RefreshTokenClaims> {
        let mut validation = Validation::new(self.header.alg);
        validation.set_issuer(&[self.config.issuer.clone()]);
        validation.validate_exp = true;
        
        let token_data = decode::<RefreshTokenClaims>(
            token,
            &self.decoding_key,
            &validation
        ).map_err(|e| Error::Jwt(e))?;
        
        Ok(token_data.claims)
    }
    
    /// Get the public key in JWK format (for auth-core)
    pub fn public_key_jwk(&self) -> serde_json::Value {
        use rsa::{RsaPrivateKey, RsaPublicKey};
        use rsa::pkcs8::DecodePrivateKey;
        use rsa::traits::PublicKeyParts;
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        
        if self.config.algorithm == "RS256" {
            // Extract RSA components from the private key
            if let Ok(private_key) = RsaPrivateKey::from_pkcs8_pem(
                self.config.signing_key.as_ref().unwrap()
            ) {
                let public_key = RsaPublicKey::from(&private_key);
                
                // Get modulus and exponent
                let n = public_key.n();
                let e = public_key.e();
                
                // Convert to base64url encoding
                let n_bytes = n.to_bytes_be();
                let e_bytes = e.to_bytes_be();
                
                return serde_json::json!({
                    "kty": "RSA",
                    "use": "sig",
                    "kid": self.header.kid.as_ref().unwrap(),
                    "alg": self.config.algorithm,
                    "n": URL_SAFE_NO_PAD.encode(&n_bytes),
                    "e": URL_SAFE_NO_PAD.encode(&e_bytes),
                });
            }
        }
        
        // Fallback for non-RSA algorithms
        serde_json::json!({
            "kty": "oct",
            "use": "sig",
            "kid": self.header.kid.as_ref().unwrap(),
            "alg": self.config.algorithm,
        })
    }
    
    /// Get the public key in PEM format
    pub fn public_key_pem(&self) -> Result<String> {
        if self.config.algorithm == "RS256" {
            Self::extract_public_key_from_private(self.config.signing_key.as_ref().unwrap())
        } else {
            Err(Error::Config("Public key only available for RS256".to_string()))
        }
    }
    
    fn roles_to_scope(&self, roles: &[String]) -> String {
        let mut scopes = vec!["openid", "profile", "email"];
        
        if roles.contains(&"admin".to_string()) {
            scopes.push("admin");
        }
        
        // Always include SIP registration scope
        scopes.push("sip.register");
        
        scopes.join(" ")
    }
    
    fn generate_rsa_key_pair() -> Result<String> {
        use rand::rngs::OsRng;
        use rsa::{RsaPrivateKey, pkcs8::EncodePrivateKey, pkcs8::LineEnding};
        
        let mut rng = OsRng;
        let bits = 2048;
        let private_key = RsaPrivateKey::new(&mut rng, bits)
            .map_err(|e| Error::Config(format!("Failed to generate RSA key: {}", e)))?;
        
        let pem = private_key.to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| Error::Config(format!("Failed to encode RSA key: {}", e)))?;
        
        Ok(pem.to_string())
    }
    
    fn extract_public_key_from_private(private_pem: &str) -> Result<String> {
        use rsa::{RsaPrivateKey, RsaPublicKey};
        use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
        
        let private_key = RsaPrivateKey::from_pkcs8_pem(private_pem)
            .map_err(|e| Error::Config(format!("Failed to parse private key: {}", e)))?;
        
        let public_key = RsaPublicKey::from(&private_key);
        
        let public_pem = public_key.to_public_key_pem(LineEnding::LF)
            .map_err(|e| Error::Config(format!("Failed to encode public key: {}", e)))?;
        
        Ok(public_pem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_jwt_config_default() {
        let config = JwtConfig::default();
        assert_eq!(config.issuer, "https://users.rvoip.local");
        assert_eq!(config.access_ttl_seconds, 900);
        assert_eq!(config.algorithm, "RS256");
    }
}
