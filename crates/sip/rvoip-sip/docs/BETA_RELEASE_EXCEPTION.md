# 0.3.2 Release Exception

The project owner approved `0.3.2` for release with one bounded performance
exception. The strict full-run result remains **FAIL / NON-RC**: 106 of 108
required gate records passed, 2 failed, and 0 were skipped.

The root deviation was the high-density full-media burst: 17,871 of 18,000
calls succeeded, for ASR 0.9928 against the required 0.995. All 129 failures
were answer timeouts. Non-timeout, media-setup, overload, teardown, cleanup,
retention, RSS, and host UDP full-buffer-drop checks remained within policy.
`report.performance-metrics` is the derived reporting roll-up of the same ASR
miss, not a second independent product deviation.

The complete immutable report, approval basis, source identity, evidence
bindings, and verifier are in
[`20260729T010954Z/exception-r1`](releases/beta/20260729T010954Z/exception-r1/BETA_RELEASE_REPORT.md).
Tracked evidence deterministically redacts personal absolute host paths while
retaining both original-source and sanitized-snapshot hashes.
Its machine-verifiable
[`exception-attestation.json`](releases/beta/20260729T010954Z/exception-r1/exception-attestation.json)
has SHA-256
`fe9f6f6ec9b0d9db16d8b7d6d2f189819ca6d2f92ffe88a87911f6215cf649d7`.

This exception permits the 0.3.2 release with the disclosed burst risk. It does
not convert the run into a strict release candidate or broaden the documented
production, carrier-SBC, topology, codec, or protocol claims.
