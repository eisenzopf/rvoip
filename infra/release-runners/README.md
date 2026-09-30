# Ephemeral EC2 release workers

The protected `Release qualification` workflow runs the complete gate catalog
on ephemeral EC2 instances in the Vapi AWS account (`us-west-2`). A single
GitHub-hosted controller assumes the provisioner role through GitHub OIDC,
plans duration-balanced shards, creates every worker concurrently with
`run-instances`, waits for each worker's immutable result in S3, verifies and
merges the evidence, and terminates the fleet. There is no idle release fleet
and no long-lived cloud credential in GitHub. The design record is
[`docs/AWS_RELEASE_WORKERS.md`](../../docs/AWS_RELEASE_WORKERS.md); this
directory holds the scripts that run on the instances.

| File | Runs on | Purpose |
| --- | --- | --- |
| `aws-release-startup.sh` | every gate worker | installs the toolchain, checks out the exact candidate, runs the shard's gates, uploads evidence, halts |
| `aws-release-shutdown.sh` | every gate worker | `ExecStop` checkpoint that commits a `PARTIAL` result when the controller stops a worker early |
| `aws-performance-prebuild-startup.sh` | the performance builder | compiles the selected performance executables once and publishes a content-addressed bundle |
| `release-infrastructure-preflight.sh` | every gate worker | proves the host contract (CPU, memory, disk, limits, UDP ceilings, tools) and records it as evidence |
| `interop-lifecycle.sh`, `pbx/` | interoperability workers | external PBX fixtures for the interop gates |

## Worker lifecycle

The controller builds each instance's user-data with
`scripts/release/aws_fanout.py user-data`. User-data writes
`/etc/rvoip-release.env` (mode 0600) with the worker parameters, installs the
reviewed scripts from the controller's checkout under
`/usr/local/lib/rvoip-release/`, installs and starts
`rvoip-release-shutdown.service`, and runs the startup script. The scripts
read their parameters only from that env file; nothing is fetched from
instance metadata beyond the credentials the instance role provides.

The startup script installs the pinned AWS CLI v2 before anything else and
verifies the archive's detached PGP signature against the AWS CLI Team key
embedded in the script. It then installs the apt package set, Rust 1.91.0
with `lld`, and a verified `sccache`, clones the exact candidate, and runs
`scripts/release/gates.py run-shard`. An `EXIT` trap packages the evidence
directory, writes `result.json`, and uploads all three objects with the
instance role:

```
s3://EVIDENCE/release/<run>-<attempt>/<shard>/result.json
s3://EVIDENCE/release/<run>-<attempt>/<shard>/release-shard.tar.gz
s3://EVIDENCE/release/<run>-<attempt>/<shard>/qualification.log
```

Each worker is bound to one candidate SHA and gate list. Instances are
created with `InstanceInitiatedShutdownBehavior=terminate` and a
delete-on-termination root volume, so the worker's own `shutdown -h now`
after the upload is its termination and disk release. The controller treats
`stopping`, `stopped`, `shutting-down`, and `terminated` as "worker finished".

`rvoip-release-shutdown.service` is a oneshot unit whose `ExecStop` runs
`aws-release-shutdown.sh` during an ACPI stop, which is what a controller
`stop-instances` delivers when it applies the early-failure cutoff to deferred
workers. The checkpoint takes the same result lock the startup trap uses,
exits if a final `result.json` already exists, and otherwise snapshots the
gate receipts paid for so far as a `PARTIAL` result with the completed gate
ids. A final result always wins; a partial result is never overwritten by a
racing `EXIT` trap.

Cleanup runs in three layers: worker self-termination after evidence upload,
controller `terminate-instances` on `always()`, and a scheduled janitor. The
janitor is an EventBridge schedule that invokes a Lambda every 30 minutes to
terminate any `managed-by=github-actions` instance whose `rvoip-expires-at`
tag is in the past; controllers set that tag to four hours after creation.
`cleanup-aws` additionally sweeps instances tagged with the run and fails if
any remain.

## Machine classes

Machine classes are policy values recorded in every attestation. The `m5`
family is the same generation as the N2 workers it replaces (Skylake-SP or
Cascade Lake, 4 GB per vCPU); AWS does not expose a minimum CPU platform, so
every worker records the actual CPU model in its evidence.

| Resource class | Instance type | vCPU | Memory | Root volume |
| --- | --- | ---: | ---: | --- |
| `ec2-performance` | `m5.4xlarge` | 16 | 64 GB | 200 GB gp3 |
| `ec2-performance-soak` | `m5.xlarge` | 4 | 16 GB | 200 GB gp3 |
| `ec2-performance-soak-long` | `m5.4xlarge` | 16 | 64 GB | 200 GB gp3 |
| `ec2-interop` | `m5.4xlarge` | 16 | 64 GB | 200 GB gp3 |
| `ec2-proxy-interop` | `m5.large` | 2 | 8 GB | 100 GB gp3 |
| performance prebuilder | `m5.8xlarge` | 32 | 128 GB | 200 GB gp3 |

The full `remote-release` shape is six short-performance workers, three
long-soak workers, seven burst/soak workers, one interoperability worker, and
two proxy-interoperability workers (192 vCPUs concurrently), plus the 32-vCPU
builder, which the interop and proxy-interop workers run alongside and which
is terminated before the performance workers start. `preflight-aws`
reads the On-Demand Standard vCPU quota and current usage before creating
anything and fails closed if peak demand does not fit.

## Profiles

`remote-preflight` launches the full release capacity shape but runs short
infrastructure probes, so controller, quota, startup, OS-limit, dependency,
evidence-transfer, and cleanup defects are visible before a full
qualification begins. It is non-publishing and cannot qualify a release.

`remote-diagnostic` accepts only named executable gates already present in
`remote-release` or `remote-preflight`. It expands their dependency closure
and uses the same machines, startup path, commands, workloads, thresholds,
immutable evidence, and cleanup, so one infrastructure shape can be checked
without provisioning the complete fleet. It cannot publish or qualify a
release. A later complete qualification can combine exact receipts from up to
five prior runs, avoiding a full rerun after a corrected or transient isolated
failure.

`remote-release` runs the complete gate catalog. No worker profile publishes
crates, creates a tag, or creates a GitHub release; every result records
`"publishing_attempted": false`.

## Host limits and UDP ceilings

The stock Ubuntu boot path inherits a soft descriptor limit of 1024. Every
worker raises it to 262,144 and fails closed if the image cannot provide it,
sets 64 MiB `net.core.rmem_max` and `net.core.wmem_max` ceilings, and proves an
8 MiB SIP socket-buffer request with `getsockopt` before load. The preflight
records the actual CPU model, process limits, file-table state, port ranges,
UDP memory limits, pressure/swap state, and Linux `/proc` UDP, softnet, socket,
and loopback-drop counters. Release burst gates fail closed when mandatory
Linux counters are missing or when receive-buffer, send-buffer, softnet, or
loopback drops rise during the measured scenario.

## Prebuilt performance bundle

For diagnostics and `remote-release` runs that select executable performance
gates, the controller first creates one ephemeral `m5.8xlarge` builder. It
compiles the exact candidate once, packages only the selected test
executables, publishes a SHA-256-bound bundle and manifest under
`s3://EVIDENCE/release-cache/performance-prebuilt-v1/<cache-key>/`, and is
terminated before the measurement fleet is created. A rerun of the identical
candidate, environment, and selected gate set reuses that finished bundle from
S3 and does not create the builder.

Cache objects are content-addressed. The builder reads back any object that
already exists at a digest path and verifies its bytes rather than writing a
second version; the controller and workers recheck the cache key, result,
manifest, bundle, and executable hashes before use. A performance worker
accepts a bundle only from its own run's `prebuild/` prefix or from a
content-addressed cache path whose file name is the expected digest, and
`prebuilt_performance.py install-bundle` verifies the archive digest and the
exact candidate before any executable runs. Runtime workers still use their
catalogued instance types, workloads, durations, and thresholds; they verify
hashes instead of recompiling the same graph, which makes compilation a shared
setup phase without contaminating performance or soak measurements.

## Compiler cache

Builders and workers use `sccache` with the S3 backend
(`SCCACHE_BUCKET`, `SCCACHE_REGION`, `SCCACHE_S3_KEY_PREFIX`) against the
dedicated cache bucket, authenticated by the instance role. The cache is an
optimisation only: a failed `sccache` download, digest mismatch, or backend
start failure falls back to direct `rustc`, so release correctness never
depends on cached state. The cache bucket expires objects after 30 days.

## Evidence bucket lifecycle and runner role

The evidence bucket is versioned, private, and SSE-S3 encrypted. Its lifecycle
rule expires only the `release-cache/` prefix after 14 days. Run-scoped
receipts, logs, and release evidence use different prefixes and are durable.

The runner instance role (`rvoip-release-runner`) needs both put and get on
the evidence bucket: put stores immutable receipts, logs, and bundles; get lets
runtime workers download the exact-candidate performance bundle. It also needs
read/write on the compiler cache bucket and has no EC2 permissions. The
builder performs an authenticated manifest read-back before its result can
pass, so a missing get grant fails during shared setup rather than after the
measurement fleet is provisioned. The provisioner role the controller assumes
can read both buckets but cannot write evidence.
