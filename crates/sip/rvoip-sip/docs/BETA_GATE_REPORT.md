# Beta Gate Report

> Evidence-complete reporting derivation for candidate `20260724T231400Z`. All 108 recorded entries are required under the effective full configuration; none is classified as merely additional.

## Result

**PASS — 108/108 required gates passed; 0 failed; 0 skipped.**

## Source integrity

4 required gates; 4 passed.

### 001 · `source.clean-start` — clean beta source fingerprint

- Result: **PASS** in 0 seconds.
- Purpose: Bind qualification to one clean, unchanged source tree. Named scope: clean beta source fingerprint.
- Recorded component/command: `verify_clean_source_fingerprint`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_canonical_2k_run_dirs=["<workspace>/target/perf-results/profiles/20260724T221956Z_clean_KRwxGm","<workspace>/target/perf-results/profiles/20260724T224300Z_clean_GbSiae","<workspace>/target/perf-results/profiles/20260724T225757Z_clean_p5XNXK"]`, `beta_require_canonical_2k_evidence=true`, `beta_require_clean_source=true`, `beta_require_configured_state_table_evidence=false`, `beta_state_table_fallback_reason=none`, `beta_state_table_sha256=a649a00aba17f6fe1390d24ada78a8936dce8b577c5e4e777714f0e70c89e3ed`, `beta_state_table_source=embedded-default`, `git_revision=8d44fb35`, `git_status=clean`.
- Expected checks: `status-pass`, `evidence-hash`, `source-fingerprint`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`1adf9d868209f6fc5fd79d43c5b1b26e74a8e02159edb8f91828cec13a5d415e` (PASS).
- Evidence: `clean_beta_source_fingerprint.log` (SHA-256 `1adf9d868209f6fc5fd79d43c5b1b26e74a8e02159edb8f91828cec13a5d415e`).
- PASS establishes: The recorded source check succeeded for the tested commit and tree.
- PASS does not establish: Does not test runtime behavior.

### 002 · `source.canonical-2k` — canonical 2k three-pass evidence

- Result: **PASS** in 0 seconds.
- Purpose: Bind qualification to one clean, unchanged source tree. Named scope: canonical 2k three-pass evidence.
- Recorded component/command: `python3 <workspace>/crates/sip/rvoip-sip/scripts/canonical_2k_evidence.py import --workspace-root <workspace> --beta-start <source-report> --artifact-dir <source-report> --run-dir <workspace>/target/perf-results/profiles/20260724T221956Z_clean_KRwxGm --run-dir <workspace>/target/perf-results/profiles/20260724T224300Z_clean_GbSiae --run-dir <workspace>/target/perf-results/profiles/20260724T225757Z_clean_p5XNXK`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_canonical_2k_run_dirs=["<workspace>/target/perf-results/profiles/20260724T221956Z_clean_KRwxGm","<workspace>/target/perf-results/profiles/20260724T224300Z_clean_GbSiae","<workspace>/target/perf-results/profiles/20260724T225757Z_clean_p5XNXK"]`, `beta_require_canonical_2k_evidence=true`, `beta_require_clean_source=true`, `beta_require_configured_state_table_evidence=false`, `beta_state_table_fallback_reason=none`, `beta_state_table_sha256=a649a00aba17f6fe1390d24ada78a8936dce8b577c5e4e777714f0e70c89e3ed`, `beta_state_table_source=embedded-default`, `git_revision=8d44fb35`, `git_status=clean`.
- Expected checks: `status-pass`, `evidence-hash`, `source-fingerprint`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`46ca72bd5dc44710c387e134e40cfdc5d8aa7c107bcaaee83752b9ef72f0956c` (PASS).
- Evidence: `canonical_2k_three-pass_evidence.log` (SHA-256 `46ca72bd5dc44710c387e134e40cfdc5d8aa7c107bcaaee83752b9ef72f0956c`).
- PASS establishes: The recorded source check succeeded for the tested commit and tree.
- PASS does not establish: Does not test runtime behavior.

### 107 · `source.final-capture` — beta final source fingerprint capture

- Result: **PASS** in 1 seconds.
- Purpose: Bind qualification to one clean, unchanged source tree. Named scope: beta final source fingerprint capture.
- Recorded component/command: `python3 <workspace>/crates/sip/rvoip-sip/scripts/canonical_2k_evidence.py fingerprint --workspace-root <workspace> --out <source-report>`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_canonical_2k_run_dirs=["<workspace>/target/perf-results/profiles/20260724T221956Z_clean_KRwxGm","<workspace>/target/perf-results/profiles/20260724T224300Z_clean_GbSiae","<workspace>/target/perf-results/profiles/20260724T225757Z_clean_p5XNXK"]`, `beta_require_canonical_2k_evidence=true`, `beta_require_clean_source=true`, `beta_require_configured_state_table_evidence=false`, `beta_state_table_fallback_reason=none`, `beta_state_table_sha256=a649a00aba17f6fe1390d24ada78a8936dce8b577c5e4e777714f0e70c89e3ed`, `beta_state_table_source=embedded-default`, `git_revision=8d44fb35`, `git_status=clean`.
- Expected checks: `status-pass`, `evidence-hash`, `source-fingerprint`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`6a1336f9f7269ea5fb7ab1273a9c0f8d3755c9aad13c68ae473b16ca32fc760a` (PASS).
- Evidence: `beta_final_source_fingerprint_capture.log` (SHA-256 `6a1336f9f7269ea5fb7ab1273a9c0f8d3755c9aad13c68ae473b16ca32fc760a`).
- PASS establishes: The recorded source check succeeded for the tested commit and tree.
- PASS does not establish: Does not test runtime behavior.

### 108 · `source.canonical-2k-unchanged` — canonical 2k beta source unchanged

- Result: **PASS** in 0 seconds.
- Purpose: Bind qualification to one clean, unchanged source tree. Named scope: canonical 2k beta source unchanged.
- Recorded component/command: `python3 <workspace>/crates/sip/rvoip-sip/scripts/canonical_2k_evidence.py verify-source --workspace-root <workspace> --beta-start <source-report>`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_canonical_2k_run_dirs=["<workspace>/target/perf-results/profiles/20260724T221956Z_clean_KRwxGm","<workspace>/target/perf-results/profiles/20260724T224300Z_clean_GbSiae","<workspace>/target/perf-results/profiles/20260724T225757Z_clean_p5XNXK"]`, `beta_require_canonical_2k_evidence=true`, `beta_require_clean_source=true`, `beta_require_configured_state_table_evidence=false`, `beta_state_table_fallback_reason=none`, `beta_state_table_sha256=a649a00aba17f6fe1390d24ada78a8936dce8b577c5e4e777714f0e70c89e3ed`, `beta_state_table_source=embedded-default`, `git_revision=8d44fb35`, `git_status=clean`.
- Expected checks: `status-pass`, `evidence-hash`, `source-fingerprint`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`4801ddc384bc3bd739e0b079826681279d92d1539f627abe2e3593a878083614` (PASS).
- Evidence: `canonical_2k_beta_source_unchanged.log` (SHA-256 `4801ddc384bc3bd739e0b079826681279d92d1539f627abe2e3593a878083614`).
- PASS establishes: The recorded source check succeeded for the tested commit and tree.
- PASS does not establish: Does not test runtime behavior.

## Build, API, and documentation

44 required gates; 44 passed.

### 003 · `build.format` — format check

- Result: **PASS** in 5 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: format check.
- Recorded component/command: `cargo fmt --all -- --check`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`9db2fa116cdaef19d98fbca05f2c18fb77454bae54dff953057db655f7dfbf6c` (PASS).
- Evidence: `format_check.log` (SHA-256 `9db2fa116cdaef19d98fbca05f2c18fb77454bae54dff953057db655f7dfbf6c`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 004 · `build.evidence-helper-tests` — beta evidence helper tests

- Result: **PASS** in 8 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: beta evidence helper tests.
- Recorded component/command: `python3 -m unittest crates/sip/rvoip-sip/scripts/test_beta_attestation.py crates/sip/rvoip-sip/scripts/test_beta_performance_gate_metrics.py crates/sip/rvoip-sip/scripts/test_beta_gate_source.py crates/sip/rvoip-sip/scripts/test_perf_audit.py crates/sip/rvoip-sip/scripts/test_canonical_2k_evidence.py crates/sip/rvoip-sip/scripts/test_perf_2k_acceptance.py crates/sip/rvoip-sip/scripts/test_perf_2k_baseline.py crates/sip/rvoip-sip/scripts/test_perf_regression_baseline.py crates/sip/rvoip-sip/scripts/test_perf_cargo_artifact.py crates/sip/rvoip-sip/scripts/test_docker_peer_snapshot.py`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`1db8706f91ea597b5a6ee11376727cceb84a91673fd6749470bfa1b1c1e66d9b` (PASS).
- Evidence: `beta_evidence_helper_tests.log` (SHA-256 `1db8706f91ea597b5a6ee11376727cceb84a91673fd6749470bfa1b1c1e66d9b`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 005 · `build.public-api` — public API compatibility

- Result: **PASS** in 338 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: public API compatibility.
- Recorded component/command: `<workspace>/crates/sip/rvoip-sip/scripts/check_public_api.sh`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`8a999fdeba10329e292d0492d8c29b659f35f50a0dd4afe3ae3fd9c606b7c29c` (PASS).
- Evidence: `public_api_compatibility.log` (SHA-256 `8a999fdeba10329e292d0492d8c29b659f35f50a0dd4afe3ae3fd9c606b7c29c`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 006 · `build.sip-all-targets` — rvoip-sip all-target check

- Result: **PASS** in 299 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: rvoip-sip all-target check.
- Recorded component/command: `cargo check -p rvoip-sip --all-targets --features generated-validation,dev-insecure-tls`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`6fef36bff74625e7fa849f903441c9c40dc6de04b7c3d74479259b202906b686` (PASS).
- Evidence: `rvoip-sip_all-target_check.log` (SHA-256 `6fef36bff74625e7fa849f903441c9c40dc6de04b7c3d74479259b202906b686`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 007 · `build.claimed-lower-crates` — claimed lower-crate check

- Result: **PASS** in 22 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: claimed lower-crate check.
- Recorded component/command: `cargo check -p rvoip-sip-core -p rvoip-sip-transport -p rvoip-sip-dialog -p rvoip-media-core -p rvoip-rtp-core -p rvoip-auth-core -p rvoip-sip-registrar -p rvoip-sip-proxy --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`35b5629cc87bb26d21c12bb30b8fb5e17307a606997f497e4d700865af227c4e` (PASS).
- Evidence: `claimed_lower-crate_check.log` (SHA-256 `35b5629cc87bb26d21c12bb30b8fb5e17307a606997f497e4d700865af227c4e`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 008 · `test.supporting-sip-crates` — supporting SIP crate tests

- Result: **PASS** in 36 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: supporting SIP crate tests.
- Recorded component/command: `cargo test -p rvoip-auth-core -p rvoip-sip-registrar -p rvoip-sip-proxy --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`56b4c548f68695f12bf5467884c430fa8df2db0a5344c87e2232569b297c43e0` (PASS).
- Evidence: `supporting_sip_crate_tests.log` (SHA-256 `56b4c548f68695f12bf5467884c430fa8df2db0a5344c87e2232569b297c43e0`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 009 · `test.rtp-core` — rtp-core tests

- Result: **PASS** in 41 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: rtp-core tests.
- Recorded component/command: `cargo test -p rvoip-rtp-core --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`9dcd604482cd1016771fda97c8fc95a195cf2d014a4841e67cbb2816519b82a3` (PASS).
- Evidence: `rtp-core_tests.log` (SHA-256 `9dcd604482cd1016771fda97c8fc95a195cf2d014a4841e67cbb2816519b82a3`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 010 · `test.workspace-unit` — workspace unit tests

- Result: **PASS** in 305 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: workspace unit tests.
- Recorded component/command: `cargo test --workspace --exclude rvoip-sip --lib`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`0b1727c5dd807bbd2ac59984382fb76f45e5d44e6d2061ac566eeff1ff51459b` (PASS).
- Evidence: `workspace_unit_tests.log` (SHA-256 `0b1727c5dd807bbd2ac59984382fb76f45e5d44e6d2061ac566eeff1ff51459b`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 011 · `test.workspace-integration` — workspace target and integration tests

- Result: **PASS** in 608 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: workspace target and integration tests.
- Recorded component/command: `cargo test --workspace --exclude rvoip-sip --bins --examples --tests`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`4cb11455a29b8639fafdb07c835da0a6ae97db89f5bd340bd1568d0461a499e3` (PASS).
- Evidence: `workspace_target_and_integration_tests.log` (SHA-256 `4cb11455a29b8639fafdb07c835da0a6ae97db89f5bd340bd1568d0461a499e3`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 012 · `test.workspace-doctest` — workspace doctests

- Result: **PASS** in 671 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: workspace doctests.
- Recorded component/command: `cargo test --workspace --exclude rvoip-sip --doc`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`b4b17fad379449ed54f08172463e23b417119a4e9feccfb8a484536c1f9d148d` (PASS).
- Evidence: `workspace_doctests.log` (SHA-256 `b4b17fad379449ed54f08172463e23b417119a4e9feccfb8a484536c1f9d148d`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 013 · `test.sip-unit` — rvoip-sip unit tests

- Result: **PASS** in 219 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: rvoip-sip unit tests.
- Recorded component/command: `cargo test -p rvoip-sip --lib`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`75e3c568071beb9d6c6ac1f8173e95de4ab94643b4af7987294ea8caeaeffb2c` (PASS).
- Evidence: `rvoip-sip_unit_tests.log` (SHA-256 `75e3c568071beb9d6c6ac1f8173e95de4ab94643b4af7987294ea8caeaeffb2c`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 014 · `test.sip-integration` — rvoip-sip integration tests

- Result: **PASS** in 1062 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: rvoip-sip integration tests.
- Recorded component/command: `cargo test -p rvoip-sip --tests --features generated-validation,dev-insecure-tls`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`592545053841b0274f1e75657cf0d3b4531cd30377797e7f3bcb2bc3d3735270` (PASS).
- Evidence: `rvoip-sip_integration_tests.log` (SHA-256 `592545053841b0274f1e75657cf0d3b4531cd30377797e7f3bcb2bc3d3735270`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 015 · `test.sip-doctest` — rvoip-sip doctests

- Result: **PASS** in 59 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: rvoip-sip doctests.
- Recorded component/command: `cargo test -p rvoip-sip --doc`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`75deb49c2c2cb6552fc09761d65782ebd0f93427c8735ff7ffb477ed4fdda3fe` (PASS).
- Evidence: `rvoip-sip_doctests.log` (SHA-256 `75deb49c2c2cb6552fc09761d65782ebd0f93427c8735ff7ffb477ed4fdda3fe`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 016 · `build.sip-examples` — rvoip-sip examples compile

- Result: **PASS** in 271 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: rvoip-sip examples compile.
- Recorded component/command: `cargo build -p rvoip-sip --examples --features dev-insecure-tls`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`0a302c8d94a9439140afb1d1d48e60cd1ad43f1d1671ed31804811c1a7b8e669` (PASS).
- Evidence: `rvoip-sip_examples_compile.log` (SHA-256 `0a302c8d94a9439140afb1d1d48e60cd1ad43f1d1671ed31804811c1a7b8e669`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 017 · `build.downstream-rvoip` — downstream rvoip default check

- Result: **PASS** in 147 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip default check.
- Recorded component/command: `cargo check -p rvoip --lib`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`1c4bdf4dc3b9fa189683f67db29177568269551cbf48535c8c05d9ce12ffd8ff` (PASS).
- Evidence: `downstream_rvoip_default_check.log` (SHA-256 `1c4bdf4dc3b9fa189683f67db29177568269551cbf48535c8c05d9ce12ffd8ff`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 018 · `build.downstream-rvoip-app` — downstream rvoip app check

- Result: **PASS** in 157 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip app check.
- Recorded component/command: `cargo check -p rvoip --lib --no-default-features --features app`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`990460dfa2121f1e9f3dfc91d949e108943a876e4423a9b0102c09277132479b` (PASS).
- Evidence: `downstream_rvoip_app_check.log` (SHA-256 `990460dfa2121f1e9f3dfc91d949e108943a876e4423a9b0102c09277132479b`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 019 · `build.downstream-client` — downstream rvoip-client default check

- Result: **PASS** in 15 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-client default check.
- Recorded component/command: `cargo check -p rvoip-client --lib`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`85327d274d7858a2c5f9a27143a53882c6c741537535edc0c395e4bd692ce011` (PASS).
- Evidence: `downstream_rvoip-client_default_check.log` (SHA-256 `85327d274d7858a2c5f9a27143a53882c6c741537535edc0c395e4bd692ce011`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 020 · `build.downstream-client-full` — downstream rvoip-client full check

- Result: **PASS** in 147 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-client full check.
- Recorded component/command: `cargo check -p rvoip-client --lib --no-default-features --features full`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`9a3c5f9fa7c6ae3aadc55f5d177ff7d3ce1e7f5e6b9ac51fd8ba961f28812c74` (PASS).
- Evidence: `downstream_rvoip-client_full_check.log` (SHA-256 `9a3c5f9fa7c6ae3aadc55f5d177ff7d3ce1e7f5e6b9ac51fd8ba961f28812c74`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 021 · `build.downstream-core` — downstream rvoip-core check

- Result: **PASS** in 5 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-core check.
- Recorded component/command: `cargo check -p rvoip-core --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`58191bcabf72e499eeb3165257736c08c4e15c5a5400db349e856b87ef6fdb1b` (PASS).
- Evidence: `downstream_rvoip-core_check.log` (SHA-256 `58191bcabf72e499eeb3165257736c08c4e15c5a5400db349e856b87ef6fdb1b`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 022 · `build.downstream-amazon-connect` — downstream rvoip-amazon-connect server check

- Result: **PASS** in 149 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-amazon-connect server check.
- Recorded component/command: `cargo check -p rvoip-amazon-connect --all-targets --features server`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`85dd496c88280c1c29f12a8cd305f788838e4956f2ef5d132c442c9866a3dadb` (PASS).
- Evidence: `downstream_rvoip-amazon-connect_server_check.log` (SHA-256 `85dd496c88280c1c29f12a8cd305f788838e4956f2ef5d132c442c9866a3dadb`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 023 · `build.downstream-uctp` — downstream rvoip-uctp check

- Result: **PASS** in 144 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-uctp check.
- Recorded component/command: `cargo check -p rvoip-uctp --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`85b0f9b13021202313a6785a6e3c08aa2876712a312102a3d31cb0adabda37f9` (PASS).
- Evidence: `downstream_rvoip-uctp_check.log` (SHA-256 `85b0f9b13021202313a6785a6e3c08aa2876712a312102a3d31cb0adabda37f9`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 024 · `build.downstream-quic` — downstream rvoip-quic check

- Result: **PASS** in 14 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-quic check.
- Recorded component/command: `cargo check -p rvoip-quic --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`782068e64d993a1ecd4475850581495a9f6a21b34bd0ed2003211e9dcbc6eeac` (PASS).
- Evidence: `downstream_rvoip-quic_check.log` (SHA-256 `782068e64d993a1ecd4475850581495a9f6a21b34bd0ed2003211e9dcbc6eeac`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 025 · `build.downstream-webtransport` — downstream rvoip-webtransport check

- Result: **PASS** in 3 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-webtransport check.
- Recorded component/command: `cargo check -p rvoip-webtransport --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`2c9dcf509728cb40f79ba002032e1cc351fb54e84801d1de257da24ee865bf94` (PASS).
- Evidence: `downstream_rvoip-webtransport_check.log` (SHA-256 `2c9dcf509728cb40f79ba002032e1cc351fb54e84801d1de257da24ee865bf94`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 026 · `build.downstream-websocket` — downstream rvoip-websocket media and TLS check

- Result: **PASS** in 15 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-websocket media and TLS check.
- Recorded component/command: `cargo check -p rvoip-websocket --all-targets --features media-webrtc,wss`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`88e095faa7d0d7fb6ca08af7e3f5db2af9a6cf7ae735adad9b526383330e26ee` (PASS).
- Evidence: `downstream_rvoip-websocket_media_and_tls_check.log` (SHA-256 `88e095faa7d0d7fb6ca08af7e3f5db2af9a6cf7ae735adad9b526383330e26ee`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 027 · `build.downstream-webrtc` — downstream rvoip-webrtc interop check

- Result: **PASS** in 165 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-webrtc interop check.
- Recorded component/command: `cargo check -p rvoip-webrtc --all-targets --features comprehensive,tls-rustls,bridge-quic`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`68ba97eb4f7fab361e976c395b2497f5e7b53f6cdd41cc2a65d9ea7bb5a0de72` (PASS).
- Evidence: `downstream_rvoip-webrtc_interop_check.log` (SHA-256 `68ba97eb4f7fab361e976c395b2497f5e7b53f6cdd41cc2a65d9ea7bb5a0de72`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 028 · `build.downstream-audio-device` — downstream rvoip-audio-device check

- Result: **PASS** in 1 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: downstream rvoip-audio-device check.
- Recorded component/command: `cargo check -p rvoip-audio-device --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`8d45a12c1a43b51265370eb131d7cf468db69d79fef801308c450050103e5086` (PASS).
- Evidence: `downstream_rvoip-audio-device_check.log` (SHA-256 `8d45a12c1a43b51265370eb131d7cf468db69d79fef801308c450050103e5086`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 029 · `test.example-01` — standalone example 01-quickstart-p2p tests

- Result: **PASS** in 16 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 01-quickstart-p2p tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/01-quickstart-p2p/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`b85bf012dfe0b50af3c312a8ab203a4bf9de310402e99a5e0370281cd72df28c` (PASS).
- Evidence: `standalone_example_01-quickstart-p2p_tests.log` (SHA-256 `b85bf012dfe0b50af3c312a8ab203a4bf9de310402e99a5e0370281cd72df28c`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 030 · `test.example-02` — standalone example 02-softphone-audio tests

- Result: **PASS** in 17 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 02-softphone-audio tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/02-softphone-audio/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`08b90cd1b9e4aba18fdc458a72b50231b43a3d7b9a3a705759585fbcc0d9de4e` (PASS).
- Evidence: `standalone_example_02-softphone-audio_tests.log` (SHA-256 `08b90cd1b9e4aba18fdc458a72b50231b43a3d7b9a3a705759585fbcc0d9de4e`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 031 · `test.example-03` — standalone example 03-register-to-pbx tests

- Result: **PASS** in 16 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 03-register-to-pbx tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/03-register-to-pbx/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`0fb2c8017cecf2614b9ef228aff2706ba028dd7ea087753ec3ee98e3af8c644c` (PASS).
- Evidence: `standalone_example_03-register-to-pbx_tests.log` (SHA-256 `0fb2c8017cecf2614b9ef228aff2706ba028dd7ea087753ec3ee98e3af8c644c`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 032 · `test.example-04` — standalone example 04-call-control tests

- Result: **PASS** in 15 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 04-call-control tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/04-call-control/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`96bb2e7cd3f171f61482f453cb2f3d12ce7a3c72326cb90e3f8bb46a3c7ea01f` (PASS).
- Evidence: `standalone_example_04-call-control_tests.log` (SHA-256 `96bb2e7cd3f171f61482f453cb2f3d12ce7a3c72326cb90e3f8bb46a3c7ea01f`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 033 · `test.example-05` — standalone example 05-blind-transfer tests

- Result: **PASS** in 15 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 05-blind-transfer tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/05-blind-transfer/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`bb1f11d0db7076e182a5ebabfc16bd09fc6a4044438c091b241dcb6d2dc7057a` (PASS).
- Evidence: `standalone_example_05-blind-transfer_tests.log` (SHA-256 `bb1f11d0db7076e182a5ebabfc16bd09fc6a4044438c091b241dcb6d2dc7057a`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 034 · `test.example-06` — standalone example 06-attended-transfer tests

- Result: **PASS** in 16 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 06-attended-transfer tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/06-attended-transfer/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`5b41902fb9f7fae7f6640cc23927b0ad9880560ba8478dcdb94805b9faffe0e1` (PASS).
- Evidence: `standalone_example_06-attended-transfer_tests.log` (SHA-256 `5b41902fb9f7fae7f6640cc23927b0ad9880560ba8478dcdb94805b9faffe0e1`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 035 · `test.example-07` — standalone example 07-secure-call-srtp tests

- Result: **PASS** in 15 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 07-secure-call-srtp tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/07-secure-call-srtp/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`e001bf7bc3b705a4924d5d4070c93b2ccd073378fbe555f208761029dd6eae8d` (PASS).
- Evidence: `standalone_example_07-secure-call-srtp_tests.log` (SHA-256 `e001bf7bc3b705a4924d5d4070c93b2ccd073378fbe555f208761029dd6eae8d`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 036 · `test.example-08` — standalone example 08-tls-transport tests

- Result: **PASS** in 15 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 08-tls-transport tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/08-tls-transport/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`ae34fca76baab9628a49dde3b7ecc9a9f35db5e1058504e67b0a184131657653` (PASS).
- Evidence: `standalone_example_08-tls-transport_tests.log` (SHA-256 `ae34fca76baab9628a49dde3b7ecc9a9f35db5e1058504e67b0a184131657653`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 037 · `test.example-09` — standalone example 09-ivr-server tests

- Result: **PASS** in 16 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 09-ivr-server tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/09-ivr-server/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`113cebb7a93ef436fc1526b7300c014e14ca98fac36f99102c13ac3c363f6f14` (PASS).
- Evidence: `standalone_example_09-ivr-server_tests.log` (SHA-256 `113cebb7a93ef436fc1526b7300c014e14ca98fac36f99102c13ac3c363f6f14`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 038 · `test.example-10` — standalone example 10-call-center-b2bua tests

- Result: **PASS** in 15 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 10-call-center-b2bua tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/10-call-center-b2bua/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`65f27baec025d84751b6102c8131f2becfefd09b9dddda87c0f214fc63d2d3c1` (PASS).
- Evidence: `standalone_example_10-call-center-b2bua_tests.log` (SHA-256 `65f27baec025d84751b6102c8131f2becfefd09b9dddda87c0f214fc63d2d3c1`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 039 · `test.example-11` — standalone example 11-ai-harness-demo tests

- Result: **PASS** in 1 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 11-ai-harness-demo tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/11-ai-harness-demo/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`a2dcba84a72909f0c08eeb48517c4ffda18195253da9989cddf75d3de8d5715e` (PASS).
- Evidence: `standalone_example_11-ai-harness-demo_tests.log` (SHA-256 `a2dcba84a72909f0c08eeb48517c4ffda18195253da9989cddf75d3de8d5715e`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 040 · `test.example-12` — standalone example 12-customer-escalation-sip-webrtc tests

- Result: **PASS** in 17 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 12-customer-escalation-sip-webrtc tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/12-customer-escalation-sip-webrtc/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`637ff21961457331777fda901e0da66c41bc59cd6655623e0b3db4e0267af0a5` (PASS).
- Evidence: `standalone_example_12-customer-escalation-sip-webrtc_tests.log` (SHA-256 `637ff21961457331777fda901e0da66c41bc59cd6655623e0b3db4e0267af0a5`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 041 · `test.example-13` — standalone example 13-sip-to-amazon-connect tests

- Result: **PASS** in 16 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: standalone example 13-sip-to-amazon-connect tests.
- Recorded component/command: `cargo test --manifest-path <workspace>/examples/13-sip-to-amazon-connect/Cargo.toml --all-targets`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`180b9dad033864328f132bb0d83f39e30c3ee81d1403936e5f9326ac53839873` (PASS).
- Evidence: `standalone_example_13-sip-to-amazon-connect_tests.log` (SHA-256 `180b9dad033864328f132bb0d83f39e30c3ee81d1403936e5f9326ac53839873`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 042 · `test.pbx-analyzer` — PBX analyzer unit tests

- Result: **PASS** in 12 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: PBX analyzer unit tests.
- Recorded component/command: `cargo test -p rvoip-sip --example pbx_analyze --features dev-insecure-tls`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`fc3f378799b016d856a4b25912d08eb8a30187f784d9220159aa6339c0de4802` (PASS).
- Evidence: `pbx_analyzer_unit_tests.log` (SHA-256 `fc3f378799b016d856a4b25912d08eb8a30187f784d9220159aa6339c0de4802`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 043 · `build.rustdoc` — rvoip-sip rustdoc

- Result: **PASS** in 20 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: rvoip-sip rustdoc.
- Recorded component/command: `env RUSTDOCFLAGS=-D warnings cargo doc -p rvoip-sip --no-deps --features generated-validation,dev-insecure-tls`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`73617f09203b1242453d651958ac94fa9c81edaa49a7a5a5c17749588edffe43` (PASS).
- Evidence: `rvoip-sip_rustdoc.log` (SHA-256 `73617f09203b1242453d651958ac94fa9c81edaa49a7a5a5c17749588edffe43`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 044 · `test.rfc4475` — sip-core RFC 4475 torture tests

- Result: **PASS** in 12 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: sip-core RFC 4475 torture tests.
- Recorded component/command: `cargo test -p rvoip-sip-core --features lenient_parsing --test torture_tests`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`9489db99c535062ab6985187f0ee2dee7a4b264462b0cb459891fb5b151f4c20` (PASS).
- Evidence: `sip-core_rfc_4475_torture_tests.log` (SHA-256 `9489db99c535062ab6985187f0ee2dee7a4b264462b0cb459891fb5b151f4c20`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 045 · `test.generated-message` — sip-core generated message validation

- Result: **PASS** in 6 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: sip-core generated message validation.
- Recorded component/command: `cargo test -p rvoip-sip-core --features generated-validation --test generated_message_compliance`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`92497ccdc323366997bea13269827548dcfd30c9d5dc913d90fac40e38e36e07` (PASS).
- Evidence: `sip-core_generated_message_validation.log` (SHA-256 `92497ccdc323366997bea13269827548dcfd30c9d5dc913d90fac40e38e36e07`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

### 046 · `test.generated-dialog` — sip dialog generated validation

- Result: **PASS** in 26 seconds.
- Purpose: Compile or execute the named Rust scope under the recorded release configuration. Named scope: sip dialog generated validation.
- Recorded component/command: `cargo test -p rvoip-sip-dialog --features generated-validation --test generated_sip_compliance`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_attestation_features=["generated-validation","dev-insecure-tls","perf-tests","g729"]`, `beta_attestation_target=rustc-host`, `beta_deny_warnings=true`, `rvoip_require_api_tools=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`05180f07c92727ff35e9e7d510d1b45207c27d74417871fe1e92e131a0591619` (PASS).
- Evidence: `sip_dialog_generated_validation.log` (SHA-256 `05180f07c92727ff35e9e7d510d1b45207c27d74417871fe1e92e131a0591619`).
- PASS establishes: The named compiler, unit, integration, documentation, or compatibility scope completed successfully.
- PASS does not establish: Does not establish behavior outside the named feature, target, or test scope.

## Security

11 required gates; 11 passed.

### 047 · `security.advisory-audit` — dependency advisory audit

- Result: **PASS** in 3 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: dependency advisory audit.
- Recorded component/command: `env SECURITY_DIR=<source-report> bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`3ce24b9fa41a5f277e80fdbeb50df1a37f6abff80d9b6ec60a5bd513d3027d7f` (PASS).
- Evidence: `dependency_advisory_audit.log` (SHA-256 `3ce24b9fa41a5f277e80fdbeb50df1a37f6abff80d9b6ec60a5bd513d3027d7f`), `security/cargo-audit.txt` (SHA-256 `c6857f784c5e93e5872e6d28269da26178e4247d19f3a1359b81c2e73ed09c92`), `security/cargo-audit.json` (SHA-256 `e45ff8defbe79e5430ec455b27885b69226a09145b74abd25d04bbefb6782224`), `security/accepted-advisories.md` (SHA-256 `1c649141cc4b45f24d4a3f8556be2d83d67ff32bd22b8fc56772771cacbcb63a`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 048 · `security.fuzz-sip-message` — parser fuzz smoke (sip_message)

- Result: **PASS** in 38 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (sip_message).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/sip/rvoip-sip/../fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=sip_message FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`557431e36a2704e51a92f98efc58decbd86180055cec9ad38fc81d91289409d6` (PASS).
- Evidence: `parser_fuzz_smoke_sip_message.log` (SHA-256 `557431e36a2704e51a92f98efc58decbd86180055cec9ad38fc81d91289409d6`), `security/fuzz/sip_message.log` (SHA-256 `bfd610a74f0a379607a31c3c40c8562f8337e47c9873057a6f8294821b214da4`), `security/fuzz/sip_message.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 049 · `security.fuzz-uri` — parser fuzz smoke (uri)

- Result: **PASS** in 1 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (uri).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/sip/rvoip-sip/../fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=uri FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`e70080dac6dbc8a15555f31e60cb571523312634bc1105aeb3e38ebe1bbd117e` (PASS).
- Evidence: `parser_fuzz_smoke_uri.log` (SHA-256 `e70080dac6dbc8a15555f31e60cb571523312634bc1105aeb3e38ebe1bbd117e`), `security/fuzz/uri.log` (SHA-256 `77572d024b2cdab625802208fe7919f4aec17aed4fbe6c142f76d40bfcb4e3ac`), `security/fuzz/uri.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 050 · `security.fuzz-header` — parser fuzz smoke (header)

- Result: **PASS** in 1 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (header).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/sip/rvoip-sip/../fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=header FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`b296068cf01eeefe6acbcaf6a34710dded95f4958d4545fc7f90930e1c81674b` (PASS).
- Evidence: `parser_fuzz_smoke_header.log` (SHA-256 `b296068cf01eeefe6acbcaf6a34710dded95f4958d4545fc7f90930e1c81674b`), `security/fuzz/header.log` (SHA-256 `96e22025f5c27cce0d44f29b4af1bfdadd707ab8aa7359a7ceba67e6da3016a3`), `security/fuzz/header.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 051 · `security.fuzz-sdp` — parser fuzz smoke (sdp)

- Result: **PASS** in 2 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (sdp).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/sip/rvoip-sip/../fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=sdp FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`e54e069c07bc10bb13c154bebd436e066aa2beb8edf0aee6cd996594a3a4c3fb` (PASS).
- Evidence: `parser_fuzz_smoke_sdp.log` (SHA-256 `e54e069c07bc10bb13c154bebd436e066aa2beb8edf0aee6cd996594a3a4c3fb`), `security/fuzz/sdp.log` (SHA-256 `57618a0f52863aef6d893c5a6015f011c9bb60d53ee17ea89f167acd6281ce3f`), `security/fuzz/sdp.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 052 · `security.fuzz-rtp` — parser fuzz smoke (rtp_packet)

- Result: **PASS** in 55 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (rtp_packet).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/media/fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=rtp_packet FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`e75c7c5ee334d3cf3635182e3af527124465857a671323d3869f7980704e800d` (PASS).
- Evidence: `parser_fuzz_smoke_rtp_packet.log` (SHA-256 `e75c7c5ee334d3cf3635182e3af527124465857a671323d3869f7980704e800d`), `security/fuzz/rtp_packet.log` (SHA-256 `aaf6aedb5085c538dd36431d5d676b192ad087cc54d49d2067655632a4784b39`), `security/fuzz/rtp_packet.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 053 · `security.fuzz-rtcp` — parser fuzz smoke (rtcp_packet)

- Result: **PASS** in 1 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (rtcp_packet).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/media/fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=rtcp_packet FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`bfea81079b5d04ca089d09ad8aadf7ce3d6eb439e0b6906cf62f61df4b6ba226` (PASS).
- Evidence: `parser_fuzz_smoke_rtcp_packet.log` (SHA-256 `bfea81079b5d04ca089d09ad8aadf7ce3d6eb439e0b6906cf62f61df4b6ba226`), `security/fuzz/rtcp_packet.log` (SHA-256 `606e2fd59844743ba605ce9249505b3397a4035a6fc4b741579b548cd2024675`), `security/fuzz/rtcp_packet.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 054 · `security.fuzz-srtp` — parser fuzz smoke (srtp_unprotect)

- Result: **PASS** in 2 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (srtp_unprotect).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/media/fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=srtp_unprotect FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`55960fb9026c06208a57c638546f87d086478da51dc1433b5ced59d27c0428d8` (PASS).
- Evidence: `parser_fuzz_smoke_srtp_unprotect.log` (SHA-256 `55960fb9026c06208a57c638546f87d086478da51dc1433b5ced59d27c0428d8`), `security/fuzz/srtp_unprotect.log` (SHA-256 `da2945c3c9ce6678d581cfc5d7c4e5f7c09ae5bef1b7fd92a36e9b99d821f782`), `security/fuzz/srtp_unprotect.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 055 · `security.fuzz-dtls` — parser fuzz smoke (dtls_record)

- Result: **PASS** in 2 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (dtls_record).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/media/fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=dtls_record FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`de38c62fe62799a2b80277b9af625262d1a98b16a15648cb3a84180d0c7636f2` (PASS).
- Evidence: `parser_fuzz_smoke_dtls_record.log` (SHA-256 `de38c62fe62799a2b80277b9af625262d1a98b16a15648cb3a84180d0c7636f2`), `security/fuzz/dtls_record.log` (SHA-256 `3f768704bc0163f46141c37b0e08b2c635fe9f6797521def920d7fee6534cb1f`), `security/fuzz/dtls_record.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 056 · `security.fuzz-stun` — parser fuzz smoke (stun_response)

- Result: **PASS** in 2 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (stun_response).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/media/fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=stun_response FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`44336b37bd8d57610234f52810d602efaf58c62920c35f7d5e897443b27a2f0b` (PASS).
- Evidence: `parser_fuzz_smoke_stun_response.log` (SHA-256 `44336b37bd8d57610234f52810d602efaf58c62920c35f7d5e897443b27a2f0b`), `security/fuzz/stun_response.log` (SHA-256 `cae3000ea6adc27d60615405e2a394350620a3290b4577e5154ff21a43606457`), `security/fuzz/stun_response.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

### 057 · `security.fuzz-g711` — parser fuzz smoke (g711_unpack)

- Result: **PASS** in 2 seconds.
- Purpose: Exercise dependency and parser-hardening evidence required by the security gate. Named scope: parser fuzz smoke (g711_unpack).
- Recorded component/command: `env FUZZ_CRATE_DIR=<workspace>/crates/media/fuzz WORKSPACE_ROOT=<workspace> FUZZ_TARGET=g711_unpack FUZZ_LOG=<source-report> BETA_FUZZ_SMOKE_RUNS=1000 BETA_FUZZ_SMOKE_SECONDS=10 BETA_FUZZ_TOOLCHAIN=nightly bash -c `.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_gate_require_external=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `command-exit-zero`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`c179347f61ca7a97298abb2e8c2bddd6a8cdd746f6c002a66e08de12f2519273` (PASS).
- Evidence: `parser_fuzz_smoke_g711_unpack.log` (SHA-256 `c179347f61ca7a97298abb2e8c2bddd6a8cdd746f6c002a66e08de12f2519273`), `security/fuzz/g711_unpack.log` (SHA-256 `4ddc8ff6efe7f021c8545c1f07e0471c23d332170b56640dd73b164abf4f5f93`), `security/fuzz/g711_unpack.version.txt` (SHA-256 `96d55979026675c25fd3870116aa604bd21b0a36d67378fe6199cb5b2ff4e0b7`).
- PASS establishes: The named security check completed without a gate failure.
- PASS does not establish: Does not prove absence of vulnerabilities or exhaustive parser safety.

## PBX and interoperability

16 required gates; 16 passed.

### 058 · `interop.freeswitch-down-before-asterisk` — local FreeSWITCH down before Asterisk

- Result: **PASS** in 0 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: local FreeSWITCH down before Asterisk.
- Recorded component/command: `<local-path>`.
- Why required: Required in full mode because `beta_run_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`c3ff1b05d9bcdb6d9f0baf9f92465923dec30b0fc6e77d66c4c29915e5ce3477` (PASS).
- Evidence: `local_freeswitch_down_before_asterisk.log` (SHA-256 `c3ff1b05d9bcdb6d9f0baf9f92465923dec30b0fc6e77d66c4c29915e5ce3477`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 059 · `interop.asterisk-up` — local Asterisk up

- Result: **PASS** in 2 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: local Asterisk up.
- Recorded component/command: `<local-path>`.
- Why required: Required in full mode because `beta_run_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`16c477d5bd1837eb06b7c42905b76dd7b730f440e22b9ef11c8fd20159122ac4` (PASS).
- Evidence: `local_asterisk_up.log` (SHA-256 `16c477d5bd1837eb06b7c42905b76dd7b730f440e22b9ef11c8fd20159122ac4`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 060 · `interop.asterisk-matrix` — local Asterisk PBX matrix

- Result: **PASS** in 767 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: local Asterisk PBX matrix.
- Recorded component/command: `env PBX_OUT_ROOT=<source-report> PBX_REPORT_APPEND=1 PBX_G729_PROFILES=g729a g729ab <workspace>/crates/sip/rvoip-sip/examples/pbx/run.sh --pbx asterisk --api all --scenario all`.
- Why required: Required in full mode because `beta_run_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`210fef4aec154c8e15e87653292c61d6f10f50c410aef285cbf6bac4a20fa51b` (PASS); asterisk PBX matrix rows=`{"pass":144,"rows":144}` (PASS).
- Evidence: `local_asterisk_pbx_matrix.log` (SHA-256 `210fef4aec154c8e15e87653292c61d6f10f50c410aef285cbf6bac4a20fa51b`), `pbx/matrix.tsv` (SHA-256 `c328238f7c70e5cbaca8cb2412c7014d1b4941fcf1341121373ec4d305f02365`), `pbx/summary.md` (SHA-256 `58a0140591e3d76021eb3b2cda659ac139ff3f7b292941302085cfbddb807a57`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 061 · `interop.asterisk-down-after` — local Asterisk down after matrix

- Result: **PASS** in 4 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: local Asterisk down after matrix.
- Recorded component/command: `<local-path>`.
- Why required: Required in full mode because `beta_run_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`c60a6d3fb3785c30dc3773ac1d04a0b39855fc3d98b71d787ed91091c06f72c6` (PASS).
- Evidence: `local_asterisk_down_after_matrix.log` (SHA-256 `c60a6d3fb3785c30dc3773ac1d04a0b39855fc3d98b71d787ed91091c06f72c6`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 062 · `interop.asterisk-down-before-freeswitch` — local Asterisk down before FreeSWITCH

- Result: **PASS** in 0 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: local Asterisk down before FreeSWITCH.
- Recorded component/command: `<local-path>`.
- Why required: Required in full mode because `beta_run_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`ed1127e5a292372c441c1ea515a818b9ab09c73e3b50b65995336279ff98a2bf` (PASS).
- Evidence: `local_asterisk_down_before_freeswitch.log` (SHA-256 `ed1127e5a292372c441c1ea515a818b9ab09c73e3b50b65995336279ff98a2bf`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 063 · `interop.freeswitch-up` — local FreeSWITCH up

- Result: **PASS** in 7 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: local FreeSWITCH up.
- Recorded component/command: `<local-path>`.
- Why required: Required in full mode because `beta_run_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`2bb38ae69e5d2bd3f5f59f2db1ee497ae46af42184d789d47edfcec2533e8787` (PASS).
- Evidence: `local_freeswitch_up.log` (SHA-256 `2bb38ae69e5d2bd3f5f59f2db1ee497ae46af42184d789d47edfcec2533e8787`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 064 · `interop.freeswitch-matrix` — local FreeSWITCH PBX matrix

- Result: **PASS** in 420 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: local FreeSWITCH PBX matrix.
- Recorded component/command: `env PBX_OUT_ROOT=<source-report> PBX_REPORT_APPEND=1 PBX_G729_PROFILES=g729a g729ab <workspace>/crates/sip/rvoip-sip/examples/pbx/run.sh --pbx freeswitch --api all --scenario all`.
- Why required: Required in full mode because `beta_run_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`5863084deb7129d506da0860baf706646e74c7f61156e75959f3f56f0b01efa0` (PASS); freeswitch PBX matrix rows=`{"pass":144,"rows":144}` (PASS).
- Evidence: `local_freeswitch_pbx_matrix.log` (SHA-256 `5863084deb7129d506da0860baf706646e74c7f61156e75959f3f56f0b01efa0`), `pbx/matrix.tsv` (SHA-256 `c328238f7c70e5cbaca8cb2412c7014d1b4941fcf1341121373ec4d305f02365`), `pbx/summary.md` (SHA-256 `58a0140591e3d76021eb3b2cda659ac139ff3f7b292941302085cfbddb807a57`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 065 · `interop.freeswitch-down-after` — local FreeSWITCH down after matrix

- Result: **PASS** in 11 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: local FreeSWITCH down after matrix.
- Recorded component/command: `<local-path>`.
- Why required: Required in full mode because `beta_run_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`18938c319821b90ca099a734906174ebfdd17f691645e13a4e07332354ee61e9` (PASS).
- Evidence: `local_freeswitch_down_after_matrix.log` (SHA-256 `18938c319821b90ca099a734906174ebfdd17f691645e13a4e07332354ee61e9`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 066 · `interop.restore-asterisk-down` — restore local Asterisk down

- Result: **PASS** in 0 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: restore local Asterisk down.
- Recorded component/command: `<local-path>`.
- Why required: Required in full mode because `beta_restore_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`a980729c57320f576d9f9a69652c1561a95e1d57b99728c96fdc82b94cf92a20` (PASS).
- Evidence: `restore_local_asterisk_down.log` (SHA-256 `a980729c57320f576d9f9a69652c1561a95e1d57b99728c96fdc82b94cf92a20`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 067 · `interop.restore-freeswitch-down` — restore local FreeSWITCH down

- Result: **PASS** in 1 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: restore local FreeSWITCH down.
- Recorded component/command: `<local-path>`.
- Why required: Required in full mode because `beta_restore_local_pbx` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`929a48851e9a3702d09d3d7265612df455f629701b950ab7f14de146e8d10cc5` (PASS).
- Evidence: `restore_local_freeswitch_down.log` (SHA-256 `929a48851e9a3702d09d3d7265612df455f629701b950ab7f14de146e8d10cc5`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 068 · `interop.sipp-build` — SIPp standalone target build

- Result: **PASS** in 429 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: SIPp standalone target build.
- Recorded component/command: `cargo build -p rvoip-sip --release --example perf_listener`.
- Why required: Required in full mode because `beta_run_sipp` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`dcef08b285fac15ebaec2efd7d61db9a673aec7eacee543fb3b6fa49dbca9a1a` (PASS).
- Evidence: `sipp_standalone_target_build.log` (SHA-256 `dcef08b285fac15ebaec2efd7d61db9a673aec7eacee543fb3b6fa49dbca9a1a`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 069 · `interop.sipp-start` — SIPp standalone target start

- Result: **PASS** in 0 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: SIPp standalone target start.
- Recorded component/command: `managed perf_listener lifecycle operation (command was not structured separately by the v1 runner)`.
- Why required: Required in full mode because `beta_run_sipp` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `legacy-v1-summary-and-shared-listener-log; future runs record the lifecycle result directly`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); shared listener log hash=`6f6dfd2be59149aa1603a73f353b5650bdf371eccec392d2b1989b51e0599c8e` (PASS).
- Evidence: `sipp/rvoip_perf_listener.log` (SHA-256 `6f6dfd2be59149aa1603a73f353b5650bdf371eccec392d2b1989b51e0599c8e`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 070 · `interop.sipp-matrix` — SIPp standalone matrix

- Result: **PASS** in 87 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: SIPp standalone matrix.
- Recorded component/command: `env RVOIP_PERF_RESULTS=<source-report> RVOIP_PERF_CPS=30 100 300 1000 2000 RVOIP_PERF_MIN_SUCCESS_PCT=99.9 <workspace>/crates/sip/rvoip-sip/tests/perf/sipp_scenarios/run_comparison.sh 127.0.0.1 35060 rvoip`.
- Why required: Required in full mode because `beta_run_sipp` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`e00fa5f76b8db5a6fb1fe39bff241d90173bc50bab11d3a8afde76e09a548e26` (PASS); SIPp matrix rows=`{"PASS":5,"rows":5}` (PASS).
- Evidence: `sipp_standalone_matrix.log` (SHA-256 `e00fa5f76b8db5a6fb1fe39bff241d90173bc50bab11d3a8afde76e09a548e26`), `sipp/runs.tsv` (SHA-256 `3175a1491c5791ede36a0239003c363300b1bd648ff6e698057090a89f4e0a34`), `sipp/run_summary.md` (SHA-256 `77fe1966e3fe741de69bb5b01c596ede855c507ae2b727d855450e414f1c1680`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 071 · `interop.sipp-stop` — SIPp standalone target stop

- Result: **PASS** in 1 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: SIPp standalone target stop.
- Recorded component/command: `managed perf_listener lifecycle operation (command was not structured separately by the v1 runner)`.
- Why required: Required in full mode because `beta_run_sipp` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `legacy-v1-summary-and-shared-listener-log; future runs record the lifecycle result directly`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); shared listener log hash=`6f6dfd2be59149aa1603a73f353b5650bdf371eccec392d2b1989b51e0599c8e` (PASS).
- Evidence: `sipp/rvoip_perf_listener.log` (SHA-256 `6f6dfd2be59149aa1603a73f353b5650bdf371eccec392d2b1989b51e0599c8e`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 072 · `interop.strict-ua` — baresip strict-UA matrix

- Result: **PASS** in 9 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: baresip strict-UA matrix.
- Recorded component/command: `env RVOIP_STRICT_UA_RESULTS=<source-report> <workspace>/crates/sip/rvoip-sip/tests/interop/baresip/run_strict_ua.sh`.
- Why required: Required in full mode because `beta_run_strict_ua` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`55acc0cc2e26f70679853e7629212a6f64597d6110b33edc1ae85909eb9277f5` (PASS); strict-UA matrix rows=`{"PASS":7,"rows":7}` (PASS).
- Evidence: `baresip_strict-ua_matrix.log` (SHA-256 `55acc0cc2e26f70679853e7629212a6f64597d6110b33edc1ae85909eb9277f5`), `strict-ua/matrix.tsv` (SHA-256 `952c55a0a20cb104421522917cd47ee1914fadd93cc0e4b62896c7a46480b11d`), `strict-ua/summary.md` (SHA-256 `47196a8a20a29782efed7aafa5c3db67f7d543891481c3ba3404a9a9c33e8cf6`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

### 073 · `interop.proxy-descope` — Kamailio/OpenSIPS proxy de-scope audit

- Result: **PASS** in 0 seconds.
- Purpose: Exercise SIP interoperability or managed peer lifecycle required by the selected full configuration. Named scope: Kamailio/OpenSIPS proxy de-scope audit.
- Recorded component/command: `bash -c set -euo pipefail`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_pbx_api=all`, `beta_pbx_g729_profiles=["g729a","g729ab"]`, `beta_pbx_provider=both`, `beta_pbx_scenario=all`, `beta_restore_asterisk_up=false`, `beta_restore_freeswitch_up=false`, `beta_restore_local_pbx=true`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`, `beta_run_local_pbx=true`, `beta_run_long_soak=true`, `beta_run_pbx=false`, `beta_run_perf_all=true`, `beta_run_sipp=true`, `beta_run_strict_ua=true`, `beta_sipp_cps=[30,100,300,1000,2000]`, `beta_sipp_diagnostics=false`.
- Expected checks: `status-pass`, `evidence-hash`, `interop-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`d3d6b322122cfbeaa3edd62a1d2623c27b02e185de3c0f1e739cd9b0e8bf473b` (PASS).
- Evidence: `kamailio_opensips_proxy_de-scope_audit.log` (SHA-256 `d3d6b322122cfbeaa3edd62a1d2623c27b02e185de3c0f1e739cd9b0e8bf473b`).
- PASS establishes: The named peer, matrix, or lifecycle condition passed with the recorded configuration.
- PASS does not establish: Does not establish compatibility with untested peers, versions, transports, codecs, or scenarios.

## Performance and resiliency

29 required gates; 29 passed.

### 074 · `perf.capture-boundary` — perf results capture boundary

- Result: **PASS** in 0 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf results capture boundary.
- Recorded component/command: `prepare_perf_results_capture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`df329aa7d9d5181baa9afa5db40dfb8f3a1323e56154fc6e8322a41e3eec94b3` (PASS).
- Evidence: `perf_results_capture_boundary.log` (SHA-256 `df329aa7d9d5181baa9afa5db40dfb8f3a1323e56154fc6e8322a41e3eec94b3`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 075 · `perf.literal-all-config` — literal-all perf configuration

- Result: **PASS** in 0 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: literal-all perf configuration.
- Recorded component/command: `env BETA_RUN_BURST_MATRIX=1 BETA_BURST_MATRIX=all BETA_RUN_LONG_SOAK=1 RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0 bash -c `.
- Why required: Required in full mode because `beta_run_perf_all` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`d9da4356dd16b83a4726c98746d84d24a9623262faac4856d26a0444a4e9dfb8` (PASS).
- Evidence: `literal-all_perf_configuration.log` (SHA-256 `d9da4356dd16b83a4726c98746d84d24a9623262faac4856d26a0444a4e9dfb8`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 076 · `perf.call-setup-endpoint` — perf call setup CPS (endpoint)

- Result: **PASS** in 513 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf call setup CPS (endpoint).
- Recorded component/command: `env RVOIP_PERF_PROFILE=endpoint RVOIP_PERF_REPORT_SCENARIO=perf_call_setup_cps_endpoint RVOIP_PERF_SWEEP_CPS=30 cargo test -p rvoip-sip --release --features perf-tests --test perf_call_setup_cps -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`05912dcfd213c1ccb1cd9796a4d044014e36d043ec869a0a1372c0926969c9c5` (PASS).
- Evidence: `perf_call_setup_cps_endpoint.log` (SHA-256 `05912dcfd213c1ccb1cd9796a4d044014e36d043ec869a0a1372c0926969c9c5`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 077 · `perf.call-setup-pbx` — perf call setup CPS (pbx-media-server)

- Result: **PASS** in 204 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf call setup CPS (pbx-media-server).
- Recorded component/command: `env RVOIP_PERF_PROFILE=pbx-media-server RVOIP_PERF_REPORT_SCENARIO=perf_call_setup_cps_pbx-media-server RVOIP_PERF_SWEEP_CPS=30,100,300,1000,2000 cargo test -p rvoip-sip --release --features perf-tests --test perf_call_setup_cps -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`9c46e33d7260aa1bec523cbe11b32a4d05ddca8288cda9ec293b8fc436952149` (PASS).
- Evidence: `perf_call_setup_cps_pbx-media-server.log` (SHA-256 `9c46e33d7260aa1bec523cbe11b32a4d05ddca8288cda9ec293b8fc436952149`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 078 · `perf.call-setup-signaling` — perf call setup CPS (signaling-only-server-high-performance)

- Result: **PASS** in 203 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf call setup CPS (signaling-only-server-high-performance).
- Recorded component/command: `env RVOIP_PERF_PROFILE=signaling-only-server-high-performance RVOIP_PERF_REPORT_SCENARIO=perf_call_setup_cps_signaling-only-server-high-performance RVOIP_PERF_SWEEP_CPS=30,100,300,1000,2000 cargo test -p rvoip-sip --release --features perf-tests --test perf_call_setup_cps -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`81781f018d7d2d39a7629415484cfaf3d49851aaee468fd358318ad963216098` (PASS).
- Evidence: `perf_call_setup_cps_signaling-only-server-high-performance.log` (SHA-256 `81781f018d7d2d39a7629415484cfaf3d49851aaee468fd358318ad963216098`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 079 · `perf.registration` — perf registration throughput

- Result: **PASS** in 257 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf registration throughput.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests --test perf_registration_throughput -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`c8dc6a4c7632fc9afb79b7b61e9d73cd7d3667f2a5d6c201439f92183437f22b` (PASS).
- Evidence: `perf_registration_throughput.log` (SHA-256 `c8dc6a4c7632fc9afb79b7b61e9d73cd7d3667f2a5d6c201439f92183437f22b`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 080 · `perf.concurrent-calls` — perf concurrent active calls

- Result: **PASS** in 255 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf concurrent active calls.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests --test perf_concurrent_active_calls -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`262e9615030374ada97f8d7025bb1f9272577687a97fe565fcccced9a0459260` (PASS).
- Evidence: `perf_concurrent_active_calls.log` (SHA-256 `262e9615030374ada97f8d7025bb1f9272577687a97fe565fcccced9a0459260`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 081 · `perf.rtp-steady-state` — perf RTP steady state

- Result: **PASS** in 257 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf RTP steady state.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests --test perf_rtp_steady_state -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`d72d91bc9b4bdb9b0037a8f01a1242b97071d47966b11a1b60fc9857a515de89` (PASS).
- Evidence: `perf_rtp_steady_state.log` (SHA-256 `d72d91bc9b4bdb9b0037a8f01a1242b97071d47966b11a1b60fc9857a515de89`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 082 · `perf.backpressure` — perf backpressure step

- Result: **PASS** in 335 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf backpressure step.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests --test perf_backpressure_step -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`c485ebaaa73d74527cce6930ca2b848c6dcb0d1346b53ec3d8bf65383077abc1` (PASS).
- Evidence: `perf_backpressure_step.log` (SHA-256 `c485ebaaa73d74527cce6930ca2b848c6dcb0d1346b53ec3d8bf65383077abc1`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 083 · `perf.transport-recovery` — perf transport recovery

- Result: **PASS** in 276 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf transport recovery.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests --test perf_transport_recovery -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`72793c793bbe6a71b3f8350ae1bea4c1c98a9b179d297dd7527f55a295ccd15c` (PASS).
- Evidence: `perf_transport_recovery.log` (SHA-256 `72793c793bbe6a71b3f8350ae1bea4c1c98a9b179d297dd7527f55a295ccd15c`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 084 · `perf.resiliency-all` — all registered resiliency tests

- Result: **PASS** in 524 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: all registered resiliency tests.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test resilien* -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`04ad58bc72be83faa3b377e91ebcfbbbe75a75c08940ff717eee08188c5a2f93` (PASS).
- Evidence: `all_registered_resiliency_tests.log` (SHA-256 `04ad58bc72be83faa3b377e91ebcfbbbe75a75c08940ff717eee08188c5a2f93`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 085 · `perf.mid-call-signaling` — perf mid-call signaling under media

- Result: **PASS** in 254 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf mid-call signaling under media.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_mid_call_signal_under_media -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`851f7281d435cd8cc393567694c34be1042cff854712e831d96d598023946b44` (PASS).
- Evidence: `perf_mid-call_signaling_under_media.log` (SHA-256 `851f7281d435cd8cc393567694c34be1042cff854712e831d96d598023946b44`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 086 · `perf.tls-overhead` — perf TLS overhead

- Result: **PASS** in 282 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf TLS overhead.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_tls_overhead -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`1059fb98d4e17474b848459c554de1f9a038c4ac2f17f137c41df43ddec80408` (PASS).
- Evidence: `perf_tls_overhead.log` (SHA-256 `1059fb98d4e17474b848459c554de1f9a038c4ac2f17f137c41df43ddec80408`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 087 · `perf.srtp-overhead` — perf SRTP overhead

- Result: **PASS** in 254 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf SRTP overhead.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_srtp_overhead -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`ccd6fb6d4f378221f3354644e940e53482baeffa4828ff57d763815d1112e0a1` (PASS).
- Evidence: `perf_srtp_overhead.log` (SHA-256 `ccd6fb6d4f378221f3354644e940e53482baeffa4828ff57d763815d1112e0a1`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 088 · `perf.pdd-180` — perf PDD with 180 first

- Result: **PASS** in 280 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf PDD with 180 first.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_pdd_with_180_first -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`2919d554757dcd51e141d25ab66081736b19058f78fc9042dbbf1d46ca8a65a7` (PASS).
- Evidence: `perf_pdd_with_180_first.log` (SHA-256 `2919d554757dcd51e141d25ab66081736b19058f78fc9042dbbf1d46ca8a65a7`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 089 · `perf.long-duration` — perf sustained long-duration calls

- Result: **PASS** in 342 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf sustained long-duration calls.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_sustained_long_duration_calls -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`5d51a54c01b863a17cf781feb21a20781ac8910964f7ddf5036d4ed14f879715` (PASS).
- Evidence: `perf_sustained_long-duration_calls.log` (SHA-256 `5d51a54c01b863a17cf781feb21a20781ac8910964f7ddf5036d4ed14f879715`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 090 · `perf.registrar-scale` — perf registrar binding scale

- Result: **PASS** in 252 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf registrar binding scale.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_registrar_binding_scale -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`6317984a28359f47d0e66ec6970d7c657c324af3a57a2b20477e8b96b53e55c3` (PASS).
- Evidence: `perf_registrar_binding_scale.log` (SHA-256 `6317984a28359f47d0e66ec6970d7c657c324af3a57a2b20477e8b96b53e55c3`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 091 · `perf.mixed-workload` — perf mixed workload

- Result: **PASS** in 267 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf mixed workload.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_mixed_workload -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`72184a2f9dcb84e054691f357c9e4a2aecf1b531fec5c77469122c8ee8a6ebc9` (PASS).
- Evidence: `perf_mixed_workload.log` (SHA-256 `72184a2f9dcb84e054691f357c9e4a2aecf1b531fec5c77469122c8ee8a6ebc9`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 092 · `perf.b2bua` — perf B2BUA forwarding

- Result: **PASS** in 290 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf B2BUA forwarding.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_b2bua_forwarding -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`03fd0e5db955107580b14e32eaf8419b3efbff34b4a3e4ad63a12be8271c383d` (PASS).
- Evidence: `perf_b2bua_forwarding.log` (SHA-256 `03fd0e5db955107580b14e32eaf8419b3efbff34b4a3e4ad63a12be8271c383d`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 093 · `perf.ai-agent` — perf AI-agent load

- Result: **PASS** in 255 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf AI-agent load.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_ai_agent_load -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`00aa1d90b94dc4bf25e4c535e48778252781eb6d192f9f96186355c27fff6981` (PASS).
- Evidence: `perf_ai-agent_load.log` (SHA-256 `00aa1d90b94dc4bf25e4c535e48778252781eb6d192f9f96186355c27fff6981`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 094 · `perf.contact-center` — perf contact-center transfers

- Result: **PASS** in 261 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf contact-center transfers.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_contact_center_transfers -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`8678b983976c75584e04ace3203d28c74da9781049e4e79905f11e2f7115bec1` (PASS).
- Evidence: `perf_contact-center_transfers.log` (SHA-256 `8678b983976c75584e04ace3203d28c74da9781049e4e79905f11e2f7115bec1`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 095 · `perf.sipp-parity` — perf SIPp parity

- Result: **PASS** in 251 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf SIPp parity.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_sipp_parity -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`890b9ea21a0e077b2f31f0b0e41530b62de584ab8c037220d06f43a438640f63` (PASS).
- Evidence: `perf_sipp_parity.log` (SHA-256 `890b9ea21a0e077b2f31f0b0e41530b62de584ab8c037220d06f43a438640f63`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 096 · `perf.soak-invariants` — perf soak target invariant tests

- Result: **PASS** in 269 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf soak target invariant tests.
- Recorded component/command: `cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_soak_caller --test perf_soak_30min -- --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`0607d76fd063dd5b8de0153fdd56f7e55064170c09f7a9056ea5c214d5acb7a7` (PASS).
- Evidence: `perf_soak_target_invariant_tests.log` (SHA-256 `0607d76fd063dd5b8de0153fdd56f7e55064170c09f7a9056ea5c214d5acb7a7`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 097 · `perf.media-churn` — perf media churn

- Result: **PASS** in 334 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf media churn.
- Recorded component/command: `env RVOIP_PERF_SOAK_DURATION_SECS=120 RVOIP_PERF_SOAK_ACTIVE_CALLS=30 cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_media_churn perf_media_churn -- --exact --ignored --nocapture`.
- Why required: Required in full mode because `beta_run_perf_all` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`e8c52385965539525551a65899af9806a8dec185b71270bafae6bf23a61d4b9d` (PASS).
- Evidence: `perf_media_churn.log` (SHA-256 `e8c52385965539525551a65899af9806a8dec185b71270bafae6bf23a61d4b9d`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 098 · `perf.monolithic-soak` — perf monolithic soak

- Result: **PASS** in 3763 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf monolithic soak.
- Recorded component/command: `env RVOIP_PERF_SOAK_DURATION_SECS=3600 RVOIP_PERF_SOAK_ACTIVE_CALLS=30 RVOIP_PERF_SOAK_DRAIN_CPS=10 RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR=15 RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT=32 RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS=160 RVOIP_PERF_ARCHIVE_DIR=<source-report> cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_soak_30min perf_soak_30min -- --exact --ignored --nocapture`.
- Why required: Required in full mode because `beta_run_perf_all` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`0b08695ea386864f6388a7e2d90fec911b1437225e64a9c97108f96bd67c5c1d` (PASS).
- Evidence: `perf_monolithic_soak.log` (SHA-256 `0b08695ea386864f6388a7e2d90fec911b1437225e64a9c97108f96bd67c5c1d`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 099 · `perf.mass-teardown` — perf mass teardown stress

- Result: **PASS** in 180 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf mass teardown stress.
- Recorded component/command: `env RVOIP_PERF_MASS_TEARDOWN_CALLS=500 RVOIP_PERF_MASS_TEARDOWN_SETUP_CPS=30 RVOIP_PERF_SOAK_ERROR_SAMPLE_LIMIT=32 RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS=160 RVOIP_PERF_ARCHIVE_DIR=<source-report> cargo test -p rvoip-sip --release --features perf-tests,dev-insecure-tls --test perf_soak_30min perf_mass_teardown_stress -- --exact --ignored --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`20f4c00391317e79308ac60113e0b632bb67b7342ab0f00c9fdada3f1505ba9f` (PASS).
- Evidence: `perf_mass_teardown_stress.log` (SHA-256 `20f4c00391317e79308ac60113e0b632bb67b7342ab0f00c9fdada3f1505ba9f`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 100 · `perf.session-churn` — perf session churn leak

- Result: **PASS** in 416 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf session churn leak.
- Recorded component/command: `env RVOIP_PERF_RETENTION_DRAIN_WAIT_SECS=160 cargo test -p rvoip-sip --release --features perf-tests --test perf_soak_30min perf_session_churn_leak -- --ignored --nocapture`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`d73a6982152c5c025f380e927fe4bbbce88eb1c4ee43f27f45a079a26a0183ed` (PASS).
- Evidence: `perf_session_churn_leak.log` (SHA-256 `d73a6982152c5c025f380e927fe4bbbce88eb1c4ee43f27f45a079a26a0183ed`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 101 · `perf.media-burst-matrix` — perf media burst matrix

- Result: **PASS** in 5574 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf media burst matrix.
- Recorded component/command: `env RVOIP_PERF_FEATURES=perf-tests RVOIP_PERF_BURST_SCENARIO_FILE=<workspace>/crates/sip/rvoip-sip/config/perf-burst-scenarios.yaml RVOIP_PERF_BURST_SCENARIOS=all RVOIP_PERF_MEMORY_DIAGNOSTICS=0 RVOIP_PERF_ALLOCATOR_DIAGNOSTICS=0 RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS=5 RVOIP_PERF_MIMALLOC_COLLECT_AT=off RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0 RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR=15 <workspace>/crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh`.
- Why required: Required in full mode because `beta_run_burst_matrix` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`b430fd225dc226b38df59e501a667654fe1121b4bac662654eb6cff3861aa862` (PASS).
- Evidence: `perf_media_burst_matrix.log` (SHA-256 `b430fd225dc226b38df59e501a667654fe1121b4bac662654eb6cff3861aa862`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

### 102 · `perf.soak-candidate` — perf soak candidate

- Result: **PASS** in 4410 seconds.
- Purpose: Exercise one required performance, resiliency, soak, regression, or evidence-integrity condition. Named scope: perf soak candidate.
- Recorded component/command: `env RVOIP_PERF_FEATURES=perf-tests RVOIP_PERF_SOAK_DURATION_SECS=3600 RVOIP_PERF_SOAK_ACTIVE_CALLS=500 RVOIP_PERF_SOAK_MIN_HOLD_SECS=10 RVOIP_PERF_SOAK_MAX_HOLD_SECS=360 RVOIP_PERF_SOAK_CPS=0 RVOIP_PERF_MEMORY_DIAGNOSTICS=0 RVOIP_PERF_ALLOCATOR_DIAGNOSTICS=0 RVOIP_PERF_MEMORY_DIAG_INTERVAL_SECS=5 RVOIP_PERF_MIMALLOC_COLLECT_AT=off RVOIP_PERF_SYSTEM_ALLOCATOR=0 RVOIP_PERF_DHAT=0 RVOIP_PERF_HEAP_SNAPSHOTS=0 RVOIP_PERF_HEAP_SNAPSHOT_SECS= RVOIP_PERF_MALLOC_STACK_LOGGING=0 RVOIP_PERF_LEAKS_SNAPSHOTS=0 RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY=0 RVOIP_PERF_MAX_RSS_GROWTH_MB_PER_HR=15 RVOIP_PERF_EXTERNAL_RESOURCE_SAMPLER=1 <workspace>/crates/sip/rvoip-sip/scripts/perf_soak_split.sh`.
- Why required: Required in full mode because `beta_run_long_soak` was enabled; scheduled conditional gates are release-blocking.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `performance-result`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`cbcf1fbbb1e9e7d8d7886b882883b05c0899e6859650c3b3ee65badd25708dae` (PASS).
- Evidence: `perf_soak_candidate.log` (SHA-256 `cbcf1fbbb1e9e7d8d7886b882883b05c0899e6859650c3b3ee65badd25708dae`).
- PASS establishes: The named workload and its applicable thresholds passed under the recorded configuration.
- PASS does not establish: Does not predict untested hardware, workloads, durations, concurrency, or network conditions.

## Reporting and regression

4 required gates; 4 passed.

### 103 · `report.regression-baseline` — perf regression baseline evidence

- Result: **PASS** in 0 seconds.
- Purpose: Validate or capture evidence required to make the release result reproducible and auditable. Named scope: perf regression baseline evidence.
- Recorded component/command: `python3 <workspace>/crates/sip/rvoip-sip/scripts/perf_regression_baseline.py package --manifest <workspace>/crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json --source-root <workspace>/crates/sip/rvoip-sip/perf-baselines/20260706T181609Z --artifact-dir <source-report>`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `reporting-check`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`36b517ef42ec89780f5dc30f5fc59067a1166be5fb42d2828d1c6d5eb0295c01` (PASS).
- Evidence: `perf_regression_baseline_evidence.log` (SHA-256 `36b517ef42ec89780f5dc30f5fc59067a1166be5fb42d2828d1c6d5eb0295c01`).
- PASS establishes: The named evidence-integrity check completed successfully.
- PASS does not establish: Does not add runtime coverage beyond the evidence it validates.

### 104 · `report.regression-audit` — perf regression audit

- Result: **PASS** in 0 seconds.
- Purpose: Validate or capture evidence required to make the release result reproducible and auditable. Named scope: perf regression audit.
- Recorded component/command: `python3 <workspace>/crates/sip/rvoip-sip/scripts/perf_audit.py --baseline <source-report> --baseline-manifest <source-report> --current <workspace>/target/perf-results --out <source-report> --tolerance-pct 15 --latency-tolerance-pct 25 --fail-on-regression`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `reporting-check`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`093d7e71f856f32debf361ae41bf24675461240b7293b7be7c7a6041dce20682` (PASS).
- Evidence: `perf_regression_audit.log` (SHA-256 `093d7e71f856f32debf361ae41bf24675461240b7293b7be7c7a6041dce20682`).
- PASS establishes: The named evidence-integrity check completed successfully.
- PASS does not establish: Does not add runtime coverage beyond the evidence it validates.

### 105 · `report.perf-evidence-capture` — perf results evidence capture

- Result: **PASS** in 0 seconds.
- Purpose: Validate or capture evidence required to make the release result reproducible and auditable. Named scope: perf results evidence capture.
- Recorded component/command: `capture_current_perf_results`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `reporting-check`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`53a8ee8a9a49bfe92ed40f76b5f81e979d18712653edb478f588a8a41447ca42` (PASS).
- Evidence: `perf_results_evidence_capture.log` (SHA-256 `53a8ee8a9a49bfe92ed40f76b5f81e979d18712653edb478f588a8a41447ca42`).
- PASS establishes: The named evidence-integrity check completed successfully.
- PASS does not establish: Does not add runtime coverage beyond the evidence it validates.

### 106 · `report.performance-metrics` — performance gate metrics report

- Result: **PASS** in 0 seconds.
- Purpose: Validate or capture evidence required to make the release result reproducible and auditable. Named scope: performance gate metrics report.
- Recorded component/command: `write_performance_gate_metrics`.
- Why required: Unconditionally required by the full beta release profile.
- Evidence strength: `direct-v1-gate-log`.
- Relevant configuration: `beta_burst_matrix=all`, `beta_burst_scenario_file=bundled config/perf-burst-scenarios.yaml`, `beta_perf_features=["perf-tests"]`, `beta_perf_high_density_burst_cps=160`, `beta_perf_high_density_min_asr=0.995`, `beta_perf_high_density_rss_limit_mb_per_hr=15.0`, `beta_perf_infra_memory_diagnostics=false`, `beta_perf_latency_tolerance_pct=25.0`, `beta_perf_media_churn_active_calls=30`, `beta_perf_media_churn_duration_secs=120`, `beta_perf_media_diagnostics=false`, `beta_perf_media_memory_diagnostics=false`, `beta_perf_monolithic_soak_active_calls=30`, `beta_perf_monolithic_soak_duration_secs=3600`, `beta_perf_regression_baseline_id=20260706T181609Z`, `beta_perf_regression_baseline_manifest=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z/manifest.json`, `beta_perf_regression_baseline_manifest_sha256=739f35519904318db5e5d9e67755b573679530fc3dcdc290fc79e441f07eb6e9`, `beta_perf_regression_baseline_root=crates/sip/rvoip-sip/perf-baselines/20260706T181609Z`, `beta_perf_regression_fail=true`, `beta_perf_rtp_memory_diagnostics=false`, `beta_profile_matrix=endpoint:30 pbx-media-server:30,100,300,1000,2000 signaling-only-server-high-performance:30,100,300,1000,2000`, `beta_run_burst_matrix=true`, `beta_run_burst_smoke=true`, `beta_run_fuzz_smoke=true`.
- Expected checks: `status-pass`, `evidence-hash`, `reporting-check`.
- Observed checks: recorded status=`PASS` (PASS); command exit status=`0` (PASS); evidence SHA-256=`4b407e1615712fe703e87540d0a2de077826e3db410e3841e9ff7f26ae01f0d0` (PASS).
- Evidence: `performance_gate_metrics_report.log` (SHA-256 `4b407e1615712fe703e87540d0a2de077826e3db410e3841e9ff7f26ae01f0d0`).
- PASS establishes: The named evidence-integrity check completed successfully.
- PASS does not establish: Does not add runtime coverage beyond the evidence it validates.

## Interpretation

A gate PASS is bounded by its recorded command, configuration, evidence, and explicit non-claims. The gate report does not turn component evidence into a broader protocol, security, portability, or capacity claim.
