# Stateful Proxy Conformance Status

This file records provenance and gate status for the coordinated
`rvoip-sip-proxy` conformance work. It is a progress record, not a
release certificate.

## Isolated baseline

| Field | Value |
|---|---|
| Source repository | `/Users/jonathan/Developer/rvoip` |
| Source ref at worktree creation | `main` |
| Reviewed source commit | `f2f29d0e110bc9b3d6f83aa5e8f131398a2dce0a` |
| Commit tree | `fe8f80fa042e3106b5dc19bcc9bf1adb650317e5` |
| Tracked-tree manifest SHA-256 | `c5fad12a09616c507eccb812a977cbed778c2de30586ac6677635cba17baf3bb` |
| Isolated worktree | `/Users/jonathan/Developer/rvoip-sip-proxy-conformance` |
| Isolated branch | `codex/sip-proxy-conformance` |
| Initial isolated worktree state | Clean |
| Workspace version at baseline | `0.3.1` |
| Integration base | Published `v0.3.3` (`b8f2b78a321cce89a855819d0319b309c669ea88`) |
| Planned behavioral release | Coordinated `0.3.4`, after owner review and qualification |

The manifest fingerprint was produced from the reviewed commit with:

```sh
git ls-tree -r --full-tree f2f29d0e110bc9b3d6f83aa5e8f131398a2dce0a \
  | shasum -a 256
```

The original development worktree was intentionally left untouched.
The clean worktree isolates this conformance effort from unrelated
uncommitted changes. The isolated worktree is expected to become dirty
while gates are active; its current dirty state must not be confused
with the clean starting point recorded above.

## Claim boundary

- Published `0.3.3` and the current development candidate are described
  as a **partial stateful-proxy implementation**.
- No complete RFC conformance claim is made.
- The eventual claim, if qualified, is limited to the rows marked
  applicable in [`RFC3261_CONFORMANCE.md`](RFC3261_CONFORMANCE.md).
- The transaction state machines are supplied by the exact
  `rvoip-sip-dialog` dependency and are part of the qualification
  boundary.
- Because the coordinated release is `0.3.4`, every published crate must pass
  source-compatibility comparison with `0.3.3`. RFC 6026 `Accepted` state is
  internal protocol state; the exhaustive public `TransactionState` and
  `TransactionEvent` shapes must remain compatible.
- Bridgefu remains a B2BUA. Its cross-leg cancellation fix is separate
  from this proxy profile.

## Gate status

| Plan gate | State | Evidence or remaining work |
|---|---|---|
| Gate 0 — baseline and claims | Active | Clean source provenance is recorded; broad claims are downgraded; the normative matrix exists. Candidate defect tests and their final revision-pinned run still need consolidation. |
| Gate 1 — Bridgefu cancellation | Separate repository | Not evidence for proxy conformance. |
| Gate 2 — admission terminal signal | Separate rvoip API track | Additive lifecycle work; not evidence for proxy conformance. |
| Gate 3 — proxy CANCEL | Focused candidate green | Matched/unmatched, duplicate, pre-provisional latch, generated-CANCEL lifecycle, retransmission replay after target-INVITE retirement, listener-auth no-challenge behavior, normalized branch/sent-by matching, authenticated peer/flow isolation, ambiguous-write ownership, exact UDP/TCP/TLS next-hop reuse, and CANCEL/2xx race tests pass. Real independent-peer and final beta evidence remain Gate 6/Gate 7 requirements. |
| Gate 4 — RFC 6026 and response contexts | Focused candidate green | INVITE client/server `Accepted` behavior is private protocol state, preserving the `0.3.1` public enum while retaining Timer M/L at exactly `64*T1`, duplicate/distinct 2xx delivery, retransmitted-INVITE absorption, ACK-to-TU, transport-error retention, proxy-mode cache isolation, RFC 4320 method-aware response handling, and post-Timer-L stray-response discard. The full proxy suite and patch-version semver comparison pass; independent-peer and final beta evidence remain open. |
| Gate 5 — ACK, Timer C, routing | Active | Candidate ACK and Timer C paths exist. The strict retention drain exposed and now has a focused correction for ownerless transport metadata created by transactionless ACKs; eight distinct TLS ACKs leave zero retained transport entries, all 648 `rvoip-sip-dialog` library tests pass, and the complete `rvoip-sip-proxy` test suite passes. Route-set tests now preserve `lr` and `transport` as URI parameters. Final Route/Record-Route, SIPS, RFC 3263, exact-flow, and real-peer cleanup evidence remains. |
| Gate 6 — conformance/interoperability | Active | Raw UDP/TCP and verified TLS/SIPS loopback coverage is implemented and has passed on an intermediate dirty revision. On the current stable development source, bounded real-peer diagnostics pass 10/10 counted core-and-cleanup scenarios for Kamailio in the rvoip-first UDP order and 10/10 for OpenSIPS in the peer-first UDP order. A frozen OpenSIPS peer-first TLS row then completed all functional scenarios but correctly failed the 130-second retention fence on five orphaned ACK transport-context records; those five records matched the five ACK-producing packet captures exactly and cleared only at shutdown. The transaction-manager correction and focused regression are green, but that real row has not yet been rerun. Each row records real external traversal, packet-bound Via evidence, raw matched-before/after, duplicate and unmatched CANCEL behavior, INVITE/ACK/BYE, response routing, exact MESSAGE bodies, an unchanged proxy binary, and clean process/port convergence. Earlier runs exposed and led to corrections for SIPp CANCEL modeling, an INVITE-487 Via fixture error, provisional-response timing, body-length evidence, dialog target preservation, TLS audit attribution, and harness fail-open/stale-artifact risks; they did not establish proxy-core failures. The short rows deliberately used zero retention drain and omitted advanced scenarios, so they are development evidence rather than release evidence. The real-peer release harness pins both peers and requires both adjacency orders, UDP/TCP/verified TLS, packet assertions, a 130-second retention drain, and runtime leak detection. Every scenario counted for a peer must exercise that external peer; in-process tests are supplemental only. Advanced external fork, late/multiple-2xx, Timer C, routing, authentication, failover, transport-failure, and overload drivers plus final clean revision-pinned evidence remain open. |
| Gate 7 — performance/beta | Pending | Three canonical performance runs, complete beta gate, approved soaks, and retention drain are not complete for the release revision. |
| Gate 8 — integration/release | Pending owner authorization | No publication, push, or production deployment is authorized by this status record. |

## Evidence policy

A conformance row changes to green only when its evidence record
contains:

1. the exact commit and tree;
2. a clean/dirty source status and diff fingerprint when dirty;
3. the complete command, features, runtime switches, and tool versions;
4. the test result and retained logs or packet capture;
5. the transport and peer implementation used; and
6. a link from the matrix row to that immutable artifact.

An intermediate successful run may guide development, but it does not
qualify a later source revision. Skipped, ignored, or unavailable
interop cases remain pending and must be reported as such.
