# Security policy

## Supported versions

rvoip is currently beta software. Security fixes are made on the `main`
branch and included in the next release. Older beta releases are not maintained
as separate security branches.

## Reporting a vulnerability

Please report suspected vulnerabilities privately through the repository's
GitHub **Security** tab using **Report a vulnerability**. Include:

- the affected crate, version, and configuration;
- a minimal reproduction or proof of concept;
- the expected and observed security boundary;
- likely impact; and
- any suggested mitigation.

Do not include credentials or personal data. Please do not open a public issue
or pull request until the report has been acknowledged and coordinated.

The maintainer will acknowledge a complete report as soon as practical,
normally within seven days. Timelines for validation, remediation, and public
disclosure depend on severity and interoperability impact.

## Scope

Useful reports include authentication or authorization bypass, credential or
key disclosure, unsafe media/security negotiation, memory-safety issues,
protocol parsing vulnerabilities, denial of service, and release-pipeline
compromise. General support questions and already-public dependency advisories
belong in normal issues unless there is rvoip-specific exploitability.

## Code scanning and release policy

GitHub CodeQL is run for Actions, C/C++, JavaScript/TypeScript, Python, and
Rust. Every alert is reviewed at its repository alert number. A dismissal is
the durable per-alert adjudication record and must state whether the location
is test-only, a false positive, or an accepted standards-compatibility risk;
bulk rule suppression is not accepted as review.

The protected release workflow then runs
`scripts/ci/check_codeql_release_policy.py`. It waits for all five CodeQL
categories to analyze the exact current `main` commit and fails if even a low
severity alert remains open. Its machine-readable receipt is retained with
the release qualification evidence. Thus scanner completion alone is not a
release pass, and a stale clean analysis cannot qualify a newer candidate.

SIP Digest MD5 and MD5-sess are retained solely for RFC interoperability with
legacy peers. They implement the SIP challenge-response algorithm, not login
password storage. SHA-256, SHA-256-sess, SHA-512-256, and
SHA-512-256-sess are supported; deployments should select the strongest
algorithm their peer accepts. A CodeQL weak-hash alert at the explicit SIP
Digest implementation is adjudicated on that narrow basis only.
