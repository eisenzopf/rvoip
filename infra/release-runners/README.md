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
