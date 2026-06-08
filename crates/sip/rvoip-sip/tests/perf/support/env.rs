//! Environment block: captured once at scenario start so JSON output
//! carries the context needed to compare runs across machines.
//!
//! The block also enforces release-mode at startup. Debug-build perf
//! numbers are not citable, so a debug run is treated as a programming
//! error: [`EnvironmentBlock::capture`] panics if `cfg!(debug_assertions)`
//! is true.

use serde::Serialize;
use std::process::Command;
use sysinfo::System;

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
        let git_rev = git_rev();

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
            git_rev,
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

fn git_rev() -> String {
    Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
