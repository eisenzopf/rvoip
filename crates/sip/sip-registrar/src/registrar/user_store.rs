//! User credential storage for SIP registration authentication
//!
//! Simple in-memory user database for storing and retrieving user credentials.
//! In production, this should be backed by a persistent database.

use crate::error::{RegistrarError, Result};
use dashmap::DashMap;

/// User credentials for authentication
#[derive(Debug, Clone)]
pub struct UserCredentials {
    /// Username (typically SIP user part)
    pub username: String,
    /// Password (stored in plaintext for simplicity - hash in production!)
    pub password: String,
    /// Authentication realm
    pub realm: String,
    /// Display name (optional)
    pub display_name: Option<String>,
}

/// In-memory user credential store
pub struct UserStore {
    /// Username -> Credentials mapping
    users: DashMap<String, UserCredentials>,
    /// Default realm for all users
    default_realm: String,
}

impl UserStore {
    /// Create a new user store with specified default realm
    pub fn new(realm: impl Into<String>) -> Self {
        Self {
            users: DashMap::new(),
            default_realm: realm.into(),
        }
    }

    /// Add a user with username and password
    pub fn add_user(&self, username: impl Into<String>, password: impl Into<String>) -> Result<()> {
        let username = username.into();
        let password = password.into();

        let credentials = UserCredentials {
            username: username.clone(),
            password,
            realm: self.default_realm.clone(),
            display_name: None,
        };

        self.users.insert(username, credentials);
        Ok(())
    }

    /// Add a user with full credentials
    pub fn add_user_with_credentials(&self, credentials: UserCredentials) -> Result<()> {
        self.users.insert(credentials.username.clone(), credentials);
        Ok(())
    }

    /// Get user's password
    pub fn get_password(&self, username: &str) -> Option<String> {
        self.users.get(username).map(|entry| entry.password.clone())
    }

    /// Get user's full credentials
    pub fn get_credentials(&self, username: &str) -> Option<UserCredentials> {
        self.users.get(username).map(|entry| entry.clone())
    }

    /// Check if user exists
    pub fn user_exists(&self, username: &str) -> bool {
        self.users.contains_key(username)
    }

    /// Remove a user
    pub fn remove_user(&self, username: &str) -> Result<()> {
        self.users.remove(username);
        Ok(())
    }

    /// Update user's password
    pub fn update_password(&self, username: &str, new_password: impl Into<String>) -> Result<()> {
        if let Some(mut entry) = self.users.get_mut(username) {
            entry.password = new_password.into();
            Ok(())
        } else {
            Err(RegistrarError::UserNotFound(username.to_string()))
        }
    }

    /// Get number of users
    pub fn user_count(&self) -> usize {
        self.users.len()
    }

    /// List all usernames
    pub fn list_users(&self) -> Vec<String> {
        self.users.iter().map(|entry| entry.key().clone()).collect()
    }

    /// Clear all users
    pub fn clear(&self) {
        self.users.clear();
    }

    /// Get default realm
    pub fn realm(&self) -> &str {
        &self.default_realm
    }
}

impl Default for UserStore {
    fn default() -> Self {
        Self::new("rvoip.local")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_get_user() {
        let store = UserStore::new("test.realm");
        store.add_user("alice", "password123").unwrap();

        assert!(store.user_exists("alice"));
        assert_eq!(store.get_password("alice"), Some("password123".to_string()));
    }

    #[test]
    fn test_remove_user() {
        let store = UserStore::new("test.realm");
        store.add_user("bob", "secret").unwrap();

        assert!(store.user_exists("bob"));
        store.remove_user("bob").unwrap();
        assert!(!store.user_exists("bob"));
    }

    #[test]
    fn test_update_password() {
        let store = UserStore::new("test.realm");
        store.add_user("charlie", "old_pass").unwrap();

        store.update_password("charlie", "new_pass").unwrap();
        assert_eq!(store.get_password("charlie"), Some("new_pass".to_string()));
    }

    #[test]
    fn test_user_count() {
        let store = UserStore::new("test.realm");
        assert_eq!(store.user_count(), 0);

        store.add_user("user1", "pass1").unwrap();
        store.add_user("user2", "pass2").unwrap();
        assert_eq!(store.user_count(), 2);
    }
}
