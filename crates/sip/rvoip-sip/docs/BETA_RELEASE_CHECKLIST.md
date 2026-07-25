# Beta Release Policy and Checklist

This document defines the promotion procedure. It does not duplicate a
candidate's results. The versioned reporting and selection authority is
`config/beta-release-policy.yaml`; current outcomes are generated from evidence:

Current release train and runtime crate version: `0.2.5`.

- [Beta Release Candidate Report](BETA_RELEASE_REPORT.md)
- [Complete Beta Gate Report](BETA_GATE_REPORT.md)
- [Beta Performance Report](BETA_PERFORMANCE_REPORT.md)
- [Immutable release history](releases/beta/README.md)

## Promotion rule

A beta release candidate requires a `full` run that:

- starts from a clean source tree and ends with the identical source
  fingerprint;
- has every gate selected by the effective typed configuration recorded
  exactly once as `PASS`;
- has zero failed and zero skipped gates;
- includes the required unit, integration, doctest, example, downstream,
  security, PBX, SIPp, strict-UA, performance, resiliency, soak, regression,
  evidence-integrity, and final source-fence scopes;
- records peer versions/configuration hashes, build/runtime configuration,
  commands, timestamps, validators, evidence paths, and SHA-256 hashes;
- passes the packaged strict attestation verifier with clean-source,
  unchanged-source, no-skip, pass, and mode-eligibility requirements;
- generates and verifies the evidence-complete release reports before a
  successful full-run pointer can update.

The focused security-only diagnostic remains:

```sh
crates/sip/rvoip-sip/scripts/beta_gate.sh --security
```

Conditional gates are required whenever their enabling configuration schedules
them. They are never classified as optional or “additional” after execution.
Unknown, duplicate, ambiguous, uncatalogued, missing, or unvalidated gates fail
closed.

## Required release configuration

The policy catalog contains the complete typed defaults and selection
conditions. The beta profile additionally fixes these release-critical values:

| Requirement | Release value |
|---|---:|
| Application audio-frame delivery | full (`RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0`) |
| Canonical 2K evidence | three clean, source-identical passes |
| High-density full-media phase | 160 CPS |
| High-density minimum ASR | 0.995 |
| RSS slope limit | 15 MB/hour |
| Monolithic soak | 3,600 seconds, 30 active full-media calls |
| Split soak | 3,600 seconds, 500 active full-media calls |
| Split-soak hold range | 10–360 seconds |
| Controlled drain | 10 CPS |
| Mass teardown | 500 calls at 30 setup CPS |

Changing a threshold or workload requires a reviewed policy change before the
run. Reporting may not reinterpret or silently relax recorded policy.

## Performance pass meaning

The selected performance gates must include the complete profile matrix,
canonical 2K evidence, high-density full-delivery burst, monolithic and split
soaks, mass teardown, media/session churn, resiliency workloads, and regression
audit. Relevant gates require:

- the configured ASR and workload thresholds;
- no disallowed signaling, media-setup, overload, teardown, or lifecycle
  errors;
- positive application audio-frame delivery where media is exercised;
- zero retained sessions, dialogs, media resources, receivers, timers, and
  transaction runners after drain where those metrics apply;
- RSS slope no greater than 15 MB/hour in the gate's defined measurement
  window;
- result/configuration/source/executable reconciliation.

PASS is bounded by the tested source, executable, host, topology, workload,
duration, peer versions, transports, codecs, and configuration. It is not a
claim about untested deployments.

## Evidence and reporting

Future runs write `effective-gate-config.json` and `gate-results.json` natively.
Markdown is display-only. A qualifying report package also contains the source
attestation, checksums, policy/generator inputs, commands and logs, environment
evidence, peer evidence, PBX/SIPp/strict-UA matrices, all performance JSON,
regression evidence, and generated release reports.

The reporting CLI is:

```sh
python3 crates/sip/rvoip-sip/scripts/beta_release_report.py generate \
  --report-root /path/to/report \
  --output-dir /path/to/generated

python3 crates/sip/rvoip-sip/scripts/beta_release_report.py verify \
  --generated-dir /path/to/generated

python3 crates/sip/rvoip-sip/scripts/beta_release_report.py promote-docs \
  --report-root /path/to/report
```

`promote-docs` first runs strict source verification, generates in a temporary
directory, verifies every generated binding, preserves an existing immutable
snapshot only when byte-identical, and then updates the stable current reports.

## Attestation scope

SHA-256 attestation supplies integrity and reproducibility evidence. It is not
third-party cryptographic signing. A report promotion is a post-run derivation
and does not change the tested candidate's commit identity.
