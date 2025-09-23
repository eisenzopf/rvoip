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
        todo!("Implement configuration loading")
    }
}
