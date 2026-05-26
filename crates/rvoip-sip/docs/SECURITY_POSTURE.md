# rvoip-sip Beta Security Posture

Date: 2026-05-26

This document records the security claims that may be made for the beta line
and the evidence required before release notes are cut. The latest full
reference gate is `crates/rvoip-sip/beta-report/20260526T032035Z`, but that
run was from dirty git revision `d6e8beaa`; final release evidence must come
from a clean commit.

## Beta Claims

| Area | Beta status | Evidence | Beta stance |
|------|-------------|----------|-------------|
| SIP digest auth | Partial | `crates/auth-core/src/sip_digest.rs`, `crates/rvoip-sip/tests/register_423_retry.rs`, `crates/rvoip-sip/tests/invite_auth_tests.rs`, `crates/rvoip-sip/tests/bye_auth_retry.rs`, `crates/rvoip-sip/tests/info_auth_retry.rs`, `crates/rvoip-sip/tests/refer_auth_retry.rs`, `crates/rvoip-sip/tests/builder_auth_retry_preserves_headers.rs` | Client retry and challenge handling are covered for beta paths. This is not a complete registrar/security product claim. |
| SIP TLS client | Supported | `crates/rvoip-sip-transport/tests/tls_handshake_test.rs`, `crates/rvoip-sip/tests/tls_call_integration.rs`, PBX TLS rows in `crates/rvoip-sip/beta-report/20260526T032035Z/pbx/matrix.tsv` | Server validation, custom roots, SNI, failure behavior, and TLS call setup are covered for beta. |
| SIP TLS server | Supported | `crates/rvoip-sip/tests/tls_call_integration.rs`, `crates/rvoip-sip-transport/tests/tls_handshake_test.rs`, PBX TLS rows in the beta report | Cert/key loading and TLS listener behavior are beta-supported where configured. |
| mTLS | Partial | `Config::validate` cert/key pairing checks in `crates/rvoip-sip/src/api/unified.rs`; TLS transport tests cover TLS basics | Do not market broad mTLS interop until external peer-verification matrices are archived. |
| Trace redaction | Supported | `crates/infra-common/src/events/cross_crate.rs`, `crates/rvoip-sip/tests/trace_redaction.rs` | Default tracing redacts auth/proxy-auth, cookies, token-like headers, identity headers, SDES `a=crypto`, and ICE password lines. Wire bytes are unaffected. |
| SDES-SRTP | Partial | `crates/rvoip-sip/tests/srtp_call_integration.rs`, SRTP negotiation tests in `crates/rvoip-sip/src/adapters/media_adapter.rs`, config validation in `crates/rvoip-sip/tests/config_channel_capacity_integration.rs`, PBX SRTP rows where present | Beta claims are limited to tested SDES suites. DTLS-SRTP is not included. |
| STIR/SHAKEN | Partial | `crates/rvoip-stir-shaken/tests/sign_verify_round_trip.rs`, `crates/rvoip-stir-shaken/tests/chain_validation.rs`, `crates/rvoip-sip-dialog/tests/identity_sign_outbound.rs`, `crates/rvoip-sip-dialog/tests/identity_verify_inbound.rs`, byte-preservation tests in `rvoip-sip-transport` | Library support and SIP `Identity` preservation only. No carrier certification claim. |

## Release Security Gates

Run the security gate before the final full beta gate:

```sh
crates/rvoip-sip/scripts/beta_gate.sh --security
```

The gate archives:

- `security/cargo-audit.txt`
- `security/cargo-audit.json`
- `security/fuzz/sip_message.log`
- `security/fuzz/uri.log`
- `security/fuzz/header.log`
- `security/fuzz/sdp.log`

The final release gate must include the same security evidence under the final
clean beta report directory. Any unaccepted dependency advisory or parser fuzz
crash blocks beta.

Current short security run:

- Summary: `target/beta-gate/20260526T070702Z/summary.md`
- Fuzz smoke: passed for SIP message, URI, header, and SDP parsing with
  `BETA_FUZZ_SMOKE_RUNS=1`.
- Dependency audit: failed. Several advisories were remediated by dependency
  updates, but the release remains blocked by 7 unresolved vulnerabilities
  across Hickory DNS,
  rustls-webpki/rustls 0.21, ring 0.16 through DTLS/rcgen, `rsa`, and `time`
  advisories. Hickory 0.26 and the fixed `time` release require Rust 1.88, so
  final remediation needs an MSRV decision or documented accepted exclusions.

## Explicit Non-Claims

- DTLS-SRTP is post-beta.
- ICE and TURN are post-beta; STUN remains limited best-effort address discovery.
- Browser/WebRTC security is post-beta.
- ZRTP and MIKEY are not beta claims.
- WSS outbound is not supported for beta.
- STIR/SHAKEN support is library support, not carrier certification.
- `dev-insecure-tls` is only for local tests and examples. It must not appear
  in production recipes.

## Remaining Release Checks

| Check | Status |
|-------|--------|
| Dependency advisory audit archived with no unaccepted advisories | Blocking: current short run fails on unresolved dependency advisories. |
| Parser fuzz smoke logs archived for SIP message, URI, header, and SDP parsing | Short smoke passed; final release-length smoke pending in clean report. |
| Final full beta gate run from clean commit | Pending. |
| 24-hour soak evidence archived | Pending. |
