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
| **PBX trunking with carrier-presented identity** | ✅ | P-Asserted-Identity (RFC 3325) end-to-end via `Config::pai_uri` / `make_call_with_pai` (B1) |
| **Cloud carriers — Twilio, Vonage, Bandwidth (TLS + SRTP + PAI)** | ✅ | TLS (A1), SDES-SRTP (B2), PAI (B1) all shipped — `Config::tls_*` + `offer_srtp=true` + `pai_uri` |
| **Carriers requiring TLS 5061** | ✅ | A1 — `Config::tls_cert_path` + `tls_key_path` + auto-bind 5060→5061 + URI-aware multiplexer routes `sips:` |
| **UAC behind NAT, long-duration registration** | ✅ | A3 — RFC 3581 NAT discovery + RFC 5626 §5 Contact rewrite. Long-duration NAT-traversal via SIP Outbound (RFC 5626) keep-alive still pending — see A5 |
| **Carrier with geo-failover via `_sip._udp` SRV** | ❌ | RFC 3263 SRV/NAPTR — A4 still pending |
| **WebRTC peer (browser ↔ session-core)** | ❌ | WebSocket client transport (RFC 7118), DTLS-SRTP SDP, ICE/TURN — Tier D, future |
| **PBX requiring TCP for large SDP / video** | ✅ | A2 — TCP enable flag flipped + URI-aware selection routes `;transport=tcp` + RFC 3261 §18.1.1 MTU fallback (when used) |

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
| A1 | TLS client connector + config wiring | RFC 5630, RFC 3261 §26 | ✅ | `sip-transport/src/transport/tls/mod.rs` (consolidated, ~600 LOC); `tokio_rustls::TlsConnector` with `rustls-native-certs` + `webpki-roots` fallback + optional extra CA + dev-only insecure-skip; `dialog-core/src/transaction/transport/mod.rs` enables TLS via `add_tcp_transport(addr, true)` with `bind_addr.port() + 1` (RFC 3261 5060→5061 convention); `session-core/src/api/unified.rs::Config` carries `tls_cert_path` / `tls_key_path` / `tls_extra_ca_path` / `tls_insecure_skip_verify`; `MultiplexedTransport::pick_transport` routes Responses via `Transport::has_connection_to(addr)` (RFC 3261 §17.2 / §18.2.2). Tests: `crates/sip-transport/tests/tls_handshake_test.rs` + `crates/session-core/tests/tls_call_integration.rs`. See `crates/TLS_SIP_IMPLEMENTATION_PLAN.md`. |
| A2 | TCP enable-flag wiring + URI-aware transport selection | RFC 3261 §18.3, §19.1.5, §26.2 | ✅ | `MultiplexedTransport` (`crates/dialog-core/src/transaction/transport/multiplexed.rs`) wraps the per-flavour registry; `TransactionManager::with_transport_manager` installs it as the single `Arc<dyn Transport>`, routing outbound by URI scheme + `;transport=` parameter; `enable_tcp: true` flipped at `crates/session-core/src/api/unified.rs`. `resolve_uri_to_socketaddr` uses typed `Uri` fields + `sips:`→5061 default + async DNS. 10 multiplexer unit tests (6 selection + 4 dispatch). |
| A3 | RFC 3581 NAT discovery + RFC 5626 §5 Contact rewrite | RFC 3581, RFC 5626 §5 | ✅ | `DialogManager::nat_discovered_addr` cache populated by `ResponseHandler::handle_response_message` from typed `Param::Received` + `Param::Rport(Some(_))` on every inbound response (no-op suppression when discovered == local); `UnifiedDialogApi::discovered_public_addr()` exposes the cache; `DialogAdapter::send_register` pre-rewrites Contact host:port via `rewrite_contact_host` helper (preserves scheme + user-part + URI params). 5 NAT-discovery extraction tests + 5 Contact-rewrite tests. |
| A4 | RFC 3263 SRV + A/AAAA resolution (NAPTR deferred) | RFC 3263 | ✅ core | `hickory-resolver 0.24` added to workspace + dialog-core; new module `crates/dialog-core/src/dialog/dns_resolver.rs` with `resolve_uri(uri) → ResolvedTarget { addr, transport }` implementing RFC 3263 §4 ladder (IP literal → A/AAAA-with-explicit-port → `_service._proto.host` SRV with RFC 2782 weighted selection → A/AAAA fallback); scheme-aware SRV service labels (`sip:`→`_sip._udp`/`_sip._tcp`, `sips:`→`_sips._tcp`, `ws`/`wss` for RFC 7118). Back-compat wrapper `resolve_uri_to_socketaddr` preserved at `crates/dialog-core/src/dialog/dialog_utils.rs`. `crates/dialog-core/src/transaction/manager/handlers.rs::resolve_uri_to_socketaddr` (ACK destination helper) redirected to the shared resolver. 15 unit tests covering scheme-to-default-port, SRV service-name derivation, RFC 2782 weighted selection (zero-weight, weighted, priority gating), and IP-literal short-circuit paths. NAPTR (RFC 3263 §4.1) deferred — its incremental value is narrow once `;transport=` and scheme-based selection already work; add if a deployment needs it. |
| A5 | SIP Outbound + CRLF keep-alive, `+sip.instance`, flow-id, `reg-id` | RFC 5626 | ⬜ | new module in `dialog-core` |
| A6 | STUN client + `Config::public_address` | RFC 8489 | ⬜ | new module in `rtp-core` or `media-core` |
| A7 | Digest enhancements: `nc` counter tracking, `auth-int` qop, `-sess` algorithms | RFC 8760 | ⬜ | `auth-core/src/sip_digest.rs:354` |

### Tier B — PBX trunking essentials (NEW — not in prior docs)

These bite the moment session-core points at a real Asterisk/FreeSWITCH **trunk** (vs LAN extension) or any cloud SIP trunk.

| # | Item | RFC | Status | Files |
|---|------|-----|--------|-------|
| B1 | P-Asserted-Identity / P-Preferred-Identity | RFC 3325 | ✅ end-to-end | sip-core types + parser + builder ext ✅, dialog-core `make_call_with_extra_headers[_for_session]` + `send_initial_invite_with_extra_headers` ✅, session-core `Config::pai_uri` + `UnifiedCoordinator::make_call_with_pai` + `SessionState.pai_uri` + `Action::SendINVITE` routing ✅, inbound PAI on `IncomingCallInfo.p_asserted_identity` ✅. Wire-level multi-binary integration test still TODO (call this `tests/pai_integration.rs`). |
| B2 | SDES-SRTP — full end-to-end (SDP builder + parser + key exchange + transport encrypt/decrypt) | RFC 4568, RFC 3711 | ✅ | sip-core `CryptoSuite` + `CryptoAttribute` types/parser/builder; session-core `SrtpNegotiator` + `Config::offer_srtp` / `srtp_required` / `srtp_offered_suites`; rtp-core `UdpRtpTransport::set_srtp_contexts` wraps send/receive; media-core `install_srtp_contexts`. See `crates/STEP_2B_SRTP_INTEGRATION_PLAN.md` for the four-phase landing log. |
| B3 | Service-Route processing on REGISTER 200 | RFC 3608 | ✅ core + cache | sip-core: new `ServiceRoute` type + parser + builder at `crates/sip-core/src/types/service_route.rs` / `crates/sip-core/src/builder/headers/service_route.rs`; `HeaderName::ServiceRoute` + `TypedHeader::ServiceRoute` wired across `header_name.rs` / `typed_header.rs` / `request.rs` / `response.rs`. dialog-core: `DialogManager::service_route_by_aor` cache (same pattern as A3 `nat_discovered_addr`); `extract_service_route` + `record_service_route_from_response` in `crates/dialog-core/src/protocol/response_handler.rs` called from `handle_response_message` on every 2xx REGISTER response (CSeq-method gated); exposed via `UnifiedDialogApi::service_route_for_aor(aor)`. **Preload** of cached Service-Route into outbound dialog `route_set` is a follow-up (consumer: session-core media adapter / dialog creation path) — the cache is the primitive that unblocks it. |
| B4 | DTMF RTP events end-to-end | RFC 4733 | ⬜ | `media-core/src/codec/audio/mod.rs:26`, transmitter/receiver wiring |
| B5 | Compact header forms on receive | RFC 3261 §7.3.3 + IANA later-RFC additions | ✅ | `sip-core/src/types/headers/header_name.rs:232-305` — `FromStr` maps all RFC 3261 compact forms (`i/m/e/l/c/f/s/k/t/v`) plus later-RFC additions (`o` Event, `u` Allow-Events, `r` Refer-To, `b` Referred-By, `x` Session-Expires). Case-insensitive. Parser dispatch goes through this by default. |

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
| **1 — make it talk to real PBXs** ✅ | A1 (TLS) ✅ + A2 (TCP flip) ✅ + A3 (Contact rewrite) ✅ + B1 (P-Asserted-Identity) ✅ + B2 (SDES-SRTP SDP) ✅ | **Shipped 2026-04-22.** Asterisk/FreeSWITCH with `srtp` and trunk identity; Twilio/Vonage/Bandwidth on TLS+SRTP+PAI; UAs behind NAT register correctly via Contact rewrite. |
| **2 — production-NAT and registration robustness** | A4 (SRV/NAPTR) + A5 (RFC 5626 SIP Outbound) + B3 (Service-Route) + B4 (RFC 4733 DTMF) + ~~B5 (compact headers RX)~~ ✅ | Long-duration registrations through SBCs survive; carrier geo-failover; in-band DTMF |
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
| 2026-04-20 | A2 ✅ | TCP enable + URI-aware transport selection. `MultiplexedTransport` (`crates/dialog-core/src/transaction/transport/multiplexed.rs`) wraps the per-flavour registry; `TransactionManager::with_transport_manager` installs it as the single `Arc<dyn Transport>`, dispatching by Request-URI scheme + `;transport=` parameter (RFC 3261 §18.1.1, §19.1.5, §26.2). `resolve_uri_to_socketaddr` uses typed `Uri` fields + `sips:`→5061 default + async DNS. 10 multiplexer unit tests (6 selection + 4 dispatch). dialog-core 184/184 + session-core 18/18 + audio_roundtrip still green. |
| 2026-04-21 | A1 ✅ | TLS Steps 1B + 1C — TLS client connector (replaced placeholder at `crates/sip-transport/src/transport/tls/mod.rs` with ~600-LOC working server+client impl using `tokio_rustls::TlsConnector`; `rustls-native-certs` + `webpki-roots` fallback + optional extra CA + dev-only insecure-skip; `bind_with_client_config(addr, cert, key, event_tx, client_cfg)`). `TransportManager` enables TLS via `add_tcp_transport(addr, true)` with `bind_addr.port() + 1` (RFC 3261 5060→5061 convention). session-core `Config::tls_cert_path` / `tls_key_path` / `tls_extra_ca_path` / `tls_insecure_skip_verify`; auto-flips `enable_tls` when cert+key both set. Critical response-routing fix: new `Transport::has_connection_to(addr)` method; `MultiplexedTransport::pick_transport` for Responses probes connection-oriented transports for a live connection before falling back to UDP (RFC 3261 §17.2 / §18.2.2). Tests: `crates/sip-transport/tests/tls_handshake_test.rs` (handshake-then-REGISTER round-trip with `rcgen` self-signed cert + default-validation rejects self-signed) + `crates/session-core/tests/tls_call_integration.rs` (real `sips:` call between two `UnifiedCoordinator`s). |
| 2026-04-21 | B1 wire-level test gap closed | Inbound PAI on `IncomingCallInfo.p_asserted_identity` exercised end-to-end through TLS test path; PAI surfaces correctly on the receiving side. |
| 2026-04-22 | B2 ✅ | RFC 4568 SDES-SRTP — full end-to-end across four phases (2B.1 SDP refactor + `SrtpNegotiator`; 2B.2 wire encryption via `UdpRtpTransport::set_srtp_contexts` + media-core `install_srtp_contexts`; 2B.3 in-process `srtp_call_integration` test; 2B.4 public-API exposure). sip-core CRLF fix + `a=crypto:` parser. session-core media adapter refactored from format-strings to `SdpBuilder` + `SdpSession::from_str` (D11/D12). Two-key asymmetric SDES (D4); RFC 3711 §3.4 silent-drop on auth failure (D7); `Config::offer_srtp` / `srtp_required` / `srtp_offered_suites` exposed publicly. Tests: sip-core 2004 + rtp-core udp 12 + session-core lib 30 + audio_roundtrip + tls_call + srtp_call all green. See `crates/STEP_2B_SRTP_INTEGRATION_PLAN.md`. |
| 2026-04-22 | A3 ✅ | RFC 3581 NAT discovery + RFC 5626 §5 Contact rewrite. `DialogManager::nat_discovered_addr` cache populated by `ResponseHandler::handle_response_message` from typed `Param::Received` + `Param::Rport(Some(_))` on every inbound response (no-op suppression when discovered == local). `UnifiedDialogApi::discovered_public_addr()` exposes the cache. `DialogAdapter::send_register` pre-rewrites Contact host:port via `rewrite_contact_host` helper (preserves scheme + user-part + URI params). 5 NAT-discovery extraction tests + 5 Contact-rewrite tests. dialog-core 189/189 + session-core lib 35/35 + all integration suites green. |
| 2026-04-22 | Sprint 1 ✅ | A1 (TLS) + A2 (TCP+URI selection) + A3 (NAT discovery + Contact rewrite) + B1 (PAI) + B2 (SDES-SRTP) all shipped. session-core is now production-credible against TLS-required cloud carriers (Twilio/Vonage/Bandwidth/Teams Direct Routing), modern Asterisk/FreeSWITCH with `srtp=mandatory`, PBX trunks requiring PAI, and UAs behind NAT registering through carrier SBCs. Outstanding (separate workstreams): A4–A7 (SRV/NAPTR, SIP Outbound, STUN, digest enhancements), B3–B5 (Service-Route, RFC 4733 DTMF, compact headers RX), Tier C (media polish), Tier D (WebRTC), Tier E (advanced/aesthetic). |
| 2026-04-22 | B5 ✅ | Sprint 2 audit found B5 already shipped — `sip-core/src/types/headers/header_name.rs:232-305` `FromStr` maps all RFC 3261 §7.3.3 compact forms (`i/m/e/l/c/f/s/k/t/v`) + later-RFC additions (`o` RFC 6665 Event, `r` RFC 3515 Refer-To, `b` RFC 3892 Referred-By, `x` RFC 4028 Session-Expires) case-insensitively. Fixed one miswiring: `u` was mapped to `Unsupported`; IANA registry + RFC 6665 §7.2 assign `u` to `Allow-Events`. Added `test_compact_header_forms_rfc3261_and_later` covering all compact forms both cases. sip-core 2005/2005 + dialog-core 189/189 green. |
| 2026-04-22 | B3 ✅ core | RFC 3608 Service-Route — full header stack in sip-core (new `ServiceRoute` type at `crates/sip-core/src/types/service_route.rs`, builder ext at `crates/sip-core/src/builder/headers/service_route.rs`, `HeaderName::ServiceRoute` + `TypedHeader::ServiceRoute` wiring including parser dispatch, Display impl, and appendable-header classification in request/response builders) with 13 new unit tests. dialog-core `DialogManager::service_route_by_aor` cache populated by `record_service_route_from_response` from `handle_response_message` (gated on CSeq method = REGISTER + 2xx status + To URI as AoR key); `UnifiedDialogApi::service_route_for_aor` exposes it. 5 extraction unit tests (`extracts_service_route_on_register_200`, `returns_empty_vec_when_register_200_has_no_service_route`, `ignores_non_2xx_register_responses`, `ignores_non_register_responses`, `concatenates_multiple_service_route_headers`). sip-core 2018/2018 + dialog-core 194/194 + session-core 35/35 green. Preload into outbound dialog route_set is a follow-up. |
| 2026-04-22 | A4 ✅ core | RFC 3263 SRV + A/AAAA resolution. `hickory-resolver 0.24` added as workspace dep + dialog-core dep. New `crates/dialog-core/src/dialog/dns_resolver.rs` implementing RFC 3263 §4: (1) IP literal short-circuit, (2) hostname-with-explicit-port skips SRV, (3) hostname-without-port does `_service._proto.host` SRV (`_sip._udp`/`_sip._tcp`/`_sips._tcp` based on URI transport/scheme per RFC 3263 §4.2 + RFC 7118 for ws/wss) with RFC 2782 weighted selection within the lowest-priority group, (4) A/AAAA fallback. New `ResolvedTarget { addr, transport }` surface; back-compat `resolve_uri_to_socketaddr` wrapper preserved so existing A3-era callers (`dialog-core/src/api/unified.rs::send_register`, `transaction/manager/handlers.rs::determine_ack_destination`, `dialog/dialog_impl.rs::remote_target_addr`) compile unchanged. Process-wide `TokioAsyncResolver::tokio_from_system_conf()` cached in a `OnceCell`; logs a one-time warning and degrades to `None` if the system resolver can't be constructed. 15 new unit tests covering SRV service-name derivation across schemes/transports, RFC 2782 weighted selection (zero-weight, weighted, priority gating), scheme-to-default-port, and IP-literal short-circuits. NAPTR deferred — `;transport=` + scheme selection covers the common carrier case, add if a deployment needs NAPTR-based transport discovery. dialog-core 209/209 + session-core lib 35/35 + audio_roundtrip_integration green. |
