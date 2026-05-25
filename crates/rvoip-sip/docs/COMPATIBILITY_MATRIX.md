# rvoip-sip Beta Compatibility Matrix

Date: 2026-05-25

This matrix is the beta release contract. Entries marked `Supported` or
`Interop tested` must have repeatable test evidence before beta release notes
can claim them. Entries marked `Partial`, `Experimental`, `Not supported`, or
`Post-beta` must not be marketed as beta capabilities.

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
| Not audited | Needs review before any claim is made. |

## Application API Surfaces

| Surface | Beta status | Evidence required | Notes |
|---------|-------------|-------------------|-------|
| `Endpoint` | Supported | Rustdoc, examples, Asterisk/FreeSWITCH profile runs | Preferred user-facing API for account-like apps. |
| `StreamPeer` | Supported | Sequential client examples and PBX scenarios | Best for tests, scripts, and simple clients. |
| `CallbackPeer` | Supported | Server/reactive PBX scenarios | Best for IVR/server style apps. |
| `UnifiedCoordinator` | Supported | Integration tests and B2BUA examples | Lower-level orchestration surface. |
| `SessionHandle` | Supported | Call control and media tests | Per-call control for answer, hangup, DTMF, hold/resume, transfer. |

## SIP Methods

| Method | Parser | Transaction/dialog | Public API | Beta status | Required evidence |
|--------|--------|--------------------|------------|-------------|-------------------|
| INVITE | Supported | Supported | Supported | Interop tested | SIPp, Asterisk, FreeSWITCH, loopback tests. |
| ACK | Supported | Supported | Stack managed | Interop tested | INVITE lifecycle tests and SIPp traces. |
| BYE | Supported | Supported | Supported | Interop tested | Cleanup and teardown tests. |
| CANCEL | Supported | Supported | Supported | Interop tested | Pre-answer cancel tests and PBX scenarios. |
| REGISTER | Supported | Supported | Supported | Interop tested | Registrar tests plus Asterisk/FreeSWITCH. |
| OPTIONS | Supported | Supported | Supported | Supported | Loopback and SIPp scenario. |
| re-INVITE | Supported | Supported | Supported | Supported | Hold/resume and glare tests. |
| UPDATE | Supported | Supported | Supported | Supported | Mid-dialog update smoke and glare tests. |
| PRACK | Supported | Partial | Stack managed | Partial | Needs broader B2BUA and PBX reliable-provisional matrix. |
| REFER | Supported | Supported | Supported | Interop tested | Blind transfer and REFER/NOTIFY progress. |
| NOTIFY | Supported | Supported | Supported | Supported | REFER progress and subscription tests. |
| INFO | Supported | Supported | Supported | Supported | DTMF/info package tests. |
| SUBSCRIBE | Supported | Partial | Supported | Partial | Event package matrix incomplete. |
| MESSAGE | Supported | Partial | Supported | Partial | Direct path needs release-gated tests. |
| PUBLISH | Parser only | Not supported | Not supported | Post-beta | Must remain unsupported until wired end to end. |

## Transport

| Feature | Beta status | Required evidence | Notes |
|---------|-------------|-------------------|-------|
| UDP | Interop tested | SIPp, Asterisk, FreeSWITCH | Primary beta transport. |
| TCP | Supported | Unit/integration plus PBX where feasible | Must be included in matrix where peers support it. |
| TLS client | Supported | TLS registration/call tests | Server cert validation must be tested. |
| TLS server | Supported | Listener cert/key tests | mTLS matrix required where claimed. |
| mTLS | Partial | Cert-chain and negative tests | Supported only where tests pass. |
| WS | Partial | WebSocket round-trip tests | Browser/WebRTC is post-beta. |
| WSS outbound | Not supported | Negative validation or explicit docs | Known `NotImplemented` paths remain. |
| RFC 3263 DNS | Supported | Resolver tests plus interop profile | NAPTR/SRV/A/AAAA failover must be covered. |
| IPv6 | Not audited | Socket and DNS tests | Do not claim until audited. |

## Media and Security

| Feature | Beta status | Required evidence | Notes |
|---------|-------------|-------------------|-------|
| SDP RFC 8866 | Supported | Parser/serializer and offer/answer tests | WebRTC SDP attributes are parser-only unless higher layers support them. |
| SDP offer/answer RFC 3264 | Supported | Hold/resume and codec matching tests | Glare/conflict tests required. |
| RTP/RTCP RFC 3550 | Supported | RTP loopback and audio round-trip tests | Full RTCP feedback matrix is not a beta claim. |
| PCMU/PCMA | Supported | Codec and RTP media tests | Only beta audio codecs for full media. |
| telephone-event DTMF | Supported | Send/receive tests and PBX interop | RFC 4733 behavior must stay covered. |
| Comfort Noise PT 13 | Supported | Config validation and SDP/media tests | Requires explicit `comfort_noise_enabled`. |
| Opus/G.722/G.729 | Post-beta | None for beta | `Config::validate` rejects these for beta full-media advertising. |
| SDES-SRTP | Partial | Asterisk/FreeSWITCH plus SRTP tests | Beta claims limited to tested suites. |
| DTLS-SRTP | Post-beta | Not beta | Remove production claims until complete. |
| ICE/TURN/WebRTC browser | Post-beta | Not beta | STUN remains limited best-effort address discovery. |
| STIR/SHAKEN | Partial | Unit vectors and docs | Library support, not carrier certification. |

## Performance Profiles

| Profile | Beta status | Target | Notes |
|---------|-------------|--------|-------|
| General full-media | Beta target | Up to 2,000 CPS | Default claim only after published run meets exit criteria. |
| Signaling-only tuned | Experimental | Above 2,000 CPS | Requires explicit tuning docs and caveats. |
| Tuned high-scale | Experimental | Near 10,000 CPS where proven | Not a general-user promise. |
