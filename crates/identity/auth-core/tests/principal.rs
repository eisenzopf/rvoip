use async_trait::async_trait;
use chrono::{Duration, Utc};
use rvoip_auth_core::{
    ensure_principal_active, AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError,
    BearerValidator, ValidatedBearer, MAX_BEARER_ISSUER_BYTES, MAX_BEARER_SUBJECT_BYTES,
    MAX_BEARER_TENANT_BYTES, MAX_BEARER_TOKEN_ID_BYTES,
};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;
use std::time::{Duration as StdDuration, SystemTime};

struct AssuranceOnlyValidator;

struct ExpiredAssuranceValidator;

#[test]
fn validated_bearer_is_public_at_root_and_module_paths() {
    fn root_path(_: Option<rvoip_auth_core::ValidatedBearer>) {}
    fn module_path(_: Option<rvoip_auth_core::bearer::ValidatedBearer>) {}

    root_path(None);
    module_path(None);
}

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
async fn existing_validate_only_implementations_gain_empty_credential_metadata() {
    let credential = AssuranceOnlyValidator
        .validate_credential("opaque-token")
        .await
        .unwrap();

    assert_eq!(credential.principal.subject, "task-subject");
    assert!(credential.token_id.is_none());
    assert!(credential.issued_at.is_none());
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

    for (field, value) in [
        ("issuer", Some(" \t".to_string())),
        ("tenant", Some("".to_string())),
    ] {
        let mut principal = AuthenticatedPrincipal::anonymous();
        principal.subject = "user:alice".into();
        match field {
            "issuer" => principal.issuer = value,
            "tenant" => principal.tenant = value,
            _ => unreachable!(),
        }
        assert!(matches!(
            ensure_principal_active(principal),
            Err(BearerAuthError::Invalid(ref reason)) if reason.contains(field) && reason.contains("empty")
        ));
    }
}

#[test]
fn principal_boundary_rejects_oversized_subject_issuer_and_tenant() {
    for (field, bytes) in [
        ("subject", MAX_BEARER_SUBJECT_BYTES),
        ("issuer", MAX_BEARER_ISSUER_BYTES),
        ("tenant", MAX_BEARER_TENANT_BYTES),
    ] {
        let mut principal = AuthenticatedPrincipal::anonymous();
        principal.subject = "user:alice".into();
        let oversized = "x".repeat(bytes + 1);
        match field {
            "subject" => principal.subject = oversized,
            "issuer" => principal.issuer = Some(oversized),
            "tenant" => principal.tenant = Some(oversized),
            _ => unreachable!(),
        }
        assert!(matches!(
            ensure_principal_active(principal),
            Err(BearerAuthError::Invalid(ref reason)) if reason.contains(field) && reason.contains("exceeds")
        ));
    }
}

#[test]
fn validated_bearer_fails_closed_on_token_id_and_time_metadata() {
    let principal = AuthenticatedPrincipal::anonymous();
    for token_id in [
        "".to_string(),
        " \t".to_string(),
        "line\nbreak".to_string(),
        "x".repeat(MAX_BEARER_TOKEN_ID_BYTES + 1),
    ] {
        assert!(matches!(
            ValidatedBearer::new(principal.clone(), Some(token_id), None),
            Err(BearerAuthError::Invalid(_))
        ));
    }

    let expires_at = Utc::now() + Duration::minutes(5);
    let mut expiring = principal;
    expiring.expires_at = Some(expires_at);
    let after_expiry = SystemTime::from(expires_at) + StdDuration::from_secs(1);
    assert!(matches!(
        ValidatedBearer::new(expiring, Some("token-1".into()), Some(after_expiry)),
        Err(BearerAuthError::Invalid(ref reason)) if reason.contains("issued-at") && reason.contains("expiry")
    ));
}
