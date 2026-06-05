//! Bridge from users-core to auth-core provider traits.

use std::sync::Arc;

use async_trait::async_trait;
use rvoip_auth_core::{
    ApiKeyVerifier, BearerAuthError, BearerValidator, CredentialAuthError, DigestSecret,
    DigestSecretProvider, JwtValidator, PasswordVerifier, TokenRevocationChecker,
    TokenRevocationContext, TokenRevocationStatus,
};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;

use crate::{AuthenticationService, Error, SipDigestAlgorithmFamily, User};

/// users-core backed auth provider for crates that consume `auth-core` traits.
///
/// This adapter validates users-core JWTs, verifies users-core passwords for
/// Basic-style flows, validates users-core API keys, and exposes dedicated SIP
/// Digest HA1 material. Protocol crates should accept trait objects from
/// `auth-core`, not depend on users-core directly.
pub struct UsersCoreAuthProvider {
    auth_service: Arc<AuthenticationService>,
    jwt_validator: JwtValidator,
}

impl UsersCoreAuthProvider {
    /// Build an auth-core provider bridge from an initialized users-core
    /// [`AuthenticationService`].
    pub fn new(auth_service: Arc<AuthenticationService>) -> Self {
        let issuer = auth_service.jwt_issuer();
        let revocation_checker = Arc::new(UsersCoreTokenRevocationChecker {
            auth_service: auth_service.clone(),
        });
        let jwt_validator =
            JwtValidator::from_decoding_key(issuer.decoding_key().clone(), issuer.algorithm())
                .with_issuer([issuer.issuer().to_string()])
                .with_audience(issuer.audience().iter().cloned())
                .with_revocation_checker(revocation_checker);
        Self {
            auth_service,
            jwt_validator,
        }
    }

    /// Convenience constructor for sharing the bridge across multiple auth
    /// trait objects.
    pub fn shared(auth_service: Arc<AuthenticationService>) -> Arc<Self> {
        Arc::new(Self::new(auth_service))
    }

    pub fn auth_service(&self) -> &Arc<AuthenticationService> {
        &self.auth_service
    }
}

struct UsersCoreTokenRevocationChecker {
    auth_service: Arc<AuthenticationService>,
}

#[async_trait]
impl TokenRevocationChecker for UsersCoreTokenRevocationChecker {
    async fn check_token(
        &self,
        context: &TokenRevocationContext,
    ) -> Result<TokenRevocationStatus, CredentialAuthError> {
        match self
            .auth_service
            .is_access_token_revoked(&context.token_id)
            .await
        {
            Ok(true) => Ok(TokenRevocationStatus::Revoked),
            Ok(false) => Ok(TokenRevocationStatus::Active),
            Err(err) => Err(CredentialAuthError::Unavailable(err.to_string())),
        }
    }
}

#[async_trait]
impl BearerValidator for UsersCoreAuthProvider {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        let assurance = self.jwt_validator.validate(token).await?;
        self.ensure_assurance_user_active(&assurance).await?;
        Ok(assurance)
    }
}

impl UsersCoreAuthProvider {
    async fn ensure_assurance_user_active(
        &self,
        assurance: &IdentityAssurance,
    ) -> Result<(), BearerAuthError> {
        let user_id = match assurance {
            IdentityAssurance::UserAuthorized { user_id, .. } => user_id.as_str(),
            IdentityAssurance::TaskScoped { identity, .. } => identity.as_str(),
            _ => {
                return Err(BearerAuthError::Invalid(
                    "users-core bearer token did not identify a user".to_string(),
                ))
            }
        };

        match self.auth_service.user_store().get_user(user_id).await {
            Ok(Some(user)) if user.active => Ok(()),
            Ok(Some(_)) | Ok(None) => Err(BearerAuthError::Invalid(
                "users-core bearer token user is inactive or missing".to_string(),
            )),
            Err(err) => Err(BearerAuthError::Unavailable(err.to_string())),
        }
    }
}

#[async_trait]
impl TokenRevocationChecker for UsersCoreAuthProvider {
    async fn check_token(
        &self,
        context: &TokenRevocationContext,
    ) -> Result<TokenRevocationStatus, CredentialAuthError> {
        match self
            .auth_service
            .is_access_token_revoked(&context.token_id)
            .await
        {
            Ok(true) => Ok(TokenRevocationStatus::Revoked),
            Ok(false) => Ok(TokenRevocationStatus::Active),
            Err(err) => Err(CredentialAuthError::Unavailable(err.to_string())),
        }
    }
}

#[async_trait]
impl PasswordVerifier for UsersCoreAuthProvider {
    async fn verify_password(
        &self,
        username: &str,
        password: &str,
    ) -> Result<IdentityAssurance, CredentialAuthError> {
        let user = self
            .auth_service
            .verify_password_only(username, password)
            .await
            .map_err(map_users_error)?;
        Ok(identity_for_user(&user, user_scopes(&user)))
    }
}

#[async_trait]
impl ApiKeyVerifier for UsersCoreAuthProvider {
    async fn verify_api_key(
        &self,
        api_key: &str,
    ) -> Result<IdentityAssurance, CredentialAuthError> {
        let (user, key) = self
            .auth_service
            .verify_api_key_only(api_key)
            .await
            .map_err(map_users_error)?;
        Ok(identity_for_user(&user, key.permissions))
    }
}

#[async_trait]
impl DigestSecretProvider for UsersCoreAuthProvider {
    async fn lookup_digest_secret(
        &self,
        username: &str,
        realm: &str,
        algorithm: rvoip_auth_core::DigestAlgorithm,
    ) -> Result<Option<DigestSecret>, CredentialAuthError> {
        let family = SipDigestAlgorithmFamily::from(algorithm);
        let credential = self
            .auth_service
            .lookup_sip_digest_credential(username, realm, family)
            .await
            .map_err(map_users_error)?;
        Ok(credential.map(|credential| DigestSecret::Ha1(credential.ha1)))
    }
}

fn map_users_error(err: Error) -> CredentialAuthError {
    match err {
        Error::InvalidCredentials | Error::UserNotFound(_) | Error::ApiKeyNotFound => {
            CredentialAuthError::Invalid
        }
        other => CredentialAuthError::Unavailable(other.to_string()),
    }
}

fn identity_for_user(user: &User, scopes: Vec<String>) -> IdentityAssurance {
    let identity = IdentityId::from_string(user.id.clone());
    IdentityAssurance::UserAuthorized {
        identity: identity.clone(),
        user_id: identity,
        scopes,
    }
}

fn user_scopes(user: &User) -> Vec<String> {
    let mut scopes = vec![
        "openid".to_string(),
        "profile".to_string(),
        "email".to_string(),
        "sip.register".to_string(),
    ];
    if user.roles.iter().any(|role| role == "admin") {
        scopes.push("admin".to_string());
    }
    scopes
}
