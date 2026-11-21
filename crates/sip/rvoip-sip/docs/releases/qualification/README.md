# Protected release qualification history

These directories are immutable, source-matched reporting derivations from the
protected `remote-release` pipeline. The GitHub artifact retains the complete
receipts, command logs, packet captures, and raw measurements; the committed
bundle retains their checked, human-readable ledger and cryptographic identity.

| Run | Release | Result | Evidence |
|---|---|---|---|
| [`34074372543`](https://github.com/eisenzopf/rvoip/actions/runs/34074372543) | `0.3.10` at `77a99cd38a07641294cf7dc547146b115b135dc7` | **PASS — 213/213 fresh**, including 108/108 legacy requirements | [release](20260907T042726Z-34074372543/BETA_RELEASE_REPORT.md) · [gates](20260907T042726Z-34074372543/BETA_GATE_REPORT.md) · [performance](20260907T042726Z-34074372543/BETA_PERFORMANCE_REPORT.md) · [evaluation](20260907T042726Z-34074372543/current-performance-evaluation.md) · [artifact index](20260907T042726Z-34074372543/current-performance-artifact-index.json) · [attestation](20260907T042726Z-34074372543/QUALIFICATION_REPORT_ATTESTATION.json) |
| [`33969263241`](https://github.com/eisenzopf/rvoip/actions/runs/33969263241) | `0.3.9` at `8cab44b10f872d21b304c02111d5d203ee8226da` | **PASS — 208/208**, including 108/108 legacy requirements | [release](20260905T133559Z-33969263241/BETA_RELEASE_REPORT.md) · [gates](20260905T133559Z-33969263241/BETA_GATE_REPORT.md) · [performance](20260905T133559Z-33969263241/BETA_PERFORMANCE_REPORT.md) · [attestation](20260905T133559Z-33969263241/QUALIFICATION_REPORT_ATTESTATION.json) |

Generate a bundle after an exact-candidate run with
`scripts/release/render_qualification_reports.py`. The renderer rejects missing,
duplicated, stale, cross-candidate, unclean, or hash-mismatched evidence.
