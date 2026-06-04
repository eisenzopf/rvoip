# rvoip-sip Beta Security Posture

Date: 2026-05-26

This document records the security claims that may be made for the beta line
and the evidence required before release notes are cut. The final clean beta
gate is `crates/sip/rvoip-sip/beta-report/20260526T221457Z`, from git revision
`865430d4` with `git_status: clean`.

## Beta Claims

Developer-facing auth API and crate-boundary guidance is in
`crates/sip/rvoip-sip/docs/AUTHENTICATION.md`.

| Area | Beta status | Evidence | Beta stance |
|------|-------------|----------|-------------|
| SIP digest auth | Supported | `crates/identity/auth-core/src/sip_digest.rs`, `crates/sip/rvoip-sip/tests/register_423_retry.rs`, `crates/sip/rvoip-sip/tests/generated_sip_compliance.rs`, `crates/sip/rvoip-sip/tests/oob_auth_retry.rs`, `crates/sip/rvoip-sip/tests/bye_auth_retry.rs`, `crates/sip/rvoip-sip/tests/info_auth_retry.rs`, `crates/sip/rvoip-sip/tests/refer_auth_retry.rs`, `crates/sip/rvoip-sip/tests/builder_auth_retry_preserves_headers.rs` | Client and server Digest support covers REGISTER, INVITE, selected in-dialog requests, credentialed OOB MESSAGE/OPTIONS/SUBSCRIBE, 401/407, stale nonce recovery, qop `auth`, qop `auth-int` where a request body is available, and MD5/MD5-sess/SHA-256/SHA-256-sess/SHA-512-256/SHA-512-256-sess. Unsupported algorithms fail instead of downgrading. This is not a complete registrar/security product claim. |
| SIP Basic auth | Supported, explicit opt-in | `crates/sip/rvoip-sip/src/auth/mod.rs`, `crates/sip/rvoip-sip/tests/oob_auth_retry.rs` | Legacy compatibility only. Basic is disabled over cleartext SIP unless explicitly allowed; prefer TLS and Digest/Bearer where possible. |
| SIP Bearer auth | Supported | `crates/identity/auth-core/src/bearer.rs`, `crates/identity/auth-core/src/jwt.rs`, `crates/identity/auth-core/src/jwks.rs`, `crates/sip/rvoip-sip/src/auth/mod.rs`, `crates/sip/rvoip-sip/tests/oob_auth_retry.rs` | `rvoip-sip` exposes UAC Bearer challenge response and UAS validation through `auth-core` validators, mapping accepted tokens into `AuthIdentity`. |
| IMS AKA auth | Provider-backed | `crates/sip/rvoip-sip/src/auth/mod.rs`, `crates/sip/rvoip-sip/src/api/respond/challenge.rs` | `rvoip-sip` negotiates AKA as a Digest-family SIP auth scheme through application-provided client/vector providers. It does not claim built-in SIM/USIM infrastructure or carrier IMS certification. |
| SIP TLS client | Supported | `crates/sip/rvoip-sip-transport/tests/tls_handshake_test.rs`, `crates/sip/rvoip-sip/tests/tls_call_integration.rs`, PBX TLS rows in `crates/sip/rvoip-sip/beta-report/20260526T221457Z/pbx/matrix.tsv` | Server validation, custom roots, SNI, failure behavior, and TLS call setup are covered for beta. |
| SIP TLS server | Supported | `crates/sip/rvoip-sip/tests/tls_call_integration.rs`, `crates/sip/rvoip-sip-transport/tests/tls_handshake_test.rs`, PBX TLS rows in the beta report | Cert/key loading and TLS listener behavior are beta-supported where configured. |
| mTLS | Partial | `Config::validate` cert/key pairing checks in `crates/sip/rvoip-sip/src/api/unified.rs`; TLS transport tests cover TLS basics | Do not market broad mTLS interop until external peer-verification matrices are archived. |
| Trace redaction | Supported | `crates/foundation/infra-common/src/events/cross_crate.rs`, `crates/sip/rvoip-sip/tests/trace_redaction.rs` | Default tracing redacts auth/proxy-auth, cookies, token-like headers, identity headers, SDES `a=crypto`, and ICE password lines. Wire bytes are unaffected. |
| SDES-SRTP | Partial | `crates/sip/rvoip-sip/tests/srtp_call_integration.rs`, SRTP negotiation tests in `crates/sip/rvoip-sip/src/adapters/media_adapter.rs`, config validation in `crates/sip/rvoip-sip/tests/config_channel_capacity_integration.rs`, PBX SRTP rows where present | Beta claims are limited to tested SDES suites. DTLS-SRTP is not included. |
| STIR/SHAKEN | Partial | `crates/extensions/rvoip-stir-shaken/tests/sign_verify_round_trip.rs`, `crates/extensions/rvoip-stir-shaken/tests/chain_validation.rs`, `crates/sip/rvoip-sip-dialog/tests/identity_sign_outbound.rs`, `crates/sip/rvoip-sip-dialog/tests/identity_verify_inbound.rs`, byte-preservation tests in `rvoip-sip-transport` | Library support and SIP `Identity` preservation only. No carrier certification claim. |

## Release Security Gates

Run the security gate before the final full beta gate:

```sh
crates/sip/rvoip-sip/scripts/beta_gate.sh --security
```

The gate archives:

- `security/cargo-audit.txt`
- `security/cargo-audit.json`
- `security/fuzz/sip_message.log`
- `security/fuzz/uri.log`
- `security/fuzz/header.log`
- `security/fuzz/sdp.log`

The final release gate includes the same security evidence under the final
clean beta report directory. Any future unaccepted dependency advisory or
parser fuzz crash blocks beta.

Final security evidence:

- Summary: `crates/sip/rvoip-sip/beta-report/20260526T221457Z/summary.md`
- Fuzz smoke: passed for SIP message, URI, header, and SDP parsing with
  archived logs under `security/fuzz/`.
- Dependency audit: passed with no vulnerabilities. Remaining advisory output
  is limited to allowed/documented warnings for `async-std`, `audiopus_sys`,
  `paste`, `rustls-pemfile`, `yaml-rust`, and `lru`.

## Explicit Non-Claims

- DTLS-SRTP is post-beta.
- ICE and TURN are post-beta; STUN remains limited best-effort address discovery.
- Browser/WebRTC security is post-beta.
- ZRTP and MIKEY are not beta claims.
- WSS outbound is not supported for beta.
- SIP Basic authentication is supported only for explicit legacy
  compatibility and should not be recommended for cleartext SIP.
- IMS AKA support is provider-backed. Built-in SIM/USIM infrastructure,
  Milenage certification, and carrier IMS certification are not beta claims.
- STIR/SHAKEN support is library support, not carrier certification.
- `dev-insecure-tls` is only for local tests and examples. It must not appear
  in production recipes.

## Completed Release Checks

| Check | Status |
|-------|--------|
| Dependency advisory audit archived with no unaccepted advisories | Complete in the final Rust 1.88 clean report. |
| Parser fuzz smoke logs archived for SIP message, URI, header, and SDP parsing | Complete in the final Rust 1.88 clean report. |
| Final full beta gate run from clean commit | Complete: `865430d4`, `0` failures, `0` skips. |
| 24-hour soak evidence archived | Waived for beta in `BETA_RELEASE_CHECKLIST.md`; 30-minute soak accepted as the beta bar. |
