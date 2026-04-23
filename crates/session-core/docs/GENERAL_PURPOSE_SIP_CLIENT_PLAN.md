# General-Purpose SIP Client Roadmap

Tracking doc for the remaining blockers between today's `session-core`
(signalling-complete, RFC 3261 §14.1 hold/resume correct, NOTIFY +
RFC 3515 §2.4.5 wired) and a credible "general-purpose SIP client" that
can connect to Asterisk, FreeSWITCH, and cloud carriers (Twilio,
Vonage, Bandwidth) end-to-end.

This is the **next document after** `PRE_B2BUA_ROADMAP.md`. The
b2bua/IVR/recording API surface (per-call event streams, bridge
primitive, NOTIFY) has all landed. What remains is everything outside
the call-control story: transport (TLS/TCP/SRV/NAT), media security
(SRTP), and trunking headers (PAI, Service-Route).

Cross-references:
- `RFC_COMPLIANCE_STATUS.md` — current method/header matrix.
- `PRE_B2BUA_ROADMAP.md` — multi-session control plane (closed).
- `TELCO_USE_CASE_ANALYSIS.md` — softphone / B2BUA / IVR / PBX / E911 use cases.

---

## Reality check by deployment target

| Target | Today | What still blocks it |
|--------|-------|----------------------|
| **Asterisk / FreeSWITCH on LAN, IP-based, PCMU/PCMA** | ✅ works | nothing |
| **Asterisk / FreeSWITCH with `srtp` enabled** (default in modern installs) | ✅ | RFC 4568 SDES-SRTP fully wired — `Config::offer_srtp = true` |
| **PBX trunking with carrier-presented identity** | ⚠️ | P-Asserted-Identity (RFC 3325), Diversion / History-Info (RFC 7044) |
| **Cloud carriers — Twilio, Vonage, Bandwidth (UDP+digest)** | ⚠️ | DNS resolves; production tier requires TLS 5061; some require P-Asserted-Identity for trunk auth |
| **Carriers requiring TLS 5061** | ❌ | TLS client connector + session-core config wiring |
| **UAC behind NAT, long-duration registration** | ⚠️ | Contact rewrite from `received=` / `rport=`; SIP Outbound (RFC 5626) keep-alive |
| **Carrier with geo-failover via `_sip._udp` SRV** | ❌ | RFC 3263 SRV/NAPTR |
| **WebRTC peer (browser ↔ session-core)** | ❌ | WebSocket client transport (RFC 7118), DTLS-SRTP SDP, ICE/TURN |
| **PBX requiring TCP for large SDP / video** | ❌ | TCP enable flag wiring (TCP impl itself is ready) |

---

## Source-of-truth findings

Audit of the workspace as of 2026-04-20.

### Transport — `crates/sip-transport`

- **TCP**: production-ready in `crates/sip-transport/src/transport/tcp/mod.rs` (255 LOC, server + client pool, RFC 3261 §18.3 framing, 300 s idle keep-alive). Just gated off by `enable_tcp: false` at `crates/session-core/src/api/unified.rs:822`.
- **TLS**: two conflicting impls.
  - `crates/sip-transport/src/tls.rs` — server-side complete (rustls), **client `connect()` returns `NotImplemented` at line 254**.
  - `crates/sip-transport/src/transport/tls/mod.rs` — placeholder; `send_message()` returns `NotImplemented` at line 66.
  - dialog-core's `add_tcp_transport(addr, tls=true)` at `transaction/transport/mod.rs:256` explicitly rejects TLS, so even flipping the flag wouldn't help today.
- **WebSocket (RFC 7118)**: server complete in `crates/sip-transport/src/transport/ws/mod.rs` (subprotocol negotiation, WSS, accept loop). **Client `connect_to()` stubbed at line 229.** Feature-gated `ws` is on by default in Cargo.toml.
- session-core hardcodes `enable_tls: false`, `enable_tcp: false`, `enable_ws: false`. `Config` exposes no `tls_cert_path`, `tls_key_path`, or transport-preference fields.

### Media & RTP — `crates/media-core` + `crates/rtp-core`

- **Codecs registered**: PCMU (PT 0), PCMA (PT 8), G.722 (PT 9), G.729 (PT 18), Opus (PT 96). Telephone-event (PT 101) constant defined; **RTP DTMF event encode/decode not wired** (`crates/media-core/src/codec/audio/mod.rs:26`).
- **SRTP crypto core present** at `crates/rtp-core/src/srtp/` (AES-CM, AES-F8, HMAC-SHA1).
- **DTLS-SRTP framework present** at `crates/rtp-core/src/dtls/` (RFC 5764 key extraction, role negotiation; depends on `webrtc-dtls 0.9`).
- **SDES-SRTP client** stubbed at `crates/rtp-core/src/api/client/security/srtp/sdes.rs` but **no `a=crypto:` SDP builder** — Asterisk and FreeSWITCH cannot negotiate encrypted media with us today even though the crypto primitives are in the workspace.
- **SDP parser** recognises `a=crypto:`, `a=fingerprint:`, `a=setup:`, `a=ice-ufrag:`, `a=candidate:` (`crates/sip-core/src/sdp/parser/attribute_parser.rs`). **None of those have builders** in `crates/sip-core/src/builder/`.
- **SDP offer/answer matching** is a TODO stub at `crates/dialog-core/src/sdp/negotiation.rs:31` — current flows work because session-core handles negotiation in its own actions.
- **STUN / ICE / TURN**: parser sees `a=ice-ufrag:` / `a=candidate:`, no agent or candidate gatherer exists.
- **RFC 3389 Comfort Noise (PT 13)**: not implemented. VAD exists at `crates/media-core/src/processing/audio/vad.rs` but no CN packet generation.
- **Video**: VP8 (PT 97) / VP9 (PT 98) RTP payload handlers exist. No H.264. No `m=video` builder. No video transmitter wiring in media-core.

### SIP headers — `crates/sip-core` + `crates/dialog-core`

Implemented (verified):
- Record-Route / Route + loose routing (`crates/dialog-core/src/dialog/dialog_impl.rs:58-59`).
- `Path` header type + builder (`crates/sip-core/src/types/path.rs`).
- `Max-Forwards` defaults to 70 (`crates/dialog-core/src/transaction/client/builders.rs:79`).
- `Supported`, `Allow`, `Require`, `Unsupported`.
- `Reason` header (RFC 3326) — landed in the session-timer-failure work.
- Multipart/mixed bodies.
- IPv6 in transport + Via.

Absent (not implemented anywhere in the workspace):
- **P-Asserted-Identity / P-Preferred-Identity** (RFC 3325) — *load-bearing for SIP trunking*.
- **Service-Route** (RFC 3608) — registrar-returned route set; RFC 5626 SIP Outbound depends on it.
- **GRUU** (RFC 5627) — `+sip.instance` / `pub-gruu` / `temp-gruu`.
- **History-Info** (RFC 7044) and **Diversion** (RFC 4244).
- **Privacy** (RFC 3323).
- **Compact header forms (RX)** (RFC 3261 §7.3.3) — `f=`/`t=`/`v=`/`m=`/etc. Parser doesn't expand them.
- **Outbound proxy** static route knob.
- **PUBLISH** (RFC 3903) — silent-fallthrough stub already documented.
- **OPTIONS outbound helper** for keep-alive.

---

## Tiered backlog

Each tier is independently shippable. Tier letter is value-per-effort, not severity.

### Tier A — Carrier transport

Already enumerated in `PRE_B2BUA_ROADMAP.md` as P1–P7. This audit confirms each against the source.

| # | Item | RFC | Status | Files |
|---|------|-----|--------|-------|
| A1 | TLS client connector + config wiring | RFC 5630, RFC 3261 §26 | ⬜ | `sip-transport/src/tls.rs:254`, `dialog-core/src/transaction/transport/mod.rs:256`, `session-core/src/api/unified.rs:822`, `Config` struct |
| A2 | TCP enable-flag wiring | RFC 3261 §18.3 | ⬜ | `session-core/src/api/unified.rs:822` |
| A3 | Contact rewrite from `received=`/`rport=` | RFC 3581, RFC 5626 §5 | ⬜ | `dialog-core/src/transaction/client/builders.rs:690-728`, `dialog-core/src/transaction/manager/handlers.rs:564-587` |
| A4 | RFC 3263 SRV + NAPTR | RFC 3263 | ⬜ | `dialog-core/src/dialog/dialog_utils.rs:125-174`; new dep `hickory-resolver` |
| A5 | SIP Outbound + CRLF keep-alive, `+sip.instance`, flow-id, `reg-id` | RFC 5626 | ⬜ | new module in `dialog-core` |
| A6 | STUN client + `Config::public_address` | RFC 8489 | ⬜ | new module in `rtp-core` or `media-core` |
| A7 | Digest enhancements: `nc` counter tracking, `auth-int` qop, `-sess` algorithms | RFC 8760 | ⬜ | `auth-core/src/sip_digest.rs:354` |

### Tier B — PBX trunking essentials (NEW — not in prior docs)

These bite the moment session-core points at a real Asterisk/FreeSWITCH **trunk** (vs LAN extension) or any cloud SIP trunk.

| # | Item | RFC | Status | Files |
|---|------|-----|--------|-------|
| B1 | P-Asserted-Identity / P-Preferred-Identity | RFC 3325 | ✅ end-to-end | sip-core types + parser + builder ext ✅, dialog-core `make_call_with_extra_headers[_for_session]` + `send_initial_invite_with_extra_headers` ✅, session-core `Config::pai_uri` + `UnifiedCoordinator::make_call_with_pai` + `SessionState.pai_uri` + `Action::SendINVITE` routing ✅, inbound PAI on `IncomingCallInfo.p_asserted_identity` ✅. Wire-level multi-binary integration test still TODO (call this `tests/pai_integration.rs`). |
| B2 | SDES-SRTP — full end-to-end (SDP builder + parser + key exchange + transport encrypt/decrypt) | RFC 4568, RFC 3711 | ✅ | sip-core `CryptoSuite` + `CryptoAttribute` types/parser/builder; session-core `SrtpNegotiator` + `Config::offer_srtp` / `srtp_required` / `srtp_offered_suites`; rtp-core `UdpRtpTransport::set_srtp_contexts` wraps send/receive; media-core `install_srtp_contexts`. See `crates/STEP_2B_SRTP_INTEGRATION_PLAN.md` for the four-phase landing log. |
| B3 | Service-Route processing on REGISTER 200 | RFC 3608 | ⬜ | `dialog-core/src/protocol/register_handler.rs` (or equivalent), `dialog-core/src/transaction/client/builders.rs` |
| B4 | DTMF RTP events end-to-end | RFC 4733 | ⬜ | `media-core/src/codec/audio/mod.rs:26`, transmitter/receiver wiring |
| B5 | Compact header forms on receive | RFC 3261 §7.3.3 | ⬜ | `sip-core/src/parser/header.rs` |

### Tier C — Media-path completeness (NEW)

QoS items that don't break interop in lab tests but matter in production.

| # | Item | RFC | Status | Files |
|---|------|-----|--------|-------|
| C1 | Comfort Noise (PT 13) | RFC 3389 | ⬜ | new `media-core/src/codec/audio/cn.rs`; transmitter pause-with-CN |
| C2 | SDP offer/answer matching helper | RFC 3264 | ⬜ | `dialog-core/src/sdp/negotiation.rs:31` |

### Tier D — WebRTC interop (NEW; only if browser interop is in scope)

| # | Item | RFC | Status | Files |
|---|------|-----|--------|-------|
| D1 | WebSocket client transport | RFC 7118 | ⬜ | `sip-transport/src/transport/ws/mod.rs:229` |
| D2 | DTLS-SRTP SDP builder (`a=fingerprint:`, `a=setup:`) | RFC 5763, RFC 5764 | ⬜ | new `sip-core/src/sdp/builder/dtls.rs`; consume existing `rtp-core/src/dtls/` |
| D3 | ICE agent + candidate gathering | RFC 8445 | ⬜ | new `rtp-core/src/ice/` |
| D4 | TURN client | RFC 8656 | ⬜ | new `rtp-core/src/turn/` |
| D5 | H.264 RTP payload + `m=video` builder | RFC 6184 | ⬜ | `rtp-core/src/payload/h264.rs`; SDP builder |

### Tier E — Aesthetic / advanced

| # | Item | RFC | Status |
|---|------|-----|--------|
| E1 | GRUU | RFC 5627 | ⬜ |
| E2 | History-Info / Diversion | RFC 7044 / RFC 4244 | ⬜ |
| E3 | Privacy header | RFC 3323 | ⬜ |
| E4 | Outbound proxy static route knob | — | ⬜ |
| E5 | 305 Use Proxy / 380 Alternative Service proxy semantics | RFC 3261 §21.3.4-5 | ⬜ |
| E6 | PUBLISH — wire through dialog-core or remove the silent-fallthrough YAML stub | RFC 3903 | ⬜ |
| E7 | Outbound OPTIONS helper (superseded by RFC 5626 keep-alive) | RFC 3261 §11 | ⬜ |

---

## Sprint plan

| Sprint | Items | Outcome |
|--------|-------|---------|
| **1 — make it talk to real PBXs** | A1 (TLS) + A2 (TCP flip) + A3 (Contact rewrite) + B1 (P-Asserted-Identity) + B2 (SDES-SRTP SDP) | Asterisk/FreeSWITCH with `srtp` and trunk identity; Twilio/Vonage/Bandwidth on TLS |
| **2 — production-NAT and registration robustness** | A4 (SRV/NAPTR) + A5 (RFC 5626 SIP Outbound) + B3 (Service-Route) + B4 (RFC 4733 DTMF) + B5 (compact headers RX) | Long-duration registrations through SBCs survive; carrier geo-failover; in-band DTMF |
| **3 — media polish** | A6 (STUN) + A7 (digest enhancements) + C1 (Comfort Noise) + C2 (SDP negotiation helper) | NAT traversal without RFC 5626; carrier QoS dashboards stop complaining |
| **4 — WebRTC (opt-in)** | D1–D5 in order | Browser ↔ session-core works |
| **Backlog** | Tier E | Pull individually when a deployment asks |

---

## Verification strategy (per item)

- **TLS / TCP flip**: extend `examples/streampeer/run_all.sh` with a TLS-enabled Bob; assert UDP-Alice ↔ TLS-Bob fails (config mismatch) and TLS-Alice ↔ TLS-Bob succeeds. Run `audio_roundtrip_integration` over TLS to confirm media still flows.
- **SDES-SRTP**: round-trip integration test mirroring `audio_roundtrip_integration` with `a=crypto:` in both SDPs; assert wire-captured RTP is encrypted (no readable PCMU header in payload).
- **P-Asserted-Identity**: unit test on the builder + multi-binary test asserting an inbound INVITE's PAI is surfaced on `IncomingCallInfo`.
- **RFC 3263 SRV**: in-process mock DNS resolver returning weighted SRV records; assert UAC tries highest-priority first and fails over.
- **Contact rewrite**: in-process mock UAS that responds with `received=`/`rport=` differing from request source; assert next REGISTER carries the rewritten Contact.
- **RFC 4733 DTMF**: extend `audio_roundtrip_integration` with a DTMF event mid-call; assert receiver decodes the digit.
- **Asterisk / FreeSWITCH end-to-end**: a `tests/asterisk_interop/` directory that spins up an Asterisk container in docker-compose and runs the example suite against it. Optional but the only way to catch real-world quirks before users do.

---

## Progress log

| Date | Item | Notes |
|------|------|-------|
| 2026-04-20 | doc | Initial plan written from audit of `sip-transport`, `media-core`, `rtp-core`, `sip-core`, `dialog-core`. |
| 2026-04-20 | B1 sip-core | PAssertedIdentity + PPreferredIdentity typed headers + parser + builder ext landed in `sip-core` (RFC 3325). 18 new unit tests; full sip-core suite still green (1997/1997). dialog-core + session-core wiring tracked as separate follow-up. |
| 2026-04-20 | B1 dialog-core + session-core | Plumbed PAI all the way through. dialog-core gained `make_call_with_extra_headers[_for_session]` + `send_initial_invite_with_extra_headers` (mirrors `send_invite_with_auth`). session-core gained `Config::pai_uri`, `UnifiedCoordinator::make_call_with_pai`, `SessionState.pai_uri`, and `Action::SendINVITE` now routes through `DialogAdapter::send_invite_with_extra_headers` when PAI is set. Inbound PAI surfaces on `IncomingCallInfo.p_asserted_identity` via the existing event_hub headers-map path. Workspace builds clean; sip-core 1997 / dialog-core 174 / session-core 18 lib tests still green. Wire-level multi-binary integration test deferred. |
