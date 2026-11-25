---
section: Added
breaking: false
---
Patch releases can explicitly select `release-patch-smoke` to run regular unit,
integration, regression, example, and doc tests, including direct ICE and TURN
relay checks, without benchmark, soak/load, or extended release jobs. Package
verification and signed exact-commit evidence remain required. Reports disclose
the reduced scope and mark performance qualification NOT RUN; feature and
breaking releases retain full qualification.
