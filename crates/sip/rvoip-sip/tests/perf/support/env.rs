//! Environment block: captured once at scenario start so JSON output
//! carries the context needed to compare runs across machines.
//!
//! The block also enforces release-mode at startup. Debug-build perf
//! numbers are not citable, so a debug run is treated as a programming
//! error: [`EnvironmentBlock::capture`] panics if `cfg!(debug_assertions)`
//! is true.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use sysinfo::System;

#[derive(Debug, Clone)]
struct SourceProvenance {
    git_commit: String,
    git_rev: String,
    git_dirty: bool,
    source_fingerprint_sha256: String,
}

static SOURCE_PROVENANCE: OnceLock<SourceProvenance> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentBlock {
    pub rustc: String,
    pub os: String,
    pub cpu_model: String,
    pub cpu_count_physical: usize,
    pub cpu_count_logical: usize,
    pub total_ram_gb: f64,
    pub build_profile: &'static str,
    pub global_allocator: &'static str,
    pub mimalloc_enabled: bool,
    pub tokio_version: &'static str,
    pub network_topology: &'static str,
    pub nic: &'static str,
    pub rvoip_sip_version: &'static str,
    pub git_rev: String,
    pub git_commit: String,
    pub git_dirty: bool,
    /// SHA-256 over HEAD, the tracked diff, and the names and bytes of
    /// untracked files. This distinguishes two dirty trees at the same commit.
    pub source_fingerprint_sha256: String,
    /// rvoip-sip Cargo features that are active in this test executable.
    pub cargo_features: Vec<&'static str>,
    /// Feature string requested by the runner, when it supplied one. The
    /// compile-time `cargo_features` field remains authoritative.
    pub requested_cargo_features: Option<String>,
}

impl EnvironmentBlock {
    /// Capture the current environment. Panics if running in a debug build
    /// — debug-mode perf numbers are misleading and not citable.
    pub fn capture() -> Self {
        assert!(
            !cfg!(debug_assertions),
            "perf tests must be run with --release; debug-build numbers are not citable"
        );

        let mut sys = System::new();
        sys.refresh_memory();
        sys.refresh_cpu_all();

        let cpu_model = sys
            .cpus()
            .first()
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let cpu_count_logical = sys.cpus().len();
        let cpu_count_physical = sys.physical_core_count().unwrap_or(cpu_count_logical);
        let total_ram_gb = (sys.total_memory() as f64) / (1024.0 * 1024.0 * 1024.0);

        let os = format!(
            "{} {}",
            System::name().unwrap_or_else(|| "unknown".to_string()),
            System::os_version().unwrap_or_else(|| "?".to_string())
        );

        let rustc = rustc_version();
        let source = SOURCE_PROVENANCE
            .get_or_init(capture_source_provenance)
            .clone();

        let (global_allocator, mimalloc_enabled) = if cfg!(feature = "dhat") {
            ("dhat", false)
        } else if cfg!(feature = "perf-system-allocator") {
            ("system", false)
        } else {
            ("mimalloc", true)
        };

        Self {
            rustc,
            os,
            cpu_model,
            cpu_count_physical,
            cpu_count_logical,
            total_ram_gb,
            build_profile: "release",
            global_allocator,
            mimalloc_enabled,
            // Hard-coded to match the workspace pin in the root
            // Cargo.toml. Bump this when the workspace tokio dep moves.
            tokio_version: "1.40",
            network_topology: "loopback",
            nic: "loopback",
            rvoip_sip_version: env!("CARGO_PKG_VERSION"),
            git_rev: source.git_rev,
            git_commit: source.git_commit,
            git_dirty: source.git_dirty,
            source_fingerprint_sha256: source.source_fingerprint_sha256,
            cargo_features: active_cargo_features(),
            requested_cargo_features: std::env::var("RVOIP_PERF_BUILD_FEATURES").ok(),
        }
    }

    pub fn cpu_count_physical(&self) -> usize {
        self.cpu_count_physical
    }
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn capture_source_provenance() -> SourceProvenance {
    let Some(workspace) = workspace_root() else {
        return unknown_source_provenance();
    };

    let git_commit = git_stdout(&workspace, &["rev-parse", "HEAD"])
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let git_rev = git_stdout(&workspace, &["rev-parse", "--short", "HEAD"])
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let status = git_stdout(
        &workspace,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .unwrap_or_default();
    let tracked_diff =
        git_stdout(&workspace, &["diff", "--binary", "HEAD", "--", "."]).unwrap_or_default();
    let mut untracked = git_stdout(
        &workspace,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .map(|bytes| {
        bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| path.to_vec())
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();
    untracked.sort();

    let mut fingerprint = Sha256::new();
    fingerprint.update(b"rvoip-source-fingerprint-v1\0");
    hash_framed(&mut fingerprint, git_commit.as_bytes());
    hash_framed(&mut fingerprint, &status);
    hash_framed(&mut fingerprint, &tracked_diff);
    for path_bytes in untracked {
        hash_framed(&mut fingerprint, &path_bytes);
        let path = workspace.join(path_from_git_bytes(&path_bytes));
        match fs::read(path) {
            Ok(bytes) => hash_framed(&mut fingerprint, &bytes),
            Err(error) => hash_framed(
                &mut fingerprint,
                format!("unreadable:{:?}", error.kind()).as_bytes(),
            ),
        }
    }

    SourceProvenance {
        git_commit,
        git_rev,
        git_dirty: !status.is_empty(),
        source_fingerprint_sha256: format!("{:x}", fingerprint.finalize()),
    }
}

fn unknown_source_provenance() -> SourceProvenance {
    SourceProvenance {
        git_commit: "unknown".to_string(),
        git_rev: "unknown".to_string(),
        git_dirty: false,
        source_fingerprint_sha256: "unknown".to_string(),
    }
}

fn workspace_root() -> Option<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn git_stdout(workspace: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(args)
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn hash_framed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

#[cfg(unix)]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).as_ref())
}

fn active_cargo_features() -> Vec<&'static str> {
    let mut features = Vec::new();
    for (name, active) in [
        ("event-history", cfg!(feature = "event-history")),
        ("persistence", cfg!(feature = "persistence")),
        (
            "generated-validation",
            cfg!(feature = "generated-validation"),
        ),
        ("dev-insecure-tls", cfg!(feature = "dev-insecure-tls")),
        ("g729", cfg!(feature = "g729")),
        ("opus", cfg!(feature = "opus")),
        ("dhat", cfg!(feature = "dhat")),
        ("tokio-console", cfg!(feature = "tokio-console")),
        ("perf-tests", cfg!(feature = "perf-tests")),
        (
            "perf-infra-memory-diagnostics",
            cfg!(feature = "perf-infra-memory-diagnostics"),
        ),
        (
            "perf-media-diagnostics",
            cfg!(feature = "perf-media-diagnostics"),
        ),
        (
            "perf-media-memory-diagnostics",
            cfg!(feature = "perf-media-memory-diagnostics"),
        ),
        (
            "perf-rtp-memory-diagnostics",
            cfg!(feature = "perf-rtp-memory-diagnostics"),
        ),
        (
            "perf-call-setup-diagnostics",
            cfg!(feature = "perf-call-setup-diagnostics"),
        ),
        (
            "perf-system-allocator",
            cfg!(feature = "perf-system-allocator"),
        ),
    ] {
        if active {
            features.push(name);
        }
    }
    features
}
