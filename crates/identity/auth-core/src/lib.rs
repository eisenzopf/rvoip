//! # Auth-Core - Authentication and Authorization Primitives for RVoIP
//!
//! `rvoip-auth-core` provides cryptographic and token-validation primitives
//! used across the RVoIP workspace. It does not own SIP dialog state, SIP
//! request retry orchestration, or `WWW-Authenticate` / `Authorization` header
//! routing. The SIP application layer in `rvoip-sip` owns those protocol
//! workflows and calls into this crate for the underlying algorithms.
//!
//! ## SIP Digest Primitives
//!
//! [`DigestAuthenticator`] generates Digest challenges and validates Digest
//! responses. [`DigestClient`] computes UAC responses for challenges. They are
//! used by `rvoip-sip` for SIP Digest over `401 WWW-Authenticate` /
//! `Authorization` and `407 Proxy-Authenticate` / `Proxy-Authorization`.
//!
//! Supported Digest algorithms are MD5, MD5-sess, SHA-256, SHA-256-sess,
//! SHA-512-256, and SHA-512-256-sess. An omitted Digest `algorithm` defaults
//! to MD5 for legacy SIP/PBX compatibility; unknown algorithms fail instead of
//! silently downgrading.
//!
//! ## Bearer and Token Validators
//!
//! [`BearerValidator`] is the common async validation trait for Bearer tokens.
//! [`JwtValidator`], [`JwksJwtValidator`], [`OAuth2IntrospectionValidator`],
//! and [`AAuthValidator`] are concrete validator families that can be plugged
//! into `rvoip-sip`'s `SipAuthService` for UAS-side SIP Bearer validation.
//! JWT and JWKS validators can optionally call [`TokenRevocationChecker`] with
//! token `jti` context after signature, issuer, audience, and expiry checks.
//!
//! ## Provider Contracts
//!
//! Protocol crates and applications use provider traits to integrate with
//! users-core, Keycloak/OIDC, LDAP/AD, Redis, IMS infrastructure, or custom
//! services without coupling protocol code to a database. These contracts
//! include [`PasswordVerifier`], [`DigestSecretProvider`], [`ApiKeyVerifier`],
//! [`TokenRevocationChecker`], [`DigestReplayStore`], [`AuthAuditSink`], and
//! [`AuthRateLimiter`].
//!
//! ## Other Auth Building Blocks
//!
//! The crate also includes DPoP validation ([`DpopValidator`]) and HTTP
//! Message Signatures style envelope verification ([`Sig9421Verifier`]).
//! These are standalone primitives; protocol-specific negotiation and retry
//! behavior belongs in the crate that owns that protocol surface.

pub mod aauth;
pub mod bearer;
pub mod dpop;
pub mod error;
pub mod introspection;
pub mod jwks;
pub mod jwt;
pub mod providers;
pub mod sig9421;
pub mod sip_digest;
pub mod types;

pub use aauth::{AAuthValidator, ActorClaims, ActorTokenValidator};
pub use bearer::{bearer_stub, BearerAuthError, BearerValidator};
pub use dpop::{
    jwk_thumbprint, DpopError, DpopProof, DpopValidator, ValidatedDpop, DEFAULT_IAT_LEEWAY,
    DEFAULT_JTI_CACHE_CAPACITY,
};
pub use error::{AuthError, Result};
pub use introspection::OAuth2IntrospectionValidator;
pub use jwks::{JwksJwtValidator, DEFAULT_JWKS_CACHE_TTL};
pub use jwt::JwtValidator;
pub use providers::{
    ApiKeyVerifier, AuthAuditEvent, AuthAuditOutcome, AuthAuditScheme, AuthAuditSink,
    AuthFailureReason, AuthRateLimitKey, AuthRateLimitKind, AuthRateLimitVerdict, AuthRateLimiter,
    CredentialAuthError, DigestNonceStatus, DigestReplayStore, DigestSecret, DigestSecretProvider,
    PasswordVerifier, TokenRevocationChecker, TokenRevocationContext, TokenRevocationStatus,
};
pub use sig9421::{
    EnvelopeSignature, KeyResolver, Sig9421Error, Sig9421Verifier, StaticKeyResolver,
    DEFAULT_REPLAY_CACHE_CAPACITY, DEFAULT_SIG_REPLAY_TTL,
};
pub use sip_digest::{
    DigestAlgorithm, DigestAuthenticator, DigestChallenge, DigestChallengeDetails, DigestClient,
    DigestComputed, DigestResponse,
};
