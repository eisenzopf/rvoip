# rvoip-sip Beta Compatibility Matrix

Date: 2026-05-26

This matrix is the beta release contract. `Supported` and `Interop tested`
entries have repeatable in-repo or external-peer evidence. `Partial`,
`Experimental`, `Not supported`, and `Post-beta` entries must not be marketed
as general beta capabilities.

The latest full reference report is
`crates/rvoip-sip/beta-report/20260526T221457Z`, generated from clean git
revision `865430d4`.

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

## SIP Methods

| Method | Parser | Transaction/dialog | Public API | Beta status | Evidence |
|--------|--------|--------------------|------------|-------------|----------|
| INVITE | Supported | Supported | Supported | Interop tested | SIPp, Asterisk, FreeSWITCH, baresip, loopback tests. |
| ACK | Supported | Supported | Stack managed | Interop tested | INVITE lifecycle tests, SIPp traces, strict-UA run. |
| BYE | Supported | Supported | Supported | Interop tested | Cleanup tests, PBX matrix, SIPp. |
| CANCEL | Supported | Supported | Supported | Interop tested | `cancel_integration.rs`, ring-cancel PBX rows. |
| REGISTER | Supported | Supported | Supported | Interop tested | `registration_test.rs`, `register_423_retry.rs`, PBX registration rows. |
| OPTIONS | Supported | Supported | Supported | Supported | `options` send/response tests and SIPp scenario. |
| re-INVITE | Supported | Supported | Supported | Supported | Hold/resume PBX rows, glare retry tests. |
| UPDATE | Supported | Supported | Supported | Supported | Update send tests and glare/session-timer coverage. |
| PRACK | Supported | Partial | Stack managed | Partial | PRACK integration and dialog tests; broader PBX 100rel matrix pending. |
| REFER | Supported | Supported | Supported | Interop tested | Blind-transfer PBX rows, REFER/NOTIFY progress tests. |
| NOTIFY | Supported | Supported | Supported | Supported | REFER progress, subscription, and notify-send tests. |
| INFO | Supported | Supported | Supported | Supported | INFO auth retry and DTMF tests. |
| SUBSCRIBE | Supported | Partial | Supported | Partial | Subscription dialog tests; event-package matrix incomplete. |
| MESSAGE | Supported | Partial | Supported | Partial | Message send/receive tests; direct interop gate is not a beta headline. |
| PUBLISH | Parser only | Not supported | Not supported | Post-beta | Parser-only/non-claim until wired end to end. |

## Transport

| Feature | Beta status | Evidence | Notes |
|---------|-------------|----------|-------|
| UDP | Interop tested | SIPp, Asterisk, FreeSWITCH matrices | Primary beta transport. |
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
| PCMU/PCMA | Supported | Codec and RTP media tests | Only beta full-media audio codecs. |
| telephone-event DTMF | Supported | DTMF tests and PBX interop | RFC 4733 behavior must stay covered. |
| Comfort Noise PT 13 | Supported | Config validation and SDP/media tests | Requires `comfort_noise_enabled=true`. |
| Opus/G.722/G.729 | Post-beta | Config validation rejects unsupported beta full-media advertising | No beta full-media claim. |
| SDES-SRTP | Partial | SRTP integration/negotiator tests and PBX rows where present | Limited to tested suites. |
| DTLS-SRTP | Post-beta | Explicit non-claim | Do not claim. |
| ICE/TURN/WebRTC browser | Post-beta | Explicit non-claim | STUN remains limited address discovery. |
| STIR/SHAKEN | Partial | STIR/SHAKEN crate tests and dialog identity tests | Library support, not certification. |
| Trace redaction | Supported | `trace_redaction.rs`, infra-common redaction tests | Redacts auth, tokens, identity headers, SDES keys, and ICE passwords. |

## Performance Profiles

| Profile | Beta status | Target | Notes |
|---------|-------------|--------|-------|
| General full-media | Beta target | Up to 2,000 CPS | Default claim is backed by the final clean report; 24-hour soak is waived for beta and the 30-minute soak is accepted as the beta bar. |
| Signaling-only tuned | Experimental | Above 2,000 CPS | Requires explicit tuning docs and caveats. |
| Tuned high-scale | Experimental | Near 10,000 CPS where proven | Not a general-user promise. |
