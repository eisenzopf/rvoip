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
