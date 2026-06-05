//! WebAuthn/passkey adapter for RVoIP users-core.
//!
//! This crate wraps `webauthn-rs` and uses users-core as the optional
//! first-party credential and token store. Ceremony state is kept server-side
//! with short TTLs; completed passkeys are serialized into users-core passkey
//! credential storage. On successful authentication, users-core JWT/refresh
//! tokens are issued through `AuthenticationService::issue_tokens_for_user`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use parking_lot::Mutex;
use serde::Serialize;
use thiserror::Error;
use url::Url;
use users_core::{
    AuthenticationResult as UsersAuthenticationResult, AuthenticationService, Error as UsersError,
    PasskeyCredential, TokenIssueContext, UpsertPasskeyCredentialRequest,
};
use uuid::Uuid;
use webauthn_rs::prelude::{
    CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse, Webauthn,
    WebauthnBuilder,
};

/// WebAuthn adapter error.
#[derive(Debug, Error)]
pub enum WebauthnAdapterError {
    #[error("WebAuthn configuration error: {0}")]
    Config(String),
    #[error("WebAuthn ceremony failed: {0}")]
    Ceremony(String),
    #[error("WebAuthn ceremony was not found or has expired")]
    UnknownCeremony,
    #[error("users-core error: {0}")]
    Users(#[from] UsersError),
}

pub type Result<T> = std::result::Result<T, WebauthnAdapterError>;

/// Relying-party and ceremony settings.
#[derive(Debug, Clone)]
pub struct WebauthnConfig {
    pub rp_id: String,
    pub rp_origin: Url,
    pub rp_name: String,
    pub ceremony_ttl: Duration,
}

impl WebauthnConfig {
    pub fn new(rp_id: impl Into<String>, rp_origin: Url, rp_name: impl Into<String>) -> Self {
        Self {
            rp_id: rp_id.into(),
            rp_origin,
            rp_name: rp_name.into(),
            ceremony_ttl: Duration::from_secs(300),
        }
    }

    pub fn with_ceremony_ttl(mut self, ttl: Duration) -> Self {
        self.ceremony_ttl = ttl;
        self
    }
}

/// Registration challenge returned to a browser/client.
#[derive(Debug, Clone, Serialize)]
pub struct RegistrationStart {
    pub ceremony_id: String,
    pub challenge: CreationChallengeResponse,
}

/// Authentication challenge returned to a browser/client.
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticationStart {
    pub ceremony_id: String,
    pub challenge: RequestChallengeResponse,
}

#[derive(Clone)]
struct RegistrationCeremony {
    user_id: String,
    state: PasskeyRegistration,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
struct AuthenticationCeremony {
    user_id: String,
    state: PasskeyAuthentication,
    expires_at: chrono::DateTime<Utc>,
}

/// WebAuthn/passkey service backed by users-core.
#[derive(Clone)]
pub struct WebauthnService {
    webauthn: Webauthn,
    auth_service: Arc<AuthenticationService>,
    config: WebauthnConfig,
    registrations: Arc<Mutex<HashMap<String, RegistrationCeremony>>>,
    authentications: Arc<Mutex<HashMap<String, AuthenticationCeremony>>>,
}

impl WebauthnService {
    pub fn new(config: WebauthnConfig, auth_service: Arc<AuthenticationService>) -> Result<Self> {
        if auth_service.enterprise_identity_store().is_none() {
            return Err(WebauthnAdapterError::Config(
                "AuthenticationService must be configured with an EnterpriseIdentityStore"
                    .to_string(),
            ));
        }
        let webauthn = WebauthnBuilder::new(&config.rp_id, &config.rp_origin)
            .map_err(|err| WebauthnAdapterError::Config(err.to_string()))?
            .rp_name(&config.rp_name)
            .build()
            .map_err(|err| WebauthnAdapterError::Config(err.to_string()))?;
        Ok(Self {
            webauthn,
            auth_service,
            config,
            registrations: Arc::new(Mutex::new(HashMap::new())),
            authentications: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Start registering a new passkey for an existing users-core user.
    pub async fn start_registration(&self, user_id: &str) -> Result<RegistrationStart> {
        self.prune_expired();
        let user = self
            .auth_service
            .user_store()
            .get_user(user_id)
            .await?
            .ok_or_else(|| UsersError::UserNotFound(user_id.to_string()))?;
        if !user.active {
            return Err(UsersError::InvalidCredentials.into());
        }
        let user_uuid = Uuid::parse_str(&user.id).unwrap_or_else(|_| Uuid::new_v4());
        let existing = self.stored_passkeys_for_user(&user.id).await?;
        let exclude_credentials = if existing.is_empty() {
            None
        } else {
            Some(
                existing
                    .iter()
                    .map(|passkey| passkey.cred_id().clone())
                    .collect(),
            )
        };
        let display_name = user
            .display_name
            .as_deref()
            .unwrap_or(user.username.as_str());
        let (challenge, state) = self
            .webauthn
            .start_passkey_registration(
                user_uuid,
                &user.username,
                display_name,
                exclude_credentials,
            )
            .map_err(|err| WebauthnAdapterError::Ceremony(err.to_string()))?;
        let ceremony_id = Uuid::new_v4().to_string();
        self.registrations.lock().insert(
            ceremony_id.clone(),
            RegistrationCeremony {
                user_id: user.id,
                state,
                expires_at: Utc::now()
                    + chrono::Duration::from_std(self.config.ceremony_ttl)
                        .unwrap_or_else(|_| chrono::Duration::minutes(5)),
            },
        );
        Ok(RegistrationStart {
            ceremony_id,
            challenge,
        })
    }

    /// Finish passkey registration and persist the completed credential.
    pub async fn finish_registration(
        &self,
        ceremony_id: &str,
        credential: RegisterPublicKeyCredential,
        display_name: Option<String>,
    ) -> Result<PasskeyCredential> {
        let ceremony = self.take_registration(ceremony_id)?;
        let passkey = self
            .webauthn
            .finish_passkey_registration(&credential, &ceremony.state)
            .map_err(|err| WebauthnAdapterError::Ceremony(err.to_string()))?;
        let stored = stored_passkey_request(&ceremony.user_id, &passkey, display_name)?;
        let store = self.enterprise_store()?;
        if store
            .get_passkey_credential(&stored.credential_id)
            .await?
            .is_some()
        {
            return Err(WebauthnAdapterError::Ceremony(
                "credential id is already registered".to_string(),
            ));
        }
        store.upsert_passkey_credential(stored).await?;
        Ok(store
            .get_passkey_credential(&credential_id_for_passkey(&passkey)?)
            .await?
            .ok_or(WebauthnAdapterError::UnknownCeremony)?)
    }

    /// Start authenticating with a user's registered passkeys.
    pub async fn start_authentication(&self, user_id: &str) -> Result<AuthenticationStart> {
        self.prune_expired();
        let user = self
            .auth_service
            .user_store()
            .get_user(user_id)
            .await?
            .ok_or_else(|| UsersError::UserNotFound(user_id.to_string()))?;
        if !user.active {
            return Err(UsersError::InvalidCredentials.into());
        }
        let passkeys = self.stored_passkeys_for_user(&user.id).await?;
        if passkeys.is_empty() {
            return Err(WebauthnAdapterError::Ceremony(
                "user has no passkeys".to_string(),
            ));
        }
        let (challenge, state) = self
            .webauthn
            .start_passkey_authentication(&passkeys)
            .map_err(|err| WebauthnAdapterError::Ceremony(err.to_string()))?;
        let ceremony_id = Uuid::new_v4().to_string();
        self.authentications.lock().insert(
            ceremony_id.clone(),
            AuthenticationCeremony {
                user_id: user.id,
                state,
                expires_at: Utc::now()
                    + chrono::Duration::from_std(self.config.ceremony_ttl)
                        .unwrap_or_else(|_| chrono::Duration::minutes(5)),
            },
        );
        Ok(AuthenticationStart {
            ceremony_id,
            challenge,
        })
    }

    /// Finish authentication, update credential state, and issue users-core
    /// tokens for the authenticated user.
    pub async fn finish_authentication(
        &self,
        ceremony_id: &str,
        credential: PublicKeyCredential,
    ) -> Result<UsersAuthenticationResult> {
        let ceremony = self.take_authentication(ceremony_id)?;
        let result = self
            .webauthn
            .finish_passkey_authentication(&credential, &ceremony.state)
            .map_err(|err| WebauthnAdapterError::Ceremony(err.to_string()))?;
        let credential_id = serde_json::to_string(result.cred_id())
            .map_err(|err| WebauthnAdapterError::Ceremony(err.to_string()))?;
        let mut passkeys = self.stored_passkeys_for_user(&ceremony.user_id).await?;
        let mut updated = false;
        for passkey in &mut passkeys {
            if passkey.update_credential(&result).is_some() {
                let stored = stored_passkey_request(
                    &ceremony.user_id,
                    passkey,
                    Some("Passkey".to_string()),
                )?;
                self.enterprise_store()?
                    .upsert_passkey_credential(stored)
                    .await?;
                updated = true;
                break;
            }
        }
        if !updated {
            self.enterprise_store()?
                .update_passkey_usage(&credential_id, result.counter() as u64)
                .await?;
        }
        Ok(self
            .auth_service
            .issue_tokens_for_user(
                &ceremony.user_id,
                TokenIssueContext::new("webauthn-passkey"),
            )
            .await?)
    }

    pub async fn list_credentials(&self, user_id: &str) -> Result<Vec<PasskeyCredential>> {
        Ok(self
            .enterprise_store()?
            .list_passkey_credentials_for_user(user_id)
            .await?)
    }

    pub async fn delete_credential(&self, credential_id: &str) -> Result<()> {
        Ok(self
            .enterprise_store()?
            .delete_passkey_credential(credential_id)
            .await?)
    }

    fn enterprise_store(&self) -> Result<&Arc<dyn users_core::EnterpriseIdentityStore>> {
        self.auth_service
            .enterprise_identity_store()
            .ok_or_else(|| {
                WebauthnAdapterError::Config(
                    "EnterpriseIdentityStore is not configured".to_string(),
                )
            })
    }

    async fn stored_passkeys_for_user(&self, user_id: &str) -> Result<Vec<Passkey>> {
        let stored = self
            .enterprise_store()?
            .list_passkey_credentials_for_user(user_id)
            .await?;
        stored
            .into_iter()
            .map(|credential| {
                serde_json::from_str::<Passkey>(&credential.public_key)
                    .map_err(|err| WebauthnAdapterError::Ceremony(err.to_string()))
            })
            .collect()
    }

    fn take_registration(&self, ceremony_id: &str) -> Result<RegistrationCeremony> {
        let ceremony = self
            .registrations
            .lock()
            .remove(ceremony_id)
            .ok_or(WebauthnAdapterError::UnknownCeremony)?;
        if ceremony.expires_at <= Utc::now() {
            return Err(WebauthnAdapterError::UnknownCeremony);
        }
        Ok(ceremony)
    }

    fn take_authentication(&self, ceremony_id: &str) -> Result<AuthenticationCeremony> {
        let ceremony = self
            .authentications
            .lock()
            .remove(ceremony_id)
            .ok_or(WebauthnAdapterError::UnknownCeremony)?;
        if ceremony.expires_at <= Utc::now() {
            return Err(WebauthnAdapterError::UnknownCeremony);
        }
        Ok(ceremony)
    }

    fn prune_expired(&self) {
        let now = Utc::now();
        self.registrations
            .lock()
            .retain(|_, ceremony| ceremony.expires_at > now);
        self.authentications
            .lock()
            .retain(|_, ceremony| ceremony.expires_at > now);
    }
}

fn stored_passkey_request(
    user_id: &str,
    passkey: &Passkey,
    display_name: Option<String>,
) -> Result<UpsertPasskeyCredentialRequest> {
    let credential_id = credential_id_for_passkey(passkey)?;
    let public_key = serde_json::to_string(passkey)
        .map_err(|err| WebauthnAdapterError::Ceremony(err.to_string()))?;
    Ok(UpsertPasskeyCredentialRequest {
        credential_id,
        user_id: user_id.to_string(),
        public_key,
        sign_count: 0,
        transports: Vec::new(),
        backup_eligible: false,
        backup_state: false,
        display_name,
    })
}

fn credential_id_for_passkey(passkey: &Passkey) -> Result<String> {
    serde_json::to_string(passkey.cred_id())
        .map_err(|err| WebauthnAdapterError::Ceremony(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use users_core::config::{PasswordConfig, TlsSettings};
    use users_core::jwt::JwtConfig;
    use users_core::{init, CreateUserRequest, UsersConfig};

    fn test_config(db_url: String) -> UsersConfig {
        UsersConfig {
            database_url: db_url,
            jwt: JwtConfig {
                issuer: "https://users.rvoip.local".to_string(),
                audience: vec!["rvoip-app".to_string()],
                access_ttl_seconds: 300,
                refresh_ttl_seconds: 3600,
                algorithm: "HS256".to_string(),
                signing_key: Some("webauthn-test-secret".to_string()),
            },
            password: PasswordConfig {
                min_length: 12,
                require_uppercase: true,
                require_lowercase: true,
                require_numbers: true,
                require_special: false,
                argon2_memory_cost: 1024,
                argon2_time_cost: 2,
                argon2_parallelism: 1,
            },
            api_bind_address: "127.0.0.1:0".to_string(),
            tls: TlsSettings::default(),
        }
    }

    async fn service_and_user() -> (tempfile::TempDir, WebauthnService, String) {
        let temp = tempfile::tempdir().unwrap();
        let db_url = format!(
            "sqlite://{}?mode=rwc",
            temp.path().join("users.db").display()
        );
        let auth = Arc::new(init(test_config(db_url)).await.unwrap());
        let user = auth
            .create_user(CreateUserRequest {
                username: "alice".to_string(),
                password: "SecurePass2026".to_string(),
                email: Some("alice@example.test".to_string()),
                display_name: Some("Alice".to_string()),
                roles: vec!["user".to_string()],
            })
            .await
            .unwrap();
        let config = WebauthnConfig::new(
            "localhost",
            Url::parse("http://localhost:8080").unwrap(),
            "RVoIP Test",
        );
        let service = WebauthnService::new(config, auth).unwrap();
        (temp, service, user.id)
    }

    #[tokio::test]
    async fn service_can_list_and_delete_stored_credentials() {
        let (_temp, service, user_id) = service_and_user().await;
        service
            .enterprise_store()
            .unwrap()
            .upsert_passkey_credential(UpsertPasskeyCredentialRequest {
                credential_id: "cred-1".to_string(),
                user_id: user_id.clone(),
                public_key: "{}".to_string(),
                sign_count: 0,
                transports: vec!["internal".to_string()],
                backup_eligible: false,
                backup_state: false,
                display_name: Some("test passkey".to_string()),
            })
            .await
            .unwrap();

        let credentials = service.list_credentials(&user_id).await.unwrap();
        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].credential_id, "cred-1");

        service.delete_credential("cred-1").await.unwrap();
        assert!(service.list_credentials(&user_id).await.unwrap().is_empty());
    }

    #[test]
    fn invalid_rp_origin_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let db_url = format!(
            "sqlite://{}?mode=rwc",
            temp.path().join("users.db").display()
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let auth = Arc::new(runtime.block_on(init(test_config(db_url))).unwrap());
        let config = WebauthnConfig::new(
            "example.com",
            Url::parse("https://different.example.test").unwrap(),
            "RVoIP Test",
        );
        assert!(matches!(
            WebauthnService::new(config, auth),
            Err(WebauthnAdapterError::Config(_))
        ));
    }
}
