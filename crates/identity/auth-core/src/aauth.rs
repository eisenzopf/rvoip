//! AAuth — IETF actor-authentication ([draft-ietf-aauth-*]).
//!
//! AAuth carries two tokens on every authenticated request:
//!
//! - The **subject token** identifies the *user* the call is acting on
//!   behalf of (the human / principal). It is the standard bearer token
//!   today's [`crate::bearer::BearerValidator`] surface validates.
//! - The **actor token** identifies the *agent* that is performing the
//!   action (the bot, the assistant, the headless service). It is
//!   validated by [`ActorTokenValidator`] introduced here.
//!
//! The combined result maps to
//! `rvoip_core::identity::IdentityAssurance::UserAuthorized` with the
//! subject's `sub` claim as `user_id` and the actor's `sub` claim as
//! `identity`. Scopes union both tokens' `scope` / `scopes` claims.
//!
//! v0 ships an [`AAuthValidator`] backed by two [`crate::jwt::JwtValidator`]
//! instances (one per token type). Production deployments swap in
//! whatever AAuth issuer they negotiate with; the wire-protocol shape
//! is fixed by the gap plan §5.1 spec change to `crate::AuthResponse`
//! (`actor_token: Option<String>`).

use std::sync::Arc;

use async_trait::async_trait;
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;

use crate::bearer::{
    ensure_principal_active, AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError,
    BearerValidator,
};

/// Validates an AAuth actor token. Mirrors [`BearerValidator`]'s
/// shape but returns the actor's identity + scopes rather than a
/// full `IdentityAssurance` (the caller combines actor + subject).
#[async_trait]
pub trait ActorTokenValidator: Send + Sync {
    async fn validate_actor(&self, token: &str) -> Result<ActorClaims, BearerAuthError>;

    /// Validate an actor token without discarding issuer or expiry metadata.
    ///
    /// Existing actor validators remain source compatible through this
    /// mapping. Validators backed by JWT/JWKS should override this method so
    /// the combined AAuth credential expires no later than the actor token.
    async fn validate_actor_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        let claims = self.validate_actor(token).await?;
        let assurance = IdentityAssurance::UserAuthorized {
            identity: claims.identity.clone(),
            user_id: claims.identity.clone(),
            scopes: claims.scopes.clone(),
        };
        ensure_principal_active(AuthenticatedPrincipal {
            subject: claims.identity.to_string(),
            tenant: None,
            scopes: claims.scopes,
            issuer: None,
            expires_at: None,
            method: AuthenticationMethod::Bearer,
            assurance,
        })
    }
}

/// Output of [`ActorTokenValidator::validate_actor`] — the actor's
/// identity (typically the `sub` claim of the actor JWT) and any
/// scopes the actor was granted.
#[derive(Clone)]
pub struct ActorClaims {
    pub identity: IdentityId,
    pub scopes: Vec<String>,
}

impl std::fmt::Debug for ActorClaims {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActorClaims")
            .field("identity_present", &!self.identity.as_str().is_empty())
            .field("scope_count", &self.scopes.len())
            .finish()
    }
}

fn actor_claims_from_principal(principal: &AuthenticatedPrincipal) -> ActorClaims {
    let identity = match &principal.assurance {
        IdentityAssurance::UserAuthorized { identity, .. }
        | IdentityAssurance::TaskScoped { identity, .. } => identity.clone(),
        _ => IdentityId::from_string(principal.subject.clone()),
    };
    ActorClaims {
        identity,
        scopes: principal.scopes.clone(),
    }
}

#[async_trait]
impl ActorTokenValidator for crate::jwt::JwtValidator {
    async fn validate_actor(&self, token: &str) -> Result<ActorClaims, BearerAuthError> {
        let principal = BearerValidator::validate_principal(self, token).await?;
        Ok(actor_claims_from_principal(&principal))
    }

    async fn validate_actor_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        BearerValidator::validate_principal(self, token).await
    }
}

#[async_trait]
impl ActorTokenValidator for crate::jwks::JwksJwtValidator {
    async fn validate_actor(&self, token: &str) -> Result<ActorClaims, BearerAuthError> {
        let principal = BearerValidator::validate_principal(self, token).await?;
        Ok(actor_claims_from_principal(&principal))
    }

    async fn validate_actor_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        BearerValidator::validate_principal(self, token).await
    }
}

/// AAuth combined validator. Wraps a subject [`BearerValidator`] and
/// an [`ActorTokenValidator`]; the [`Self::validate_aauth`] method
/// runs both and combines the result into a single
/// [`IdentityAssurance::UserAuthorized`].
///
/// The combined identity assurance reflects the AAuth model:
/// `IdentityAssurance::UserAuthorized` already carries distinct
/// `user_id` and `identity` fields (added in v0.x precisely for this
/// shape), where `user_id` is the human subject and `identity` is
/// the acting agent. v0 stamps
/// [`CredentialKind::AAuth`](rvoip_core_traits::identity::CredentialKind::AAuth) as the
/// credential kind for diagnostics — that's metadata only; the
/// `IdentityAssurance::UserAuthorized` variant has no credential
/// kind field. See `CONVERSATION_PROTOCOL.md` §5.6.
pub struct AAuthValidator {
    subject: Arc<dyn BearerValidator>,
    actor: Arc<dyn ActorTokenValidator>,
}

impl AAuthValidator {
    pub fn new(
        subject: Arc<dyn BearerValidator>,
        actor: Arc<dyn ActorTokenValidator>,
    ) -> Arc<Self> {
        Arc::new(Self { subject, actor })
    }

    /// Validate an AAuth pair while retaining the subject token's ownership
    /// boundary. Neither credential string is copied into the result.
    pub async fn validate_principal(
        &self,
        subject_token: &str,
        actor_token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        if subject_token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        if actor_token.is_empty() {
            return Err(BearerAuthError::Invalid(
                "actor_token required for method=aauth".into(),
            ));
        }
        let subject_principal =
            ensure_principal_active(self.subject.validate_principal(subject_token).await?)?;
        let actor_principal =
            ensure_principal_active(self.actor.validate_actor_principal(actor_token).await?)?;

        // The subject must validate as user-authorized. Anonymous /
        // pseudonymous / identified-without-authorization subject
        // tokens are not enough to support an AAuth claim because the
        // resulting IdentityAssurance::UserAuthorized requires a
        // concrete `user_id` to attach to.
        let subject_identity = match &subject_principal.assurance {
            IdentityAssurance::UserAuthorized { user_id, .. } => user_id.clone(),
            other => {
                return Err(BearerAuthError::Invalid(format!(
                    "AAuth subject token must validate to UserAuthorized; got {}",
                    discriminant_label(other)
                )));
            }
        };

        let actor_identity = match &actor_principal.assurance {
            IdentityAssurance::UserAuthorized { identity, .. }
            | IdentityAssurance::TaskScoped { identity, .. } => identity.clone(),
            _ => IdentityId::from_string(actor_principal.subject.clone()),
        };
        authorize_actor_delegation(&subject_principal, &actor_principal)?;

        let mut merged_scopes = subject_principal.scopes.clone();
        for s in &actor_principal.scopes {
            if !is_delegation_scope(s, &subject_principal.subject) && !merged_scopes.contains(s) {
                merged_scopes.push(s.clone());
            }
        }

        let assurance = IdentityAssurance::UserAuthorized {
            user_id: subject_identity,
            identity: actor_identity,
            scopes: merged_scopes.clone(),
        };
        let expires_at = earliest_expiry(subject_principal.expires_at, actor_principal.expires_at);

        ensure_principal_active(AuthenticatedPrincipal {
            subject: subject_principal.subject,
            tenant: subject_principal.tenant,
            scopes: merged_scopes,
            issuer: subject_principal.issuer,
            expires_at,
            method: AuthenticationMethod::AAuth,
            assurance,
        })
    }

    /// Compatibility projection for callers that only consume assurance.
    pub async fn validate_aauth(
        &self,
        subject_token: &str,
        actor_token: &str,
    ) -> Result<IdentityAssurance, BearerAuthError> {
        Ok(self
            .validate_principal(subject_token, actor_token)
            .await?
            .assurance)
    }
}

fn authorize_actor_delegation(
    subject: &AuthenticatedPrincipal,
    actor: &AuthenticatedPrincipal,
) -> Result<(), BearerAuthError> {
    let same_issuer = subject
        .issuer
        .as_deref()
        .zip(actor.issuer.as_deref())
        .is_some_and(|(subject, actor)| subject == actor);
    let same_tenant = subject
        .tenant
        .as_deref()
        .zip(actor.tenant.as_deref())
        .is_some_and(|(subject, actor)| subject == actor);
    if !same_issuer || !same_tenant {
        return Err(BearerAuthError::Invalid(
            "AAuth actor and subject ownership domains do not match".into(),
        ));
    }
    if !actor
        .scopes
        .iter()
        .any(|scope| is_delegation_scope(scope, &subject.subject))
    {
        return Err(BearerAuthError::Invalid(
            "AAuth actor is not delegated for this subject".into(),
        ));
    }
    Ok(())
}

fn is_delegation_scope(scope: &str, subject: &str) -> bool {
    scope == "aauth:act" || scope.strip_prefix("aauth:act:") == Some(subject)
}

fn earliest_expiry(
    subject: Option<chrono::DateTime<chrono::Utc>>,
    actor: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    match (subject, actor) {
        (Some(subject), Some(actor)) => Some(subject.min(actor)),
        (Some(expiry), None) | (None, Some(expiry)) => Some(expiry),
        (None, None) => None,
    }
}

fn discriminant_label(a: &IdentityAssurance) -> &'static str {
    match a {
        IdentityAssurance::Anonymous => "Anonymous",
        IdentityAssurance::Pseudonymous { .. } => "Pseudonymous",
        IdentityAssurance::Identified { .. } => "Identified",
        IdentityAssurance::TaskScoped { .. } => "TaskScoped",
        IdentityAssurance::UserAuthorized { .. } => "UserAuthorized",
        IdentityAssurance::DtlsFingerprint { .. } => "DtlsFingerprint",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bearer::{bearer_stub, BearerValidator};
    use async_trait::async_trait;
    use rvoip_core_traits::identity::IdentityAssurance;
    use rvoip_core_traits::ids::IdentityId;

    struct StaticActor {
        identity: IdentityId,
        scopes: Vec<String>,
        tenant: &'static str,
    }

    #[async_trait]
    impl ActorTokenValidator for StaticActor {
        async fn validate_actor(&self, token: &str) -> Result<ActorClaims, BearerAuthError> {
            if token.is_empty() {
                return Err(BearerAuthError::Empty);
            }
            Ok(ActorClaims {
                identity: self.identity.clone(),
                scopes: self.scopes.clone(),
            })
        }

        async fn validate_actor_principal(
            &self,
            token: &str,
        ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
            let claims = self.validate_actor(token).await?;
            let assurance = IdentityAssurance::UserAuthorized {
                identity: claims.identity.clone(),
                user_id: claims.identity.clone(),
                scopes: claims.scopes.clone(),
            };
            Ok(AuthenticatedPrincipal {
                subject: claims.identity.to_string(),
                tenant: Some(self.tenant.into()),
                scopes: claims.scopes,
                issuer: Some("https://aauth.example".into()),
                expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(5)),
                method: AuthenticationMethod::AAuth,
                assurance,
            })
        }
    }

    /// Returns a subject validator that yields UserAuthorized so the
    /// combined check actually exercises the merge path (the bundled
    /// `bearer_stub` returns Pseudonymous, which AAuth explicitly
    /// rejects as too weak).
    struct StaticSubject {
        user_id: IdentityId,
        scopes: Vec<String>,
        tenant: &'static str,
    }

    #[async_trait]
    impl BearerValidator for StaticSubject {
        async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
            if token.is_empty() {
                return Err(BearerAuthError::Empty);
            }
            Ok(IdentityAssurance::UserAuthorized {
                identity: self.user_id.clone(),
                user_id: self.user_id.clone(),
                scopes: self.scopes.clone(),
            })
        }

        async fn validate_principal(
            &self,
            token: &str,
        ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
            let assurance = self.validate(token).await?;
            Ok(AuthenticatedPrincipal {
                subject: self.user_id.to_string(),
                tenant: Some(self.tenant.into()),
                scopes: self.scopes.clone(),
                issuer: Some("https://aauth.example".into()),
                expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(5)),
                method: AuthenticationMethod::Bearer,
                assurance,
            })
        }
    }

    fn id(s: &str) -> IdentityId {
        IdentityId::from_string(s.to_string())
    }

    #[tokio::test]
    async fn aauth_combines_subject_and_actor_into_user_authorized() {
        let subject = Arc::new(StaticSubject {
            user_id: id("user:alice"),
            scopes: vec!["calls.write".into()],
            tenant: "tenant-a",
        });
        let actor = Arc::new(StaticActor {
            identity: id("agent:assistant-7"),
            scopes: vec![
                "aauth:act:user:alice".into(),
                "calls.write".into(),
                "calls.transfer".into(),
            ],
            tenant: "tenant-a",
        });
        let v = AAuthValidator::new(subject, actor);

        let assurance = v
            .validate_aauth("subject-tok", "actor-tok")
            .await
            .expect("aauth combine");
        match assurance {
            IdentityAssurance::UserAuthorized {
                user_id,
                identity,
                scopes,
            } => {
                assert_eq!(user_id.as_str(), "user:alice");
                assert_eq!(identity.as_str(), "agent:assistant-7");
                // Scopes union (subject-first ordering).
                assert_eq!(
                    scopes,
                    vec!["calls.write".to_string(), "calls.transfer".to_string()]
                );
            }
            other => panic!(
                "expected UserAuthorized; got {:?}",
                discriminant_label(&other)
            ),
        }
    }

    #[tokio::test]
    async fn aauth_requires_actor_token() {
        let subject = Arc::new(StaticSubject {
            user_id: id("user:alice"),
            scopes: vec![],
            tenant: "tenant-a",
        });
        let actor = Arc::new(StaticActor {
            identity: id("agent:7"),
            scopes: vec![],
            tenant: "tenant-a",
        });
        let v = AAuthValidator::new(subject, actor);
        let err = v.validate_aauth("subj", "").await.unwrap_err();
        match err {
            BearerAuthError::Invalid(msg) => assert!(msg.contains("actor_token"), "{msg}"),
            other => panic!("expected Invalid for empty actor token; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn aauth_rejects_pseudonymous_subject() {
        // bearer_stub returns Pseudonymous — AAuth requires UserAuthorized
        // for the subject, so this must fail.
        let subject = bearer_stub();
        let actor = Arc::new(StaticActor {
            identity: id("agent:7"),
            scopes: vec!["aauth:act".into()],
            tenant: "tenant-a",
        });
        let v = AAuthValidator::new(subject, actor);
        let err = v.validate_aauth("subj-tok", "actor-tok").await.unwrap_err();
        match err {
            BearerAuthError::Invalid(msg) => {
                assert!(msg.contains("Pseudonymous") || msg.contains("unsupported"));
            }
            other => panic!("expected Invalid; got {other:?}"),
        }
    }

    #[tokio::test]
    async fn aauth_rejects_cross_tenant_actor_even_with_delegation_scope() {
        let subject = Arc::new(StaticSubject {
            user_id: id("user:alice"),
            scopes: vec!["calls.write".into()],
            tenant: "tenant-a",
        });
        let actor = Arc::new(StaticActor {
            identity: id("agent:7"),
            scopes: vec!["aauth:act".into(), "calls.transfer".into()],
            tenant: "tenant-b",
        });
        let error = AAuthValidator::new(subject, actor)
            .validate_aauth("subject", "actor")
            .await
            .unwrap_err();
        assert!(matches!(error, BearerAuthError::Invalid(_)));
    }

    #[tokio::test]
    async fn aauth_rejects_actor_without_explicit_delegation() {
        let subject = Arc::new(StaticSubject {
            user_id: id("user:alice"),
            scopes: vec!["calls.write".into()],
            tenant: "tenant-a",
        });
        let actor = Arc::new(StaticActor {
            identity: id("agent:7"),
            scopes: vec!["calls.transfer".into()],
            tenant: "tenant-a",
        });
        let error = AAuthValidator::new(subject, actor)
            .validate_aauth("subject", "actor")
            .await
            .unwrap_err();
        assert!(matches!(error, BearerAuthError::Invalid(_)));
    }
}
