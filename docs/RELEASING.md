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
