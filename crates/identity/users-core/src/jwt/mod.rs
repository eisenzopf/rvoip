//! JWT token issuance

use crate::{Error, Result, User};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rsa::{pkcs1::DecodeRsaPrivateKey, pkcs8::DecodePrivateKey, traits::PublicKeyParts};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

/// JWT issuer
pub struct JwtIssuer {
    pub(crate) config: JwtConfig,
    encoding_key: Arc<EncodingKey>,
    decoding_key: Arc<DecodingKey>,
    header: Header,
}

/// JWT claims for user tokens
#[derive(Clone, Serialize, Deserialize)]
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
    /// Issuer-controlled tenant binding. Optional while legacy tokens remain
    /// parseable; tenant-bound consumers must require it explicitly.
    #[serde(
        default,
        alias = "tenant",
        alias = "tid",
        skip_serializing_if = "Option::is_none"
    )]
    pub tenant_id: Option<String>,
}

impl UserClaims {
    /// Construct access-token claims without optional profile, role, or tenant
    /// fields. Builder methods can then add those values explicitly.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        issuer: impl Into<String>,
        subject: impl Into<String>,
        audience: Vec<String>,
        expires_at: u64,
        issued_at: u64,
        token_id: impl Into<String>,
        username: impl Into<String>,
        scope: impl Into<String>,
    ) -> Self {
        Self {
            iss: issuer.into(),
            sub: subject.into(),
            aud: audience,
            exp: expires_at,
            iat: issued_at,
            jti: token_id.into(),
            username: username.into(),
            email: None,
            roles: Vec::new(),
            scope: scope.into(),
            tenant_id: None,
        }
    }

    pub fn with_email(mut self, email: Option<String>) -> Self {
        self.email = email;
        self
    }

    pub fn with_roles(mut self, roles: Vec<String>) -> Self {
        self.roles = roles;
        self
    }

    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }
}

impl std::fmt::Debug for UserClaims {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserClaims")
            .field("issuer_present", &!self.iss.is_empty())
            .field("subject_present", &!self.sub.is_empty())
            .field("audience_count", &self.aud.len())
            .field("expires_at", &self.exp)
            .field("issued_at", &self.iat)
            .field("token_id_present", &!self.jti.is_empty())
            .field("username_present", &!self.username.is_empty())
            .field("email_present", &self.email.is_some())
            .field("role_count", &self.roles.len())
            .field("scope_present", &!self.scope.is_empty())
            .field("scope_len", &self.scope.len())
            .field("tenant_present", &self.tenant_id.is_some())
            .finish()
    }
}

/// Refresh token claims (minimal)
#[derive(Clone, Serialize, Deserialize)]
pub struct RefreshTokenClaims {
    pub iss: String,
    pub sub: String, // User ID
    pub jti: String, // Unique ID for revocation
    pub exp: u64,
    pub iat: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct RefreshTokenWireClaims {
    #[serde(flatten)]
    claims: RefreshTokenClaims,
    #[serde(
        default,
        alias = "tenant",
        alias = "tid",
        skip_serializing_if = "Option::is_none"
    )]
    tenant_id: Option<String>,
}

impl std::fmt::Debug for RefreshTokenClaims {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RefreshTokenClaims")
            .field("issuer_present", &!self.iss.is_empty())
            .field("subject_present", &!self.sub.is_empty())
            .field("token_id_present", &!self.jti.is_empty())
            .field("expires_at", &self.exp)
            .field("issued_at", &self.iat)
            .finish()
    }
}

/// JWT configuration
#[derive(Clone, Deserialize)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: Vec<String>,
    pub access_ttl_seconds: u64,
    pub refresh_ttl_seconds: u64,
    pub algorithm: String,
    /// Fixed tenant placed into every access token issued by this instance.
    /// This value is configuration-owned and is never read from login input.
    #[serde(default, alias = "tenant")]
    pub tenant_id: Option<String>,
    #[serde(skip)]
    pub signing_key: Option<String>, // Will be set programmatically
}

impl std::fmt::Debug for JwtConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JwtConfig")
            .field("issuer_present", &!self.issuer.is_empty())
            .field("audience_count", &self.audience.len())
            .field("access_ttl_seconds", &self.access_ttl_seconds)
            .field("refresh_ttl_seconds", &self.refresh_ttl_seconds)
            .field("algorithm_len", &self.algorithm.len())
            .field("tenant_present", &self.tenant_id.is_some())
            .field("signing_key_present", &self.signing_key.is_some())
            .field(
                "signing_key_len",
                &self.signing_key.as_deref().map(str::len),
            )
            .finish()
    }
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            issuer: "https://users.rvoip.local".to_string(),
            audience: vec!["rvoip-api".to_string(), "rvoip-sip".to_string()],
            access_ttl_seconds: 900,      // 15 minutes
            refresh_ttl_seconds: 2592000, // 30 days
            algorithm: "HS256".to_string(),
            tenant_id: None,
            signing_key: None,
        }
    }
}

impl JwtConfig {
    /// Construct JWT configuration with secure default lifetimes and HS256.
    /// Use the builder methods for tenant binding, custom keys, or algorithms.
    pub fn new(issuer: impl Into<String>, audience: Vec<String>) -> Self {
        Self {
            issuer: issuer.into(),
            audience,
            ..Self::default()
        }
    }

    pub fn with_token_ttls(mut self, access: Duration, refresh: Duration) -> Self {
        self.access_ttl_seconds = access.as_secs();
        self.refresh_ttl_seconds = refresh.as_secs();
        self
    }

    pub fn with_algorithm(mut self, algorithm: impl Into<String>) -> Self {
        self.algorithm = algorithm.into();
        self
    }

    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn with_signing_key(mut self, signing_key: impl Into<String>) -> Self {
        self.signing_key = Some(signing_key.into());
        self
    }
}

impl JwtIssuer {
    pub fn new(mut config: JwtConfig) -> Result<Self> {
        if let Some(tenant) = config.tenant_id.as_ref() {
            if tenant.is_empty()
                || tenant.len() > 128
                || tenant.trim() != tenant
                || tenant.chars().any(char::is_control)
            {
                return Err(Error::Config(
                    "JWT tenant_id must be 1-128 non-control characters without surrounding whitespace"
                        .to_string(),
                ));
            }
        }
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
            tenant_id: self.config.tenant_id.clone(),
        };

        encode(&self.header, &claims, &self.encoding_key).map_err(|e| Error::Jwt(e))
    }

    pub fn create_refresh_token(&self, user_id: &str) -> Result<String> {
        let now = chrono::Utc::now();
        let exp = now + chrono::Duration::seconds(self.config.refresh_ttl_seconds as i64);

        let claims = RefreshTokenWireClaims {
            claims: RefreshTokenClaims {
                iss: self.config.issuer.clone(),
                sub: user_id.to_string(),
                jti: Uuid::new_v4().to_string(),
                exp: exp.timestamp() as u64,
                iat: now.timestamp() as u64,
            },
            tenant_id: self.config.tenant_id.clone(),
        };

        encode(&self.header, &claims, &self.encoding_key).map_err(|e| Error::Jwt(e))
    }

    pub fn validate_refresh_token(&self, token: &str) -> Result<RefreshTokenClaims> {
        let mut validation = Validation::new(self.header.alg);
        validation.set_issuer(&[self.config.issuer.clone()]);
        validation.validate_exp = true;

        let token_data = decode::<RefreshTokenWireClaims>(token, &self.decoding_key, &validation)
            .map_err(|e| Error::Jwt(e))?;
        self.enforce_tenant_binding(token_data.claims.tenant_id.as_deref(), "refresh")?;

        Ok(token_data.claims.claims)
    }

    pub fn validate_access_token(&self, token: &str) -> Result<UserClaims> {
        let mut validation = Validation::new(self.header.alg);
        validation.set_issuer(&[self.config.issuer.clone()]);
        validation.set_audience(&self.config.audience);
        validation.validate_exp = true;

        let token_data = decode::<UserClaims>(token, &self.decoding_key, &validation)
            .map_err(|e| Error::Jwt(e))?;
        self.enforce_tenant_binding(token_data.claims.tenant_id.as_deref(), "access")?;

        Ok(token_data.claims)
    }

    fn enforce_tenant_binding(&self, actual: Option<&str>, token_kind: &str) -> Result<()> {
        if actual == self.config.tenant_id.as_deref() {
            return Ok(());
        }

        Err(Error::Validation(format!(
            "{token_kind} token tenant does not match issuer binding"
        )))
    }

    /// Issuer configured for generated tokens.
    pub fn issuer(&self) -> &str {
        &self.config.issuer
    }

    /// Audiences configured for generated access tokens.
    pub fn audience(&self) -> &[String] {
        &self.config.audience
    }

    /// Fixed tenant bound to access tokens from this issuer, when configured.
    pub fn tenant_id(&self) -> Option<&str> {
        self.config.tenant_id.as_deref()
    }

    /// Algorithm configured for issued JWTs.
    pub fn algorithm(&self) -> Algorithm {
        self.header.alg
    }

    /// Verification key for tokens issued by this service.
    pub fn decoding_key(&self) -> &DecodingKey {
        &self.decoding_key
    }

    /// Get the public key in JWK format for asymmetric JWT validation.
    ///
    /// HS256 uses a symmetric secret and is intentionally not exposed through
    /// JWKS. Use an in-process HMAC validator or configure RS256 for public
    /// JWKS-based validation.
    pub fn public_key_jwk(&self) -> Result<serde_json::Value> {
        if self.config.algorithm != "RS256" {
            return Err(Error::Config(format!(
                "{} uses symmetric signing; public JWKS is only available for RS256",
                self.config.algorithm
            )));
        }

        let signing_key = self.config.signing_key.as_ref().ok_or_else(|| {
            Error::Config("RS256 public JWK requires the configured signing key".to_string())
        })?;
        let private_key = rsa::RsaPrivateKey::from_pkcs8_pem(signing_key)
            .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(signing_key))
            .map_err(|err| Error::Config(format!("invalid RSA signing key for JWK: {err}")))?;
        let public_key = private_key.to_public_key();

        Ok(serde_json::json!({
            "kty": "RSA",
            "use": "sig",
            "kid": self.header.kid.as_ref().unwrap(),
            "alg": self.config.algorithm,
            "n": URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be()),
        }))
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

    fn configured_issuer(tenant: Option<&str>) -> JwtIssuer {
        JwtIssuer::new(JwtConfig {
            issuer: "https://users.test".into(),
            audience: vec!["rvoip-api".into()],
            access_ttl_seconds: 300,
            refresh_ttl_seconds: 3600,
            algorithm: "HS256".into(),
            tenant_id: tenant.map(str::to_owned),
            signing_key: Some("shared-test-signing-key".into()),
        })
        .unwrap()
    }

    fn test_user() -> User {
        let now = chrono::Utc::now();
        User {
            id: "user-a".into(),
            username: "alice".into(),
            email: Some("alice@example.test".into()),
            display_name: None,
            password_hash: "not-used-by-jwt-tests".into(),
            roles: vec!["user".into()],
            active: true,
            created_at: now,
            updated_at: now,
            last_login: None,
        }
    }

    #[test]
    fn test_jwt_config_default() {
        let config = JwtConfig::default();
        assert_eq!(config.issuer, "https://users.rvoip.local");
        assert_eq!(config.access_ttl_seconds, 900);
        assert_eq!(config.algorithm, "HS256");
        assert!(config.tenant_id.is_none());
    }

    #[test]
    fn legacy_claims_without_tenant_remain_parseable() {
        let claims: UserClaims = serde_json::from_value(serde_json::json!({
            "iss": "https://users.test",
            "sub": "user-a",
            "aud": ["rvoip-api"],
            "exp": 1,
            "iat": 1,
            "jti": "token-a",
            "username": "alice",
            "email": null,
            "roles": ["user"],
            "scope": "openid"
        }))
        .unwrap();
        assert!(claims.tenant_id.is_none());
        let encoded = serde_json::to_value(claims).unwrap();
        assert!(encoded.get("tenant_id").is_none());
    }

    #[test]
    fn configured_tenant_is_enforced_on_direct_access_validation() {
        let tenant_a = configured_issuer(Some("tenant-a"));
        let tenant_b = configured_issuer(Some("tenant-b"));
        let access = tenant_a.create_access_token(&test_user()).unwrap();

        assert!(tenant_a.validate_access_token(&access).is_ok());
        assert!(matches!(
            tenant_b.validate_access_token(&access),
            Err(Error::Validation(_))
        ));
    }

    #[test]
    fn refresh_tokens_carry_and_enforce_the_issuer_tenant() {
        let tenant_a = configured_issuer(Some("tenant-a"));
        let tenant_b = configured_issuer(Some("tenant-b"));
        let refresh = tenant_a.create_refresh_token("user-a").unwrap();

        assert!(tenant_a.validate_refresh_token(&refresh).is_ok());
        assert!(matches!(
            tenant_b.validate_refresh_token(&refresh),
            Err(Error::Validation(_))
        ));

        let mut validation = Validation::new(tenant_a.header.alg);
        validation.set_issuer(&[tenant_a.config.issuer.clone()]);
        let decoded =
            decode::<RefreshTokenWireClaims>(&refresh, &tenant_a.decoding_key, &validation)
                .unwrap();
        assert_eq!(decoded.claims.tenant_id.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn tenant_bound_issuer_rejects_legacy_tenantless_tokens_fail_closed() {
        let issuer = configured_issuer(Some("tenant-a"));
        let now = chrono::Utc::now().timestamp() as u64;
        let legacy_access = UserClaims::new(
            "https://users.test",
            "user-a",
            vec!["rvoip-api".into()],
            now + 300,
            now,
            "access-a",
            "alice",
            "openid",
        );
        let access = encode(&issuer.header, &legacy_access, &issuer.encoding_key).unwrap();
        assert!(matches!(
            issuer.validate_access_token(&access),
            Err(Error::Validation(_))
        ));

        let legacy_refresh = RefreshTokenClaims {
            iss: "https://users.test".into(),
            sub: "user-a".into(),
            jti: "refresh-a".into(),
            exp: now + 3600,
            iat: now,
        };
        let refresh = encode(&issuer.header, &legacy_refresh, &issuer.encoding_key).unwrap();
        assert!(matches!(
            issuer.validate_refresh_token(&refresh),
            Err(Error::Validation(_))
        ));
    }

    #[test]
    fn unbound_issuer_accepts_only_legacy_tenantless_tokens() {
        let issuer = configured_issuer(None);
        let access = issuer.create_access_token(&test_user()).unwrap();
        let refresh = issuer.create_refresh_token("user-a").unwrap();
        assert!(issuer.validate_access_token(&access).is_ok());
        assert!(issuer.validate_refresh_token(&refresh).is_ok());

        let tenant_access = configured_issuer(Some("tenant-a"))
            .create_access_token(&test_user())
            .unwrap();
        assert!(matches!(
            issuer.validate_access_token(&tenant_access),
            Err(Error::Validation(_))
        ));
    }

    #[test]
    fn migration_constructors_cover_new_public_fields() {
        let config = JwtConfig::new("https://users.test", vec!["rvoip-api".into()])
            .with_token_ttls(Duration::from_secs(60), Duration::from_secs(120))
            .with_tenant_id("tenant-a")
            .with_signing_key("test-key");
        assert_eq!(config.access_ttl_seconds, 60);
        assert_eq!(config.refresh_ttl_seconds, 120);
        assert_eq!(config.tenant_id.as_deref(), Some("tenant-a"));

        let claims = UserClaims::new(
            "https://users.test",
            "user-a",
            vec!["rvoip-api".into()],
            2,
            1,
            "token-a",
            "alice",
            "openid",
        )
        .with_tenant_id("tenant-a");
        assert_eq!(claims.tenant_id.as_deref(), Some("tenant-a"));
    }
}
