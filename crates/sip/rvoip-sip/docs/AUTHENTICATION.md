# rvoip-sip Authentication Guide

This document mirrors the `cargo doc` guidance for SIP authentication in
`rvoip-sip`. Use it when deciding which auth scheme to configure, which side
owns the work, and which crate provides the underlying crypto or token
validation.

## Protocol Scope

SIP access authentication is negotiated in SIP headers:

| Challenge | Retry header | Side challenged |
|-----------|--------------|-----------------|
| `401 WWW-Authenticate` | `Authorization` | Origin server / UAS |
| `407 Proxy-Authenticate` | `Proxy-Authorization` | Proxy |

SDP is not an authentication negotiation channel. SDP or other SIP bodies only
matter to Digest when `qop=auth-int` hashes the message body into the response.

## API Map

| Need | API | Side |
|------|-----|------|
| PBX-style Digest account | `SipAccount`, `EndpointBuilder::sip_account`, `StreamPeer::register_account`, `CallbackPeer::register_account` | UAC |
| Default outbound auth | `Config::auth`, `EndpointBuilder::auth`, `StreamPeerBuilder::with_auth`, `CallbackPeerBuilder::with_auth` | UAC |
| Per-request outbound auth | `.with_auth(...)`, `.with_bearer_token(...)`, `.with_basic_credentials(...)`, `.with_credentials(...)` | UAC |
| Multi-scheme negotiation | `SipClientAuth::any(...)` | UAC |
| Inbound validation | `SipAuthService`, `IncomingCall::authenticate_with`, `IncomingRequest::authenticate_with`, `IncomingRegister::authenticate_with` | UAS |
| Digest-only compatibility | `SipDigestAuthService` | UAS |
| Redacted audit events | `SipAuthService::with_audit_sink(...)` | UAS |
| Rate limit / lockout | `SipAuthService::with_rate_limiter(...)` | UAS |
| Shared Digest replay state | `SipAuthService::with_digest_replay_store(...)`, `SipDigestAuthService::authenticate_authorization_with_replay_store(...)` | UAS |
| Digest algorithms and Bearer validators | `rvoip-auth-core` | Shared primitives |

`with_credentials(...)` remains Digest username/password shorthand. Use
`with_auth(SipClientAuth::...)` for Bearer, Basic, AKA, or multi-challenge
selection.

## Scheme Support

| Scheme | UAC support | UAS support | Algorithms / providers | Default posture |
|--------|-------------|-------------|-------------------------|-----------------|
| Digest | `SipAccount`, `Credentials`, `SipClientAuth::digest` | `SipAuthService::digest`, `SipDigestAuthService` | MD5, MD5-sess, SHA-256, SHA-256-sess, SHA-512-256, SHA-512-256-sess; `qop=auth`; `qop=auth-int` where the body is available | Enabled when configured |
| Bearer | `SipClientAuth::bearer_token` | `SipAuthService::with_bearer_validator` | Validator-dependent JWT/JWKS/OAuth2 introspection/AAuth/opaque tokens through `rvoip-auth-core` | Enabled when configured |
| Basic | `SipClientAuth::basic` | `SipAuthService::with_basic_realm`, `with_basic_user` | None | Cleartext disabled unless explicitly allowed |
| AKA | `SipClientAuth::aka` | `SipAuthService::with_aka_provider` | Provider-backed `AKAv1-MD5` / `AKAv2-MD5` | Disabled unless providers are supplied |

`SipClientAuth::any(...)` chooses the strongest configured compatible challenge
in this order: AKA, Bearer, Digest, Basic. Basic still obeys its TLS/cleartext
policy after selection.

## UAC Behavior

UAC configuration can be attached at several levels:

- `SipAccount` configures PBX Digest registration and default challenged
  outbound requests from one account shape.
- `Config::auth` configures default full auth for the coordinator.
- `EndpointBuilder::auth`, `StreamPeerBuilder::with_auth`, and
  `CallbackPeerBuilder::with_auth` configure peer defaults.
- Per-request `.with_auth(...)` overrides defaults for an individual request.

Retries use `Authorization` for `401` and `Proxy-Authorization` for `407`.
REGISTER, INVITE, in-dialog requests, and out-of-dialog MESSAGE, OPTIONS, and
SUBSCRIBE all use real challenge-response headers; builders must not emit
placeholder auth.

## UAS Behavior

`SipAuthService` is the general UAS facade. Enable one or more schemes, then
authenticate inbound requests:

```rust
# use rvoip_sip::{SipAuthDecision, SipAuthService};
# async fn example(incoming: rvoip_sip::IncomingRequest) -> rvoip_sip::Result<()> {
let mut auth = SipAuthService::digest("pbx.example.com")
    .with_bearer_scope("calls:write")
    .with_basic_realm("legacy");
auth.add_digest_user("1001", "secret");
auth.add_basic_user("legacy", "secret");

match incoming.authenticate_with(&auth).await? {
    SipAuthDecision::Authorized(identity) => {
        let _source = identity.source;
    }
    SipAuthDecision::Rejected { challenges } => {
        let _headers_to_send = challenges;
    }
}
# Ok(())
# }
```

`SipDigestAuthService` remains available when a server only needs Digest. Use
`SipAuthService` for Bearer, Basic, AKA, multiple challenge headers, and unified
identity results.

### Enterprise UAS Hooks

`SipAuthService` can be configured with enterprise providers from
`rvoip-auth-core`:

- `with_audit_sink(...)` records redacted success and failure events. Audit
  events include scheme, outcome, peer/context metadata, method, and origin vs
  proxy source. They do not include passwords, HA1 values, bearer tokens, API
  keys, full JWTs, or raw Authorization headers.
- `with_audit_failure_policy(...)` defaults to fail-open. Use
  `AuditFailurePolicy::FailClosed` when audit durability is a hard security
  control.
- `with_rate_limiter(...)` checks policy before credential validation and
  records the outcome afterward. Rate-limiter provider failures fail closed.
- `with_digest_replay_store(...)` uses shared nonce and nonce-count storage on
  the async auth path. Use `authenticate_authorization_with_context(...)` or
  inbound `authenticate_with(...)` helpers so newly issued nonces are recorded
  in shared storage.

The sync `challenges(...)` helper is for local/simple challenge generation. In
clustered deployments with external `DigestReplayStore`, use
`challenges_async(...)` or the async authentication helpers. The Digest-only
compatibility service also exposes
`challenge_with_replay_store(...)` and
`authenticate_authorization_with_replay_store(...)` for deployments that need
shared replay state without the full multi-scheme facade.

## Crate Boundaries

`rvoip-sip` owns SIP protocol orchestration: parsing SIP authentication
headers, selecting `Authorization` versus `Proxy-Authorization`, retrying
challenged requests, generating challenge headers for inbound responses, and
mapping successful UAS validation into `AuthIdentity`.

`rvoip-auth-core` owns reusable primitives: Digest challenge/response
calculation, Digest validation, Bearer validation traits, JWT/JWKS validators,
AAuth validators, DPoP, and HTTP-signature style validators. It does not own
SIP transactions or retry state.

Application code owns secrets and infrastructure:

- passwords and PBX account data,
- Bearer token issuance and validator configuration,
- Basic cleartext policy decisions,
- AKA client response providers and UAS vector providers.

## Security Notes

For enterprise review material, see:

- `AUTH_THREAT_MODEL.md`
- `AUTH_SECURITY_ARCHITECTURE.md`
- `AUTH_KEY_MANAGEMENT.md`
- `AUTH_COMPLIANCE_MAPPING.md`

Digest is the common PBX baseline and should be preferred over Basic for
password-based SIP access authentication.

MD5 and MD5-sess remain supported for PBX compatibility. For first-party or
provider-backed services, prefer SHA-256, SHA-256-sess, SHA-512-256, or
SHA-512-256-sess when the peer supports them.

Digest HA1 values are password-equivalent for their username, realm, and
algorithm family. Store them as secrets. Do not use users-core Argon2 login
password hashes as SIP Digest material; use dedicated SIP Digest credentials.

Bearer can be stronger than Digest when tokens are short-lived and validation is
backed by trusted JWT/JWKS/AAuth infrastructure.

HS256 is a shared-secret validation mode. Public JWKS validation requires
asymmetric signing such as RS256. Generated users-core HS256 secrets are
development defaults and should be replaced with stable configured signing keys
in production.

Basic exists for legacy compatibility. Use it over TLS. Cleartext Basic
requires an explicit opt-in on both UAC and UAS paths.

Single-process nonce and nonce-count replay tracking protects a standalone UAS.
Clustered deployments need shared replay state through `DigestReplayStore` on
the async auth path. JWT deployments that require immediate access-token
revocation need short TTLs plus `TokenRevocationChecker`, introspection, or a
revocation cache in addition to signature validation. users-core provides a
reference access-token JTI revocation store for its SQLite service.

AKA is provider-backed. The API supports negotiation and provider integration,
but `rvoip-sip` does not claim built-in SIM/USIM infrastructure, Milenage
certification, or carrier IMS certification.
