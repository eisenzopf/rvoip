# rvoip-sip Crypto Capability Boundaries

This file defines the public, end-to-end crypto claim for `rvoip-sip 0.3.8`.
A lower crate containing a type, constant, parser, or partial state machine does
not by itself make that mechanism available through SIP offer/answer.

## Media security

| Mechanism | Public `rvoip-sip` status | Boundary |
| --- | --- | --- |
| SDES `AES_CM_128_HMAC_SHA1_80` | Supported | Offer/answer, directional SRTP/SRTCP contexts, replay/rollover handling, and external libSRTP vectors are tested. |
| SDES `AES_CM_128_HMAC_SHA1_32` | Supported | Same end-to-end path as the `_80` profile. |
| SDES `AES_256_CM_HMAC_SHA1_80` | Supported | Exact 32-byte key plus 14-byte salt; canonical and compatible unpadded-answer coverage. |
| SDES `AES_256_CM_HMAC_SHA1_32` | Supported | Same end-to-end path as the AES-256 `_80` profile. |
| AEAD AES-128-GCM / AES-256-GCM | Unsupported | Profile identities remain for compatibility, but construction, configuration, advertisement, conversion, and negotiation fail closed. |
| DTLS-SRTP | Unsupported through SIP | Lower-layer profile identifiers remain, but DTLS construction returns a typed unsupported error and `rvoip-sip` has no fingerprint/setup negotiation path. |
| MIKEY | Unsupported through SIP | Incomplete lower-layer code is not advertised, negotiated, or claimed as an end-to-end capability. |
| ZRTP | Unsupported through SIP | Incomplete lower-layer code is not advertised, negotiated, or claimed as an end-to-end capability. |
| NULL encryption/authentication suites | Test primitives only | They cannot become ready secure transports and are never placed in SIP capability or SDP offer lists. |

Outbound SDES key material always uses canonical RFC 4648 Base64. Inbound key
material defaults to `SdesBase64Mode::Compatible`, which accepts canonical
Base64 or omission of only the required trailing `=` characters. Set
`EndpointBuilder::sdes_base64_mode` or
`StreamPeerBuilder::sdes_base64_mode` to `SdesBase64Mode::Strict` to require
canonical padding. Direct coordinator users pass
`SipRuntimeConfig::default().with_sdes_base64_mode(...)` to
`UnifiedCoordinator::new_with_runtime`. Both modes require the suite's exact
decoded key-plus-salt length.

`DiagnosticEvent::SdesNegotiationFailed`, received from
`subscribe_diagnostics`, reports the stage, failure class, crypto tag, suite,
encoded length, padding classification, and expected/actual decoded length. It
also carries a response envelope: the received response for answer failures,
or the locally authored 488 outcome plus rejected remote offer for offer
failures, so an application can inspect the rejected message. The bounded
diagnostic stream is observational and never
blocks signaling. The structured diagnostic and the event's `Debug` output
never render the encoded key, decoded key material, lifetime/MKI content,
parser source text, or response body. Applications must still treat the
explicitly accessed response body as sensitive SDP.

## SIP authentication

The public Digest client and server paths support MD5, MD5-sess, SHA-256,
SHA-256-sess, SHA-512-256, and SHA-512-256-sess. They support `qop=auth` and
`qop=auth-int` when the request body is available. `CallAuthRetrying` is emitted
only after a challenged INVITE was successfully authorized and dispatched. The
source-compatible `Event::CallAuthRetrying` reports status and realm;
`DiagnosticEvent::CallAuthRetrying` adds the selected algorithm and qop on the
bounded opt-in diagnostic stream. Neither reports a nonce, password, HA1,
cnonce, or response hash.

Digest algorithm availability does not mean a remote service can be forced to
challenge with every algorithm. SIP trace correlation identifies the initial
request, challenge, authenticated retry, and final response when tracing is
enabled under the configured redaction policy.

## SIP TLS terminology

`sips:` is a secure-URI requirement. `sip:...;transport=tls` is an ordinary SIP
URI selecting TLS transport and is not equivalent to `sips:`. Target-refresh
rejection diagnostics expose only the URI-scheme class and transport class so
operators can distinguish these cases without logging the Contact URI.
