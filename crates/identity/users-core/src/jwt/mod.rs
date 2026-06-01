//! JWT token issuance

use crate::{Error, Result, User};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

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
    pub iss: String,      // Issuer
    pub sub: String,      // Subject (user ID)
    pub aud: Vec<String>, // Audience
    pub exp: u64,         // Expiration
    pub iat: u64,         // Issued at
    pub jti: String,      // JWT ID

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
    pub sub: String, // User ID
    pub jti: String, // Unique ID for revocation
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
    pub signing_key: Option<String>, // Will be set programmatically
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            issuer: "https://users.rvoip.local".to_string(),
            audience: vec!["rvoip-api".to_string(), "rvoip-sip".to_string()],
            access_ttl_seconds: 900,      // 15 minutes
            refresh_ttl_seconds: 2592000, // 30 days
            algorithm: "HS256".to_string(),
            signing_key: None,
        }
    }
}

impl JwtIssuer {
    pub fn new(mut config: JwtConfig) -> Result<Self> {
        if config.signing_key.is_none() {
            match config.algorithm.as_str() {
                "HS256" => config.signing_key = Some(Self::generate_hs256_secret()),
                "RS256" => {
                    return Err(Error::Config(
                        "RS256 requires a caller-supplied PEM signing key; users-core no longer \
                         generates RSA keys internally"
                            .to_string(),
                    ))
                }
                _ => {}
            }
        }

        let signing_key = config.signing_key.as_ref().unwrap();

        // Create encoding key
        let encoding_key = match config.algorithm.as_str() {
            "RS256" => EncodingKey::from_rsa_pem(signing_key.as_bytes())
                .map_err(|e| Error::Config(format!("Invalid RSA key: {}", e)))?,
            "HS256" => EncodingKey::from_secret(signing_key.as_bytes()),
            _ => {
                return Err(Error::Config(format!(
                    "Unsupported algorithm: {}",
                    config.algorithm
                )))
            }
        };

        let decoding_key = match config.algorithm.as_str() {
            "RS256" => DecodingKey::from_rsa_pem(signing_key.as_bytes())
                .map_err(|e| Error::Config(format!("Invalid RSA verification key: {}", e)))?,
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

        encode(&self.header, &claims, &self.encoding_key).map_err(|e| Error::Jwt(e))
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

        encode(&self.header, &claims, &self.encoding_key).map_err(|e| Error::Jwt(e))
    }

    pub fn validate_refresh_token(&self, token: &str) -> Result<RefreshTokenClaims> {
        let mut validation = Validation::new(self.header.alg);
        validation.set_issuer(&[self.config.issuer.clone()]);
        validation.validate_exp = true;

        let token_data = decode::<RefreshTokenClaims>(token, &self.decoding_key, &validation)
            .map_err(|e| Error::Jwt(e))?;

        Ok(token_data.claims)
    }

    /// Algorithm configured for issued JWTs.
    pub fn algorithm(&self) -> Algorithm {
        self.header.alg
    }

    /// Verification key for tokens issued by this service.
    pub fn decoding_key(&self) -> &DecodingKey {
        &self.decoding_key
    }

    /// Get the public key in JWK format (for auth-core)
    pub fn public_key_jwk(&self) -> serde_json::Value {
        if self.config.algorithm == "RS256" {
            return serde_json::json!({
                "kty": "RSA",
                "use": "sig",
                "kid": self.header.kid.as_ref().unwrap(),
                "alg": self.config.algorithm,
            });
        }

        serde_json::json!({
            "kty": "oct",
            "use": "sig",
            "kid": self.header.kid.as_ref().unwrap(),
            "alg": self.config.algorithm,
        })
    }

    /// Get the public key in PEM format
    pub fn public_key_pem(&self) -> Result<String> {
        Err(Error::Config(
            "public key PEM export is unavailable without an RSA key parser dependency".to_string(),
        ))
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

    fn generate_hs256_secret() -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use rand::{rngs::OsRng, RngCore};

        let mut secret = [0u8; 32];
        OsRng.fill_bytes(&mut secret);
        URL_SAFE_NO_PAD.encode(secret)
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
        assert_eq!(config.algorithm, "HS256");
    }
}
