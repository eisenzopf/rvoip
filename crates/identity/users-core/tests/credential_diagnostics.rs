use chrono::Utc;
use users_core::api::{
    ApiKeyResponse, AuthContext, AuthType, ChangePasswordRequest, CreateApiKeyResponse,
    ErrorDetail, ErrorResponse, LoginRequest, LoginResponse, RefreshRequest, UpdateRolesRequest,
    UserResponse,
};
use users_core::api_keys::{ApiKey, CreateApiKeyRequest};
use users_core::jwt::{JwtConfig, RefreshTokenClaims, UserClaims};
use users_core::validation::ValidatedCreateUserRequest;
use users_core::{
    AuthenticationResult, CreateSipDigestCredentialRequest, CreateUserRequest, Error,
    ExternalIdentity, PasskeyCredential, SipDigestAlgorithmFamily, SipDigestCredential,
    TokenIssueContext, TokenPair, UpdateUserRequest, UpsertExternalIdentityRequest,
    UpsertPasskeyCredentialRequest, User, UserFilter, UsersConfig,
};

const CANARY: &str = "users-credential-canary\r\nAuthorization: exposed";

#[test]
fn request_response_and_stored_user_debug_redact_credentials() {
    let login = LoginRequest {
        username: CANARY.into(),
        password: CANARY.into(),
    };
    let response = LoginResponse {
        access_token: CANARY.into(),
        refresh_token: CANARY.into(),
        token_type: "Bearer".into(),
        expires_in: 60,
    };
    let refresh = RefreshRequest {
        refresh_token: CANARY.into(),
    };
    let change = ChangePasswordRequest {
        old_password: CANARY.into(),
        new_password: CANARY.into(),
    };
    let create = CreateUserRequest {
        username: CANARY.into(),
        password: CANARY.into(),
        email: Some(CANARY.into()),
        display_name: Some(CANARY.into()),
        roles: vec![CANARY.into()],
    };
    let now = Utc::now();
    let user = User {
        id: CANARY.into(),
        username: CANARY.into(),
        email: Some(CANARY.into()),
        display_name: Some(CANARY.into()),
        password_hash: CANARY.into(),
        roles: vec![CANARY.into()],
        active: true,
        created_at: now,
        updated_at: now,
        last_login: None,
    };

    for rendered in [
        format!("{login:?}"),
        format!("{response:?}"),
        format!("{refresh:?}"),
        format!("{change:?}"),
        format!("{create:?}"),
        format!("{user:?}"),
    ] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }
    assert_eq!(login.password, CANARY);
    assert_eq!(response.access_token, CANARY);
    assert_eq!(refresh.refresh_token, CANARY);
    assert_eq!(change.new_password, CANARY);
    assert_eq!(create.password, CANARY);
    assert_eq!(user.password_hash, CANARY);
}

#[test]
fn serialized_contracts_keep_exact_values() {
    let login: LoginRequest = serde_json::from_str(&format!(
        "{{\"username\":{},\"password\":{}}}",
        serde_json::to_string(CANARY).unwrap(),
        serde_json::to_string(CANARY).unwrap()
    ))
    .unwrap();
    let response = LoginResponse {
        access_token: CANARY.into(),
        refresh_token: CANARY.into(),
        token_type: "Bearer".into(),
        expires_in: 60,
    };
    let wire = serde_json::to_value(response).unwrap();
    assert_eq!(login.password, CANARY);
    assert_eq!(wire["access_token"], CANARY);
    assert_eq!(wire["refresh_token"], CANARY);
}

#[test]
fn every_users_core_token_password_and_hash_container_is_redacted() {
    let now = Utc::now();
    let user = User {
        id: CANARY.into(),
        username: CANARY.into(),
        email: Some(CANARY.into()),
        display_name: Some(CANARY.into()),
        password_hash: CANARY.into(),
        roles: vec![CANARY.into()],
        active: true,
        created_at: now,
        updated_at: now,
        last_login: None,
    };
    let auth = AuthenticationResult {
        user: user.clone(),
        access_token: CANARY.into(),
        refresh_token: CANARY.into(),
        expires_in: std::time::Duration::from_secs(60),
    };
    let pair = TokenPair {
        access_token: CANARY.into(),
        refresh_token: CANARY.into(),
        expires_in: std::time::Duration::from_secs(60),
    };
    let issue = TokenIssueContext::external_identity(CANARY, CANARY, CANARY);
    let digest = SipDigestCredential {
        id: CANARY.into(),
        user_id: CANARY.into(),
        sip_username: CANARY.into(),
        realm: CANARY.into(),
        algorithm: SipDigestAlgorithmFamily::Sha256,
        ha1: CANARY.into(),
        created_at: now,
        updated_at: now,
    };
    let digest_request = CreateSipDigestCredentialRequest {
        user_id: CANARY.into(),
        sip_username: CANARY.into(),
        realm: CANARY.into(),
        algorithm: SipDigestAlgorithmFamily::Sha256,
        password: CANARY.into(),
    };
    let api_key = ApiKey {
        id: CANARY.into(),
        user_id: CANARY.into(),
        name: CANARY.into(),
        key_hash: CANARY.into(),
        permissions: vec![CANARY.into()],
        active: true,
        expires_at: None,
        last_used: None,
        created_at: now,
    };
    let api_response = CreateApiKeyResponse {
        key: CANARY.into(),
        key_info: ApiKeyResponse {
            id: CANARY.into(),
            name: CANARY.into(),
            permissions: vec![CANARY.into()],
            active: true,
            expires_at: None,
            created_at: now,
            last_used: None,
        },
    };
    let validated = ValidatedCreateUserRequest {
        username: CANARY.into(),
        password: CANARY.into(),
        email: Some(CANARY.into()),
        display_name: Some(CANARY.into()),
        roles: vec![CANARY.into()],
    };
    let claims = UserClaims {
        iss: CANARY.into(),
        sub: CANARY.into(),
        aud: vec![CANARY.into()],
        exp: 2,
        iat: 1,
        jti: CANARY.into(),
        username: CANARY.into(),
        email: Some(CANARY.into()),
        roles: vec![CANARY.into()],
        scope: CANARY.into(),
        tenant_id: Some(CANARY.into()),
    };
    let refresh_claims = RefreshTokenClaims {
        iss: CANARY.into(),
        sub: CANARY.into(),
        jti: CANARY.into(),
        exp: 2,
        iat: 1,
    };
    let jwt = JwtConfig {
        issuer: CANARY.into(),
        audience: vec![CANARY.into()],
        access_ttl_seconds: 60,
        refresh_ttl_seconds: 120,
        algorithm: "HS256".into(),
        tenant_id: None,
        signing_key: Some(CANARY.into()),
    };
    let mut config = UsersConfig::default();
    config.database_url = CANARY.into();
    config.api_bind_address = CANARY.into();
    config.jwt = jwt.clone();

    for rendered in [
        format!("{auth:?}"),
        format!("{pair:?}"),
        format!("{issue:?}"),
        format!("{digest:?}"),
        format!("{digest_request:?}"),
        format!("{api_key:?}"),
        format!("{api_response:?}"),
        format!("{validated:?}"),
        format!("{claims:?}"),
        format!("{refresh_claims:?}"),
        format!("{jwt:?}"),
        format!("{config:?}"),
    ] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }

    assert_eq!(auth.access_token, CANARY);
    assert_eq!(pair.refresh_token, CANARY);
    assert_eq!(digest.ha1, CANARY);
    assert_eq!(digest_request.password, CANARY);
    assert_eq!(api_key.key_hash, CANARY);
    assert_eq!(api_response.key, CANARY);
    assert_eq!(validated.password, CANARY);
    assert_eq!(claims.sub, CANARY);
    assert_eq!(refresh_claims.jti, CANARY);
    assert_eq!(jwt.signing_key.as_deref(), Some(CANARY));
    assert_eq!(config.database_url, CANARY);
}

#[test]
fn authorization_and_identity_diagnostics_never_expose_boundary_values() {
    let now = Utc::now();
    let auth = AuthContext {
        user_id: CANARY.into(),
        username: CANARY.into(),
        roles: vec![CANARY.into()],
        permissions: vec![CANARY.into()],
        auth_type: AuthType::ApiKey,
    };
    let update = UpdateUserRequest {
        email: Some(CANARY.into()),
        display_name: Some(CANARY.into()),
        roles: Some(vec![CANARY.into()]),
        active: Some(true),
    };
    let filter = UserFilter {
        active: Some(true),
        role: Some(CANARY.into()),
        search: Some(CANARY.into()),
        limit: Some(1),
        offset: Some(0),
    };
    let external = ExternalIdentity {
        provider_id: CANARY.into(),
        external_subject: CANARY.into(),
        user_id: CANARY.into(),
        email: Some(CANARY.into()),
        username: Some(CANARY.into()),
        display_name: Some(CANARY.into()),
        groups: vec![CANARY.into()],
        active: true,
        last_seen_at: Some(now),
        created_at: now,
        updated_at: now,
    };
    let external_request = UpsertExternalIdentityRequest {
        provider_id: CANARY.into(),
        external_subject: CANARY.into(),
        user_id: CANARY.into(),
        email: Some(CANARY.into()),
        username: Some(CANARY.into()),
        display_name: Some(CANARY.into()),
        groups: vec![CANARY.into()],
        active: true,
    };
    let passkey = PasskeyCredential {
        credential_id: CANARY.into(),
        user_id: CANARY.into(),
        public_key: CANARY.into(),
        sign_count: 1,
        transports: vec![CANARY.into()],
        backup_eligible: true,
        backup_state: true,
        display_name: Some(CANARY.into()),
        created_at: now,
        last_used_at: Some(now),
    };
    let passkey_request = UpsertPasskeyCredentialRequest {
        credential_id: CANARY.into(),
        user_id: CANARY.into(),
        public_key: CANARY.into(),
        sign_count: 1,
        transports: vec![CANARY.into()],
        backup_eligible: true,
        backup_state: true,
        display_name: Some(CANARY.into()),
    };
    let key_request = CreateApiKeyRequest {
        user_id: CANARY.into(),
        name: CANARY.into(),
        permissions: vec![CANARY.into()],
        expires_at: Some(now),
    };
    let roles = UpdateRolesRequest {
        roles: vec![CANARY.into()],
    };
    let response = UserResponse {
        id: CANARY.into(),
        username: CANARY.into(),
        email: Some(CANARY.into()),
        display_name: Some(CANARY.into()),
        roles: vec![CANARY.into()],
        active: true,
        created_at: now,
        updated_at: now,
        last_login: Some(now),
    };
    let error = ErrorResponse {
        error: ErrorDetail {
            code: CANARY.into(),
            message: CANARY.into(),
            details: Some(serde_json::json!(CANARY)),
        },
    };

    for rendered in [
        format!("{auth:?}"),
        format!("{update:?}"),
        format!("{filter:?}"),
        format!("{external:?}"),
        format!("{external_request:?}"),
        format!("{passkey:?}"),
        format!("{passkey_request:?}"),
        format!("{key_request:?}"),
        format!("{roles:?}"),
        format!("{response:?}"),
        format!("{error:?}"),
    ] {
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }
    assert_eq!(auth.user_id, CANARY);
    assert_eq!(external.external_subject, CANARY);
    assert_eq!(passkey.public_key, CANARY);
    assert_eq!(key_request.user_id, CANARY);
    assert_eq!(error.error.message, CANARY);
}

#[test]
fn security_store_and_exchange_errors_are_class_only() {
    for rendered in [
        format!(
            "{:?}",
            Error::SecurityStoreUnavailable { operation: CANARY }
        ),
        format!(
            "{:?}",
            Error::ApiKeyTokenExchangeDisabled { contract: CANARY }
        ),
    ] {
        assert!(!rendered.contains(CANARY), "boundary leaked: {rendered}");
    }
}
