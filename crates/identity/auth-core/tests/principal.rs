use async_trait::async_trait;
use chrono::{Duration, Utc};
use rvoip_auth_core::{
    ensure_principal_active, AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError,
    BearerValidator,
};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;

struct AssuranceOnlyValidator;

struct ExpiredAssuranceValidator;

#[async_trait]
impl BearerValidator for AssuranceOnlyValidator {
    async fn validate(&self, _token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        Ok(IdentityAssurance::TaskScoped {
            identity: IdentityId::from_string("task-subject"),
            task_id: "task-1".into(),
            scopes: vec!["broadcast:subscribe".into()],
            expires_at: Utc::now() + Duration::minutes(5),
        })
    }
}

#[async_trait]
impl BearerValidator for ExpiredAssuranceValidator {
    async fn validate(&self, _token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        Ok(IdentityAssurance::TaskScoped {
            identity: IdentityId::from_string("expired-task"),
            task_id: "task-expired".into(),
            scopes: vec!["calls:read".into()],
            expires_at: Utc::now() - Duration::seconds(1),
        })
    }
}

#[tokio::test]
async fn default_bearer_mapping_preserves_available_assurance_claims() {
    let principal = AssuranceOnlyValidator
        .validate_principal("opaque-token")
        .await
        .unwrap();

    assert_eq!(principal.subject, "task-subject");
    assert_eq!(principal.scopes, vec!["broadcast:subscribe"]);
    assert_eq!(principal.method, AuthenticationMethod::Bearer);
    assert!(principal.expires_at.is_some());
    assert!(!principal.is_expired());
}

#[tokio::test]
async fn default_bearer_mapping_rejects_expired_assurance() {
    let result = ExpiredAssuranceValidator
        .validate_principal("opaque-token")
        .await;

    assert!(
        matches!(result, Err(BearerAuthError::Invalid(ref reason)) if reason.contains("expired"))
    );
}

#[test]
fn principal_boundary_rejects_empty_and_control_character_identities() {
    let mut principal = AuthenticatedPrincipal::anonymous();
    principal.subject = " \t".into();
    assert!(matches!(
        ensure_principal_active(principal),
        Err(BearerAuthError::Invalid(ref reason)) if reason.contains("subject is empty")
    ));

    let mut principal = AuthenticatedPrincipal::anonymous();
    principal.subject = "user:alice".into();
    principal.issuer = Some("https://issuer.example\r\nforged".into());
    assert!(matches!(
        ensure_principal_active(principal),
        Err(BearerAuthError::Invalid(ref reason)) if reason.contains("issuer") && reason.contains("control")
    ));
}
