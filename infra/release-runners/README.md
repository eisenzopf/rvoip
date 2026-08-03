# Ephemeral GCP qualification pilot

The `GCP qualification pilot` workflow is a non-publishing bridge between
GitHub-hosted orchestration and an ephemeral Google Compute Engine worker. It
uses GitHub OIDC, creates exactly one worker, runs a fixed test profile, stores
the result in the evidence bucket, and deletes the worker and its disk.

The pilot intentionally accepts no arbitrary command. Fork pull requests
cannot invoke it, and the workload identity provider restricts authentication
to the exact repository, workflow, and protected ref configured in GCP.

`workspace` runs the 44-package audit, automation tests, workspace unit and
integration tests, binaries, examples, doctests, and Clippy. `smoke` substitutes
an all-target WebRTC dependency check for the full workspace suite. Neither
profile publishes crates, creates a tag, or creates a GitHub release.

This older pilot does not claim coverage for the external PBX, long-soak, or
performance gates. Those gates now have structured replacements in the
protected `remote-release` profile; its coverage ledger still fails closed if
any canonical gate becomes unmapped.

The protected `Release qualification` workflow uses the same ephemeral model
for the complete gate catalog. Its `remote-preflight` profile launches the full
release capacity shape—six `n2-standard-8` short-performance workers, two
`n2-standard-8` long-soak workers, seven `n2-standard-4` burst/soak workers,
and one `n2-standard-4` interoperability worker—but runs short infrastructure probes. That makes controller, quota,
startup, OS-limit, dependency, evidence-transfer, and cleanup defects visible
before a full qualification begins. It is non-publishing and cannot qualify a
release.

Its `remote-diagnostic` profile accepts only named executable GCP gates already
present in `remote-release`. It expands their dependency closure and uses the
same machines, startup path, commands, workloads, thresholds, immutable
evidence, and cleanup. It cannot publish or qualify a release. A later complete
qualification can combine exact receipts from up to five prior runs, avoiding
a full rerun after a corrected or transient isolated failure.

For both preflight and the complete `remote-release` profile, a single GitHub
controller launches every duration-balanced GCP shard concurrently rather than
consuming one GitHub job slot per cloud worker. Each worker is bound to one
candidate SHA and gate list, uploads an immutable result and evidence archive,
shuts down, and is deleted with its auto-delete disk. Controller and follow-up
sweep cleanup both run on failure; there is no idle release fleet.

Release builders and workers use N2 machines with an `Intel Cascade Lake`
minimum CPU platform. Every worker keeps a 262,144 descriptor limit, sets
64 MiB receive and send UDP ceilings, and proves an 8 MiB SIP socket-buffer
request with `getsockopt` before load. Performance evidence records the actual
CPU model, process limits, file-table state, port ranges, UDP memory limits,
pressure/swap state, and Linux `/proc` UDP, softnet, socket, and loopback-drop
counters. Release burst gates fail closed when mandatory Linux counters are
missing or when receive-buffer, send-buffer, softnet, or loopback drops rise
during the measured scenario.

For diagnostics and `remote-release` runs that select executable performance
gates, the controller first creates one ephemeral `n2-standard-32` builder. It
compiles the exact candidate once, packages only the selected test executables,
uploads a SHA-256-bound bundle, and is deleted before the measurement fleet is
created. A rerun of the identical candidate, environment, and selected gate set
reuses that finished bundle from GCS and does not create the builder. Cache
objects are content-addressed; the controller and workers recheck the cache
key, result, manifest, bundle, and executable hashes before use. Runtime workers
still use their catalogued real GCP machine types,
workloads, durations, and thresholds; they verify the bundle and executable
hashes instead of recompiling the same graph. This makes compilation a shared
setup phase without contaminating performance or soak measurements.

The evidence bucket lifecycle deletes only the `release-cache/` prefix after
14 days. Run-scoped receipts, logs, and release evidence use different prefixes
and are not covered by that transient-build-cache rule.

The release-runner service account requires both
`roles/storage.objectCreator` and `roles/storage.objectViewer` on the evidence
bucket. Creator access stores immutable receipts and logs; viewer access lets
runtime workers download the exact-candidate performance bundle. The builder
performs an authenticated manifest read-back before its result can pass, so a
missing viewer grant fails during shared setup rather than after measurement
workers are provisioned.
