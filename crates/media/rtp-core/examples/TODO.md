# RTP-core example status for 0.3.7

This file replaces an older debugging report that described prototype security
paths as complete or production-ready. Those claims are not valid for 0.3.7.

## Available security examples

- Direct SRTP examples require one implemented AES-CM profile and exactly 30
  bytes of provisioned AES-128 key/salt material.
- SDES examples may generate and process SDP crypto attributes using the
  implemented AES-CM suites. Lifetime, MKI, unencrypted, unauthenticated, KDR,
  and other unsupported session parameters are rejected.
- SRTP receive examples surface authenticated media through the transport event
  path. Direct receive on `SecurityRtpTransport` is unavailable in secure mode
  to avoid racing the authenticated interceptor.
- SRTCP is unavailable. Secure transports reject plaintext RTCP.

## Retained but unavailable protocols

DTLS-SRTP, MIKEY (including PSK and PKE), ZRTP, AES-GCM SRTP, placeholder key
derivation, and automatic key rotation are retained in public configuration
types for compatibility. Their checked constructors, validation, negotiation,
and examples return typed `UnsupportedFeature` errors before state or key
mutation. They must not be advertised or used as fallback methods.

`dtls_test.rs` and `direct_dtls_media_streaming.rs` are availability checks,
not successful-handshake demonstrations. Certificate generation remains a
standalone utility and does not make DTLS-SRTP available.

## Qualification rule

An example that describes a security property must either exercise real crypto
on the wire or assert the typed unavailable result. Simulated handshakes, fake
keys, plaintext fallback, and historical output logs are not release evidence.
