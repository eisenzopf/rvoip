# Unified Workspace Release

All 44 publishable workspace crates use `[workspace.package].version` and ship
together. `scripts/release.sh` is the only release authority; it discovers the
package graph from Cargo metadata and publishes normal/build dependencies
before their dependents.

Feature maturity remains independent of package version. In particular, the
SIP surface has release-gated beta evidence while other optional surfaces may
remain experimental.

## Prepare

Use the **Prepare release PR** workflow with the next version, for example
`0.3.6`. It checks out the current `main`, runs the unified preparation and
release-tooling tests, and opens a draft release pull request. The release PR
must pass the normal `PR Gate`; it is never pushed directly to `main`.

The underlying `scripts/release.sh prepare` command rejects unstable SemVer
strings, version downgrades, versions already present on crates.io, dirty
trees, missing internal dependency versions, and a workspace inventory other
than the expected 44 publishable packages. It updates package inheritance and
the lockfile transactionally.

## Verify

After the preparation PR merges, run **Release qualification** for that exact
`main` commit. After release-orchestration changes merge, use
`remote-preflight` first. It launches the same 18-worker, 100-vCPU GCP shape as
a full run, but
executes short infrastructure probes so credentials, quota, VM startup, OS
limits, tool installation, repository checkout, GCS evidence transfer,
controller reconciliation, and cleanup fail within a target of 15 minutes.
The preflight is deliberately non-publishing and is not release evidence.

Use `remote-core` for a hosted-runner dry run and `remote-release` for the
complete release profile. Do not start the full profile unless the exact
release machinery has a recent successful preflight. The first candidate
should set `first_candidate=true`; after a fix, provide the prior qualification
run and the previous candidate SHA, and set `first_candidate=false`, so exact
matching evidence can be reused while failed and affected gates are rerun.
Diagnose and reproduce a failed gate by itself before spending another complete
qualification run.

For a real GCP performance, soak, or interoperability failure, dispatch
`remote-diagnostic` on protected `main` and enter one or more exact catalog gate
IDs in `diagnostic_gates`, separated by commas. The planner accepts only
executable GCP gates from `remote-release`, adds their declared dependencies,
and forces them to run fresh with their release commands, machines, workloads,
and thresholds. At most 20 gates may be requested. The resulting profile is
non-publishing and cannot qualify a release.

The next complete qualification may combine up to five evidence runs by
entering their comma-separated IDs in `prior_run_id`. This lets it consume the
successful receipts from a failed full run plus corrected diagnostic runs,
while still rerunning anything failed, changed, stale, missing, or
digest-mismatched. Candidate inventory and final aggregate receipts are always
regenerated. Use `changed_since` for the previous candidate SHA after a code
fix; unknown mappings still invalidate the full profile.

The workflow produces a candidate-bound plan, per-gate receipts, and one
aggregate qualification receipt. A gate can be reused only when its source,
transitive workspace dependency closure, gate definition, toolchain/container,
resource class, and thresholds have identical SHA-256 inputs. Unknown changes
fail closed to a full run. The aggregate is signed as a release evidence
artifact and is the only qualification input accepted by protected
publication.

The release verifier also checks the catalog's 108-gate coverage ledger. It
rejects a publication aggregate while any legacy gate lacks a structured remote
replacement, even if the selected remote profile itself passes. This prevents
the migration from silently reducing release coverage; the existing beta
wrapper remains available for that compatibility work.

The local command remains available for diagnosis or emergency recovery, but
it is not the normal release authority:

```sh
scripts/release.sh audit
scripts/release.sh verify --version X.Y.Z \
  --remote-qualification /path/to/aggregate.json
```

Verification still validates the unified workspace metadata, exact 44-crate
package inventory, package file manifests, and registry-resolvable archive
hashes. Before first publication, Cargo cannot build a dependent `.crate`
archive until that crate's new internal dependency version is visible on
crates.io, so archive hashes are completed during the topological publication
run.

The `remote-core` profile uses GitHub-hosted runners. The complete
`remote-release` profile sends performance, soak, and PBX/SIPp gates to real
ephemeral Compute Engine workers. One GitHub controller creates all planned
workers concurrently through workload identity, monitors their immutable GCS
results, verifies and merges their evidence, and deletes every instance and
auto-delete disk. A separate cleanup job sweeps interrupted runs. The workers
never receive the crates.io token and no release worker remains provisioned
between qualifications.

The current full profile is balanced across six `n2-standard-8` short-performance
workers, two `n2-standard-8` one-hour-soak workers, seven `n2-standard-4`
burst/soak workers, one `n2-standard-4` stateful interoperability worker, and
two `n2-standard-2` proxy-interoperability workers. Each proxy worker runs six
of the twelve required peer/order/transport rows. The two
long soaks receive the additional cores for the measured workload; the total
runtime shape is 100 N2 vCPUs. These are real
performance machines; the workflow
does not substitute GitHub-hosted capacity or reduce workloads and thresholds.
Before creating that runtime shape, the controller uses one ephemeral
`n2-standard-32` builder with balanced persistent disk to compile the selected
performance executables exactly once. The builder uploads a candidate-, source
tree-, toolchain-, and environment-bound bundle, then is deleted before the
measurement workers start. Every performance worker verifies the bundle and
each executable by SHA-256 and records both bundle and manifest digests in its
gate receipt. Compilation therefore cannot perturb performance measurements or
be repeated independently on every worker.
The `remote-preflight` profile recreates that complete capacity shape, including
all 18 concurrent VM creations, but its short probes never substitute for the
real performance, interoperability, and soak commands in `remote-release`.
The one-hour soak establishes a physical lower bound of one hour for a fresh
qualification, plus short provisioning and evidence overhead. The shared build
stage is intentionally outside the measured workloads. Its finished bundle is
reused across workflow reruns only when the exact candidate SHA, release
environment, and selected performance gate set produce the same cache key. A
hit avoids the approximately full-LTO build delay without shortening or
dividing the continuous soak requirement. Any missing object or cache-key,
manifest, bundle, or executable digest mismatch fails closed. Gates
whose exact source, dependency, definition, environment, and threshold digests
remain unchanged may reuse successful prior evidence on a later candidate.
Each proxy row has its own stable gate ID, so a later diagnostic can rerun only
the failed combination without rerunning the other eleven rows.

The shared builder and GCP workers use a verified, pinned `sccache` binary and a private,
lifecycle-managed GCS compiler-cache bucket. The cache is content-addressed,
contains no release credentials, and is shared only by trusted protected-main
or same-repository diagnostic workflows. Build processes record cache
statistics in their evidence. A cache
download or backend failure falls back to direct compilation without changing
the command, workload, machine class, or acceptance threshold. Hosted release
checks are balanced across twelve standard shards; together with five nightly
shards, one evidence shard, and the one GCP controller, the workflow remains
below the repository's twenty-job concurrency ceiling.

This verification is the version/package delta boundary. It does not claim
that a prior beta run exercised a later version-only commit.

### Approved beta carry-forward verification

For the owner-approved `0.3.4` release, verification may consume the dedicated
carry-forward attestation instead of a newly completed full beta report:

```sh
scripts/release.sh verify --version 0.3.4 \
  --beta-carry-forward-attestation \
  /path/to/0.3.4/carry-forward-attestation.json
```

The `rvoip-release-carry-forward-attestation-v1` verifier is limited to
`0.3.4`. It requires a clean exact release commit and source fingerprint,
`v0.3.3` as the delta base, the unchanged SHA-256-bound `0.3.2` exception, and
one fresh canonical 2,000-CPS/65,000-call real-media PASS. The canonical
evidence is copied in full and hash-manifested; its persisted acceptance,
performance audit, cleanup convergence, source binding, executable hash, and
evidence-tree hash are independently rechecked.

This mode still runs the normal current workspace compile, library, target,
integration, example, doctest, and 44-package verification. Its receipt says
`OWNER-APPROVED-CARRY-FORWARD` and `NOT-RERUN`; it cannot label the inherited
`0.3.2` evidence as a current beta PASS. The `0.3.4` full beta,
interoperability matrix, and long soaks remain explicitly `NOT-RERUN`.

### Approved targeted-delta verification

For an owner-approved, narrowly scoped vCon patch, verification can consume a
commit-bound targeted-delta attestation instead of rerunning the full SIP beta
suite and broad workspace test/doc suites:

```sh
scripts/release.sh verify --version X.Y.Z \
  --targeted-delta-attestation /path/to/targeted-delta.json
```

This mode still validates the unified workspace manifest, runs
`cargo check --workspace --all-targets --locked`, and produces every package
file manifest and every registry-resolvable archive hash. It then reruns this
fixed targeted matrix:

- `vcon-all-targets`: `cargo test -p rvoip-vcon --all-targets --locked`
- `core-vcon-lib`: `cargo test -p rvoip-core --lib vcon --locked`
- `core-vcon-emission`: `cargo test -p rvoip-core --test vcon_emission --locked`
- `core-no-default-features`: `cargo check -p rvoip-core --no-default-features --all-targets --locked`
- `core-all-features`: `cargo check -p rvoip-core --all-features --all-targets --locked`
- `quic-e2e-full-stack`: `cargo test -p rvoip-quic --test e2e_full_stack --locked`
- `facade-voip-3`: `cargo check -p rvoip --features voip-3 --locked`
- `ai-harness-example`: `cargo check --manifest-path examples/11-ai-harness-demo/Cargo.toml`
- `release-unit-tests`: `python3 -m unittest scripts/test_release.py`

The live
`cargo test -p rvoip-vcon-postgres --all-targets --features core-store,live-tests --locked`
result (`postgres-core-store-live`) is not rerun by this command. It must
instead be supplied as hash-bound JSON evidence with
schema `rvoip-vcon-postgres-live-evidence-v1`, the exact release commit, the
exact approved Cargo argv, exit status `0`, `live_database: true`, a PostgreSQL
server version, `ephemeral_database: true`, an environment object containing
the provider, image, database name, and unique run identifier (never
credentials), and a timezone-aware `recorded_at`.

The attestation schema is `rvoip-targeted-delta-attestation-v1`. It must bind
the exact release version and commit, the vCon schema commit
`2342aba64bdb71d9e80ab6e274a3921e2b1c769e`, an existing ancestor base commit,
the exact changed-path list, all named commands with their exact argv/commit
and exit status `0`, the live PostgreSQL evidence path and SHA-256, and
timezone-aware owner approval metadata. For 0.3.3, the base commit must resolve
exactly from the immutable `v0.3.2` tag; a later arbitrary ancestor cannot hide
out-of-scope changes. The release tool checks the declared
path list against the Git diff and against its hard-coded vCon-only policy;
self-declaring an unrelated SIP, media, or workspace path does not authorize
it. The resulting receipt copies the ephemeral PostgreSQL environment so the
targeted qualification cannot be mistaken for a test against an unspecified
or persistent database.

The verification inputs `--beta-report-root`, `--beta-exception-attestation`,
`--beta-carry-forward-attestation`, `--targeted-delta-attestation`, and
`--remote-qualification` are mutually exclusive. A targeted receipt says
`NOT-RERUN` for the beta, broad workspace tests, and doctests; it never
relabels them as passing. Receipt schema v4 records the attestation hash,
targeted commands, PostgreSQL evidence hash, and the manifest/compile/package
checks that did run.

## Publish

Use the protected **Publish protected release** workflow and provide the
successful remote-release qualification run ID. Its default is a
non-publishing dry run:

```sh
scripts/release.sh publish --version X.Y.Z
```

The workflow resolves and verifies the signed aggregate, checks that it binds
the current `main` commit and current gate catalog, and runs the complete
topological package preflight. The crates.io token exists only in the
approval-protected `release-publish` environment and is never available to PR,
nightly, or performance runners.

After environment approval, set `execute=true` in the workflow to publish:

```sh
scripts/release.sh publish --version X.Y.Z --execute
```

Publication requires a clean `main` equal to `origin/main`, the matching
verification receipt, and an unused `vX.Y.Z` tag. Each crate is packaged and
dry-run immediately before publication, and its file manifest must match the
verified receipt. The tool records the final archive hash, waits for each
version to become visible on crates.io, and only then packages and publishes
dependents.

An interrupted run is resumable. A version already on crates.io is skipped
only when its registry checksum matches the locally verified `.crate` artifact;
any mismatch fails closed.

After all 44 versions are visible, the workflow creates the protected
annotated tag and GitHub release with generated notes listing merged PRs by
release-note label. It refuses to tag or release a partial crates.io
publication. An interrupted publication is resumable; an existing version is
skipped only when its registry checksum matches the locally verified archive.

## Deprecated Entrypoints

- `scripts/bump_version.sh X.Y.Z` forwards to unified preparation.
- `scripts/publish_train.sh --audit` forwards to unified audit.
- Alpha/beta `--train` publication fails closed. Split release trains are no
  longer supported.
