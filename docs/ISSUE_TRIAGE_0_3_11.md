# Issue triage for the 0.3.11 bundle

Reviewed 2026-09-27 against `codex/downstream-feature-review` (the 0.3.11
candidate). Every open issue was checked against the code on that branch;
"landed" claims cite the file or test that proves them. Closed issues carry
the evidence in their closing comment.

## Closed as implemented

| Issue | Evidence |
| --- | --- |
| #100 RvoipApp authoritative inbound admission and operational streams | `RvoipAppBuilder::authoritative_ingress`, `take_authoritative_ingress`, `ingress_health`, `drain` in `crates/rvoip/src/app.rs`; `authoritative_ingress_*` tests |
| #102 Production remote SIP endpoint registration, flow recovery, NAT profile | RFC 5626 flow ownership in `sip-registrar/src/api/mod.rs`, `SipConfig::remote_endpoint_profile()`, `docs/sip/REMOTE_ENDPOINT_PROFILE.md`; 0.3.11 adds STUN on the RTP socket (#236) and DTLS-SRTP SAVPF interop (#234) |
| #209 Query live registrar bindings for an AOR | `UnifiedCoordinator::start_registration_server` returns `Arc<RegistrarService>`; `lookup_aor` / `lookup_live_contacts` |

## Candidates to pull into 0.3.11

Small, contained correctness or API items. None are started.

| Issue | What is needed | Effort |
| --- | --- | --- |
| #111 Linearizable tenant session admission and quota resizing | Hold a per-tenant permit from Initiating through the terminal state instead of counting only `Active`; track configured capacity separately from `available_permits()`; idempotent same-value resize. `orchestrator.rs` `check_session_quota` / `set_tenant_quotas` | S/M |
| #118 Trusted-network authentication method (part 1) | Add `AuthenticationMethod::TrustedNetwork` (and `#[non_exhaustive]`) in `rvoip-core-traits/src/identity.rs`; stop `trusted_trunk_principal` in `crates/rvoip/src/app.rs` reporting `ApiKey`. Strict bearer parsing and principal fingerprint helpers can follow later | S |
| #207 Inbound OPTIONS application policy | A pre-response `OptionsPolicy` hook in `sip-dialog/src/manager/protocol_handlers.rs` returning 200 or 503 + Retry-After while draining; today OPTIONS is answered 200 before any application sees it | S/M |
| #164 / #206 Vapi tool-result over WSS | Verify whether Vapi's WebSocket transport accepts a tool-result control message. If yes: add a `VapiCommand` variant and `VapiAgentCall::tool_result` (#85 becomes the typed API). If no: document the webhook-only limitation in the crate README. #85 and #164 close against #206 either way | S/M |

## Partially landed in 0.3.11 (status posted on the issue, kept open)

| Issue | Landed | Still open |
| --- | --- | --- |
| #79 Atomic same-participant connection replacement | `Orchestrator::replace_bridge_destination` with generation fence, rollback, `PeerHandoffCommitted`; SIP/QUIC/Vapi fenced | Participant-identity binding, `operation_id`, replacement events, moving recordings/AI attachments; WebTransport/WebSocket fence |
| #81 Per-attachment stateful harness dialog sessions | `InProcessAiSessionFactory` per-call isolated state with provider references | Typed tool/context continuation on `DialogManager` (under #205) |
| #84 UCTP call control and portable attended transfer | `MuteDirection`, `transfer_with_attempt`, `ConnectionTransferStatus`; SIP hold/resume/transfer, Vapi mute, harness hold/resume | QUIC/WebTransport/WebSocket hold/resume/transfer, UCTP coordinator media effect, SIP transfer to Connection/Session targets |
| #91 Multi-party UCTP/SFU | Selective forwarding (`add_subscription` / `fanout_frame`) proven over QUIC; N-way mixer in `rvoip-core/src/conference.rs` | Production claim, degraded-path observability, acceptance matrix |
| #92 Atomic different-participant bridge-peer handoff | The atomic core (`replace_bridge_destination*`, role verbs `hand_off` / `take_over`) | Single op with `source_disposition`, `RemainAsObserver` topology, Vapi `transfer`, recording-continuity assertion |
| #97 Harness pause/mute/observer | Paused semantics via `hold` / `resume` on the in-process AI Connection with provider acknowledgement | ListeningOnly (`mute` is `NotImplemented`), Observer mode, `operation_id` (under #205) |
| #98 Distinct AI Participant and media provenance | Vapi joins a distinct `Ai` Participant; in-process AI is its own Connection with its own participant | `AiAttached` carries no participant; legacy `attach_ai` joins none; no vCon two-Parties assertion (under #205) |
| #103 Idempotent Vapi call creation and provider reference | `attach_existing_agent`, `ExternalConnectionReference` carries the Vapi call id | Caller idempotency key, redacted `call_reference()` accessor, lookup/terminate for reconciliation, crash-boundary tests |
| #205 Harness AI runtime tracker | `rvoip-harness` exists with bounded lifecycle, drain, hold/resume, `ParticipantKind::Ai` | Tool continuation (#81), DTMF/input delivery (#101), observer/mute (#97), failure taxonomy (#82), provenance (#98). Re-scope the tracker body |

## Kept open, not started, later than 0.3.11

| Issue | Note | Effort |
| --- | --- | --- |
| #82 Provider-neutral AI runtime failure lifecycle | Only `AiAttached` / `AiDetached` exist; belongs in the #205 build | M |
| #83 Real UCTP reconnect, reachability, session grace | Coordinator still mints throwaway tokens; no `AuthKeepalive` dispatch | L |
| #85 Typed Vapi tool-result API | Closes against #206 once the protocol question is answered | M |
| #87 Recording-to-vCon provenance linkage | `RecordingComplete.vcon_ref` is always `None` | S/M |
| #88 JWE in rvoip-vcon | No `jwe` module; Thelve has a workaround | M |
| #89 UCTP mobile push hook | Blocked on #83 | M |
| #90 WebSocket fallback multi-publisher subscriber streams | `allocate_subscriber_stream` is `NotImplemented` on the WS adapter | M/L |
| #94 Production storage boundaries for SAML/SCIM/LDAP/WebAuthn | All four crates still process-local; no commits since August | L |
| #95 Named vCon extension for sibling-Session grouping | Generic `extension()` exists; no named extension defined | M |
| #96 Complete rvoip-client UCTP surface | `accept` is a no-op; `hold/resume/mute/send_dtmf` are `NotImplemented` | L |
| #99 Loss-observable recording/transcription/vCon lifecycle | Pause is a relaxed flag; no `RecordingFailed` / `TranscriptionFailed` / `VconFailed` | L |
| #101 Provider-neutral AI input events incl. DTMF | Both AI runtimes return `NotImplemented` for `send_dtmf` | M/L |
| #104 Cancellation-safe harness provider teardown | Legacy `attach_ai` still aborts before `close().await`; under #205 | M |
| #106 Official @rvoip TypeScript packages | No JS packages exist | L |
| #108 RvoipApp production multi-call runtime | `AppState` still holds one call; only `drain` / `ingress_health` added | L |
| #109 Translated RTCP across transcoding bridges | Per-stream primitives only; design first | L |
| #110 Managed broadcast composition | Low-level publisher/authority pieces exist; no managed owner | L |
| #112 Versioned durable orchestration contracts | `conversation_store` is put/get/delete/list and unused by the orchestrator | L |
| #113 Generalize RvoipApp routing/directory seams, wire UCTP | `UctpConfig` still a rejected placeholder | L |
| #114 Remote ConnectionAdapter over authenticated UCTP | No remote adapter crate | L |
| #116 rvoip-testkit | Only the harness-local `test-reference` feature exists | M |
| #117 Redacted secret references and strict config loading | No `SecretRef`; loader still uses deprecated `config::merge` | M |
