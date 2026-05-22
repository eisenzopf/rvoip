//! Scenario report: stdout summary table + JSON file writer.
//!
//! The JSON shape follows the canonical schema in
//! `crates/rvoip-sip/docs/BENCHMARKING.md`. Adding scenario-specific
//! keys is fine; removing one of the canonical keys
//! (`scenario` / `environment` / `load` / `results` / `latency_ns` /
//! `resources`) is a breaking change for downstream dashboards and is
//! asserted against in [`ScenarioReport::write_json`].

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

use super::env::EnvironmentBlock;
use super::histogram::{LatencyHistogram, LatencySnapshot};
use super::load::LoadProfile;
use super::sampler::{ResourceSample, ResourceSummary};

/// One result column in the JSON `results` block.
#[derive(Debug, Clone, Serialize)]
pub struct ResultEntry {
    pub key: String,
    pub value: Value,
}

pub struct ScenarioReport {
    scenario: String,
    environment: EnvironmentBlock,
    load: LoadProfile,
    results: Map<String, Value>,
    latencies: Vec<LatencySnapshot>,
    baseline_rss_mb: Option<f64>,
    peak_rss_mb: Option<f64>,
    rss_growth_mb_per_min: Option<f64>,
    avg_cpu_pct: Option<f64>,
    rss_samples: Vec<ResourceSample>,
    started: std::time::Instant,
}

impl ScenarioReport {
    pub fn new(scenario: impl Into<String>, load: LoadProfile) -> Self {
        let scenario = scenario.into();
        let environment = EnvironmentBlock::capture();
        Self {
            scenario,
            environment,
            load,
            results: Map::new(),
            latencies: Vec::new(),
            baseline_rss_mb: None,
            peak_rss_mb: None,
            rss_growth_mb_per_min: None,
            avg_cpu_pct: None,
            rss_samples: Vec::new(),
            started: std::time::Instant::now(),
        }
    }

    /// Read the captured environment block (needed for per-core
    /// normalisation: `achieved / cpu_count_physical`).
    pub fn environment(&self) -> &EnvironmentBlock {
        &self.environment
    }

    /// Populate every resource field from a [`ResourceSummary`] in one
    /// call. Scenarios that use [`super::ResourceSampler`] hand the
    /// result of `sampler.stop().await` here.
    pub fn with_resources(&mut self, summary: ResourceSummary) -> &mut Self {
        self.baseline_rss_mb = Some(summary.baseline_rss_mb);
        self.peak_rss_mb = Some(summary.peak_rss_mb);
        self.rss_growth_mb_per_min = Some(summary.rss_growth_mb_per_min);
        self.avg_cpu_pct = Some(summary.avg_cpu_pct);
        self.rss_samples = summary.samples;
        self
    }

    /// Record a scalar result under `results.<key>`.
    pub fn result<V: Into<Value>>(&mut self, key: &str, value: V) -> &mut Self {
        self.results.insert(key.to_string(), value.into());
        self
    }

    /// Record a structured (nested) result block.
    pub fn result_block(&mut self, key: &str, value: Value) -> &mut Self {
        self.results.insert(key.to_string(), value);
        self
    }

    /// Snapshot and attach a latency histogram.
    pub fn latency(&mut self, hist: &LatencyHistogram) -> &mut Self {
        self.latencies.push(hist.snapshot());
        self
    }

    pub fn peak_rss_mb(&mut self, value: f64) -> &mut Self {
        self.peak_rss_mb = Some(value);
        self
    }

    pub fn avg_cpu_pct(&mut self, value: f64) -> &mut Self {
        self.avg_cpu_pct = Some(value);
        self
    }

    /// Convert to the canonical JSON value (validated for schema
    /// stability — the canonical keys must be present).
    pub fn to_json(&self) -> Value {
        let mut latency_obj = Map::new();
        for snap in &self.latencies {
            latency_obj.insert(
                snap.label.clone(),
                json!({
                    "count":  snap.count,
                    "min":    snap.min,
                    "max":    snap.max,
                    "mean":   snap.mean_ns,
                    "p50":    snap.p50,
                    "p95":    snap.p95,
                    "p99":    snap.p99,
                    "p99_9":  snap.p99_9,
                }),
            );
        }

        // The resources block carries baseline + peak + leak indicator
        // (rss_growth_mb_per_min) + the raw time-series so a reader can
        // distinguish "120 MB peak, stable" from "120 MB peak after 60s
        // but growing 1 MB/min" — the latter is a leak.
        let resources = json!({
            "baseline_rss_mb": self.baseline_rss_mb,
            "peak_rss_mb": self.peak_rss_mb,
            "rss_growth_mb_per_min": self.rss_growth_mb_per_min,
            "avg_cpu_pct": self.avg_cpu_pct,
            "rss_samples_mb": self.rss_samples,
        });

        let value = json!({
            "scenario":   self.scenario,
            "duration_secs": self.started.elapsed().as_secs(),
            "environment": self.environment,
            "load":       self.load,
            "results":    Value::Object(self.results.clone()),
            "latency_ns": Value::Object(latency_obj),
            "resources":  resources,
        });

        // Schema invariant: every emitted JSON file must contain the
        // canonical top-level keys, otherwise downstream tooling
        // (regression dashboards, comparison scripts) silently breaks.
        for key in ["scenario", "environment", "load", "results", "latency_ns", "resources"] {
            assert!(
                value.get(key).is_some(),
                "ScenarioReport::to_json missing canonical key `{key}`"
            );
        }
        value
    }

    /// Write `target/perf-results/<scenario>.json` (creates the
    /// directory on first call). Returns the absolute path.
    pub fn write_json(&self) -> PathBuf {
        let dir = target_dir().join("perf-results");
        fs::create_dir_all(&dir).expect("create perf-results dir");
        let path = dir.join(format!("{}.json", self.scenario));
        let value = self.to_json();
        let pretty = serde_json::to_string_pretty(&value).expect("serialize");
        fs::write(&path, pretty).expect("write perf JSON");
        path
    }

    /// Print a stdout summary table (human-readable, suitable for CI
    /// logs and copy-paste into a README or blog post).
    pub fn print_summary(&self, json_path: &std::path::Path) {
        println!();
        println!("════════════════════════════════════════════════════════════════════════");
        println!(" rvoip-sip perf scenario: {}", self.scenario);
        println!("────────────────────────────────────────────────────────────────────────");
        println!(
            " host    : {}  ({} physical / {} logical cores, {:.1} GB RAM)",
            self.environment.cpu_model,
            self.environment.cpu_count_physical,
            self.environment.cpu_count_logical,
            self.environment.total_ram_gb,
        );
        println!(" os      : {}", self.environment.os);
        println!(" rustc   : {}", self.environment.rustc);
        println!(
            " version : rvoip-sip {} @ {}",
            self.environment.rvoip_sip_version, self.environment.git_rev,
        );
        println!(
            " load    : target_cps={}  ramp={}s  steady={}s  cooldown={}s",
            self.load.target_cps, self.load.ramp_secs, self.load.steady_secs, self.load.cooldown_secs,
        );
        println!("────────────────────────────────────────────────────────────────────────");
        println!(" results:");
        for (k, v) in self.results.iter() {
            println!("   {:<22}  {}", k, v);
        }
        println!(" latency:");
        for snap in &self.latencies {
            println!("{}", snap.format_row());
        }
        if let Some(rss) = self.peak_rss_mb {
            println!(" peak RSS    : {:.1} MB", rss);
        }
        if let Some(base) = self.baseline_rss_mb {
            if let Some(peak) = self.peak_rss_mb {
                println!(" RSS Δ       : {:.1} MB  (baseline {:.1} → peak {:.1})", peak - base, base, peak);
            }
        }
        if let Some(growth) = self.rss_growth_mb_per_min {
            // Highlight any non-trivial growth — the "no leaks" pitch
            // wants this near zero for short runs.
            let marker = if growth.abs() > 1.0 { "  ⚠" } else { "" };
            println!(" RSS growth  : {:+.2} MB/min{marker}", growth);
        }
        if let Some(cpu) = self.avg_cpu_pct {
            println!(" avg CPU     : {:.1}%", cpu);
        }
        println!(" json        : {}", json_path.display());
        println!("════════════════════════════════════════════════════════════════════════");
        println!();
    }
}

fn target_dir() -> PathBuf {
    // Walk up from CARGO_MANIFEST_DIR (crate root) to the workspace
    // root (where `target/` lives). Cargo always sets this env var when
    // building tests.
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set under cargo"),
    );
    // crates/rvoip-sip/ → ../../target
    manifest_dir
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .map(|p| p.join("target"))
        .unwrap_or_else(|| PathBuf::from("target"))
}
