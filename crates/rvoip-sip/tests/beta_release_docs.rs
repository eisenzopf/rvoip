use std::fs;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref())
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.as_ref().display()))
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
        "BETA_PERFORMANCE_REPORT.md",
        "RELEASE_NOTES_NEXT.md",
        "PRODUCTION_READINESS_GAP_PLAN.md",
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
fn crate_readmes_do_not_make_unqualified_beta_production_claims() {
    let workspace = manifest_dir().join("../..");
    let readmes = [
        "README.md",
        "crates/rvoip-sip/README.md",
        "crates/rvoip-sip-core/README.md",
        "crates/rvoip-sip-transport/README.md",
        "crates/rvoip-sip-dialog/README.md",
        "crates/media-core/README.md",
        "crates/rtp-core/README.md",
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
