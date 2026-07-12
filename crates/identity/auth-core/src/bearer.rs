//! Bearer-token validation surface.
//!
//! The [`BearerValidator`] trait is implemented by JWT, JWKS/OIDC, AAuth, and
//! test/demo validators. [`bearer_stub`] remains available for local tests, but
//! production deployments should use a real validator such as
//! [`crate::JwtValidator`] or [`crate::JwksJwtValidator`].
//!
//! Named `BearerAuthError` (not `AuthError`) to avoid colliding with
//! the crate's existing SIP Digest [`crate::AuthError`] type.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rvoip_core_traits::identity::{IdentityAssurance, Jwk};

// Compatibility re-export: the canonical type now lives in the dependency-
// cycle-free core trait surface, but existing `auth_core::bearer::*` imports
// continue to work.
pub use rvoip_core_traits::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError, PrincipalOwnershipKey,
};

/// Maximum accepted UTF-8 byte length for a bearer credential identifier.
pub const MAX_BEARER_TOKEN_ID_BYTES: usize = 512;
/// Maximum accepted UTF-8 byte length for a bearer principal subject.
pub const MAX_BEARER_SUBJECT_BYTES: usize = 1_024;
/// Maximum accepted UTF-8 byte length for a bearer principal issuer.
pub const MAX_BEARER_ISSUER_BYTES: usize = 2_048;
/// Maximum accepted UTF-8 byte length for a bearer principal tenant identifier.
pub const MAX_BEARER_TENANT_BYTES: usize = 512;

/// A validated bearer principal plus credential lifecycle metadata.
///
/// `token_id` is an issuer-provided identifier such as JWT `jti` when
/// available. An opaque-token validator may use a one-way credential
/// fingerprint when its provider omits an identifier. Token identifiers and
/// fingerprints are sensitive correlation material and must never be logged;
/// this type's [`fmt::Debug`] implementation therefore always redacts them.
/// `issued_at` uses [`SystemTime`] to match [`crate::TokenRevocationContext`].
#[derive(Clone)]
pub struct ValidatedBearer {
    /// Complete authorization principal established by the credential.
    pub principal: AuthenticatedPrincipal,
    /// Stable credential identifier, when the validator can establish one.
    pub token_id: Option<String>,
    /// Credential issue time, when supplied and validated by the issuer.
    pub issued_at: Option<SystemTime>,
}

impl ValidatedBearer {
    /// Construct and validate a credential result.
    ///
    /// This enforces the same active-principal boundary as
    /// [`BearerValidator::validate_principal`], validates token identifier
    /// bounds, and rejects an issue time later than the credential expiry.
    pub fn new(
        principal: AuthenticatedPrincipal,
        token_id: Option<String>,
        issued_at: Option<SystemTime>,
    ) -> Result<Self, BearerAuthError> {
        let principal = ensure_principal_active(principal)?;
        let token_id = validate_optional_token_id(token_id)?;
        if let (Some(issued_at), Some(expires_at)) = (issued_at, principal.expires_at) {
            if issued_at > SystemTime::from(expires_at) {
                return Err(BearerAuthError::Invalid(
                    "bearer credential issued-at time is later than expiry".into(),
                ));
            }
        }
        Ok(Self {
            principal,
            token_id,
            issued_at,
        })
    }
}

impl fmt::Debug for ValidatedBearer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedBearer")
            .field("principal", &self.principal)
            .field("token_id", &self.token_id.as_ref().map(|_| "<redacted>"))
            .field("issued_at", &self.issued_at)
            .finish()
    }
}

/// Reject a validator result whose credential has already expired.
///
/// JWT libraries commonly allow clock-skew leeway while decoding.  The
/// principal boundary is deliberately stricter: once the advertised expiry
/// has passed, protocol adapters must not retain or authorize the principal.
pub fn ensure_principal_active(
    principal: AuthenticatedPrincipal,
) -> Result<AuthenticatedPrincipal, BearerAuthError> {
    validate_required_identifier("subject", &principal.subject, MAX_BEARER_SUBJECT_BYTES)?;
    validate_optional_identifier(
        "issuer",
        principal.issuer.as_deref(),
        MAX_BEARER_ISSUER_BYTES,
    )?;
    validate_optional_identifier(
        "tenant",
        principal.tenant.as_deref(),
        MAX_BEARER_TENANT_BYTES,
    )?;
    if principal.is_expired() {
        Err(BearerAuthError::Invalid(
            "authenticated principal is expired".into(),
        ))
    } else {
        Ok(principal)
    }
}

fn validate_required_identifier(
    name: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), BearerAuthError> {
    if value.trim().is_empty() {
        return Err(BearerAuthError::Invalid(format!(
            "authenticated principal {name} is empty"
        )));
    }
    if value.len() > max_bytes {
        return Err(BearerAuthError::Invalid(format!(
            "authenticated principal {name} exceeds {max_bytes} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(BearerAuthError::Invalid(format!(
            "authenticated principal {name} contains control characters"
        )));
    }
    Ok(())
}

fn validate_optional_identifier(
    name: &str,
    value: Option<&str>,
    max_bytes: usize,
) -> Result<(), BearerAuthError> {
    if let Some(value) = value {
        validate_required_identifier(name, value, max_bytes)?;
    }
    Ok(())
}

pub(crate) fn validate_optional_token_id(
    token_id: Option<String>,
) -> Result<Option<String>, BearerAuthError> {
    let Some(token_id) = token_id else {
        return Ok(None);
    };
    if token_id.trim().is_empty() {
        return Err(BearerAuthError::Invalid(
            "bearer credential token id is empty".into(),
        ));
    }
    if token_id.len() > MAX_BEARER_TOKEN_ID_BYTES {
        return Err(BearerAuthError::Invalid(format!(
            "bearer credential token id exceeds {MAX_BEARER_TOKEN_ID_BYTES} bytes"
        )));
    }
    if token_id.chars().any(char::is_control) {
        return Err(BearerAuthError::Invalid(
            "bearer credential token id contains control characters".into(),
        ));
    }
    Ok(Some(token_id))
}

pub(crate) fn unix_time_from_seconds(
    seconds: u64,
    field: &str,
) -> Result<SystemTime, BearerAuthError> {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| {
            BearerAuthError::Invalid(format!(
                "bearer credential {field} is outside the supported range"
            ))
        })
}

/// Validates a bearer token and produces the resulting [`IdentityAssurance`]
/// for the authenticated peer.
#[async_trait]
pub trait BearerValidator: Send + Sync {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError>;

    /// Validate a token while retaining the authorization attributes needed
    /// by protocol adapters. Existing third-party validators remain source
    /// compatible through this default mapping.
    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        let assurance = self.validate(token).await?;
        ensure_principal_active(AuthenticatedPrincipal::from_assurance(assurance))
    }

    /// Validate a token while retaining credential lifecycle metadata.
    ///
    /// Existing validators remain source compatible: the default delegates to
    /// [`Self::validate_principal`] and leaves metadata absent. Validators that
    /// can authenticate an issuer-provided token ID or issue time should
    /// override this method.
    async fn validate_credential(&self, token: &str) -> Result<ValidatedBearer, BearerAuthError> {
        ValidatedBearer::new(self.validate_principal(token).await?, None, None)
    }
}

/// Build the v0 stub validator.
///
/// Behavior: any non-empty token is accepted; the returned
/// `IdentityAssurance::Pseudonymous { ephemeral_key }` carries a freshly
/// generated throwaway JWK (the rvoip-core `Jwk` type is intentionally
/// opaque in v0). Empty tokens are rejected with [`BearerAuthError::Empty`].
pub fn bearer_stub() -> Arc<dyn BearerValidator> {
    Arc::new(StubBearerValidator)
}

struct StubBearerValidator;

#[async_trait]
impl BearerValidator for StubBearerValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        let ephemeral_key = Jwk(serde_json::json!({
            "kty": "stub",
            "kid": uuid::Uuid::new_v4().simple().to_string(),
        }));
        Ok(IdentityAssurance::Pseudonymous { ephemeral_key })
    }

    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        let assurance = self.validate(token).await?;
        let mut principal = AuthenticatedPrincipal::from_assurance_with_method(
            assurance,
            AuthenticationMethod::Bearer,
        );
        // The stub is explicitly a local-development validator. Giving it a
        // wildcard keeps secure protocol defaults usable in examples without
        // weakening any real JWT/JWKS/introspection validator.
        principal.scopes = vec!["*".into()];
        // Atomic inbound routing is tenant-bound even in local development.
        // A stable, explicit development tenant keeps the stub usable without
        // teaching protocol adapters to special-case tenantless principals.
        principal.tenant = Some("development".into());
        ensure_principal_active(principal)
    }
}
