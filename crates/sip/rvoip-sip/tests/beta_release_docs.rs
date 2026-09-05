use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref())
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.as_ref().display()))
}

fn markdown_cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

fn inline_code_tokens(cell: &str) -> Vec<&str> {
    cell.split('`')
        .enumerate()
        .filter_map(|(index, token)| (index % 2 == 1).then_some(token))
        .collect()
}

fn executable_evidence_catalog(catalog: &str) -> HashMap<String, String> {
    catalog
        .lines()
        .filter_map(|line| {
            let cells = markdown_cells(line);
            if cells.len() < 4 {
                return None;
            }
            let evidence_id = cells[0].trim_matches('`');
            if !evidence_id.starts_with("T-") {
                return None;
            }
            let exact_test = cells[2].trim_matches('`');
            Some((evidence_id.to_string(), exact_test.to_string()))
        })
        .collect()
}

fn test_declaration_line(source: &str, test_name: &str) -> Option<usize> {
    let signature = format!("fn {test_name}(");
    source.lines().position(|line| {
        let line = line.trim_start();
        ["", "async ", "pub ", "pub async "].iter().any(|prefix| {
            line.strip_prefix(prefix)
                .is_some_and(|line| line.starts_with(&signature))
        })
    })
}

fn test_is_executable_and_live(source: &str, declaration_line: usize) -> bool {
    let lines: Vec<_> = source.lines().collect();
    let attribute_start = lines[..declaration_line]
        .iter()
        .rposition(|line| {
            let line = line.trim_start();
            line == "}"
                || line.starts_with("fn ")
                || line.starts_with("async fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("pub async fn ")
        })
        .map_or(0, |index| index + 1);
    let attributes = &lines[attribute_start..declaration_line];
    let executable = attributes.iter().any(|line| {
        let line = line.trim_start();
        line.starts_with("#[test]") || line.starts_with("#[tokio::test")
    });
    let ignored = attributes
        .iter()
        .any(|line| line.trim_start().starts_with("#[ignore"));

    let next_test = lines[declaration_line + 1..]
        .iter()
        .position(|line| {
            let line = line.trim_start();
            line.starts_with("#[test]") || line.starts_with("#[tokio::test")
        })
        .map_or(lines.len(), |offset| declaration_line + 1 + offset);
    let body = lines[declaration_line..next_test].join("\n");
    let stub_markers = [
        "document_stub(",
        "todo!(",
        "unimplemented!(",
        "assert!(true",
        "panic!(\"stub",
    ];

    executable && !ignored && !stub_markers.iter().any(|marker| body.contains(marker))
}

#[test]
fn beta_release_docs_exist_and_archived_docs_are_out_of_active_set() {
    let docs = manifest_dir().join("docs");
    let required = [
        "BETA_RELEASE_CHECKLIST.md",
        "COMPATIBILITY_MATRIX.md",
        "RFC_COMPLIANCE_MATRIX.md",
        "TOPOLOGY_PROFILES.md",
        "INTEROP_CI_PLAN.md",
        "SECURITY_POSTURE.md",
        "BETA_RELEASE_REPORT.md",
        "BETA_GATE_REPORT.md",
        "BETA_PERFORMANCE_REPORT.md",
        "RELEASE_NOTES_NEXT.md",
        "TUNING.md",
    ];

    for file in required {
        assert!(docs.join(file).is_file(), "missing beta doc {file}");
    }

    let archived = [
        "NEXT_STEPS.md",
        "RVOIP_VS_ASTERISK.md",
        "CLEANUP_BACKLOG_5000_CPS_INVESTIGATION.md",
        "CONFIG_ONLY_8000_CPS_INVESTIGATION.md",
        "SIGNALING_SHARDING_PERF_EXPERIMENT.md",
        "DIALOG_CORE_HOT_PATH_INVESTIGATION_PLAN.md",
        "DIALOG_CORE_NEXT_HOT_PATH_INVESTIGATION_PLAN.md",
        "DIALOG_CORE_BYE_TERMINATION_HOT_PATH_PLAN.md",
    ];

    for file in archived {
        assert!(
            !docs.join(file).exists(),
            "{file} must stay out of active docs"
        );
        assert!(
            docs.join("archived").join(file).is_file(),
            "{file} must exist under docs/archived"
        );
    }
}

#[test]
fn current_beta_reports_are_complete_and_match_immutable_snapshot() {
    let crate_dir = manifest_dir();
    let docs = crate_dir.join("docs");
    let snapshot = docs.join("releases/qualification/20260905T133559Z-33969263241");
    let reports = [
        "BETA_RELEASE_REPORT.md",
        "BETA_GATE_REPORT.md",
        "BETA_PERFORMANCE_REPORT.md",
        "QUALIFICATION_SUMMARY.json",
        "QUALIFICATION_REPORT_ATTESTATION.json",
        "QUALIFICATION_REPORT_ATTESTATION.json.sha256",
    ];

    for report in reports {
        let current = docs.join(report);
        let immutable = snapshot.join(report);
        assert!(
            immutable.is_file(),
            "missing immutable beta report {report}"
        );
        assert_eq!(
            fs::read(&current).unwrap(),
            fs::read(&immutable).unwrap(),
            "current {report} must match the immutable candidate snapshot"
        );
    }

    let release = read(docs.join("BETA_RELEASE_REPORT.md"));
    let gates = read(docs.join("BETA_GATE_REPORT.md"));
    let performance = read(docs.join("BETA_PERFORMANCE_REPORT.md"));
    let summary = read(docs.join("QUALIFICATION_SUMMARY.json"));
    let attestation = read(docs.join("QUALIFICATION_REPORT_ATTESTATION.json"));
    let policy = read(crate_dir.join("config/beta-release-policy.yaml"));
    let index = read(docs.join("releases/qualification/README.md"));

    assert!(release.contains("208/208 passed"));
    assert!(release.contains("108/108 covered"));
    assert!(release.contains("8cab44b10f872d21b304c02111d5d203ee8226da"));
    assert!(gates.contains("PASS — 208/208 remote-release gates passed; 0 failed"));
    assert!(gates.contains("workspace unit tests"));
    assert!(gates.contains("SIPp standalone target start"));
    assert!(performance.contains("perf_call_setup_cps_pbx-media-server"));
    assert!(performance.contains("up to 2,000 CPS with media enabled"));
    assert!(performance.contains("not public-network latency or carrier capacity"));
    assert!(summary.contains("\"gate_count\": 208"));
    assert!(summary.contains("\"legacy_covered_count\": 108"));
    assert!(attestation.contains("rvoip-release-qualification-report-attestation-v1"));
    assert!(attestation
        .contains("sha256:0b5dd80b42be87b0823bba9224a983db4be855712e2f463d6849fb2d4f21b051"));
    assert!(policy.contains("\"expected_selected_gate_count\": 108"));
    assert!(index.contains("33969263241"));
    assert!(index.contains("8cab44b10f872d21b304c02111d5d203ee8226da"));
}

#[test]
fn active_release_docs_keep_high_cps_and_webrtc_as_non_claims() {
    let docs = manifest_dir().join("docs");
    let compatibility = read(docs.join("COMPATIBILITY_MATRIX.md"));
    let topology = read(docs.join("TOPOLOGY_PROFILES.md"));
    let performance = read(docs.join("BETA_PERFORMANCE_REPORT.md"));
    let notes = read(docs.join("RELEASE_NOTES_NEXT.md"));

    assert!(compatibility.contains("| General full-media | Beta target | Up to 2,000 CPS |"));
    assert!(performance.contains("up to 2,000 CPS with media enabled"));
    assert!(topology.contains("| Browser/WebRTC edge | Post-beta |"));
    assert!(compatibility.contains("| DTLS-SRTP | Post-beta |"));
    assert!(compatibility.contains("| ICE/TURN/WebRTC browser | Post-beta |"));
    assert!(notes.contains("General-user 10,000 CPS full-media capability"));
}

#[test]
fn beta_release_docs_require_security_gate_and_no_placeholder_results() {
    let docs = manifest_dir().join("docs");
    let checklist = read(docs.join("BETA_RELEASE_CHECKLIST.md"));
    let security = read(docs.join("SECURITY_POSTURE.md"));
    let performance = read(docs.join("BETA_PERFORMANCE_REPORT.md"));

    assert!(checklist.contains("scripts/beta_gate.sh --security"));
    assert!(security.contains("security/cargo-audit.txt"));
    assert!(security.contains("security/fuzz/sip_message.log"));
    assert!(
        !performance.contains("TBD"),
        "performance report must contain current values, not placeholders"
    );
}

#[test]
fn verified_rfc_claims_use_existing_non_ignored_non_stub_tests() {
    let workspace = manifest_dir().join("../../..");
    let matrix = read(workspace.join("docs/sip/SIP_RFC_COMPLIANCE.md"));
    let catalog = read(manifest_dir().join("docs/RFC_COMPLIANCE_MATRIX.md"));
    let evidence = executable_evidence_catalog(&catalog);
    let mut verified_rows = 0_usize;

    for line in matrix
        .lines()
        .filter(|line| line.starts_with('|') && line.contains("✅ **Verified**"))
    {
        let cells = markdown_cells(line);
        if cells.len() == 2 && cells[0] == "✅ **Verified**" {
            continue;
        }
        verified_rows += 1;
        assert_eq!(
            cells.len(),
            5,
            "Verified RFC row must retain the five-column matrix shape: {line}"
        );
        let evidence_cell = cells[4];
        assert!(
            !evidence_cell.contains(".rs")
                && !evidence_cell.contains("#[ignore")
                && !evidence_cell.to_ascii_lowercase().contains("stub"),
            "Verified RFC rows must cite catalog IDs, not raw or ignored/stub evidence: {line}"
        );
        let evidence_ids: Vec<_> = inline_code_tokens(evidence_cell)
            .into_iter()
            .filter(|token| token.starts_with("T-"))
            .collect();
        assert!(
            !evidence_ids.is_empty(),
            "Verified RFC row has no executable T-* evidence ID: {line}"
        );

        for evidence_id in evidence_ids {
            let exact_test = evidence.get(evidence_id).unwrap_or_else(|| {
                panic!("Verified RFC row cites unknown evidence ID {evidence_id}: {line}")
            });
            let (relative_path, test_name) = exact_test.rsplit_once("::").unwrap_or_else(|| {
                panic!("evidence {evidence_id} has no path::test target: {exact_test}")
            });
            let source_path = workspace.join(relative_path);
            assert!(
                source_path.is_file(),
                "evidence {evidence_id} source does not exist: {}",
                source_path.display()
            );
            let source = read(&source_path);
            let declaration_line = test_declaration_line(&source, test_name).unwrap_or_else(|| {
                panic!(
                    "evidence {evidence_id} test {test_name} does not exist in {}",
                    source_path.display()
                )
            });
            assert!(
                test_is_executable_and_live(&source, declaration_line),
                "Verified RFC row cites non-executable, ignored, or stub evidence {evidence_id}: {exact_test}"
            );
        }
    }

    assert!(
        verified_rows > 0,
        "RFC matrix must retain at least one bounded Verified claim so this fence cannot pass vacuously"
    );
}

#[test]
fn crate_readmes_do_not_make_unqualified_beta_production_claims() {
    // rvoip-sip now lives at crates/sip/rvoip-sip, so the workspace root is
    // three levels up (was two before the directory reorg).
    let workspace = manifest_dir().join("../../..");
    let readmes = [
        "README.md",
        "crates/sip/rvoip-sip/README.md",
        "crates/sip/sip-core/README.md",
        "crates/sip/sip-transport/README.md",
        "crates/sip/sip-dialog/README.md",
        "crates/media/media-core/README.md",
        "crates/media/rtp-core/README.md",
    ];
    let forbidden = [
        "Production Ready",
        "production-ready",
        "production deployment",
        "Ready for production",
        "WebRTC-compatible secure transport",
    ];

    for readme in readmes {
        let body = read(workspace.join(readme));
        for phrase in forbidden {
            assert!(
                !body.contains(phrase),
                "{readme} contains unqualified beta-forbidden claim phrase: {phrase}"
            );
        }
    }
}
