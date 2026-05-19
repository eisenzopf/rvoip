# STIR/SHAKEN + Proxy/SBC + Transport Roadmap

**Status:** In progress (2026-05-19). Phases **1, 2, 3, 4, 5, 8, 8.5, 10 shipped**; Phases 6, 7 outstanding.
**Predecessors:** `SIP_API_DESIGN_2.md`, `SIP_API_DESIGN_2_GAP_PLAN.md`, `SIP_API_DESIGN_2_REMAINING_WORK.md` (R1–R6 all closed 2026-05-14)

## Context

`SIP_API_DESIGN_2` shipped a developer-facing API for four use-case classes (endpoint, gateway, call-center, SBC/B2BUA) and explicitly deferred or excluded several adjacent capabilities. This plan inventories what is **not yet wired** for STIR/SHAKEN and proxy/SBC deployments and proposes a phased roadmap to close the gaps.

**Audit findings (verified 2026-05-18 across `rvoip-sip`, `rvoip-sip-dialog`, `rvoip-sip-transport`, `infra-common`):**

**STIR/SHAKEN — mostly missing.** Foundations exist (raw-bytes preservation end-to-end via `TransportEvent.raw_bytes: Option<Arc<Bytes>>`, `TransactionManager::pending_inbound_bytes`, `Arc<Bytes>` on every cross-crate `IncomingCall`/`IncomingRegister`/response variant; `Transport::send_message_raw` for verbatim outbound; custom-header pass-through). What's missing: typed `Identity` header (RFC 8224), PASSporT (RFC 8225) JWT/JWS parsing, certificate fetch from `info=` URL, SHAKEN trust-anchor config, no `jsonwebtoken`/`jose` dependency anywhere, no signing or verification hook.

**Stateless proxy (RFC 3261 §16) — explicitly out of scope** per `SIP_API_DESIGN_2_GAP_PLAN.md:822`. No Via push/pop, no branch-cookie utilities, no loop detection beyond the `LoopDetected` status constant.

**Stateful proxy — partial.** Transaction state, Max-Forwards decrement, Route-set handling all exist. No API for owning a server transaction + downstream client transactions on one request; no forking, no 3xx redirect-set, no Timer C (RFC 3261 §16.8).

**SBC — partial.** B2BUA bridge (`examples/sip_b2bua.rs`, `server/bridge.rs`), Contact rewrite (`dialog_adapter.rs:2611`), outbound-proxy Route prepend (`:2640`), rport extract (`:2091`), media bridging via media-core. Missing: topology hiding helpers (Record-Route strip below self, internal Via removal).

**Transport — partial.** Implemented: multi-transport simultaneous bind, TCP/TLS pool with RFC 5923 reuse, RFC 5626 CRLF keep-alive, mutual-TLS client auth, raw-byte preservation, WS server. **Missing:** RFC 3263 NAPTR/SRV (no DNS), RFC 3581 server-side `received=`/`rport=` restamping on responses (extracted on ingress, not echoed on egress), multi-homed source-address selection, WS client (`ws/mod.rs:248` returns NotImplemented), WSS TLS accept (`ws/listener.rs:81` TODO), MTU/size policy.

**Trust model decision (user-confirmed):** pluggable trait. `rvoip` defines `PASSporTSigner` / `PASSporTVerifier`; the application supplies key material and trust anchors. Library does NOT bundle SHAKEN STI-PA roots or any HSM driver.

## Architecture: trait surface and integration points

### Identity header type (`rvoip-sip-core`)

`HeaderName::Identity` variant + `IdentityHeader { jwt, info, alg, ppt, raw }` parser. Lives alongside `PAssertedIdentity` in `crates/rvoip-sip-core/src/types/headers/`. Parser is pure grammar; no JWT/crypto knowledge. Defer JWT parsing to the verifier.

### Pluggable traits (defined in `rvoip-sip-dialog`)

```rust
// rvoip-sip-dialog/src/manager/request_lifecycle.rs  (NEW — mirrors response_lifecycle.rs)
#[async_trait]
pub trait PASSporTSigner: Send + Sync {
    async fn sign(&self, claims: PassportClaims) -> Result<IdentityHeaderValue, SignerError>;
}

#[async_trait]
pub trait PASSporTVerifier: Send + Sync {
    async fn verify(
        &self,
        raw_bytes: &Bytes,
        identity: &IdentityHeader,
        request: &Request,
    ) -> VerificationOutcome;
}

pub enum VerificationOutcome {
    Valid { attest: Attestation, origid: Uuid, cert_chain: CertChain },
    Stale { iat_skew_secs: i64 },
    BadSig,
    BadChain(ChainError),
    ClaimMismatch { field: &'static str },
    NoIdentity,
}

pub enum VerificationPolicy { Annotate, RequireValid, StrictReject }
```

Traits live in `rvoip-sip-dialog`; no new dependencies pulled into `sip-core` or `sip-transport`.

### Reference impls (new sibling crate `rvoip-stir-shaken`)

Default implementations using `jsonwebtoken`, `x509-parser`, `webpki`, and `reqwest` for `info=` cert fetch. Opt-in dependency — applications without STIR/SHAKEN never pull crypto/HTTP deps. Mirrors the `rvoip-auth-core` separation.

### Hook attachment points

**Verify (inbound):** `crates/rvoip-sip-dialog/src/events/adapter.rs` ~lines 283-310 already calls `take_inbound_bytes(transaction_id)` to fill `raw_request` for `DialogToSessionEvent::IncomingCall`. Run verifier on the same `Arc<Bytes>` immediately after, attach `verification: Option<VerificationOutcome>` to the published event. Rejection (per `VerificationPolicy::RequireValid`) routes through `pre_send_response` to ship a 436/437/438/428 per RFC 8224 §6.2.2.

**Sign (outbound):** new `RequestLifecycle::pre_send_request(&mut Request, dest)` trait paralleling `ResponseLifecycle`. Called from `crates/rvoip-sip-dialog/src/manager/transaction/request_operations.rs` just before `transaction_manager.send_request`, after Via/Max-Forwards/Route stamping. Builds PASSporT claims from final headers, attaches typed `Identity` header.

**Configuration:** extend `crates/rvoip-sip-dialog/src/api/config.rs::DialogConfig` (line 183 area) with `signer: Option<Arc<dyn PASSporTSigner>>`, `verifier: Option<Arc<dyn PASSporTVerifier>>`, `verification_policy: VerificationPolicy`. Flow through existing `DialogManagerConfig` plumbing alongside `trace_redactor`.

**B2BUA re-sign on egress:** the outbound leg's `pre_send_request` fires naturally — no special SBC code path. Inbound Identity is verified and dropped; outbound builds fresh claims from the rewritten From/To.

**Cross-crate event additivity:** `infra-common::events::cross_crate.rs` `DialogToSessionEvent::IncomingCall` and friends gain `identity_verification: Option<IdentityVerificationStatus>` (simple enum to keep `infra-common` SIP-agnostic). Existing callers unaffected.

## Phased roadmap

| Phase | Scope | Crates | Weeks | Depends | Status |
|---|---|---|---|---|---|
| 1 | Identity header type + verify hook | `sip-core`, `sip-dialog`, **new** `rvoip-stir-shaken` | 1.0 | — | **DONE** |
| 2 | Outbound signing hook | `sip-dialog`, `rvoip-stir-shaken` | 1.0 | 1 | **DONE** |
| 3 | RFC 3581 restamp + multi-homed source selection | `sip-transport`, `sip-dialog` | 1.5 | — | **DONE** (restamp); multi-homed source deferred |
| 4 | WebSocket client + WSS accept | `sip-transport` | 1.5 | — | **DONE** (WSS client deferred) |
| 5 | RFC 3263 NAPTR/SRV | `sip-transport`, `sip-dialog` | 2.0 | — | **DONE** |
| 6 | Stateful proxy: server+client txn co-ownership + Timer C | **new** `rvoip-sip-proxy`, `rvoip-sip` (ProxyCoordinator) | 2.5 | — | pending |
| 7 | Forking + 3xx redirect-set | `rvoip-sip-proxy` | 2.0 | 6 | pending |
| 8 | SBC topology hiding (Record-Route strip, Via strip) | `rvoip-sip` | 0.75 | — | **DONE** |
| 8.5 | Stateless proxy helpers (Via push/pop/loop-detect + raw forward) | `sip-core`, `sip-transport` | 0.75 | — | **DONE** |
| 10 | MTU/message-size policy | `sip-transport`, `sip-dialog` | 0.5 | 2 | **DONE** |

**Total: ~13.5 engineer-weeks** (~8.5 calendar weeks with two engineers — similar velocity to `SIP_API_DESIGN_2`).

**Recommended order for value-per-week:** 1 → 2 → 3 → 8 → 4 → 10 → 8.5 → 5 → 6 → 7.

**Crate dependency graph after this work:**
```
rvoip-sip (UnifiedCoordinator + new ProxyCoordinator)
  ├─→ rvoip-sip-dialog (DialogManager, TransactionManager, PASSporTSigner/Verifier traits)
  │     └─→ rvoip-sip-transport (raw_bytes, send_message_raw, resolver)
  ├─→ rvoip-auth-core
  ├─→ rvoip-sip-proxy (NEW — stateful proxy: server+client txn co-ownership, forking, Timer C)
  │     └─→ rvoip-sip-dialog (consumes TransactionManager primitives only — not DialogManager)
  └─→ rvoip-stir-shaken (NEW — reference Signer/Verifier impls)
        └─→ (jsonwebtoken, x509-parser, webpki, reqwest — heavy deps isolated here)
```

### Phase 1 — Identity header + verify hook (smallest standalone value, 1.0 wk) — **DONE**

**Shipped:**
- `crates/rvoip-sip-core/src/types/identity.rs` — typed `Identity { jwt, info, alg, ppt, raw }` wrapper with byte-preservation. Added `HeaderName::Identity` + `TypedHeader::Identity` variants and a nom parser mirroring `p_asserted_identity.rs`.
- `crates/rvoip-sip-dialog/src/manager/identity_verify.rs` — `PASSporTVerifier` trait, `VerificationOutcome { Valid, Stale, BadSig, BadChain, ClaimMismatch, NoIdentity }`, `VerificationPolicy { Annotate, RequireValid, StrictReject }`.
- `DialogManager` gained `identity_verifier`, `verification_policy` fields with setters (kept off `DialogConfig` because trait objects don't impl `Serialize`). Manual `Debug` impl since `Arc<dyn Trait>` blocks derive.
- `DialogManager::run_identity_verification()` shared helper returning `Publish(status) | Drop`. Wired in `events/adapter.rs::publish_session_coordination_event` AND `events/event_hub.rs` so both publish paths run the same verification (no bypass).
- `crates/infra-common/src/events/cross_crate.rs` — additive `IdentityVerificationStatus` enum (SIP-agnostic) and `identity_verification: Option<IdentityVerificationStatus>` on `IncomingCall`.
- **New crate** `crates/rvoip-stir-shaken/` — `ShakenVerifier` (split JWT, fetch cert via `CertResolver` trait, parse X.509 SPKI, extract uncompressed P-256 pubkey, verify ES256, cross-check `orig`/`dest`, iat freshness), `CertResolver` trait + reqwest-backed fetcher, error types.
- 436/437/438/428 reject paths per RFC 8224 §6.2.2 wired via `StatusCode::from_u16` for the missing named variants.

**Acceptance (passing):**
- `crates/rvoip-sip-dialog/tests/identity_verify_inbound.rs` — canned verifier returns Valid → `IncomingCall.identity_verification == Some(Valid)`; tampered From → ClaimMismatch; missing Identity → NoIdentity (or absent under Annotate).
- `crates/rvoip-stir-shaken/tests/sign_verify_round_trip.rs` — full round-trip with rcgen P-256 cert; tampered signature → BadSignature; claim mismatch / stale iat negative paths.

**Deferred:** full SHAKEN STI-CA chain validation (verifier currently trusts the cert returned by the resolver).

### Phase 2 — Outbound signing hook (1.0 wk, depends on Phase 1) — **DONE**

**Shipped:**
- `crates/rvoip-sip-dialog/src/manager/request_lifecycle.rs` — `RequestLifecycle::pre_send_request` trait + impl on `DialogManager`; builds `PassportClaimSummary` from request, calls signer, attaches `TypedHeader::Identity`. Signer-error policy: **degrade open** (log warning, send unsigned) by default; fail-closed available via a wrapper.
- `crates/rvoip-sip-dialog/src/manager/transaction_integration.rs` — gap audit caught that signing was only wired in 1 of 5 INVITE creation paths. All five now invoke `pre_send_request`:
  - `send_request_in_dialog_with_extras` (re-INVITE)
  - `send_invite_with_auth` (auth retry)
  - `send_invite_with_session_timer_override` (422 retry)
  - `send_initial_invite_with_extra_headers` (primary outbound)
  - `create_client_transaction_for_request` (generic)
- `crates/rvoip-stir-shaken/src/signer.rs` — `ShakenSigner` builds JWS manually (`jsonwebtoken::crypto::sign` for ES256 primitive) because `jsonwebtoken::Header` doesn't support the `ppt` extension. Custom `PassportHeader` struct serialized via `serde_json` carries `ppt` + `x5u`. Tel-URI handling: `tel:+15551234567` parses with the number in `Host::Domain`, so `tn_from_uri` reads host for Tel scheme.

**Acceptance (passing):** `crates/rvoip-sip-dialog/tests/identity_sign_outbound.rs` — INVITE captured at mock transport carries a parseable `Identity:` header whose PASSporT `orig`/`dest` match the request URIs.

### Phase 3 — RFC 3581 restamp + multi-homed source (1.5 wk) — **DONE** (restamp half; multi-homed source deferred)

**Shipped (RFC 3581 server-side restamping):**
- `crates/rvoip-sip-dialog/src/transaction/utils/rport.rs` — `stamp_received_rport(via, source)` and `stamp_response_via_with_source(response, source)`. Always sets `received=` per RFC 3261 §18.2.1; only sets `rport=` when the inbound Via had the `;rport` flag (RFC 3581 opt-in).
- `crates/rvoip-sip-dialog/src/transaction/server/invite.rs` and `non_invite.rs` — top-Via restamped **before** the response is stored in `last_response`, so retransmits also carry the stamped form.

**Acceptance (passing):** `tests/rport_restamp_response.rs` — simulated NAT'd UAC; server's 200 OK Via carries `received=<NAT IP>;rport=<NAT port>`.

**Deferred (the "multi-homed source" half):**
- `Transport::send_message` `source_hint` / `LocalBindingPolicy` trait, plus per-destination source selection (`SO_BINDTODEVICE` on Linux, `IP_BOUND_IF` on macOS). The restamp half delivers immediate value for NAT'd UACs; the source-selection half waits on a concrete multi-homed deployment surfacing the need.

### Phase 4 — WebSocket client + WSS accept (1.5 wk) — **DONE**

**Shipped:**
- `crates/rvoip-sip-transport/src/transport/ws/mod.rs::connect_to()` — plain `ws://` client now runs the WS upgrade via `tokio_tungstenite::client_async`, advertises `Sec-WebSocket-Protocol: sip` per RFC 7118 §4.5, and registers the resulting connection in the existing pool. `wss://` client still returns `NotImplemented` (deferred — needs `TlsConnector` wiring and a root-store policy).
- `crates/rvoip-sip-transport/src/transport/ws/listener.rs::accept()` — when `secure=true`, builds a `tokio_rustls::TlsAcceptor` from PEM cert+key at `bind()` and runs the rustls handshake on every accepted TCP socket before the WS upgrade.
- New `crates/rvoip-sip-transport/src/transport/ws/stream.rs::SipWsStream` — wraps `Plain(TcpStream) | ClientTls(client::TlsStream) | ServerTls(server::TlsStream)`, implementing `AsyncRead+AsyncWrite`. Needed because `tokio_tungstenite::MaybeTlsStream::Rustls(_)` only covers the client direction.
- `load_certs` / `load_private_key` in `transport/tls/mod.rs` promoted from `fn` to `pub(crate) fn` so the WS listener reuses the same PEM loaders the TLS transport uses.
- New `wss` cargo feature (`ws + tls`) gates the server-side TLS plumbing. `default` includes it so `--all-features` and the default build both exercise WSS.

**Acceptance (passing):** `crates/rvoip-sip-transport/tests/ws_client_round_trip.rs`
- `plain_ws_round_trip_delivers_register_to_server_event_bus` — UAS bind + UAC dial via `WebSocketTransport`, REGISTER round-trips to `TransportEvent::MessageReceived` with `TransportType::Ws`.
- `wss_server_accepts_tls_handshake_and_negotiates_sip_subprotocol` (gated on `wss + dev-insecure-tls`) — `rcgen` self-signed cert; WSS bind; client runs rustls handshake (accept-self-signed verifier) and completes the WS upgrade with `Sec-WebSocket-Protocol: sips`.

**Deferred (tracked for follow-up):**
- `wss://` client (`connect_to()` for `secure=true`) — needs a `TlsConnector` + root-store / `extra_ca` policy plumbed through `WebSocketTransport::bind()`. Server side closes the gap that real users hit first; client side waits on a concrete user demand.

### Phase 5 — RFC 3263 NAPTR/SRV (2.0 wk) — **DONE**

**Shipped:**
- `crates/rvoip-sip-transport/src/resolver/mod.rs` — **new**: `Resolver` trait (`async fn resolve(&Uri) -> Result<Vec<ResolvedTarget>, ResolverError>`), `ResolvedTarget { addr, transport, expires }`, `ResolverError { Dns, Forbidden, NoCandidates }`, plus `pub fn select_transport_for_uri(&Uri) -> TransportType` re-homed here as the single source of truth (the `MultiplexedTransport` copy is now a `pub use` re-export). `From<ResolverError> for transport::Error` mapping at the boundary.
- `crates/rvoip-sip-transport/src/resolver/srv.rs` — **new**: pure helpers with no hickory dep: `select_srv_best` (RFC 2782 weighted selection), `expand_srv_priority_group` (walk an entire priority group in weighted-random order — required for §4.3 within-group failover), `srv_service_name` (returns `None` for `sips:`+`transport=udp`), `default_port_for_scheme`, `map_naptr_service` (`SIP+D2U`/`D2T`, `SIPS+D2T`, `SIP+D2W`/`SIPS+D2W` token map), `fallback_srv_chain`. 15 unit tests, migrated verbatim from the deleted `dns_resolver.rs` plus new NAPTR cases.
- `crates/rvoip-sip-transport/src/resolver/hickory.rs` — **new**, gated `#[cfg(feature = "dns")]`. `HickoryResolver` walks the full RFC 3263 §4 ladder: IP literal → explicit port → `;transport=`/`sips:` SRV-only → NAPTR (order+preference + service-token map, `flags="s"`) → fallback SRV chain (`_sips._tcp`, `_sip._tcp`, `_sip._udp`) → A/AAAA. `new_system()` builds from `read_system_conf()` with `edns0 = true` (forces EDNS0 so large NAPTR responses don't get truncated); `with_resolver(config, opts)` for tests pointing at fixture DNS. Maps `hickory_resolver::error::ResolveError` to `ResolverError::Dns(String)` at the boundary — the dialog crate has no transitive hickory type dep.
- `crates/rvoip-sip-transport/Cargo.toml` — new `dns = ["dep:hickory-resolver", "dep:fastrand"]` feature (NOT in `default` — opt-in). `hickory-server` + `hickory-proto` 0.24 added as dev-deps for the e2e fixture. `[[test]] resolver_hickory_e2e` gated `required-features = ["dns"]`.
- `crates/rvoip-sip-dialog/Cargo.toml` — `hickory-resolver` + `fastrand` dropped as direct deps. `rvoip-sip-transport = { workspace = true, features = ["dns"] }` so dialog's default behavior is preserved bit-for-bit.
- `crates/rvoip-sip-dialog/src/manager/core.rs` — `DialogManager` gained `resolver: Arc<RwLock<Option<Arc<dyn Resolver>>>>` field with `set_resolver(...)` / `resolver()` accessors mirroring the existing `identity_signer` / `identity_verifier` pattern. New `resolve_uri_to_socketaddr(&self, &Uri)` method consults the configured resolver first, falling back to the process-wide free function. Five existing INVITE/BYE/etc. call sites in `transaction_integration.rs` (and two in `core.rs`) now route through `self.resolve_uri_to_socketaddr` so a configured per-manager resolver actually fires on the outbound path.
- `crates/rvoip-sip-dialog/src/dialog/dialog_utils.rs` — `resolve_uri_to_socketaddr` rewritten to use a process-wide lazy `Lazy<Arc<HickoryResolver>>` default. Keeps the `pub async fn resolve_uri_to_socketaddr(uri: &Uri) -> Option<SocketAddr>` signature so all ~20 existing callers compile unchanged. IP-literal short-circuit kept *outside* the resolver so the function still works in sandboxed CI without `/etc/resolv.conf`.
- `crates/rvoip-sip-dialog/src/dialog/dns_resolver.rs` (408 LOC) — **DELETED**. Algorithm helpers moved into `rvoip-sip-transport::resolver::srv`; `SystemDnsResolver` replaced by `HickoryResolver`.
- `crates/rvoip-sip-dialog/src/transaction/transport/multiplexed.rs:71-104` — `select_transport_for_uri` body deleted; replaced with `pub use rvoip_sip_transport::resolver::select_transport_for_uri;`.
- `crates/rvoip-sip-dialog/src/transaction/manager/handlers.rs:633-638` — ACK helper now delegates to `dialog_utils::resolve_uri_to_socketaddr` (drops the direct `dns_resolver::*` call). ACK routing transparently gains NAPTR.

**Acceptance (passing):**
- `crates/rvoip-sip-transport/src/resolver/srv.rs` — 15 unit tests on the pure algorithm helpers (service-name derivation including `sips:`+`transport=udp` → `None`, RFC 2782 weighted selection, priority-group expansion, NAPTR service-token map, fallback chain ordering).
- `crates/rvoip-sip-transport/src/resolver/hickory.rs` — 5 unit tests on the `HickoryResolver` short-circuits (IP-literal default-port for `sip:` and `sips:`, explicit-port wins, `transport=` param wins, `sips:`+`transport=udp` rejected with `ResolverError::Forbidden`). No DNS server needed.
- `crates/rvoip-sip-transport/tests/resolver_mock.rs` — 10 trait-surface tests (trait-object dispatch returns canned candidates, candidate ordering is preserved for §4.3 failover, all three error variants propagate, calls are recorded for assertions, TTL is carried per-candidate, `From<ResolverError> for transport::Error` maps each variant correctly).
- `crates/rvoip-sip-transport/tests/resolver_hickory_e2e.rs` — `required-features = ["dns"]`. Binds a real `hickory_server::ServerFuture` to `127.0.0.1:0` with an `InMemoryAuthority` serving `example.test` (NAPTR records ordered `SIPS+D2T order=10` then `SIP+D2U order=20`, SRVs for both, A pointing at loopback). Points a `HickoryResolver` at the fixture and asserts the candidate vec contains both `(Tls, 127.0.0.1:5061)` and `(Udp, 127.0.0.1:5060)` with the TLS candidate ordered before UDP (NAPTR order honoured).
- `crates/rvoip-sip-dialog/tests/rfc3263_resolution.rs` — 5 dialog-layer acceptance tests: `set_resolver` round-trips (set/get/unset), `manager_uses_configured_resolver_for_invite_destination` (configured mock sees the URI and returns the addr), `manager_returns_first_candidate_when_resolver_offers_multiple` (first candidate wins), `manager_falls_back_to_default_resolver_when_unset` (IP literal short-circuits even without resolver), `configured_resolver_overrides_default_for_ip_literal_uri_resolution_path` (configured resolver is authoritative — bypasses the IP-literal short-circuit when explicitly installed).

**Deferred:**
- **NAPTR algorithm unit tests with canned-DNS mock** — exercising every §4.1 branch (priority-pair sorting, unknown-service-token skip, fall-through to SRV chain when NAPTR returns nothing usable, weighted SRV selection within a priority group) currently relies on the `resolver_hickory_e2e.rs` real-DNS smoke. A `DnsBackend` abstraction inside `HickoryResolver` would let unit tests pin every lookup to a canned response; small refactor, low risk. Add when a regression in the ladder ordering surfaces or when carriers report NAPTR responses we don't handle.
- **Multi-candidate failover at the dialog layer** — the manager-aware `resolve_uri_to_socketaddr` projects to a single `SocketAddr` (`candidates.into_iter().next()`). RFC 3263 §4.3 says the caller should try the next candidate on transport failure; today the transport layer surfaces the error and the dialog layer aborts. A `try_send_with_failover(uri, candidates)` helper is the natural follow-up; defer until a deployment needs it.
- **`wss://` client + DNS-aware connect** — see Phase 4 deferral notes; orthogonal to RFC 3263.

### Phase 6 — Stateful proxy single-target + Timer C (2.5 wk)

**Trade-off:** revisits the prior round's "stateful B2BUA covers SBC use cases" decision. Justified for carrier transit proxies and registrar deployments that should NOT terminate dialog state. Lives in a new sibling crate `rvoip-sip-proxy` rather than `rvoip-sip-dialog` — see Rec 3 below for rationale (matches reSIProcate `repro`/`dum`, pjsip core/`pjsua-lib`, Kamailio `tm`/`dialog` separation; avoids fighting `manager/core.rs:1006-1055` which assumes every inbound creates a dialog).

**Files:**
- **New crate** `crates/rvoip-sip-proxy/` — `ProxyTransaction::forward(modified: Request, dest) -> ForwardHandle`; `ProxyTransaction::forward_responses(handle) -> Stream<Response>`; `ProxyTransactionPair { server, clients, response_correlator }`. Depends on `rvoip-sip-dialog` for `TransactionManager` primitives only (not `DialogManager`).
- `crates/rvoip-sip-dialog/src/transaction/manager/mod.rs` — expose any transaction-manager hooks `rvoip-sip-proxy` needs (e.g., a `subscribe_to_transport_event` shape that bypasses `handle_request`'s dialog-creation branch).
- `crates/rvoip-sip-dialog/src/transaction/timer/factory.rs` — Timer C (default 3 min, app-overridable) added here since the timer factory is shared between dialog UAC and proxy.
- **New file** `crates/rvoip-sip/src/api/proxy_coordinator.rs` — `ProxyCoordinator` public API parallel to `UnifiedCoordinator`, wrapping `rvoip-sip-proxy::ProxyTransaction`.

**Acceptance:** `crates/rvoip-sip-proxy/tests/stateful_proxy_single_target.rs` — UAC → proxy → UAS, response flows back. Timer C fires on stalled INVITE.

### Phase 7 — Forking + 3xx (2.0 wk, depends on Phase 6)

**Files:**
- `crates/rvoip-sip-proxy/src/lib.rs` — `ProxyTransaction::fork(targets, ForkMode::{Parallel, Sequential})`.
- `crates/rvoip-sip-proxy/src/response_aggregator.rs` — **new**: aggregate across multiple client transactions (RFC 3261 §16.7 response context).
- 3xx redirect-set: emit at the `ProxyCoordinator` event API in `rvoip-sip` (not the cross-crate bus, since proxy mode is not dialog-scoped).

**Acceptance:** `crates/rvoip-sip-proxy/tests/proxy_parallel_fork.rs` — 1 INVITE fans to 3 UAS; first 200 wins, CANCEL fans to other two.

### Phase 8 — SBC topology hiding (0.75 wk) — **DONE**

**Shipped:**
- `crates/rvoip-sip/src/adapters/dialog_adapter.rs` — `strip_via_below_top(request)` and `strip_record_route_below_self(request, self_host)` (case-insensitive). Both public; `mod.rs` re-exports them.

**Acceptance (passing):** `crates/rvoip-sip/tests/sbc_topology_hiding_via_strip.rs`
- Three-Via fixture → exactly one Via remains (the SBC's, topmost) after `strip_via_below_top`.
- SBC + two-upstream RR fixture → only SBC's RR entry remains after `strip_record_route_below_self("sbc.example.com")`.
- Empty RR headers are removed entirely so wire form doesn't carry `Record-Route: ` (some parsers reject).
- Combined: positive (`sbc.example.com` survives) + negative (upstream proxy hosts and UAC internal IP do not leak).
- Case-insensitive self-host matching: `sip:SBC.Example.Com` matches self-host `"sbc.example.com"`; original case preserved on the surviving entry.

**Deferred:** `with_topology_hiding(bool)` builder option on the INVITE builder. Default behavior depends on the concrete SBC deployment posture (always-strip vs. opt-in), so it waits on a deployment user to define.

Note: the helpers cover the "forward existing Request" shape (proxy-style on top of `Transport::send_message_raw`). The codebase's default B2BUA pattern — `coord.invite(...)` + `with_headers_from(&call, ...)` + `send()` — builds a *fresh* outbound INVITE with the SBC's own Via stamped from scratch, so it never strips. Phase 8.5 stateless-proxy work will lean on these helpers.

### Phase 8.5 — Stateless proxy helpers (0.75 wk) — **DONE**

**Recommendation (per Rec 2 below):** ship only the primitives, not a full `StatelessProxy` framework. Applications that need byte-exact STIR/SHAKEN-preserving forwarding can compose these on top of `Transport::send_message_raw` (already shipped). Defer the full framework until a concrete user surfaces — modern stateless-proxy demand is narrow (edge LB / DDoS / dispatcher), and that segment is well-served by Kamailio.

**Shipped:**
- `crates/rvoip-sip-core/src/types/via.rs` — three new `Via` methods. `push_proxy_branch(transport, sent_by_host, sent_by_port, branch) -> Result<()>` inserts a new top entry (caller-supplied branch — stateful proxies pass a random `z9hG4bK…`, stateless proxies derive deterministically per RFC §16.11). `pop_top() -> Option<ViaHeader>` removes and returns the top entry. `detect_loop(against: &[Via]) -> bool` returns true if any branch in `self` collides with any branch in `against` (RFC 3261 §16.6 step 4).
- `crates/rvoip-sip-core/src/parser/via_locator.rs` — **new file**. `find_top_via_line(bytes) -> Option<Range<usize>>` byte-scan helper: case-insensitive match on `Via:` and compact form `v:`, tolerant of whitespace before the colon, returns the line range inclusive of trailing CRLF. Pure byte scan so the Identity JWT is never re-parsed/re-serialized.
- `crates/rvoip-sip-transport/src/transport/mod.rs` — `enum ViaRewrite { Push(Bytes), Pop }` and `Transport::forward_raw_with_via_rewrite(bytes, rewrite, dest)` default impl. Push = request forwarding (inserts caller's Via line above the existing top); Pop = response forwarding (removes the existing top entirely). Both delegate to the already-shipped `send_message_raw`. Errors with `Error::ProtocolError` when no Via is present (no anchor for push, nothing to pop). A standalone `pub fn apply_via_rewrite(bytes, rewrite) -> Result<Bytes>` is also exposed so callers can inspect the rewritten bytes without owning a `Transport`.

**Acceptance (passing):** `crates/rvoip-sip-transport/tests/stateless_proxy_helpers.rs`
- `request_forward_pushes_via_and_preserves_identity_bytes` — INVITE with typed Identity header (3-segment JWT) is forwarded via `ViaRewrite::Push`; capturing mock transport confirms the JWT bytes are byte-exact in the output, the proxy's Via sits above the UAC's, and the chosen deterministic branch appears exactly once.
- `response_forward_pops_top_via_and_preserves_identity_bytes` — synthesised 200 OK with proxy Via on top + UAC Via below is forwarded via `ViaRewrite::Pop`; the proxy's `sent-by` is gone from the output, UAC's survives, and the response Identity JWT is byte-exact.
- `forward_fails_loudly_when_no_via_present` — Push and Pop both return an error mentioning "Via" when given a message with no Via header.
- `via_helpers_round_trip_at_the_typed_layer` — exercises `push_proxy_branch` + `detect_loop` + `pop_top` composition on the typed `Via` (for stateful proxies that don't need byte preservation).

**Deferred (revisit only on demand):** full `StatelessProxy { transport, policy: Arc<dyn ProxyPolicy> }` framework with a routing-policy trait. Documented as "build on the helpers" until a user asks for the framework.

### Phase 10 — MTU/size policy (0.5 wk, depends on Phase 2) — **DONE**

**Shipped:**
- `crates/rvoip-sip-transport/src/transport/mod.rs` — `Transport::max_safe_message_size() -> usize` (default `usize::MAX`; stream transports are not byte-bounded at this layer).
- `crates/rvoip-sip-transport/src/transport/udp/mod.rs` — UDP overrides to `UDP_SAFE_MAX_BYTES = 1300` (RFC 3261 §18.1.1 explicit threshold).
- `crates/rvoip-sip-dialog/src/transaction/utils/mtu.rs` — `set_top_via_protocol(request, "TCP")` mutates only the top Via's `sent-protocol` field (branch and sent-by preserved, so the transaction key survives and Identity-header signature stays valid — SHAKEN PASSporT claims don't cover Via).
- `crates/rvoip-sip-dialog/src/transaction/transport/multiplexed.rs::send_message` — single chokepoint extension. After `pick_transport` returns UDP for a Request, serialize and compare against the transport's `max_safe_message_size()`. If oversized: look up TCP in the registry, flip top Via to TCP, dispatch via TCP. If no TCP registered: return `TransportError::MessageTooLarge(size)`. **Fail-closed** — RFC §18.1.1 is MUST, not SHOULD; unlike Phase 2's signer which degrades open (signing failure is a policy concern, not a wire-protocol violation).
- `MultiplexedTransport::new_without_trace(default, transports)` — public convenience constructor so external integration tests can build a mux without depending on the crate-private `SipTraceRuntime` type.

**Placement note:** The original plan suggested `transaction/client/builders.rs`, but the multiplexer is a strictly better home — every outbound caller funnels through it, it already owns the per-flavour transport registry, and it sees the message AFTER `pre_send_request` signing.

**Acceptance (passing):** `crates/rvoip-sip-dialog/tests/mtu_failover.rs`
- `oversized_udp_request_fails_over_to_tcp` — INVITE padded with a 3 KB synthetic Identity header (Phase 2 PASSporT shape); TCP mock gets the send with top Via flipped to `SIP/2.0/TCP` and branch preserved; UDP mock gets nothing.
- `small_udp_request_stays_on_udp` — bare INVITE (<1300 bytes) sends via UDP; TCP gets nothing.
- `oversized_udp_with_no_tcp_registered_is_message_too_large` — registry holds only UDP; oversized INVITE returns `MessageTooLarge(size)` and UDP is never invoked.

**Deferred:**
- Response-path MTU policy (RFC §18.2.2). Real-world response-size pressure is rare; defer until a deployment surfaces it.
- Configurable threshold. Single hard-coded `UDP_SAFE_MAX_BYTES = 1300` for now.

## Crate placement

| Capability | Lives in | Rationale |
|---|---|---|
| `IdentityHeader` typed wrapper | `rvoip-sip-core` | Pure wire-form parsing; mirrors `PAssertedIdentity` |
| `PASSporTSigner`/`PASSporTVerifier` traits | `rvoip-sip-dialog` | Hooks must live where bytes + transaction key are accessible |
| Reference STIR/SHAKEN impls | **new** `rvoip-stir-shaken` | Heavy crypto deps (`jsonwebtoken`, `x509-parser`, `webpki`, `reqwest`) — opt-in only |
| `Resolver` trait | `rvoip-sip-transport` | Returns transport+addr; same crate as `Transport` |
| Reference DNS impl | `rvoip-sip-transport` behind `dns` feature | `hickory-resolver` well-contained |
| Stateful proxy primitives | **new** `rvoip-sip-proxy` | Universal external pattern (resip `repro`/`dum`, pjsip core/`pjsua-lib`, Kamailio `tm`/`dialog`); dialog-core's protocol handlers assume dialog ownership and would fight a proxy entry point |
| Stateful proxy public API | `rvoip-sip::api::proxy_coordinator` | Parallel to `UnifiedCoordinator`; keeps `rvoip-sip` as the sole user-facing entry point |
| Stateless proxy helpers (Via push/pop, raw forward) | `rvoip-sip-core` (Via utilities) + `rvoip-sip-transport` (forward method) | Primitives only — no proxy framework until concrete demand |
| SBC topology hiding | `rvoip-sip` | Co-located with B2BUA + Contact-rewrite |

## Verification

After each phase:
- `cargo test --all-features -p rvoip-sip-core -p rvoip-sip-dialog -p rvoip-sip-transport -p rvoip-sip`
- `cargo test --doc -p rvoip-sip`
- PBX matrix `crates/rvoip-sip/examples/pbx/run.sh --pbx both --api callback --scenario all` stays green
- New per-phase integration tests listed above pass

Critical: per repo memory, use `cargo test --all-features` for migration validation — the default skips feature-gated targets like `generated_sip_compliance` and can show false-green.

## Out-of-scope (explicit, this round)

- Bundled STI-PA SHAKEN root anchors (trust model is pluggable — apps supply roots).
- HSM drivers (apps implement `PASSporTSigner` over whatever key store they use).
- Diversion / History-Info `ppt=div`/`rcd` PASSporT variants beyond the base SHAKEN profile (additive once base ships).
- Removing deprecated APIs from `SIP_API_DESIGN_2` (separate breaking-release cleanup).

## Rationale for the three architecture decisions

The phased roadmap above (and the crate placement table) reflects three concrete architecture decisions. These were originally open questions; the recommendations below are backed by a codebase audit (`auth-core` precedent, dialog/transaction seam, sip vs. sip-dialog boundary) and external research (Asterisk, FreeSWITCH, Kamailio, reSIProcate, pjsip, Sofia-SIP, ATIS SHAKEN governance, Tower/Hyper Rust idioms).

### Recommendation 1 — STIR/SHAKEN packaging: **new optional sibling crate `rvoip-stir-shaken`**

**Recommendation:** ship as a new sibling crate, NOT a feature flag on `rvoip-sip-dialog`. Trait surface (`PASSporTSigner`, `PASSporTVerifier`, `VerificationOutcome`, `VerificationPolicy`) lives in `rvoip-sip-dialog::manager::identity` (where the hooks attach); reference implementations + heavy crypto deps live in `rvoip-stir-shaken`. `rvoip-sip-dialog` takes `Arc<dyn PASSporTSigner>` / `Arc<dyn PASSporTVerifier>` and never imports the impl crate.

**Why:**
- **Repo precedent.** `rvoip-auth-core` (1,103 LOC, 4 files, no feature flags) is consumed by `rvoip-sip` only — not by `rvoip-sip-dialog`. It's a standalone, focused crate that ships all its deps unconditionally. STIR/SHAKEN sits at exactly the same conceptual layer (transport-adjacent crypto plumbing the session orchestrator wires in) and should follow the same shape.
- **External alignment.** Asterisk splits STIR/SHAKEN into `res_stir_shaken` (core) + `res_pjsip_stir_shaken` (SIP glue) as separately loadable modules. FreeSWITCH and Kamailio mirror this pattern (`stirshaken` module + `secsipid`/JWT modules). Every mature stack treats it as optional, not core SIP. The Tower/Hyper Rust idiom (`tower-service` trait crate + sibling impl crates) is the canonical equivalent.
- **Dependency hygiene.** `jsonwebtoken` + `x509-parser` + `webpki` + `reqwest` are heavyweight and ecosystem-divergent (rustls vs native-tls debates, async runtime ties). A feature flag forces every consumer of `rvoip-sip-dialog` to either accept the dep tree or build with `--no-default-features` and lose other features they want. A sibling crate makes opt-in clean: applications that need STIR/SHAKEN add one line to `Cargo.toml`.
- **Trust anchors are universally operator-configured.** ATIS SHAKEN governance (STI-PA publishes the approved-CA list over HTTPS; each VSP holds its own certs) means the library can't sensibly bundle roots. `CertResolver` trait + reference `reqwest`-backed fetcher in `rvoip-stir-shaken`; library ships no CAs.

**Action:** create `crates/rvoip-stir-shaken/` in Phase 1 as planned. `Cargo.toml` mirrors `rvoip-auth-core`'s shape (no feature flags inside the impl crate, unconditional deps).

### Recommendation 2 — Stateless proxy: **ship helpers only; defer the full framework**

**Recommendation:** ship Phase 8.5 (Via push/pop/loop-detect on `Via` + `Transport::forward_raw_with_via_rewrite()`) at ~0.75 wk. Do NOT build a full `StatelessProxy { transport, policy: Arc<dyn ProxyPolicy> }` framework until a concrete user surfaces. (An earlier draft of this roadmap reserved a separate "Phase 9 — full stateless proxy" at 3.0 wk; that phase is intentionally NOT in the phase table — the helpers cover real needs without committing to a framework that competes with Kamailio's well-trodden surface.)

**Why:**
- **Real demand is narrow.** Modern Kamailio/OpenSIPS deployments are transaction-stateful by default — both stacks recommend the unified `send_reply()` which auto-promotes to stateful whenever a transaction exists. Stateless is reserved for edge load-balancers / DDoS perimeters (5k+ CPS), pure REGISTER fan-out, and dispatchers. These users typically already deploy Kamailio in front; rvoip's value-add at that layer is small.
- **The byte-exact STIR/SHAKEN argument is real, but the helpers cover it.** Stateless DOES preserve the Identity header trivially (forward bytes unchanged), whereas stateful B2BUA must re-sign at the boundary. But applications that need byte-exact forwarding for STIR/SHAKEN can build it on `Transport::send_message_raw` (already shipped) + Via push/pop helpers. They don't need a full proxy framework.
- **Reversing scope creep cleanly.** The prior round's `SIP_API_DESIGN_2_GAP_PLAN.md:822` decision excluded stateless proxy with the rationale "stateful B2BUA covers SBC use cases." That rationale holds for the SBC market; it doesn't hold for transit proxies. The helpers-only approach gives transit-proxy authors the primitives without rvoip committing to a competing surface against Kamailio.

**Action (already reflected in the phase table above):** Phase 8.5 — stateless proxy helpers (0.75 wk) — is the only stateless-proxy work in this roadmap. Documentation should frame the helpers as supporting a build-your-own stateless proxy pattern. The full framework stays out until a real user demands it.

### Recommendation 3 — Stateful proxy: **new sibling crate `rvoip-sip-proxy`**

**Recommendation:** create a new sibling crate `rvoip-sip-proxy` for stateful-proxy primitives. Public-facing `ProxyCoordinator` lives in `rvoip-sip` (mirrors `UnifiedCoordinator`). Do NOT put proxy logic in `rvoip-sip-dialog::api::proxy`.

**Why:**
- **Universal external pattern.** Every mature SIP stack puts proxy primitives BESIDE the dialog layer at the transaction tier, never inside the dialog/UA module:
  - reSIProcate: `dum` (Dialog Usage Manager, UA-only) vs. separate `repro` proxy application built on `resip` stack.
  - pjsip: `pjsua-lib` (high-level UA) contains no proxy code; proxy primitives are in lower `pjsip-core`.
  - Kamailio: `sl` (stateless) and `tm` (stateful) are independent modules; `dialog` is a separate module above them.
  - Sofia-SIP: `nta` transaction layer exposes both UA and Proxy state engines side-by-side; `nua` (call logic) sits above.
- **Codebase audit confirms the friction.** `crates/rvoip-sip-dialog/src/manager/core.rs:1006-1055` dispatches every inbound request through method-specific protocol handlers that **assume "we own this dialog."** A proxy entry point must bypass these handlers to avoid creating spurious dialog state. Forcing this into `rvoip-sip-dialog::api::proxy` either (a) couples proxy and dialog state machines that should remain independent, or (b) requires a "this is a proxy, skip dialog creation" branch in every protocol handler — both bad.
- **Three-layer pattern already exists in this codebase.** `rvoip-sip` (coordinator) → `rvoip-sip-dialog` (dialogs/transactions). Adding `rvoip-sip-proxy` between them, depending only on `rvoip-sip-dialog`'s transaction primitives (not its dialog manager), preserves the architecture's existing layering discipline.

**Crate dependency graph after this work:**
```
rvoip-sip (UnifiedCoordinator + new ProxyCoordinator)
  ├─→ rvoip-sip-dialog (DialogManager, TransactionManager, PASSporTSigner/Verifier traits)
  │     └─→ rvoip-sip-transport (raw_bytes, send_message_raw, resolver)
  ├─→ rvoip-auth-core
  ├─→ rvoip-sip-proxy (NEW — stateful proxy: server+client txn co-ownership, forking, Timer C)
  │     └─→ rvoip-sip-dialog (consumes TransactionManager primitives only — not DialogManager)
  └─→ rvoip-stir-shaken (NEW — reference Signer/Verifier impls)
        └─→ (jsonwebtoken, x509-parser, webpki, reqwest — heavy deps isolated here)
```

**Action (already reflected in the phase table above):** `ProxyTransaction` and `ProxyTransactionPair` live in a new `crates/rvoip-sip-proxy/` crate; `crates/rvoip-sip/src/api/proxy_coordinator.rs` is the public-facing entry point (parallel to `UnifiedCoordinator`). Phase 7 (forking + 3xx) is a `rvoip-sip-proxy` extension, not a `rvoip-sip-dialog` change.
