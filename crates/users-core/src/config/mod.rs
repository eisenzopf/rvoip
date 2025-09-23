//! Configuration for users-core

use serde::Deserialize;
use crate::jwt::JwtConfig;

/// Main configuration
#[derive(Debug, Clone, Deserialize)]
pub struct UsersConfig {
    pub database_url: String,
    pub jwt: JwtConfig,
    pub password: PasswordConfig,
    pub api_bind_address: String,
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

impl UsersConfig {
    /// Load configuration from environment
    pub fn from_env() -> crate::Result<Self> {
        // In a real implementation, this would use the config crate
        // For now, return a default configuration
        Ok(Self::default())
    }
}

impl Default for UsersConfig {
    fn default() -> Self {
        Self {
            database_url: "sqlite://users.db?mode=rwc".to_string(),
            jwt: crate::jwt::JwtConfig::default(),
            password: PasswordConfig::default(),
            api_bind_address: "127.0.0.1:8081".to_string(),
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
