# Signaling Performance Architecture

This note describes the data-structure and scheduling choices that keep
`rvoip-sip` signaling lookup, transaction retirement, and retained lifecycle
state bounded under high call churn. It explains the intended benefits and
costs, records the invariants future changes must preserve, and compares the
design with other open-source SIP implementations.

This is an architecture comparison, not a cross-product performance claim.
FreeSWITCH, Kamailio, OpenSIPS, and Asterisk have different process, media,
scripting, and deployment models. A claim that one complete product is faster
requires a controlled, same-shape benchmark using the guidance in
[`BENCHMARKING.md`](BENCHMARKING.md).

## Scope and terminology

The optimizations here apply to several related but distinct owners:

- the transaction manager's active and retained transaction indexes;
- the consolidated terminal-lifecycle scheduler;
- retained client completions and INVITE route tombstones;
- compact Timer J/K tombstones and RFC 6026 Timer M/L Accepted records;
- the generation-protected transaction-timer delivery index; and
- the session lifecycle authority and its anti-reuse fences.

“Manager-owned deadline queue” means that a long-lived manager or authority
owns an ordered deadline index and a wakeable worker. It does not mean that all
SIP timers in the repository use one global queue.

Likewise, “no per-call timer task or retained 128-slot queue” has a deliberately
narrow meaning:

- terminal transaction grace/drain no longer creates a sleeper task for every
  transaction; and
- a compact Timer J/K tombstone or Timer M/L Accepted record deliberately does
  not retain the live runner, timer factory, or per-transaction command queue.

Active transactions can still have a configurable command channel; the bundled
PBX media-server profile currently selects capacity `128`. Optional automatic
`100 Trying` behavior can also arm an INVITE timer task. Those are live
protocol-path choices, not state retained merely to wait out a terminal
deadline.

## Design summary

| Technique | Mechanism | Primary benefit |
| --- | --- | --- |
| Sharded exact-key indexes | `DashMap` indexes for active transactions, retained completions/routes, and exact session generations | Concurrent lookup without a map-wide mutex or a full-table scan |
| One ordered queue per retained-state class | Scheduler-owned `BTreeMap` or authority-owned `BinaryHeap`, with authoritative records kept in their exact-key index | Work scales with scheduled and due records rather than the size of every active table |
| Bounded due and ingress batches | Lifecycle batches are capped at `1,024`; other maintenance paths have their own explicit caps | Prevents a synchronized expiry wave from monopolizing a worker |
| Exact generation/version checks | A deadline carries the generation or version recorded by the authoritative retained entry | A replaced, delayed, or stale deadline cannot remove a newer lifetime |
| Compact retained representations | Timer J/K and RFC 6026 Timer M/L retain only the immutable protocol material still required by the protocol | Releases parsed message trees, progress history, runner state, timer factory, and command queue early |
| Atomic generation-protected timer delivery | One internal timer command carries the exact schedule generation and optional transition | Prevents a partially delivered callback/transition and rejects cancelled or superseded timer work |
| Independent lazy protocol lanes | Timer M/L Accepted retention and Timer J/K teardown retention each use the configured logical bound without eager allocation | An accepted INVITE cannot consume the only slot required by its own BYE while both retained classes remain bounded |
| Wakeable manager-owned workers | New or earlier work notifies the owner, which sleeps until the next deadline | Avoids periodic full scans and one sleeping task per terminal transaction |

The consolidated terminal scheduler documents the historical problem and the
current ownership model in
[`lifecycle_scheduler.rs`](../../sip-dialog/src/transaction/lifecycle_scheduler.rs).
The transaction indexes and retained-client deadline owner are in
[`transaction/manager/mod.rs`](../../sip-dialog/src/transaction/manager/mod.rs),
while exact session-generation indexes and anti-reuse deadlines are in
[`session_lifecycle.rs`](../src/session_lifecycle.rs).

## 1. Sharded exact-key lookup

Hot paths resolve an exact transaction or exact session generation through a
keyed index. They do not scan all active or retained entries. `DashMap` supplies
multiple independently locked shards, and call sites clone the retained `Arc`
out of the shard before awaiting or acquiring object-local state.

The session lifecycle authority separates:

- raw session ID to current generation lookup;
- exact generation-qualified session cells; and
- the mutex-owned admission/anti-reuse index.

That separation keeps ordinary exact-lifetime operations away from the raw-ID
admission mutex while preserving one authoritative reuse fence.

### Benefits

- Expected constant-time lookup by protocol identity.
- Concurrent access across executor workers.
- No map shard guard held across transport I/O or an async wait.
- Cleanup can target a specific generation instead of sweeping a table.

### Costs and risks

- Each shard has buckets, locks, and allocation high-water; too many lightly
  populated maps can waste memory.
- A poor or adversarial key distribution can concentrate contention.
- Updating several indexes is not one atomic map operation. The owning manager
  must preserve publication and removal order.
- Pre-sizing for theoretical capacity can be more expensive than growing from
  observed concurrency. The session lifecycle indexes therefore use a smaller
  warm reserve and reclaim high-water only at a safe idle generation.

## 2. Manager-owned deadline queues

Terminal lifecycle scheduling historically retained one task with two sleeps
per transaction. The consolidated scheduler instead owns ordered queues for
lifecycle phases, compact expiry, standalone compatibility timers, and pending
command delivery. A repeated schedule replaces the exact previous deadline
through a reverse index where that queue supports replacement.

Retained client completions and retired INVITE routes use lean ordered indexes.
Their authoritative `DashMap` values already contain expiry/version metadata,
so the ordered queue avoids duplicating an additional transaction-keyed reverse
map. Session anti-reuse uses a manager-owned min-order `BinaryHeap` equivalent.

### Benefits

- A small, stable number of workers rather than a task per terminal call.
- Fewer Tokio timer entries, task allocations, wakeups, and cancellation paths.
- The next wakeup is derived from the earliest deadline; idle managers do not
  poll every retained entry.
- Work is organized by retained-state semantics, allowing each class to keep
  only the data and retry behavior it requires.

### Costs and risks

- Ordered insertion and removal are `O(log n)`.
- The manager is a coordination point. Large critical sections or slow work in
  the worker would turn it into a bottleneck.
- One worker can introduce head-of-line delay between deadline classes unless
  each class is bounded and the worker promptly yields/requeues.
- A wakeup protocol must handle an earlier deadline arriving while the worker
  sleeps, shutdown, and notifications sent before a waiter is installed.

## 3. Bounded due batches

The consolidated lifecycle scheduler processes no more than `1,024` due
records from a class in one batch. If a class reaches the cap, or retained work
remains immediately due, the worker requeues itself rather than continuing an
unbounded drain. This includes synchronized Timer J/K/M/L expiry; a focused
test exercises a 65,003-entry expiry wave. Compact event delivery also reserves
bounded downstream capacity before removing its authoritative deadline.

This matters when thousands of transactions share Timer J, K, M, L, or a
common grace horizon. Without a cap, one synchronized expiry wave can starve
ingress, ACK/BYE processing, shutdown, or another retained-state class.

### Tradeoff

The cap deliberately trades minimum possible expiry latency for fairness.
Under a deadline storm, records after the first batch expire slightly late.
SIP cleanup and anti-reuse deadlines tolerate late execution; they must never
execute early. Batch size is therefore a scheduling policy, not a protocol
timeout value.

## 4. Generation-qualified stale work

Every retained deadline is qualified by an allocation generation, version, or
complete exact-lifetime key. Expiry removes an authoritative record only when
the current record still has the same identity, expiry, and generation/version.

This makes stale work harmless:

1. generation A schedules a deadline;
2. A is replaced, refreshed, or retired and generation B becomes current;
3. an old A deadline is observed later; and
4. the comparison fails, so B remains untouched.

This is stronger than checking only a raw Call-ID or transaction key, which can
be reused. It also lets some paths use lazy invalidation instead of requiring
perfect synchronous cancellation.

### Costs and risks

- Every mutation must publish the new generation consistently with its
  authoritative state.
- Wrapping counters must avoid using sentinel values and must remain unique
  among live entries.
- Lazy invalidation can temporarily retain stale queue nodes. Queues that do
  not replace the old ordered key require an explicit bound or safe compaction
  policy.
- Tests must cover replacement, cancellation, same-ID reuse, and expiry racing
  with a refresh—not only ordinary timeout behavior.

## 5. Compact retained transaction state

An active transaction needs parsing state, progress history, command delivery,
timers, and state-machine execution. A completed UDP transaction waiting for
Timer J or K does not. Nor does an INVITE transaction in RFC 6026 Accepted
state need to retain its runner and command queue for all of Timer M or L.

The compact representation retains only what the RFC behavior still needs:

- an authenticated exact transaction key and terminal completion for a client
  Timer K duplicate-absorption horizon; or
- immutable final-response bytes and the exact ingress route for server Timer
  J replay;
- serialized request/response material, exact response route, ACK/dialog
  binding, ownership, and terminal-publication authority for server Timer L;
- the exact response route, serialized request, completion/event authority,
  ownership, and post-M policy for client Timer M;
- the capacity/admission lease and exact terminal publication state; and
- expiry plus generation metadata.

It deliberately drops the parsed message tree, transport object, progress
history, runner, timer factory, and command queue. At Timer M, proxy mode
removes the route; endpoint mode promotes the same record in place to the
existing compact late-2xx compatibility horizon without duplicating request or
response storage. This is the basis for the “no retained 128-slot queue”
benefit. It does not describe the live transaction's configurable command
channel.

Timer M/L and Timer J/K share the same scheduler and sharded exact-key table,
but use independent lazy capacity lanes. This distinction is a teardown safety
invariant: with a configured logical capacity of one, one accepted INVITE must
still be able to admit its own BYE transaction. The separation does not reserve
either table eagerly, and diagnostics report record counts and estimated bytes
by timer class.

## 6. Atomic timer delivery

Transition timers are delivered as one indivisible internal command. The
command identifies the timer's exact schedule generation and carries its
optional target state. The transaction runner claims that generation, runs the
timer callback, and applies the transition locally before returning to its
bounded external command channel.

This replaces the former two-message sequence in which a timer notification
could occupy the last channel slot while the separately queued transition was
cancelled or delayed. Cancelling or rescheduling a timer invalidates its old
generation. Once due delivery has begun, merely dropping the timer handle does
not cancel half of an already-fired transition. The public timer-command shape
remains compatible; the generation token is an internal manager/runner
contract.

## Comparison with other SIP implementations

The following comparison was verified against upstream source on
**2026-07-29**. It describes implementation patterns, not measured throughput.
`Partial` means the implementation has a close analogue but not the same
ownership, concurrency, or stale-work rule.

| Technique | FreeSWITCH / Sofia-SIP | Kamailio | OpenSIPS | Asterisk / PJSIP |
| --- | --- | --- | --- | --- |
| Sharded exact-key transaction lookup | Partial: exact hash tables, event-loop-owned rather than concurrently sharded | Yes: transaction hash buckets have independent locks | Yes: bucketed transaction hash with parallel timer sets | Partial: exact transaction hashes protected by a transaction-layer mutex; Asterisk separately hashes work to serializer workers |
| Manager-owned queues by retained-state class | Yes: one NTA agent timer plus incoming/outgoing state queues | Partial: centralized intrusive timing wheel; not one queue per retained class | Close: fixed-duration timer lists, optionally split into parallel timer sets | Partial: one endpoint timer heap shared by timer classes |
| Bounded due work per pass | Yes: explicit retransmit/timeout/termination limits | No explicit per-pass item cap found in the inspected core/TM paths | No explicit per-pass item cap found; detached due lists are drained | Yes: PJSIP caps expired entries per timer-heap poll |
| Generation-qualified stale deadline rejection | No direct equivalent; queue movement, unlinking, flags, and state checks avoid or tolerate stale callbacks | No direct equivalent; active/deleted flags and locks | No direct equivalent; deleted markers and handler locking tolerate delayed callbacks | Partial: timer IDs and copied-entry validation protect cancellation/reuse, but not an authoritative retained-generation comparison |
| No timer task/thread per call | Yes for Sofia NTA deadlines | Yes; transactions embed intrusive timer links | Yes; transactions embed timer links | Yes; transactions embed timer entries in the endpoint heap |

### FreeSWITCH / Sofia-SIP

Sofia-SIP is the closest match to the retained-state queue design. Its NTA
agent contains exact incoming/outgoing hash tables, a single agent timer, and
separate queues such as `trying`, `completed`, `inv_calling`,
`inv_proceeding`, and `inv_completed`. The timer routine calculates the next
deadline and rearms that one timer. It also limits retransmissions and timeouts
processed in a pass.

Sources: [NTA agent tables and queues](https://github.com/freeswitch/sofia-sip/blob/master/libsofia-sip-ua/nta/nta.c#L125-L147),
[single agent timer](https://github.com/freeswitch/sofia-sip/blob/master/libsofia-sip-ua/nta/nta.c#L1261-L1339), and
[bounded incoming timer work](https://github.com/freeswitch/sofia-sip/blob/master/libsofia-sip-ua/nta/nta.c#L7063-L7087).

The important difference is concurrency ownership. Sofia's agent/event loop
does not need a Rust-style concurrently sharded map, and it generally removes
or moves the transaction object itself instead of leaving a copied stale
deadline to be rejected by generation.

### Kamailio

Kamailio's TM table is a strong analogue for sharded exact-key lookup: its hash
entries contain their own mutex and collision list. Its core timer system uses
intrusive timer links, fast/slow processing, and a wheel of slow-timer lists,
so it also avoids a thread or async task per transaction timer.

Sources: [TM hash entries and locks](https://github.com/kamailio/kamailio/blob/master/src/modules/tm/h_table.h#L460-L491),
[embedded TM timer links](https://github.com/kamailio/kamailio/blob/master/src/modules/tm/h_table.h#L123-L155), and
[core slow-timer lists](https://github.com/kamailio/kamailio/blob/master/src/core/timer.c#L92-L108).

Kamailio primarily handles cancellation and delayed callbacks through
intrusive unlinking, active/deleted flags, and transaction locks. The inspected
paths do not use an rvoip-style deadline copy that must match an authoritative
generation, nor an explicit item count that bounds every due bucket drain.

### OpenSIPS

OpenSIPS documents a high-performance “fixed-timer-length” design: separate
lists contain timers with the same duration, allowing append instead of a
time-ordered search. Its timer process detaches expired items while holding the
mutex and invokes handlers after releasing the list lock. Timer structures can
also be divided into parallel sets.

Source: [OpenSIPS TM timer design](https://github.com/OpenSIPS/opensips/blob/master/modules/tm/timer.c#L27-L83).

This is close to manager-owned queues by class, but cancellation semantics are
different. The source explicitly warns that a detached callback can still run
after another process resets it; handlers rely on locks, state, and deleted
markers rather than generation-qualified lazy invalidation. The inspected
timer routines drain their detached due lists without a fixed item cap.

### Asterisk / PJSIP

Asterisk's supported SIP stack is pjproject/PJSIP, normally using Asterisk's
bundled and patched pjproject version. PJSIP creates one timer heap for the SIP
endpoint and sets a maximum number of expired entries per poll. Each SIP
transaction embeds retransmission and timeout timer entries rather than
creating a timer thread or task. Its exact transaction hash tables are guarded
by a transaction-layer mutex, not shard-local locks.

Sources: [Asterisk's pjproject integration](https://docs.asterisk.org/Getting-Started/Installing-Asterisk/Installing-Asterisk-From-Source/Prerequisites/PJSIP-pjproject/),
[PJSIP endpoint timer heap and poll cap](https://github.com/pjsip/pjproject/blob/master/pjsip/src/pjsip/sip_endpoint.c#L512-L527),
[bounded timer polling](https://github.com/pjsip/pjproject/blob/master/pjlib/src/pj/timer.c#L858-L938), and
[transaction hash ownership](https://github.com/pjsip/pjproject/blob/master/pjsip/src/pjsip/sip_transaction.c#L65-L80).

PJSIP timer IDs and copied-entry validation are a useful stale/reuse safety
analogue, but normal cancellation removes the heap entry. It does not perform
the same retained-state generation comparison as rvoip-sip.

## Why the combination matters

None of these techniques is individually novel. Their value comes from making
the complete retained-state path obey the same cost model:

- lookup is keyed rather than scanned;
- terminal waiting is represented as data rather than one task per call;
- retained data is smaller than live transaction data;
- synchronized expiry work is capped;
- stale work is rejected by exact lifetime; and
- every retained class has an explicit owner and capacity boundary.

Removing a sleeper task but retaining a large command queue would preserve much
of the memory cost. Centralizing timers without bounding due work could improve
idle overhead while worsening deadline storms. Sharding lookup without dropping
guards before awaits could still serialize a hot shard. The efficiency benefit
comes from preserving all of the invariants together.

## Evidence and claim boundaries

Use these documents for measured results:

- [`BENCHMARKING.md`](BENCHMARKING.md) defines reproducible scenarios, output,
  and publication rules.
- [`PROFILING.md`](PROFILING.md) explains CPU, wait, allocation, and retained
  state investigation.
- [`CARRIER_BURST_TUNING.md`](CARRIER_BURST_TUNING.md) is the experiment ledger,
  including rejected timer-consolidation attempts and retained-object/RSS
  evidence.
- [`BETA_PERFORMANCE_REPORT.md`](BETA_PERFORMANCE_REPORT.md) records the current
  release-level performance claim boundaries.

Architecture explains why a result is plausible; it is not itself benchmark
evidence. In particular, do not infer whole-product superiority from the table
above, and do not compare CPS or RSS values collected with different media,
transport, call-duration, logging, or hardware shapes.

## Maintenance checklist

Changes to signaling lifecycle or retention should preserve all of the
following:

1. Hot lookup uses the complete protocol or generation-qualified key.
2. No map guard or manager queue lock is held across an async wait or transport
   operation.
3. A retained deadline names the exact authoritative generation/version it may
   remove.
4. A due loop has a documented batch or fairness bound.
5. Downstream backpressure is reserved before authoritative retained work is
   discarded.
6. Retained compact state does not accidentally reacquire the live runner,
   parsed request, timer factory, or command queue.
7. Capacity accounting spans active and retained representations without
   double-counting or releasing the lease early.
8. Same-ID reuse and stale-deadline race tests accompany lifecycle changes.
9. External comparison statements remain dated, source-linked, and separate
   from measured rvoip performance claims.
