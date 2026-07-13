//! User credential storage for SIP registration authentication
//!
//! Simple in-memory user database for storing and retrieving user credentials.
//! In production, this should be backed by a persistent database.

use crate::error::{RegistrarError, Result};
use dashmap::DashMap;
use rvoip_auth_core::{DigestAlgorithm, DigestSecret};
use std::fmt;
use zeroize::Zeroize;

/// User credentials for authentication
#[derive(Clone)]
pub struct UserCredentials {
    /// Username (typically SIP user part)
    pub username: String,
    /// Password accepted when provisioning or rotating a user.
    ///
    /// [`UserStore`] immediately converts this value into algorithm-specific
    /// HA1 verifiers and wipes the supplied allocation; it is never retained.
    pub password: String,
    /// Authentication realm
    pub realm: String,
    /// Display name (optional)
    pub display_name: Option<String>,
}

impl fmt::Debug for UserCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserCredentials")
            .field("username_present", &!self.username.is_empty())
            .field("username_bytes", &self.username.len())
            .field("password_present", &!self.password.is_empty())
            .field("password_bytes", &self.password.len())
            .field("realm_present", &!self.realm.is_empty())
            .field("realm_bytes", &self.realm.len())
            .field("display_name_present", &self.display_name.is_some())
            .field(
                "display_name_bytes",
                &self.display_name.as_ref().map_or(0, String::len),
            )
            .finish()
    }
}

#[derive(Clone)]
struct StoredUserCredentials {
    username: String,
    realm: String,
    display_name: Option<String>,
    md5_ha1: String,
    sha256_ha1: String,
    sha512256_ha1: String,
}

impl StoredUserCredentials {
    fn from_credentials(credentials: UserCredentials) -> Self {
        let UserCredentials {
            username,
            mut password,
            realm,
            display_name,
        } = credentials;
        let stored = Self::from_password(username, realm, display_name, &password);
        password.zeroize();
        stored
    }

    fn from_password(
        username: String,
        realm: String,
        display_name: Option<String>,
        password: &str,
    ) -> Self {
        Self {
            md5_ha1: DigestAlgorithm::MD5.compute_ha1(&username, &realm, password),
            sha256_ha1: DigestAlgorithm::SHA256.compute_ha1(&username, &realm, password),
            sha512256_ha1: DigestAlgorithm::SHA512256.compute_ha1(&username, &realm, password),
            username,
            realm,
            display_name,
        }
    }

    fn digest_secret(&self, realm: &str, algorithm: DigestAlgorithm) -> Option<DigestSecret> {
        if self.realm != realm {
            return None;
        }
        let ha1 = match algorithm {
            DigestAlgorithm::MD5 | DigestAlgorithm::MD5Sess => &self.md5_ha1,
            DigestAlgorithm::SHA256 | DigestAlgorithm::SHA256Sess => &self.sha256_ha1,
            DigestAlgorithm::SHA512256 | DigestAlgorithm::SHA512256Sess => &self.sha512256_ha1,
        };
        Some(DigestSecret::Ha1(ha1.clone()))
    }
}

impl fmt::Debug for StoredUserCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredUserCredentials")
            .field("username_present", &!self.username.is_empty())
            .field("username_bytes", &self.username.len())
            .field("realm_present", &!self.realm.is_empty())
            .field("realm_bytes", &self.realm.len())
            .field("display_name_present", &self.display_name.is_some())
            .field("digest_verifier_count", &3usize)
            .finish()
    }
}

/// In-memory user credential store
pub struct UserStore {
    /// Username -> Credentials mapping
    users: DashMap<String, StoredUserCredentials>,
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
        let mut password = password.into();

        let credentials = StoredUserCredentials::from_password(
            username.clone(),
            self.default_realm.clone(),
            None,
            &password,
        );
        password.zeroize();

        self.users.insert(username, credentials);
        Ok(())
    }

    /// Add a user with full credentials
    pub fn add_user_with_credentials(&self, credentials: UserCredentials) -> Result<()> {
        let username = credentials.username.clone();
        self.users.insert(
            username,
            StoredUserCredentials::from_credentials(credentials),
        );
        Ok(())
    }

    /// Plaintext passwords are deliberately not recoverable from this store.
    #[deprecated(note = "UserStore retains HA1 verifiers; use get_digest_secret")]
    pub fn get_password(&self, username: &str) -> Option<String> {
        let _ = username;
        None
    }

    /// Return non-secret credential metadata.
    ///
    /// The compatibility `password` field is always empty because plaintext
    /// password recovery would defeat the store's security boundary.
    pub fn get_credentials(&self, username: &str) -> Option<UserCredentials> {
        self.users.get(username).map(|entry| UserCredentials {
            username: entry.username.clone(),
            password: String::new(),
            realm: entry.realm.clone(),
            display_name: entry.display_name.clone(),
        })
    }

    /// Return the algorithm-appropriate HA1 verifier for Digest validation.
    pub fn get_digest_secret(
        &self,
        username: &str,
        realm: &str,
        algorithm: DigestAlgorithm,
    ) -> Option<DigestSecret> {
        self.users
            .get(username)
            .and_then(|entry| entry.digest_secret(realm, algorithm))
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
            let mut new_password = new_password.into();
            entry.md5_ha1 =
                DigestAlgorithm::MD5.compute_ha1(&entry.username, &entry.realm, &new_password);
            entry.sha256_ha1 =
                DigestAlgorithm::SHA256.compute_ha1(&entry.username, &entry.realm, &new_password);
            entry.sha512256_ha1 = DigestAlgorithm::SHA512256.compute_ha1(
                &entry.username,
                &entry.realm,
                &new_password,
            );
            new_password.zeroize();
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
        assert!(matches!(
            store.get_digest_secret("alice", "test.realm", DigestAlgorithm::MD5),
            Some(DigestSecret::Ha1(_))
        ));
        let metadata = store.get_credentials("alice").unwrap();
        assert!(metadata.password.is_empty());
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
        let expected = DigestAlgorithm::MD5.compute_ha1("charlie", "test.realm", "new_pass");
        assert_eq!(
            store.get_digest_secret("charlie", "test.realm", DigestAlgorithm::MD5),
            Some(DigestSecret::Ha1(expected))
        );
    }

    #[test]
    fn test_user_count() {
        let store = UserStore::new("test.realm");
        assert_eq!(store.user_count(), 0);

        store.add_user("user1", "pass1").unwrap();
        store.add_user("user2", "pass2").unwrap();
        assert_eq!(store.user_count(), 2);
    }

    #[test]
    fn credential_debug_never_contains_plaintext_or_verifiers() {
        let credentials = UserCredentials {
            username: "alice".to_string(),
            password: "debug-password-canary".to_string(),
            realm: "debug-realm-canary".to_string(),
            display_name: Some("debug-name-canary".to_string()),
        };
        let debug = format!("{credentials:?}");
        for canary in [
            "alice",
            "debug-password-canary",
            "debug-realm-canary",
            "debug-name-canary",
        ] {
            assert!(!debug.contains(canary));
        }

        let stored = StoredUserCredentials::from_credentials(credentials);
        let debug = format!("{stored:?}");
        assert!(!debug.contains(&stored.md5_ha1));
        assert!(!debug.contains(&stored.sha256_ha1));
        assert!(!debug.contains(&stored.sha512256_ha1));
    }
}
