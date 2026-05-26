//! # Auth-Core - Authentication and Authorization for RVoIP
//!
//! This crate provides OAuth2 and token-based authentication services
//! for the RVoIP ecosystem, supporting multiple authentication flows
//! and token validation strategies.
//!
//! Also includes SIP Digest authentication support per RFC 2617 and RFC 3261.

pub mod aauth;
pub mod bearer;
pub mod dpop;
pub mod error;
pub mod jwks;
pub mod jwt;
pub mod sig9421;
pub mod sip_digest;
pub mod types;

pub use aauth::{AAuthValidator, ActorClaims, ActorTokenValidator};
pub use bearer::{bearer_stub, BearerAuthError, BearerValidator};
pub use sig9421::{
    EnvelopeSignature, KeyResolver, Sig9421Error, Sig9421Verifier, StaticKeyResolver,
    DEFAULT_REPLAY_CACHE_CAPACITY, DEFAULT_SIG_REPLAY_TTL,
};
pub use dpop::{
    jwk_thumbprint, DpopError, DpopProof, DpopValidator, ValidatedDpop, DEFAULT_IAT_LEEWAY,
    DEFAULT_JTI_CACHE_CAPACITY,
};
pub use jwks::{JwksJwtValidator, DEFAULT_JWKS_CACHE_TTL};
pub use jwt::JwtValidator;
pub use error::{AuthError, Result};
pub use sip_digest::{
    DigestAlgorithm, DigestAuthenticator, DigestChallenge, DigestClient, DigestComputed,
    DigestResponse,
};
