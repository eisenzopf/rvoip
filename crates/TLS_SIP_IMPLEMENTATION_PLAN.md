# TLS-for-SIP Implementation Plan

## Context

Sprint 1 of the General-Purpose SIP Client roadmap targets carrier
production-readiness. **TLS (RFC 3261 §26.2 / RFC 5630)** is the single
highest-leverage carrier blocker — every major cloud SIP provider
(Twilio, Vonage, Bandwidth, Telnyx, Microsoft Teams Direct Routing)
requires TLS for production-tier traffic. Without it, session-core can
only reach LAN-attached PBXs.

You asked: *"is TCP required for TLS?"* — yes. RFC 3261 §26.2 mandates
TLS-over-TCP. So **TCP enable is a hard prerequisite for TLS**.

You also asked whether the heaviest changes will land in **sip-transport**
(transport layer) or whether **rtp-core** + **media-core** also need
significant work. The honest answer from the audit:

- **For TLS-signalling alone**: ~80% of the work lives in `sip-transport`.
  rtp-core and media-core need **zero** changes — they're independent
  concerns.
- **For "carrier-realistic TLS"** (TLS-signalling + encrypted media via
  SDES-SRTP): an additional ~50 LOC in sip-core's SDP builder + wiring
  the existing SDES client (`crates/rtp-core/src/api/client/security/srtp/sdes.rs`,
  ~95% complete already) into media-core. **No new crypto work** —
  rtp-core's SRTP core (AES-CM/F8/HMAC-SHA1) is ready.
- **DTLS-SRTP** (WebRTC-style): the framework in `crates/rtp-core/src/dtls/`
  is scaffolding only — `dtls/mod.rs:84-86` calls `unimplemented!()`. That
  is a 2-3 week task in its own right, **not** required by Twilio /
  Vonage / Bandwidth (they take SDES-SRTP), but **is** required for
  browser-WebRTC interop. **Out of scope** for this plan.

The plan below covers the two scope tiers — a minimal "TLS-only" track
and an extended "TLS + SDES-SRTP" track.

---

## Source-of-truth audit findings

### sip-transport (`crates/sip-transport`)

**Two conflicting TLS implementations:**
- `crates/sip-transport/src/tls.rs` (254 LOC) — server-side complete
  (rustls `TlsAcceptor`, handshake, cert/key loading, message
  framing); implements `Transport` trait in full. Client `connect()`
  at line 254 returns `Error::NotImplemented`.
- `crates/sip-transport/src/transport/tls/mod.rs` (90 LOC) — placeholder.
  `send_message()` at line 66 returns `NotImplemented`. **Delete.**

**TCP is production-ready** at `crates/sip-transport/src/transport/tcp/`:
255 LOC, RFC 3261 §18.3 framing, server + client connection pool with
300s idle timeout, full unit tests at `connection.rs:279-432`.

**WebSocket is server-complete** at `crates/sip-transport/src/transport/ws/`,
client `connect_to()` stubbed at line 229. (Out of scope for this plan;
Tier D in the broader roadmap.)

**URI-aware transport selection is missing.** Both `TransportManager`s
(sip-transport's own at `manager/mod.rs:32` and dialog-core's wrapper at
`crates/dialog-core/src/transaction/transport/mod.rs:339`) do
address-based lookup only. The latter hardcodes UDP. SIP `Uri` already
exposes `transport()` (`crates/sip-core/src/types/uri.rs:675-680`) and
`scheme()` (line 370 — `Sip` / `Sips`), so the data is available; the
selection logic just doesn't read it.

**Cargo deps version drift:**
- `crates/sip-transport/Cargo.toml`: `rustls = "0.21"`, `tokio-rustls = "0.24"`, `rustls-pemfile = "1.0"`
- workspace root: `rustls = "0.23"`, `rustls-pemfile = "2.0"`, `webpki-roots = "0.25"`

Bump sip-transport to match workspace; add `webpki-roots` for system
root CAs. No `rustls-native-certs` today — add it for system root cert
loading on the client side.

### dialog-core (`crates/dialog-core`)

- `crates/dialog-core/src/transaction/transport/mod.rs:256` —
  `add_tcp_transport(addr, tls=true)` explicitly rejects TLS:
  `"TLS transport is not fully implemented in this function"`. Replace
  with a call to a new `TlsTransport::bind_with_config(addr, cert, key)`.
- `crates/dialog-core/src/dialog/dialog_utils.rs:134-174` —
  `resolve_uri_to_socketaddr` is naive A/AAAA only, no SRV/NAPTR (RFC
  3263 deferred to roadmap P3), no TLS port default (`5061`). Fix the
  port-default for `sips:` URIs as part of this work.
- `make_call → make_call_inner → core.send_request → send_request_in_dialog`
  is the call path for outbound INVITE. Today it builds the request and
  sends via the (UDP-hardcoded) `get_transport_for_destination`. The
  transport-selection seam needs to learn the URI.

### session-core (`crates/session-core`)

Hardcoded transport flags at `crates/session-core/src/api/unified.rs:822-824`:
```
enable_udp: true,
enable_tcp: false,
enable_ws:  false,
enable_tls: false,
```
`Config` exposes no `tls_cert_path` / `tls_key_path` / `transport_preference`
fields.

### sip-core (`crates/sip-core`)

**Zero changes needed for TLS-only.** `Uri::transport()` and
`Uri::scheme()` already expose what selection needs. The `sips:` scheme
parses correctly today.

**For SDES-SRTP** (extended scope): need an SDP builder method for
`a=crypto:1 AES_CM_128_HMAC_SHA1_80 inline:<base64>`. Pattern already
exists for `a=fingerprint:` / `a=setup:` in
`crates/sip-core/src/sdp/builder.rs:54-63`. Adding `crypto()` mirrors
that — ~30 LOC plus tests.

### rtp-core (`crates/rtp-core`)

**Zero changes needed for TLS-only.**

**For SDES-SRTP**: the SDES client at
`crates/rtp-core/src/api/client/security/srtp/sdes.rs` (490 LOC) is
~95% complete RFC 4568 — parses `a=crypto:`, runs the offer/answer key
exchange, produces an `SrtpContext`. Integration test
`crates/rtp-core/src/srtp/integration_tests.rs::test_srtp_with_sdes_key_exchange`
exercises the full SDES → key exchange → encrypt/decrypt loop. Gap:
nothing in media-core wires it up.

### media-core (`crates/media-core`)

**Zero changes needed for TLS-only.**

**For SDES-SRTP**: `MediaSessionController`
(`crates/media-core/src/relay/controller/mod.rs:90`) creates RTP
sessions via `RtpSessionWrapper` (line 94) using plain `RtpSession` /
`RtpSessionConfig`. No SRTP-aware variant today (grep for `srtp_enabled`
/ `with_srtp` / `crypto_context` returned zero matches). Gap: a
security-aware constructor that takes an `SrtpContext` from the SDES
client and wraps the RTP transport with SRTP encryption.

---

## Scope decision (locked)

**Tier 1 + Tier 2** — TLS-signalling **and** SDES-SRTP for encrypted
media. Total estimated effort: 2-3 weeks. This unblocks Twilio /
Vonage / Bandwidth production tier and modern Asterisk / FreeSWITCH
with `srtp=mandatory`, which is the realistic carrier target for
session-core as a general-purpose SIP client.

DTLS-SRTP (Tier D in the broader roadmap) remains out of scope —
required only for browser/WebRTC interop, separately tracked.

---

## Tier 1 — TLS signalling

Estimated effort: 3-5 days.

### Step 1A — TCP enable + URI-aware selection (1-2 days)

Prerequisite for TLS. TCP itself is production-ready; the gap is
selection.

**sip-transport changes:**
- `crates/sip-transport/src/manager/mod.rs:146` — add
  `send_message_to_uri(message: Message, uri: &Uri) -> Result<()>` that
  reads `uri.scheme()` + `uri.transport()` + destination size to choose
  UDP / TCP / TLS, then calls existing `send_message(message, addr)`.
- Honour RFC 3261 §18.1.1 MTU-based fallback (UDP request > ~1300 bytes
  → switch to TCP) inside `send_message_to_uri`.

**dialog-core changes:**
- `crates/dialog-core/src/transaction/transport/mod.rs:339` —
  `get_transport_for_destination` rewritten to delegate to
  sip-transport's URI-aware selector (architectural correction the user
  flagged earlier — transport selection belongs in sip-transport).
- `crates/dialog-core/src/manager/transaction_integration.rs:175-176` —
  thread `dialog.remote_target` (the `Uri`) alongside `destination`
  through the send path.
- `crates/dialog-core/src/dialog/dialog_utils.rs:134-174` — fix port
  default to 5061 when the URI scheme is `sips:`.

**session-core changes:**
- `crates/session-core/src/api/unified.rs:822-823` — flip
  `enable_tcp: true`. (TLS still false in this step.)

**Verification:**
- New `crates/sip-transport/tests/tcp_uri_selection_test.rs` — bind a
  TCP server, send `sip:bob@127.0.0.1:5060;transport=tcp`, assert TCP
  connection established and message delivered.
- Re-run `crates/session-core/tests/audio_roundtrip_integration.rs` over
  TCP (env var or test variant) — assert the full media path still
  works end-to-end.

### Step 1B — TLS client connector (1-2 days)

**sip-transport changes:**
- **Delete** `crates/sip-transport/src/transport/tls/mod.rs` (the
  90-LOC stub) and any module declaration that imports it.
- **Implement** `TlsTransport::connect()` at
  `crates/sip-transport/src/tls.rs:254`. Mirror
  `TcpConnection::connect()` at
  `crates/sip-transport/src/transport/tcp/connection.rs:33`:
  1. Build `rustls::ClientConfig` once per `TlsTransport` (root store +
     no client auth).
  2. Wrap in `tokio_rustls::TlsConnector`.
  3. Derive `ServerName` from the URI host (or peer `SocketAddr` IP if
     literal).
  4. `TcpStream::connect(addr).await` → `connector.connect(server_name,
     stream).await` → `Arc<TlsStream<TcpStream>>`.
  5. Insert into the existing `connections` HashMap (line 84) so
     subsequent sends reuse the connection.
  6. Spawn a reader task to forward `TransportEvent::MessageReceived`
     for incoming bytes on this connection (mirrors TCP's per-connection
     reader).
- Refactor `tls.rs`'s `connections` field (currently `Vec<(SocketAddr,
  mpsc::Sender<Bytes>)>` — list-based, no eviction) to a HashMap keyed
  on `SocketAddr` with idle-timeout eviction. Reuse the
  `crates/sip-transport/src/transport/tcp/pool.rs` pattern (60s cleanup
  interval, 300s idle timeout).
- Bump deps: `rustls = "0.23"`, `rustls-pemfile = "2.0"`, add
  `webpki-roots = "0.25"` and `rustls-native-certs = "0.7"`. Either-or
  for cert source: prefer `rustls-native-certs` (system root store)
  with `webpki-roots` as fallback.

**Cert verification policy** (default):
- System root CAs via `rustls-native-certs`.
- Optional `Config::tls_extra_ca_path: Option<PathBuf>` for custom CA
  bundle (enterprise PKI).
- Optional `Config::tls_insecure_skip_verify: bool` (default `false`,
  dev-only) for self-signed cert acceptance during development. This
  must require an explicit feature flag or env var to enable in
  production builds.
- TLS 1.2 + 1.3 (rustls 0.23 default). RFC 5630 §3.2 cipher floor is
  satisfied — rustls dropped weaker ciphers years ago.

**Verification:**
- New `crates/sip-transport/tests/tls_handshake_test.rs` — generate
  self-signed cert in-test (use `rcgen`), bind TLS server, connect TLS
  client, assert SIP message round-trip. Mirror the existing TCP test
  shape.

### Step 1C — Dialog-core + session-core TLS wiring (½ day)

**dialog-core changes:**
- `crates/dialog-core/src/transaction/transport/mod.rs:256` — replace
  the `"not implemented"` rejection. New flow:
  ```rust
  if tls {
      let (transport, rx) = TlsTransport::bind_with_config(
          bind_addr, cert_path, key_path, /* extra_ca = */ None
      ).await?;
      // store + spawn event forwarder same as TCP path
  }
  ```
- The TCP/TLS branches share most code. Extract a helper
  `add_stream_transport(bind_addr, mode: StreamMode)` where
  `StreamMode` is `Tcp` or `Tls { cert, key }`.

**session-core changes:**
- `crates/session-core/src/api/unified.rs:26` — extend `Config`:
  ```rust
  pub tls_cert_path: Option<PathBuf>,
  pub tls_key_path: Option<PathBuf>,
  pub tls_extra_ca_path: Option<PathBuf>,
  pub tls_insecure_skip_verify: bool, // dev only
  ```
  Also add to both constructors (`local`, `on`) with `None` / `false`
  defaults.
- `crates/session-core/src/api/unified.rs:822-824` — flip
  `enable_tls: true` when `tls_cert_path` + `tls_key_path` are both
  `Some` (auto-enable based on config presence — a server that doesn't
  provide certs simply doesn't accept TLS).
- Pass `tls_cert_path` / `tls_key_path` / `tls_extra_ca_path` /
  `tls_insecure_skip_verify` into `TransportManagerConfig`.
- `TransportManagerConfig` itself gains `tls_extra_ca_path` and
  `tls_insecure_skip_verify` fields (currently only has cert/key) at
  `crates/dialog-core/src/transaction/transport/mod.rs:34-37`.

**Verification:**
- New `crates/session-core/tests/tls_call_integration.rs` — multi-binary
  test mirroring `audio_roundtrip_integration` but with both peers
  configured for TLS; assert call setup + media works end-to-end.
- Manual test against a public TLS SIP echo (or a containerised
  Asterisk with TLS configured). Optional but the only way to catch
  cert-validation quirks before users do.

---

## Tier 2 — SDES-SRTP for encrypted media

Estimated effort: +1-2 weeks beyond Tier 1.

### Step 2A — SDP `a=crypto:` builder (½ day)

**sip-core changes:**
- `crates/sip-core/src/sdp/builder.rs` — add `crypto(tag, suite,
  key_inline)` method on `SdpBuilder`. Mirror the existing
  `fingerprint()` / `setup()` methods at lines 54-63.
- New typed enum `CryptoSuite { AesCm128HmacSha1_80, AesCm128HmacSha1_32,
  AesCm256HmacSha1_80, ... }` covering the suites the existing SDES
  client at `crates/rtp-core/src/api/client/security/srtp/sdes.rs:46-58`
  already supports.
- Unit tests for builder round-trip (build → serialise → parse → assert
  identity).

### Step 2B — Wire SDES into media-core (3-5 days)

**media-core changes:**
- `crates/media-core/src/relay/controller/mod.rs` — extend
  `MediaSessionController::create_rtp_session` (or add
  `create_secure_rtp_session`) to optionally accept an `SrtpContext`
  from the SDES client.
- `RtpSessionWrapper` (line 94) — wrap outgoing/incoming packets in
  SRTP encrypt/decrypt when the context is set. The actual SRTP crypto
  is already in `crates/rtp-core/src/srtp/`.

**session-core changes:**
- `Config::offer_srtp: bool` — when true, the SDP builder emits
  `a=crypto:` lines and the action handler routes incoming `a=crypto:`
  through the SDES client to derive the session keys.
- Hook into the existing SDP-construction path in session-core's media
  adapter.

**Verification:**
- Extend `crates/rtp-core/src/srtp/integration_tests.rs` — already has
  SDES key-exchange test. Add a media-core-level test that builds an
  SDP offer with `a=crypto:`, runs SDES handshake, sends RTP through
  the wrapped session, and asserts the wire bytes are encrypted (no
  readable PCMU header).
- New `crates/session-core/tests/srtp_call_integration.rs` — multi-binary
  TLS+SRTP call, capture wire RTP, assert payload is encrypted.

### Step 2C — Out of scope: DTLS-SRTP

Documented for completeness — `crates/rtp-core/src/dtls/` is
scaffolding only. `dtls/mod.rs:84-86` calls `unimplemented!()`. The
~3000 LOC framework would need 2-3 weeks of focused work to complete
the handshake. Required for browser/WebRTC interop, **not** required
for SIP-trunk carriers (they accept SDES-SRTP). Track as Tier D in the
broader `session-core/docs/GENERAL_PURPOSE_SIP_CLIENT_PLAN.md`
roadmap.

---

## Critical files to touch (by step)

### Step 1A — TCP + URI selection
- `crates/sip-transport/src/manager/mod.rs:146` — new
  `send_message_to_uri`
- `crates/dialog-core/src/transaction/transport/mod.rs:339` — delegate
  to sip-transport
- `crates/dialog-core/src/manager/transaction_integration.rs:175-176` —
  thread `Uri`
- `crates/dialog-core/src/dialog/dialog_utils.rs:134-174` — `sips:`
  port default 5061
- `crates/session-core/src/api/unified.rs:822` — `enable_tcp: true`

### Step 1B — TLS client
- `crates/sip-transport/src/tls.rs:254` — implement `connect()`
- `crates/sip-transport/src/tls.rs:84` — refactor `connections` to
  pooled HashMap
- **Delete** `crates/sip-transport/src/transport/tls/mod.rs`
- `crates/sip-transport/Cargo.toml` — bump rustls to 0.23, add
  webpki-roots + rustls-native-certs

### Step 1C — Wiring
- `crates/dialog-core/src/transaction/transport/mod.rs:256` — replace
  TLS rejection
- `crates/dialog-core/src/transaction/transport/mod.rs:34-37` —
  `TransportManagerConfig` gains `tls_extra_ca_path`,
  `tls_insecure_skip_verify`
- `crates/session-core/src/api/unified.rs:26` — `Config` TLS fields
- `crates/session-core/src/api/unified.rs:824` — auto-flip
  `enable_tls`

### Step 2A — SDP builder
- `crates/sip-core/src/sdp/builder.rs:54-63` — add `crypto()` method
- New `CryptoSuite` enum

### Step 2B — Media-core SDES integration
- `crates/media-core/src/relay/controller/mod.rs:90` — secure RTP
  session variant
- `crates/rtp-core/src/api/client/security/srtp/sdes.rs` — already
  ~95% done, just consume from media-core
- session-core media adapter — emit/parse `a=crypto:` in the SDP it
  builds

---

## Verification plan (per-tier end-to-end)

### Tier 1 acceptance
1. `cargo test -p rvoip-sip-transport tls_handshake_test` — TLS handshake
   between in-process server + client.
2. `cargo test -p rvoip-session-core tls_call_integration` — full SIP
   call setup over TLS, end-to-end.
3. `bash crates/session-core/examples/run_all.sh` — existing example
   suite still passes (regression for UDP path).
4. Manual: dial a TLS-enabled Asterisk container with `sips:` URI,
   assert `Call established` event fires.

### Tier 2 acceptance
1. `cargo test -p rvoip-sip-core sdp_crypto_builder` — SDP builder
   round-trip for `a=crypto:`.
2. `cargo test -p rvoip-rtp-core srtp_with_sdes_key_exchange` — already
   exists, confirm still passes.
3. `cargo test -p rvoip-session-core srtp_call_integration` — TLS+SRTP
   call, assert wire RTP is AES-encrypted (no readable PCMU header in
   tcpdump-style capture).
4. Manual: dial a Twilio dev account or a containerised FreeSWITCH with
   `transport=tls` + `srtp=mandatory`, assert audio works both ways.

---

## Execution order (locked)

| # | Step | Crate(s) | Effort |
|---|------|----------|--------|
| 1 | 1A — TCP enable + URI-aware selection | sip-transport, dialog-core, session-core | 1-2 days |
| 2 | 1B — TLS client connector | sip-transport | 1-2 days |
| 3 | 1C — TLS wiring | dialog-core, session-core | ½ day |
| 4 | 2A — SDP `a=crypto:` builder | sip-core | ½ day |
| 5 | 2B — Media-core SDES integration | media-core, rtp-core (consume), session-core | 3-5 days |

Total: ~2-3 weeks of focused implementation work, with verification
gates between each step.

## Progress log

| Date | Step | Notes |
|------|------|-------|
| 2026-04-20 | doc | Initial plan written from audit of `sip-transport`, `dialog-core`, `session-core`, `sip-core`, `rtp-core`, `media-core`. Scope locked to Tier 1 + Tier 2 (TLS + SDES-SRTP). |
| 2026-04-20 | 1A ✅ | TCP enabled + URI-aware transport selection landed. `MultiplexedTransport` (`crates/dialog-core/src/transaction/transport/multiplexed.rs`) wraps the per-flavour registry; `TransactionManager::with_transport_manager` installs it as the single `Arc<dyn Transport>`, dispatching outbound requests by Request-URI scheme + `;transport=` parameter (RFC 3261 §18.1.1, §19.1.5, §26.2). `resolve_uri_to_socketaddr` now uses typed `Uri` fields + `sips:`→5061 default + async DNS. 10 new unit tests (6 selection + 4 dispatch); 184/184 dialog-core lib tests pass; all dialog-core integration tests pass; session-core `audio_roundtrip_integration` still green. **Architectural note**: the per-flavour `transports_by_flavour` collapses multi-bind setups; revisit when multi-homed binds are needed. |
| 2026-04-21 | 1B ✅ | TLS client connector landed in `crates/sip-transport/src/transport/tls/mod.rs` (~600 LOC). Replaced the 90-LOC placeholder; deleted the dead duplicate at `crates/sip-transport/src/tls.rs`. `TlsTransport::connect[_with_server_name]` performs the handshake via `tokio_rustls::TlsConnector` (system root CAs via `rustls-native-certs`, fallback `webpki-roots`, optional extra CA bundle, dev-only `insecure_skip_verify`). Auto-dial on `send_message`. Generic `handle_connection<S: AsyncRead + AsyncWrite>` services both server- and client-side streams. Buffered read loop with RFC 3261 §18.3 Content-Length framing; `send_message` switched to `message.to_bytes()` for RFC 3261 §7.2 wire format. New `tests/tls_handshake_test.rs` — handshake-then-REGISTER round-trip with `rcgen` self-signed cert + a default-validation-rejects-self-signed regression. Both pass. dialog-core 184/184 + session-core 18/18 lib tests still green. |
| 2026-04-21 | 1C ✅ | TLS wired through dialog-core + session-core end-to-end. Replaced the TLS rejection in `TransportManager::add_tcp_transport(addr, true)` with a real `TlsTransport::bind_with_client_config` call. `TransportManagerConfig` gained `tls_extra_ca_path` + `tls_insecure_skip_verify`. TLS bind port auto-derived as TCP port +1 (RFC 3261 5060→5061 convention) so TCP and TLS don't fight at the OS level. session-core `Config` gained `tls_cert_path` / `tls_key_path` / `tls_extra_ca_path` / `tls_insecure_skip_verify`; auto-flips `enable_tls` when cert+key are both set. **Critical response-routing fix**: new `Transport::has_connection_to(addr)` method (default false; `TlsTransport` implements via try-lock); `MultiplexedTransport::pick_transport` for Responses probes connection-oriented transports for a live connection to the destination before falling back to UDP — RFC 3261 §17.2 / §18.2.2 (responses must reuse inbound transport). Without this fix, sips: call setup completed the INVITE→200 OK *handshake* but the 200 OK was sent back via UDP and lost. New `tests/tls_call_integration.rs` — self-signed cert, two `UnifiedCoordinator`s, real `sips:` call, asserts `CallAnswered` — passes. All regression suites still green. **Tier 1 complete**: TLS-signalling works end-to-end. |
| 2026-04-21 | 2A ✅ | SDP `a=crypto:` builder landed in `crates/sip-core` (RFC 4568 §9.1). New `CryptoSuite` enum (`AesCm128HmacSha1_80` / `_32`, `AesCm256HmacSha1_80` / `_32`) with `Display` + `FromStr`; new `CryptoAttribute` struct (tag, suite, base64-encoded inline key, optional lifetime + MKI + session-params) with wire-format `Display`; new `ParsedAttribute::Crypto` variant. Builder methods: `SdpBuilder::crypto(tag, suite, key)` + `crypto_attribute(...)` for session-level placement, plus matching `MediaBuilder::crypto` + `crypto_attribute` for the more common per-m=-section placement. 7 unit tests (round-trip, default + full Display, session-vs-media routing, end-to-end serialisation). sip-core 2004/2004 lib tests pass (was 1997 — +7). Workspace builds clean. **Step 2B** (wire the existing rtp-core SDES client into media-core via the new builder) is next. |
