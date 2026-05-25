# rvoip-sip Beta Security Posture

Date: 2026-05-25

This document defines what beta may claim about security and what remains
post-beta.

## Beta Claims

| Area | Beta status | Required evidence |
|------|-------------|-------------------|
| SIP digest auth | Partial | Success, failure, stale nonce, qop, proxy auth, and replay-oriented negative tests. |
| SIP TLS client | Supported | Server validation, custom roots, SNI, and failure tests. |
| SIP TLS server | Supported | Cert/key loading and listener tests. |
| mTLS | Partial | Client cert/key loading and peer verification tests. |
| Trace redaction | Supported | Authorization, Proxy-Authorization, cookies, tokens, SDP secrets, and identity headers. |
| SDES-SRTP | Partial | Suite matrix and PBX interop. |
| STIR/SHAKEN | Partial | Test vectors and error cases. |

## Explicit Non-Claims for Beta

- DTLS-SRTP is post-beta.
- ICE and TURN are post-beta.
- Browser/WebRTC security is post-beta.
- ZRTP and MIKEY are not beta claims unless separately audited.
- STIR/SHAKEN support is library support, not carrier certification.
- `dev-insecure-tls` is for local development only and must not appear in beta
  production recipes.

## Required Release Checks

- Run dependency audit before release.
- Run parser fuzz smoke tests before release.
- Verify no beta docs recommend insecure TLS outside test setup.
- Verify all security-sensitive logs are redacted when tracing is enabled.
- Verify unsupported security modes return clear errors or are absent from
  public beta docs.

Local beta gate coverage starts with:

```sh
crates/rvoip-sip/scripts/beta_gate.sh --local
```

Add `cargo audit`, long-running fuzz jobs, and external TLS/mTLS negative
interop evidence to the release artifact bundle before cutting beta notes.
