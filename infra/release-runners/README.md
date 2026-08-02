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

This pilot does not claim coverage for the external PBX, long-soak, or
performance gates that still require structured remote replacements. The
`remote-release` profile remains fail-closed until those mappings exist.

The protected `Release qualification` workflow uses the same ephemeral model
for the complete gate catalog. A single GitHub controller launches every
duration-balanced GCP shard concurrently, rather than consuming one GitHub job
slot per cloud worker. Each worker is bound to one candidate SHA and gate list,
uploads an immutable result and evidence archive, shuts down, and is deleted
with its auto-delete disk. Controller and follow-up sweep cleanup both run on
failure; there is no idle release fleet.
