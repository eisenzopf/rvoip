# rvoip 0.3.2 Release Exception Attestation

> Owner-approved exception derived from strict full-run package `20260729T010954Z`.
> No failed gate was rewritten as PASS. The strict source attestation remains
> `FAIL` / `NON-RC`; this document records the separate release decision.

## Decision

| Field | Value |
|---|---|
| Release | `0.3.2` |
| Disposition | **APPROVED-WITH-EXCEPTION** |
| Approval actor | `project owner/operator` |
| Approval basis | Explicit owner decision to accept the single high-density media-burst ASR deviation for the 0.3.2 release. |
| Strict automated qualification | **NON-RC** |
| Tested commit | `fbc96ed4f1736f1cc4b0a1145497183d6acf0d2f` |
| Tested tree | `2dfc781726589cf5758c6277ab5ae82441f4b495` |
| Clean and unchanged source | `True` |
| Gate inventory | `106/108` PASS, `2` FAIL, `0` SKIP |
| Root policy deviations | `1` |

## Accepted deviation

The high-density full-media burst delivered `17871` of
`18000` calls. ASR was `0.9928`
against the release requirement of `0.9950`, an
absolute shortfall of `0.0022`. All
`129` failures were answer timeouts; non-timeout errors
were zero.

The second failed record, `report.performance-metrics`, is the reporting
roll-up of the same ASR miss. It is not a second independent product failure.

## Preserved invariants

- Media setup, overload rejection, teardown, and other non-timeout errors: zero.
- Caller and receiver retained resources after drain: zero.
- Active receiver audio resources after drain: zero.
- Caller and receiver transaction managers after drain: zero.
- Host UDP full-socket-buffer drops: zero.
- Delivered application audio frames: `9922061`.
- Caller/receiver RSS gate values: `0.0` / `0.0` MB/hour.
- Canonical 2K qualification: three source-identical PASS runs.
- Monolithic and split soaks: clean completion and zero post-drain retention.

## Release meaning

This decision permits preparation and publication of `0.3.2` with the accepted
burst-risk disclosure. It does **not** convert the run into a strict beta
release candidate and does not authorize broader production, carrier-SBC, or
untested-topology claims.

Tracked evidence replaces personal absolute host paths with `<workspace>` or
`<source-report>`. Each source binding records both the original source
SHA-256 and the sanitized snapshot SHA-256.

See [the complete gate record](BETA_GATE_REPORT.md), [performance details](BETA_PERFORMANCE_REPORT.md),
and the machine-verifiable [`exception-attestation.json`](exception-attestation.json).
