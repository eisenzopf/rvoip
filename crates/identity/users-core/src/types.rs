//! Core types for users-core

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// User account
#[derive(Clone, Serialize, Deserialize)]
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

impl std::fmt::Debug for User {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("User")
            .field("id_present", &!self.id.is_empty())
            .field("username_present", &!self.username.is_empty())
            .field("email_present", &self.email.is_some())
            .field("display_name_present", &self.display_name.is_some())
            .field("password_hash_present", &!self.password_hash.is_empty())
            .field("password_hash_len", &self.password_hash.len())
            .field("role_count", &self.roles.len())
            .field("active", &self.active)
            .field("last_login_present", &self.last_login.is_some())
            .finish()
    }
}

/// Request to create a new user
#[derive(Clone, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub roles: Vec<String>,
}

impl std::fmt::Debug for CreateUserRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CreateUserRequest")
            .field("username_present", &!self.username.is_empty())
            .field("username_len", &self.username.len())
            .field("password_present", &!self.password.is_empty())
            .field("password_len", &self.password.len())
            .field("email_present", &self.email.is_some())
            .field("display_name_present", &self.display_name.is_some())
            .field("role_count", &self.roles.len())
            .finish()
    }
}

/// Request to update an existing user
#[derive(Clone, Deserialize)]
pub struct UpdateUserRequest {
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub roles: Option<Vec<String>>,
    pub active: Option<bool>,
}

impl std::fmt::Debug for UpdateUserRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdateUserRequest")
            .field("email_present", &self.email.is_some())
            .field("display_name_present", &self.display_name.is_some())
            .field("roles_present", &self.roles.is_some())
            .field("active_present", &self.active.is_some())
            .finish()
    }
}

/// Filter for listing users
#[derive(Clone, Default, Deserialize)]
pub struct UserFilter {
    pub active: Option<bool>,
    pub role: Option<String>,
    pub search: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

impl std::fmt::Debug for UserFilter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserFilter")
            .field("active_present", &self.active.is_some())
            .field("role_present", &self.role.is_some())
            .field("search_present", &self.search.is_some())
            .field("limit", &self.limit)
            .field("offset", &self.offset)
            .finish()
    }
}

/// External identity link from an IdP or provisioning source to a users-core user.
#[derive(Clone, Serialize, Deserialize)]
pub struct ExternalIdentity {
    pub provider_id: String,
    pub external_subject: String,
    pub user_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub groups: Vec<String>,
    pub active: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for ExternalIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExternalIdentity")
            .field("provider_present", &!self.provider_id.is_empty())
            .field("subject_present", &!self.external_subject.is_empty())
            .field("user_present", &!self.user_id.is_empty())
            .field("email_present", &self.email.is_some())
            .field("username_present", &self.username.is_some())
            .field("display_name_present", &self.display_name.is_some())
            .field("group_count", &self.groups.len())
            .field("active", &self.active)
            .field("last_seen_present", &self.last_seen_at.is_some())
            .finish()
    }
}

/// Request to create or update an external identity link.
#[derive(Clone, Serialize, Deserialize)]
pub struct UpsertExternalIdentityRequest {
    pub provider_id: String,
    pub external_subject: String,
    pub user_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub groups: Vec<String>,
    pub active: bool,
}

impl std::fmt::Debug for UpsertExternalIdentityRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpsertExternalIdentityRequest")
            .field("provider_present", &!self.provider_id.is_empty())
            .field("subject_present", &!self.external_subject.is_empty())
            .field("user_present", &!self.user_id.is_empty())
            .field("email_present", &self.email.is_some())
            .field("username_present", &self.username.is_some())
            .field("display_name_present", &self.display_name.is_some())
            .field("group_count", &self.groups.len())
            .field("active", &self.active)
            .finish()
    }
}

/// Stored WebAuthn/passkey credential metadata.
#[derive(Clone, Serialize, Deserialize)]
pub struct PasskeyCredential {
    pub credential_id: String,
    pub user_id: String,
    pub public_key: String,
    pub sign_count: u64,
    pub transports: Vec<String>,
    pub backup_eligible: bool,
    pub backup_state: bool,
    pub display_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for PasskeyCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PasskeyCredential")
            .field("credential_present", &!self.credential_id.is_empty())
            .field("user_present", &!self.user_id.is_empty())
            .field("public_key_present", &!self.public_key.is_empty())
            .field("transport_count", &self.transports.len())
            .field("backup_eligible", &self.backup_eligible)
            .field("backup_state", &self.backup_state)
            .field("display_name_present", &self.display_name.is_some())
            .field("last_used_present", &self.last_used_at.is_some())
            .finish()
    }
}

/// Request to create or replace a passkey credential.
#[derive(Clone, Serialize, Deserialize)]
pub struct UpsertPasskeyCredentialRequest {
    pub credential_id: String,
    pub user_id: String,
    pub public_key: String,
    pub sign_count: u64,
    pub transports: Vec<String>,
    pub backup_eligible: bool,
    pub backup_state: bool,
    pub display_name: Option<String>,
}

impl std::fmt::Debug for UpsertPasskeyCredentialRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpsertPasskeyCredentialRequest")
            .field("credential_present", &!self.credential_id.is_empty())
            .field("user_present", &!self.user_id.is_empty())
            .field("public_key_present", &!self.public_key.is_empty())
            .field("transport_count", &self.transports.len())
            .field("backup_eligible", &self.backup_eligible)
            .field("backup_state", &self.backup_state)
            .field("display_name_present", &self.display_name.is_some())
            .finish()
    }
}

impl User {
    /// Create a new user ID
    pub fn new_id() -> String {
        Uuid::new_v4().to_string()
    }
}
