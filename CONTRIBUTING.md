# Contributing to rvoip

Thank you for helping improve rvoip. Contributions of fixes, tests,
documentation, and focused features are welcome.

## Before opening a pull request

1. Open or reference an issue for behavior changes that need design discussion.
2. Keep each pull request focused on one problem.
3. Add or update tests that demonstrate the changed behavior.
4. Run the most relevant local checks you can. The repository's required
   `PR Gate` is the authoritative result and runs automatically on GitHub.
5. Do not include credentials, generated release evidence, media captures, or
   unrelated formatting changes.

You do not need to run the complete release suite on your computer. CI chooses
the directly affected workspace crates, their reverse dependencies, and any
specialty gates declared for the changed paths. Changes to shared or unmapped
build inputs deliberately select the full workspace.

## Pull request expectations

Complete the pull request template, including:

- the problem and the intended behavior;
- the affected crates or subsystems;
- tests added or changed;
- operational, compatibility, or security risks; and
- an issue reference such as `Fixes #123` when applicable.

The `PR Gate` must pass and review conversations must be resolved before a
pull request can merge. External contributors cannot merge their own changes.
Maintainers may request narrower commits, additional tests, or a release note.

Use a clear, imperative title. Apply one primary release-note label:
`feature`, `fix`, `security`, `documentation`, or `breaking-change`.

## Test guidance

Tests should be placed as close as practical to the behavior they protect:

- unit tests for isolated logic and failure handling;
- integration tests for boundaries between crates or services;
- regression tests for a reported bug;
- feature-matrix checks for optional behavior; and
- interoperability or performance gates only when those properties changed.

Avoid tests that depend on public internet services, wall-clock timing, fixed
ports, or shared global state unless the gate catalog explicitly provides that
resource. Never weaken or delete a failing test merely to make CI green.

## Security reports

Do not open public issues for suspected vulnerabilities. Follow
[SECURITY.md](SECURITY.md) instead.

## Licensing

By contributing, you agree that your contribution is licensed under the
repository's MIT license.
