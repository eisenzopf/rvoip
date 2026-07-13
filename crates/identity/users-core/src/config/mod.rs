//! Configuration for users-core

use crate::jwt::JwtConfig;
use serde::Deserialize;

/// Main configuration
#[derive(Clone, Deserialize)]
pub struct UsersConfig {
    pub database_url: String,
    pub jwt: JwtConfig,
    pub password: PasswordConfig,
    pub api_bind_address: String,
    #[serde(default)]
    pub tls: TlsSettings,
}

impl std::fmt::Debug for UsersConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UsersConfig")
            .field("database_url_present", &!self.database_url.is_empty())
            .field("jwt", &self.jwt)
            .field("password", &self.password)
            .field(
                "api_bind_address_present",
                &!self.api_bind_address.is_empty(),
            )
            .field("tls", &self.tls)
            .finish()
    }
}

/// Password configuration
#[derive(Debug, Clone, Deserialize)]
pub struct PasswordConfig {
    pub min_length: usize,
    pub require_uppercase: bool,
    pub require_lowercase: bool,
    pub require_numbers: bool,
    pub require_special: bool,
    pub argon2_memory_cost: u32,
    pub argon2_time_cost: u32,
    pub argon2_parallelism: u32,
}

/// TLS/HTTPS configuration
#[derive(Clone, Deserialize)]
pub struct TlsSettings {
    pub enabled: bool,
    pub cert_path: String,
    pub key_path: String,
    pub require_tls: bool, // If true, refuse to start without TLS
}

impl std::fmt::Debug for TlsSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TlsSettings")
            .field("enabled", &self.enabled)
            .field("cert_path_present", &!self.cert_path.is_empty())
            .field("cert_path_bytes", &self.cert_path.len())
            .field("key_path_present", &!self.key_path.is_empty())
            .field("key_path_bytes", &self.key_path.len())
            .field("require_tls", &self.require_tls)
            .finish()
    }
}

impl Default for TlsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            cert_path: "certs/server.crt".to_string(),
            key_path: "certs/server.key".to_string(),
            require_tls: true, // Fail safe by default
        }
    }
}

impl UsersConfig {
    /// Load configuration from environment
    pub fn from_env() -> crate::Result<Self> {
        let mut config = Self::default();

        if let Ok(value) = std::env::var("RVOIP_USERS_DATABASE_URL") {
            config.database_url = value;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_API_BIND_ADDRESS") {
            config.api_bind_address = value;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_JWT_ISSUER") {
            config.jwt.issuer = value;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_JWT_AUDIENCE") {
            config.jwt.audience = value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect();
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_JWT_ACCESS_TTL_SECONDS") {
            config.jwt.access_ttl_seconds =
                parse_env_u64("RVOIP_USERS_JWT_ACCESS_TTL_SECONDS", &value)?;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_JWT_REFRESH_TTL_SECONDS") {
            config.jwt.refresh_ttl_seconds =
                parse_env_u64("RVOIP_USERS_JWT_REFRESH_TTL_SECONDS", &value)?;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_JWT_ALGORITHM") {
            config.jwt.algorithm = value;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_JWT_TENANT_ID") {
            config.jwt.tenant_id = Some(value);
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_JWT_SIGNING_KEY") {
            config.jwt.signing_key = Some(value);
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_PASSWORD_MIN_LENGTH") {
            config.password.min_length =
                parse_env_usize("RVOIP_USERS_PASSWORD_MIN_LENGTH", &value)?;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_REQUIRE_SPECIAL") {
            config.password.require_special =
                parse_env_bool("RVOIP_USERS_REQUIRE_SPECIAL", &value)?;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_TLS_ENABLED") {
            config.tls.enabled = parse_env_bool("RVOIP_USERS_TLS_ENABLED", &value)?;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_TLS_REQUIRE") {
            config.tls.require_tls = parse_env_bool("RVOIP_USERS_TLS_REQUIRE", &value)?;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_TLS_CERT_PATH") {
            config.tls.cert_path = value;
        }
        if let Ok(value) = std::env::var("RVOIP_USERS_TLS_KEY_PATH") {
            config.tls.key_path = value;
        }

        Ok(config)
    }
}

fn parse_env_u64(name: &str, value: &str) -> crate::Result<u64> {
    value
        .parse()
        .map_err(|err| crate::Error::Config(format!("{name}: {err}")))
}

fn parse_env_usize(name: &str, value: &str) -> crate::Result<usize> {
    value
        .parse()
        .map_err(|err| crate::Error::Config(format!("{name}: {err}")))
}

fn parse_env_bool(name: &str, value: &str) -> crate::Result<bool> {
    value
        .parse()
        .map_err(|err| crate::Error::Config(format!("{name}: {err}")))
}

impl Default for UsersConfig {
    fn default() -> Self {
        Self {
            database_url: "sqlite://users.db?mode=rwc".to_string(),
            jwt: crate::jwt::JwtConfig::default(),
            password: PasswordConfig::default(),
            api_bind_address: "127.0.0.1:8081".to_string(),
            tls: TlsSettings::default(),
        }
    }
}

impl Default for PasswordConfig {
    fn default() -> Self {
        Self {
            min_length: 8,
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_special: false,
            argon2_memory_cost: 65536,
            argon2_time_cost: 3,
            argon2_parallelism: 4,
        }
    }
}
