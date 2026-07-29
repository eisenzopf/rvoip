# 0.3.2 Performance Exception Evidence

The immutable [performance
report](releases/beta/20260729T010954Z/exception-r1/BETA_PERFORMANCE_REPORT.md)
records the accepted high-density burst deviation and the invariants that
remained within policy.

The three canonical 2K runs each completed 65,000 of 65,000 calls with ASR
1.0000. The high-density full-media burst completed 17,871 of 18,000 calls,
with ASR 0.9928 against the strict 0.995 requirement; all 129 failures were
answer timeouts and all non-timeout error classes were zero.

The tracked machine-readable evidence and SHA-256 manifest are adjacent to the
immutable report. Focused follow-up controls are not claimed as formal
evidence because their artifacts were removed by the later requested
`cargo clean`.
