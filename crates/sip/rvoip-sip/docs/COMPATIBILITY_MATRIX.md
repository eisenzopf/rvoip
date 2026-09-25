# rvoip-sip Beta Compatibility Matrix

Date: 2026-09-05

This matrix is the beta release contract. `Supported` and `Interop tested`
entries have repeatable in-repo or external-peer evidence. `Partial`,
`Experimental`, `Not supported`, and `Post-beta` entries must not be marketed
as general beta capabilities.

The current full reference is the
[protected 0.3.9 qualification](BETA_RELEASE_REPORT.md), with exact executed
coverage in the [208-gate report](BETA_GATE_REPORT.md). It describes run
`33969263241` from clean tested commit
`8cab44b10f872d21b304c02111d5d203ee8226da`.

## Support Levels

| Level | Meaning |
|-------|---------|
| Supported | Implemented and covered by automated tests in this repo. |
| Interop tested | Supported plus validated against an external SIP peer or tool. |
| Partial | Some implementation exists, but beta behavior is incomplete or not fully tested. |
| Parser only | Syntax can be parsed/serialized, but higher-layer behavior is not claimed. |
| Experimental | Useful for labs or perf investigations, but not a beta support promise. |
| Not supported | Must fail clearly or remain unavailable. |
| Post-beta | Deliberately excluded from the beta contract. |

## Application API Surfaces

| Surface | Beta status | Evidence | Notes |
|---------|-------------|----------|-------|
| `Endpoint` | Supported | Rustdoc/examples gate, PBX `endpoint` rows in `pbx/matrix.tsv` | Preferred account-like API. |
| `StreamPeer` | Supported | Rustdoc/examples gate, PBX `stream_peer` rows, stream-peer integration tests | Best for scripts, tests, and simple clients. |
| `CallbackPeer` | Supported | Rustdoc/examples gate, PBX `callback` rows, callback integration tests | Best for IVR/server style apps. |
| `UnifiedCoordinator` | Supported | `rvoip-sip integration tests`, B2BUA/bridge examples, generated validation | Lower-level orchestration surface. |
| `SessionHandle` | Supported | Call-control, media, DTMF, hold/resume, transfer, and NOTIFY tests | Per-call control surface. |

## External SIP Peers

The published `0.3.9` qualification remains the current release authority.
The following `0.3.10` candidate row records new scope and must not be read as
published-release evidence until the protected exact-candidate report replaces
its pending status.

| Peer | Profile | APIs | Covered scenarios | Status and limits |
|---|---|---|---|---|
| Jambonz OSS 0.9.9 | Real inbound/outbound SBC, registrar, Drachtio, Redis, MySQL, and RTPengine; source revisions and images are digest-pinned and latest-version checked | `Endpoint`, `StreamPeer`, `CallbackPeer` | Authenticated registration; PCMU/PCMA bidirectional calls; provisional/final signaling; hold/resume; RFC 4733 DTMF; CANCEL/487; rejection; RFC 3515 blind transfer with ordered NOTIFY; optional RFC 3892 Referred-By propagation; replacement INVITE; either-side BYE; cleanup | Local Colima rehearsal passed; protected `0.3.10` exact-candidate result pending. UDP/plain RTP only. G.729, AMR, TLS/SRTP, RVoIP-as-B2BUA, WebRTC, PSTN, application verbs, recording, HA, and load are excluded from this peer profile. |

This is a SIP/SDP/RTP interoperability claim about the exact open-source
profile above. It is not a claim about commercial Jambonz 10.x, hosted
jambonz.cloud, or the Jambonz application API.

## SIP Methods

| Method | Parser | Transaction/dialog | Public API | Beta status | Evidence |
|--------|--------|--------------------|------------|-------------|----------|
| INVITE | Supported | Supported | Supported | Interop tested | SIPp, Asterisk, FreeSWITCH, baresip, loopback tests. |
| ACK | Supported | Supported | Stack managed | Interop tested | INVITE lifecycle tests, SIPp traces, strict-UA run. |
| BYE | Supported | Supported | Supported | Interop tested | Cleanup tests, PBX matrix, SIPp. |
| CANCEL | Supported | Supported | Supported | Interop tested | `cancel_integration.rs`, ring-cancel PBX rows. |
| REGISTER | Supported | Supported | Supported | Interop tested | `registration_test.rs`, `register_423_retry.rs`, PBX registration rows. |
| OPTIONS | Supported | Supported | Supported | Supported | `options` send/response tests, SIPp scenario, and credentialed OOB auth retry test. |
| re-INVITE | Supported | Supported | Supported | Supported | Hold/resume PBX rows, glare retry tests. |
| UPDATE | Supported | Supported | Supported | Supported | Update send and session-timer coverage; outbound 491 completes the exact UPDATE attempt and does not emit `ReinviteGlare`. |
| PRACK | Supported | Partial | Stack managed | Partial | PRACK integration and dialog tests; broader PBX 100rel matrix pending. |
| REFER | Supported | Supported | Supported | Interop tested | Blind-transfer PBX rows, REFER/NOTIFY progress tests, and the Jambonz candidate transfer profile. |
| NOTIFY | Supported | Supported | Supported | Supported | REFER progress, subscription, and notify-send tests. |
| INFO | Supported | Supported | Supported | Supported | INFO auth retry and DTMF tests. |
| SUBSCRIBE | Supported | Partial | Supported | Partial | Subscription dialog tests and credentialed OOB auth retry test; event-package matrix incomplete. |
| MESSAGE | Supported | Partial | Supported | Partial | Message send/receive tests and credentialed OOB auth retry test; direct interop gate is not a beta headline. |
| PUBLISH | Parser only | Not supported | Not supported | Post-beta | Parser-only/non-claim until wired end to end. |

## Authentication

Developer-facing auth API and crate-boundary guidance is in
`crates/sip/rvoip-sip/docs/AUTHENTICATION.md`.

| Feature | Beta status | Evidence | Notes |
|---------|-------------|----------|-------|
| SIP Digest MD5 / MD5-sess | Supported | `auth-core` digest tests, REGISTER/INVITE/OOB auth retry tests, PBX registration rows | Common PBX baseline. |
| SIP Digest SHA-256 / SHA-256-sess | Supported | `auth-core` digest tests | Supported for challenge/response computation and validation. |
| SIP Digest SHA-512-256 / SHA-512-256-sess | Supported | `auth-core` digest tests and `SipDigestAuthService` tests | Unsupported algorithm tokens fail clearly rather than falling back to MD5. |
| `401 WWW-Authenticate` | Supported | REGISTER, INVITE, in-dialog, and OOB auth retry tests | Retries use `Authorization`. |
| `407 Proxy-Authenticate` | Supported | REGISTER 407, INVITE 407, and OOB 407 auth retry tests | Retries use `Proxy-Authorization`. |
| Digest `qop=auth` | Supported | REGISTER, INVITE, OOB, and server-side validation tests | Nonce-count is monotonic per nonce. |
| Digest `qop=auth-int` | Supported where the request body is available | `auth-core`, `SipDigestAuthService`, and MESSAGE OOB auth-int tests | MESSAGE and INVITE bodies are included in HA2 when offered. |
| Digest `stale=true` nonce recovery | Supported | INVITE 407 stale recovery and OOB stale recovery tests | Exactly one additional stale retry is allowed with a fresh nonce. |
| SIP Basic auth | Supported, explicit opt-in | `SipAuthService` unit tests and OOB MESSAGE retry tests | Legacy compatibility only. UAC/UAS reject cleartext Basic unless the caller explicitly opts in or the request is over `sips:`. |
| SIP Bearer auth | Supported | `SipAuthService` Bearer tests and OOB MESSAGE retry tests | UAC responds to Bearer challenges; UAS delegates validation to `auth-core` Bearer/JWT/JWKS/AAuth validators and exposes `AuthIdentity`. |
| IMS AKA / AKAv1-MD5 / AKAv2-MD5 | Provider-backed | Public `AkaClientProvider` / `AkaVectorProvider` API and challenge builders | AKA is negotiated as a Digest-family scheme. Production vector issuance/USIM/Milenage integration is supplied by the application/provider. |
| Multi-challenge negotiation | Supported | Composite-auth unit and OOB integration tests | UAC `SipClientAuth::any(...)` prefers AKA, then Bearer, then Digest, then Basic among configured compatible options. |

## Transport

| Feature | Beta status | Evidence | Notes |
|---------|-------------|----------|-------|
| UDP | Interop tested | SIPp, Asterisk, FreeSWITCH, and the Jambonz candidate matrix | Primary beta transport. |
| TCP | Supported | Transport/dialog tests | Include in external matrix where peers support it. |
| TLS client | Supported | TLS transport tests, TLS call integration, PBX TLS rows | Server validation and SNI are tested. |
| TLS server | Supported | TLS listener/call tests, PBX TLS rows | Requires cert/key configuration. |
| mTLS | Partial | TLS config validation and transport primitives | Broad external mTLS interop is not claimed. |
| WS | Partial | WebSocket transport round-trip tests | Browser/WebRTC is post-beta. |
| WSS outbound | Not supported | Explicit non-claim and known `NotImplemented` paths | Do not claim. |
| RFC 3263 DNS | Supported | Resolver failover and Hickory tests | External DNS lab evidence remains useful. |
| IPv6 | Not audited | URI/parser support exists | Do not claim network-stack IPv6 interop until audited. |

## Media and Security

| Feature | Beta status | Evidence | Notes |
|---------|-------------|----------|-------|
| SDP RFC 8866 | Supported | SDP parser/builder tests, generated validation, SDP fuzz target | WebRTC attributes are parser/carry-through only unless wired higher. |
| SDP offer/answer RFC 3264 | Supported | Hold/resume, codec matching, glare tests | Advanced media changes are not beta-scoped. |
| RTP/RTCP RFC 3550 | Supported | RTP steady-state perf, audio round-trip, bridge round-trip | Full RTCP feedback matrix is not a beta claim. |
| PCMU/PCMA | Supported | Codec and RTP media tests | Available in the default build. |
| telephone-event DTMF | Supported | DTMF tests and PBX interop | RFC 4733 behavior must stay covered. |
| Comfort Noise PT 13 | Supported | Config validation and SDP/media tests | Requires `comfort_noise_enabled=true`. |
| Opus | Feature-gated | Real codec round trips plus SIP/RTP dynamic-payload tests | Requires the `opus` feature. |
| G.729 | Feature-gated | Codec, SDP, and media tests | Requires the `g729` feature. |
| AMR-NB | Feature-gated | Bit-exact against the 3GPP reference plus the normative sequences; RFC 4867 framing checked against Wireshark's dissector; loopback and SRTP call tests; PBX and proxy lab rows | Requires the `amr-nb` feature. Not part of the bounded SIP beta media claim. |
| AMR-WB | Feature-gated | Same evidence set as AMR-NB at 16 kHz, all 9 modes | Requires the `amr-wb` feature. Not part of the bounded SIP beta media claim. |
| G.722 | Not supported | Construction/negotiation rejection tests | RTP payload parsing metadata remains available. |
| SDES-SRTP | Partial | SRTP integration/negotiator tests and PBX rows where present | Limited to tested suites. |
| DTLS-SRTP | Post-beta | Explicit non-claim | Do not claim. |
| ICE/TURN/WebRTC browser | Post-beta | Explicit non-claim | STUN remains limited address discovery. |
| STIR/SHAKEN | Partial | STIR/SHAKEN crate tests and dialog identity tests | Library support, not certification. |
| Trace redaction | Supported | `trace_redaction.rs`, infra-common redaction tests | Redacts auth, tokens, identity headers, SDES keys, and ICE passwords. |

## Performance Profiles

| Profile | Beta status | Target | Notes |
|---------|-------------|--------|-------|
| General full-media | Beta target | Up to 2,000 CPS | Backed by three canonical clean passes plus the current monolithic and split one-hour soak configurations; no 24-hour claim is made. |
| Signaling-only tuned | Experimental | Above 2,000 CPS | Requires explicit tuning docs and caveats. |
| Tuned high-scale | Experimental | Near 10,000 CPS where proven | Not a general-user promise. |
