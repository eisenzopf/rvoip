# Unified Workspace Release

All 44 publishable workspace crates use `[workspace.package].version` and ship
together. `scripts/release.sh` is the only release authority; it discovers the
package graph from Cargo metadata and publishes normal/build dependencies
before their dependents.

Feature maturity remains independent of package version. In particular, the
SIP surface has release-gated beta evidence while other optional surfaces may
remain experimental.

## Prepare

Begin from a clean branch based on current `origin/main`:

```sh
scripts/release.sh audit
scripts/release.sh prepare --version X.Y.Z
git diff
git add Cargo.toml Cargo.lock crates
git commit -m "release: unify workspace at X.Y.Z"
git push origin HEAD:main
```

`prepare` rejects unstable SemVer strings, version downgrades, versions already
present on crates.io, dirty trees, missing internal dependency versions, and a
workspace inventory other than the expected 44 publishable packages. It
updates package inheritance and the lockfile transactionally and runs the
workspace all-target check.

## Verify

Switch to `main` and require it to equal the current remote:

```sh
git switch main
git pull --ff-only origin main
scripts/release.sh verify --version X.Y.Z \
  --beta-report-root /path/to/the/verified/beta-report
```

Verification checks the tracked beta reporting attestation, workspace metadata,
all targets, unit tests, integration/example targets, doctests, and the exact
packaged file manifest for every crate. Before first publication, Cargo cannot
build a dependent `.crate` archive until that crate's new internal dependency
version is visible on crates.io, so verification hashes archives only where
the target-version registry graph is already resolvable. Its receipt under
`target/release-logs/X.Y.Z/verification.json` binds the exact Git commit,
ordered 44-crate graph, all 44 file-manifest hashes, and every archive hash
available before publication.

This verification is the version/package delta boundary. It does not claim
that a prior beta run exercised a later version-only commit.

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

The three verification inputs `--beta-report-root`,
`--beta-exception-attestation`, and `--targeted-delta-attestation` are mutually
exclusive. A targeted receipt says `NOT-RERUN` for the beta, broad workspace
tests, and doctests; it never relabels them as passing. Receipt schema v4
records the attestation hash, targeted commands, PostgreSQL evidence hash, and
the manifest/compile/package checks that did run.

## Publish

The default is non-publishing:

```sh
scripts/release.sh publish --version X.Y.Z
```

Review the dry-run log, authenticate Cargo, and then execute:

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

After all 44 versions are visible, create the annotated tag and GitHub release:

```sh
git tag -a vX.Y.Z -m "rvoip X.Y.Z"
git push origin vX.Y.Z
gh release create vX.Y.Z --target main --title "rvoip X.Y.Z" --notes-file NOTES.md
```

Do not create the tag or GitHub release for a partial crates.io publication.

## Deprecated Entrypoints

- `scripts/bump_version.sh X.Y.Z` forwards to unified preparation.
- `scripts/publish_train.sh --audit` forwards to unified audit.
- Alpha/beta `--train` publication fails closed. Split release trains are no
  longer supported.
