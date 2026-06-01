//! Core types for users-core

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// User account
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub roles: Vec<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
}

/// Request to create a new user
#[derive(Debug, Clone, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub roles: Vec<String>,
}

/// Request to update an existing user
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateUserRequest {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub roles: Option<Vec<String>>,
    pub active: Option<bool>,
}

/// Filter for listing users
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserFilter {
    pub active: Option<bool>,
    pub role: Option<String>,
    pub search: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl User {
    /// Create a new user ID
    pub fn new_id() -> String {
        Uuid::new_v4().to_string()
    }
}
