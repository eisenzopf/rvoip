//! User storage interface

use async_trait::async_trait;
use crate::{Result, User, CreateUserRequest, UpdateUserRequest, UserFilter};

/// User storage trait
#[async_trait]
pub trait UserStore: Send + Sync {
    async fn create_user(&self, request: CreateUserRequest) -> Result<User>;
    async fn get_user(&self, id: &str) -> Result<Option<User>>;
    async fn get_user_by_username(&self, username: &str) -> Result<Option<User>>;
    async fn update_user(&self, id: &str, updates: UpdateUserRequest) -> Result<User>;
    async fn delete_user(&self, id: &str) -> Result<()>;
    async fn list_users(&self, filter: UserFilter) -> Result<Vec<User>>;
}

/// SQLite-based user store
#[derive(Clone)]
pub struct SqliteUserStore {
    // TODO: Implement
}

impl SqliteUserStore {
    pub async fn new(_database_url: &str) -> Result<Self> {
        todo!("Implement SqliteUserStore")
    }
}

// Implement UserStore trait
#[async_trait]
impl UserStore for SqliteUserStore {
    async fn create_user(&self, _request: CreateUserRequest) -> Result<User> {
        todo!("Implement create_user")
    }
    
    async fn get_user(&self, _id: &str) -> Result<Option<User>> {
        todo!("Implement get_user")
    }
    
    async fn get_user_by_username(&self, _username: &str) -> Result<Option<User>> {
        todo!("Implement get_user_by_username")
    }
    
    async fn update_user(&self, _id: &str, _updates: UpdateUserRequest) -> Result<User> {
        todo!("Implement update_user")
    }
    
    async fn delete_user(&self, _id: &str) -> Result<()> {
        todo!("Implement delete_user")
    }
    
    async fn list_users(&self, _filter: UserFilter) -> Result<Vec<User>> {
        todo!("Implement list_users")
    }
}

// Also implement ApiKeyStore trait
#[async_trait]
impl crate::ApiKeyStore for SqliteUserStore {
    async fn create_api_key(&self, _request: crate::api_keys::CreateApiKeyRequest) -> Result<(crate::ApiKey, String)> {
        todo!("Implement create_api_key")
    }
    
    async fn validate_api_key(&self, _key: &str) -> Result<Option<crate::ApiKey>> {
        todo!("Implement validate_api_key")
    }
    
    async fn revoke_api_key(&self, _id: &str) -> Result<()> {
        todo!("Implement revoke_api_key")
    }
    
    async fn list_api_keys(&self, _user_id: &str) -> Result<Vec<crate::ApiKey>> {
        todo!("Implement list_api_keys")
    }
}
