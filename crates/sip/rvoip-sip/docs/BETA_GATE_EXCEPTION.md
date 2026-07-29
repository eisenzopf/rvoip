# 0.3.2 Complete Gate Record

The immutable [complete ordered gate
record](releases/beta/20260729T010954Z/exception-r1/BETA_GATE_REPORT.md)
contains all 108 required gate records from the clean full run:

- 106 PASS
- 2 FAIL
- 0 SKIP

The two failed records are `perf.media-burst-matrix` and its derived
`report.performance-metrics` roll-up. Their source statuses are preserved.
Commands, timestamps, checks, evidence paths, and SHA-256 bindings remain in
the adjacent structured source files.
