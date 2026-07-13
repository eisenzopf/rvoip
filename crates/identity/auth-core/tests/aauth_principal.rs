use std::sync::Arc;

use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use rvoip_auth_core::{
    AAuthValidator, ActorTokenValidator, AuthenticationMethod, BearerValidator, JwksJwtValidator,
    JwtValidator,
};
use rvoip_core_traits::identity::IdentityAssurance;
use serde::Serialize;

const SUBJECT_SECRET: &[u8] = b"aauth-subject-test-secret";
const ACTOR_SECRET: &[u8] = b"aauth-actor-test-secret";

#[derive(Serialize)]
struct Claims<'a> {
    sub: &'a str,
    exp: i64,
    iss: &'a str,
    tenant_id: &'a str,
    scope: &'a str,
}

fn mint(secret: &[u8], claims: &Claims<'_>) -> String {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret),
    )
    .expect("mint AAuth test token")
}

#[test]
fn first_party_jwt_validators_are_actor_validators() {
    fn assert_actor_validator<T: ActorTokenValidator>() {}
    assert_actor_validator::<JwtValidator>();
    assert_actor_validator::<JwksJwtValidator>();
}

#[tokio::test]
async fn combined_principal_uses_subject_ownership_and_earliest_expiry() {
    let now = Utc::now();
    let subject_expiry = (now + chrono::Duration::minutes(10)).timestamp();
    let actor_expiry = (now + chrono::Duration::minutes(2)).timestamp();
    let subject_token = mint(
        SUBJECT_SECRET,
        &Claims {
            sub: "user:alice",
            exp: subject_expiry,
            iss: "https://shared-issuer.example",
            tenant_id: "tenant-shared",
            scope: "calls:read calls:write",
        },
    );
    let actor_token = mint(
        ACTOR_SECRET,
        &Claims {
            sub: "agent:assistant-7",
            exp: actor_expiry,
            iss: "https://shared-issuer.example",
            tenant_id: "tenant-shared",
            scope: "aauth:act:user:alice calls:write calls:transfer",
        },
    );

    let subject: Arc<dyn BearerValidator> =
        Arc::new(JwtValidator::from_hmac_secret(SUBJECT_SECRET));
    let actor: Arc<dyn ActorTokenValidator> =
        Arc::new(JwtValidator::from_hmac_secret(ACTOR_SECRET));
    let validator = AAuthValidator::new(subject, actor);

    let principal = validator
        .validate_principal(&subject_token, &actor_token)
        .await
        .expect("valid AAuth pair");

    assert_eq!(principal.subject, "user:alice");
    assert_eq!(
        principal.issuer.as_deref(),
        Some("https://shared-issuer.example")
    );
    assert_eq!(principal.tenant.as_deref(), Some("tenant-shared"));
    assert_eq!(principal.method, AuthenticationMethod::AAuth);
    assert_eq!(
        principal.expires_at.expect("combined expiry").timestamp(),
        actor_expiry,
        "combined credential must expire at the earlier component expiry"
    );
    assert_eq!(
        principal.scopes,
        vec!["calls:read", "calls:write", "calls:transfer"]
    );
    match &principal.assurance {
        IdentityAssurance::UserAuthorized {
            user_id,
            identity,
            scopes,
        } => {
            assert_eq!(user_id.as_str(), "user:alice");
            assert_eq!(identity.as_str(), "agent:assistant-7");
            assert_eq!(scopes, &principal.scopes);
        }
        other => panic!("expected UserAuthorized assurance, got {other:?}"),
    }

    let diagnostic = format!("{principal:?}");
    assert!(!diagnostic.contains(&subject_token));
    assert!(!diagnostic.contains(&actor_token));
}
