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
use rvoip_core_traits::identity::{CredentialKind, IdentityAssurance};
use rvoip_core_traits::ids::IdentityId;

use crate::bearer::{BearerAuthError, BearerValidator};

/// Validates an AAuth actor token. Mirrors [`BearerValidator`]'s
/// shape but returns the actor's identity + scopes rather than a
/// full `IdentityAssurance` (the caller combines actor + subject).
#[async_trait]
pub trait ActorTokenValidator: Send + Sync {
    async fn validate_actor(&self, token: &str) -> Result<ActorClaims, BearerAuthError>;
}

/// Output of [`ActorTokenValidator::validate_actor`] — the actor's
/// identity (typically the `sub` claim of the actor JWT) and any
/// scopes the actor was granted.
#[derive(Clone, Debug)]
pub struct ActorClaims {
    pub identity: IdentityId,
    pub scopes: Vec<String>,
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
/// the acting agent. v0 stamps [`CredentialKind::AAuth`] as the
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

    /// Validate an AAuth pair. The subject token is validated via the
    /// subject [`BearerValidator`]; the actor token is validated via
    /// the [`ActorTokenValidator`]. On success, returns
    /// [`IdentityAssurance::UserAuthorized`] with the actor as
    /// `identity` and the subject as `user_id`. Scopes are the union
    /// of both tokens' scopes (preserving subject-first order).
    pub async fn validate_aauth(
        &self,
        subject_token: &str,
        actor_token: &str,
    ) -> Result<IdentityAssurance, BearerAuthError> {
        if subject_token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        if actor_token.is_empty() {
            return Err(BearerAuthError::Invalid(
                "actor_token required for method=aauth".into(),
            ));
        }
        let subject_assurance = self.subject.validate(subject_token).await?;
        let actor_claims = self.actor.validate_actor(actor_token).await?;

        // The subject must validate as user-authorized. Anonymous /
        // pseudonymous / identified-without-authorization subject
        // tokens are not enough to support an AAuth claim because the
        // resulting IdentityAssurance::UserAuthorized requires a
        // concrete `user_id` to attach to.
        let (subject_identity, subject_scopes) = match subject_assurance {
            IdentityAssurance::UserAuthorized {
                user_id, scopes, ..
            } => (user_id, scopes),
            other => {
                return Err(BearerAuthError::Invalid(format!(
                    "AAuth subject token must validate to UserAuthorized; got {}",
                    discriminant_label(&other)
                )));
            }
        };

        let mut merged_scopes = subject_scopes;
        for s in actor_claims.scopes {
            if !merged_scopes.contains(&s) {
                merged_scopes.push(s);
            }
        }

        let _ = CredentialKind::AAuth; // tagged in design docs; not stored on UserAuthorized.

        Ok(IdentityAssurance::UserAuthorized {
            user_id: subject_identity,
            identity: actor_claims.identity,
            scopes: merged_scopes,
        })
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
    }

    /// Returns a subject validator that yields UserAuthorized so the
    /// combined check actually exercises the merge path (the bundled
    /// `bearer_stub` returns Pseudonymous, which AAuth explicitly
    /// rejects as too weak).
    struct StaticSubject {
        user_id: IdentityId,
        scopes: Vec<String>,
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
    }

    fn id(s: &str) -> IdentityId {
        IdentityId::from_string(s.to_string())
    }

    #[tokio::test]
    async fn aauth_combines_subject_and_actor_into_user_authorized() {
        let subject = Arc::new(StaticSubject {
            user_id: id("user:alice"),
            scopes: vec!["calls.write".into()],
        });
        let actor = Arc::new(StaticActor {
            identity: id("agent:assistant-7"),
            scopes: vec!["calls.write".into(), "calls.transfer".into()],
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
            other => panic!("expected UserAuthorized; got {:?}", discriminant_label(&other)),
        }
    }

    #[tokio::test]
    async fn aauth_requires_actor_token() {
        let subject = Arc::new(StaticSubject {
            user_id: id("user:alice"),
            scopes: vec![],
        });
        let actor = Arc::new(StaticActor {
            identity: id("agent:7"),
            scopes: vec![],
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
            scopes: vec![],
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
}
