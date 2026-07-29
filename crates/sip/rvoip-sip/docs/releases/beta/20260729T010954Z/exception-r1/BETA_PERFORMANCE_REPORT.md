# 0.3.2 Beta Performance Evidence

> Evidence from full run `20260729T010954Z`. The high-density burst retains
> its strict FAIL status and is released only under the adjacent owner exception.

## Canonical 2K three-pass evidence

| Run | Target CPS | Achieved CPS | ASR | Calls | Setup p99 ms | Cycle p99 ms |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 2000.0 | 1857.1 | 1.0000 | 65000/65000 | 1.243 | 1.962 |
| 2 | 2000.0 | 1857.05 | 1.0000 | 65000/65000 | 1.126 | 1.750 |
| 3 | 2000.0 | 1857.05 | 1.0000 | 65000/65000 | 1.453 | 2.327 |

All three runs share source fingerprint
`cd26a7d50a512633344d931acbed15cf4523c87808d14b7c8acd4dbeb42dd70c` and executable
SHA-256 `48e2c81731ef3424a412b67d7541f9f0e416b02291812cdd984c0023449ede57`.

## High-density full-media burst

| Metric | Requirement | Observed | Strict result |
|---|---:|---:|---|
| Offered CPS | 160 | 160 | PASS |
| ASR | >= 0.9950 | 0.9928 | **FAIL / WAIVED** |
| Calls | 18,000 | 17871 succeeded, 129 timed out | **FAIL / WAIVED** |
| Non-timeout errors | 0 | 0 | PASS |
| Caller retained after drain | 0 | 0 | PASS |
| Receiver retained after drain | 0 | 0 | PASS |
| Receiver active audio resources | 0 | 0 | PASS |
| Delivered audio frames | > 0 | 9922061 | PASS |
| UDP full-buffer drops | 0 | 0 | PASS |
| Caller RSS MB/hour | <= 15 | 0.0 | PASS |
| Receiver RSS MB/hour | <= 15 | 0.0 | PASS |

## Monolithic soak

| Metric | Observed |
|---|---:|
| Duration | 3600 seconds |
| Calls | 587/587 |
| Retained after drain | 0 |
| Active audio receivers after drain | 0 |
| RSS gate | 12.7 MB/hour |

The tracked evidence files in `evidence/` are the machine-readable authority.
Focused follow-up controls are not used as formal evidence here because their
artifacts were removed by the subsequently requested `cargo clean`.
