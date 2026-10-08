# Bounded lifecycle cleanup proposal

Observed against published 0.3.12, October 6, 2026. Local application qualification uses those published dependencies with these core/core-traits source patches.

## Defects

- The orchestrator stores active and retired ConnectionIds indefinitely and rejects all unseen IDs at its default 262,144-entry budget. This is a lifetime admission quota, even after media/ports/calls are gone.
- Ended sessions and closed conversations remain in maps; tenant conversation membership remains as well. The application cannot release that history through the published API.
- Auto-ending a session on final connection detachment does not finalize its vCon builder. The default memory message/vCon stores and builder arrays have no retention policy.

A recording worker hit exactly 262,144 completed/audited admissions and generated 2,520 subsequent pre-answer failures during a 120,000-attempt segment. It had no remaining calls or media ports. The result remains a failed quality result; process replacement is containment.

## Patch

Opt into `configure_bounded_connection_lifecycles(maximum)` before registering adapters. The budget bounds each retained connection/session/conversation map, rather than cumulative calls.

ConnectionId and ConversationId factories issue a process-incarnation UUID plus a checked, never-wrapping 64-bit sequence. Each minted identity owns a shared private one-use capability. Retirement follows clones in queued events and asynchronous setup. No global collection of spent capabilities exists; the final clone frees the capability.

Serde and `from_string` preserve an opaque correlation string for lookup, not creation authority. Bounded-mode inbound/outbound lifecycle birth and conversation creation require fresh minted authority. Operational/terminal callbacks may echo a string lookup ID only for an existing lifecycle. Unknown strings cannot mint or tombstone a lifecycle. This preserves adapter command callbacks without allowing replayed identifiers to create new calls.

Core retires authority before dropping ownership. Connection records are reclaimed after media shutdown/session detachment and conclusive adapter cleanup. Unresolved quarantines retain reservations; successful later cleanup reclaims them without requiring new calls. The existing ticket pointer/generation checks still reject stale asynchronous commits. No live call is expired by age.

`release_closed_conversation` explicitly releases completed models and tenant membership/empty index rows after the application has retained required durable evidence. **Applications must call it after every conversation teardown in bounded mode.** Closed conversations and their ended sessions keep counting against the retained conversation/session budget until released; core never releases them on its own, so an application that only closes conversations will eventually have `open_conversation*` and `start_session` rejected with `AdmissionRejected`. `close_conversation` and the periodic idle closer (`spawn_idle_closer`, supervised separately) close conversations but do not release them; wiring the idle closer to release is not part of this change. It requires bounded mode, a closed/unowned conversation, terminal sessions with no live connections, and no outstanding vCon builder. Removed conversation IDs cannot be reused to create new conversations. Applications needing reopening/history must retain it externally according to their own policy.

`Config.capture_session_vcon` preserves the old default `true`, allowing deployments without a provisioned vCon exporter to explicitly disable the default memory-only capture. This does not repair vCon auto-finalization or provide production retention for the default memory stores.

The `connection_id_budget_usage` and configuration accessors support continuous object-count and RSS qualification. Compatibility mode retains the previous fail-closed connection tombstone behavior for legacy external IDs.

## Compatibility and release review

The new/from_string/as_str, Display, Eq/Hash/Ord and serde string APIs remain. Direct tuple construction and public `.0` mutation of ConnectionId/ConversationId become unavailable, because changing text on a capability would violate its identity. Review this source compatibility change and release version before publishing.

Imported adapter IDs need a local minted ID and bounded wire-to-local mapping. Cross-process gateways must remint authority at the trusted local boundary. Imported conversation creation must likewise use a locally minted ID; existing known conversations remain queryable by their string identity. Legacy compatibility mode is not an indefinite-history reclamation guarantee.

Persistent conversations still need a paged/retained session/message window: a capacity bound alone will eventually reject work if the owner never releases completed history. The current API targets ephemeral call conversations; it does not claim complete long-running storage management for all applications.

## Local qualification

- 270,000 connection lifecycles with a four-row budget and one continuously live survivor; retained rows return to one throughout, IDs increase without wrap/repetition.
- Retirement-before-originate-return, stale tickets and delayed admission decisions, ID replay, foreign-core capability rejection, lookup-terminal callbacks, quarantine timeout cleanup, and capacity reclamation.
- 1,000 closed conversation/session cycles; retired conversation capabilities and string copies cannot create another conversation.
- Concurrent unique ID issuance and serde authority stripping; final queued clone release frees its fence.
- vCon-enabled test proves outstanding audit builders prevent history deletion; disabling unprovisioned capture permits bounded ephemeral cleanup.
- Application recording/drain and handoff tests use the patch; no new production image has been promoted. An existing pinned load campaign continues separately.

See the PR for final per-command counts. These tests do not establish months of provider/media qualification.

## Further audit findings

1. Default MemoryMessageStore/MemoryVconStore and live conversation vectors need byte/count limits, externally durable history and retention.
2. RTP per-SSRC stream/sender-report maps lack an observed per-source cap/idle expiry; bound long-call SSRC churn, including statistics and SRTP contexts. Session teardown bounds ordinary short calls, not an indefinitely long session.
3. RTP transmit packet tracking is naturally capped by 16-bit sequence space (65,536 entries) but should have a smaller feedback age/count window.
4. Existing SIP session authority has separate active/retained bounds and scheduler-driven 64-second fence expiry; do not remove that protocol fence to fix core retention. In-dialog replay queues, RTP jitter queues/pools, packet-loss windows and RTT history have explicit bounds.
5. Application SQLite snapshot/audit catalogs, resource timelines and fleet metadata/alarms need verified archival followed by compaction/retention. Never drop active/replay references or unverified mandatory evidence.
6. Provider transcript/context/audio windows and periodic task ownership still need separate long-call and failure qualification.

Final local results: vCon-enabled core **132/132**, adapter dispatch/race **91/91**, identity factory/serde **2/2**, application carrier **8 passed / 1 provider-dependent ignored**, drain **1/1**, handoff failures **3/3**. Final recording rerun also asserts zero retained connection/session/conversation rows. These counts come from published 0.3.12 dependency builds with the identical source patch.

Additional tenant-management finding: repeating `set_tenant_quotas` while permits are held can inflate semaphore total capacity because it compares the desired cap with available permits. Track configured total capacity, resize by total delta, and add explicit tenant deprovisioning. This patch does not claim to repair that separate setter; the current application worker does not call it.
