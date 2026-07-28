//! Concurrency-sweep runner: drives one scenario across a vector of
//! load points and emits both per-point JSON files and an aggregated
//! `_sweep.{json,md}` report.
//!
//! Industry comparable: OpenSIPS' 14-test sweep, Kamailio's capacity
//! tables, AudioCodes Miercom reports — every comparable product
//! publishes a curve, not a single point. See `docs/BENCHMARKING.md`
//! §3.5 "Reading a sweep table & the knee point".
//!
//! Knee detection: the first sweep point where any of
//!
//! - `asr < 0.95` (Answer-Seizure Ratio below the standard threshold), or
//! - `setup_p99 / setup_p99_at_first_point > 5×` (tail latency blew up), or
//! - `error_rate > 0.005` (>0.5% failed transactions)
//!
//! is marked the **knee**. The sweep does NOT stop at the knee — VoIP
//! benchmarks routinely keep ramping past it to characterise the
//! degradation shape, not just locate the failure point.
//!
//! Output paths:
//!
//! - Single-point sweep (vec![p]): writes the flat
//!   `target/perf-results/<scenario>.json` (backwards-compatible with
//!   the pre-sweep layout).
//! - Multi-point sweep: writes
//!   - `target/perf-results/<scenario>/<point>.json` per point,
//!   - `target/perf-results/<scenario>/_sweep.json` (aggregated),
//!   - `target/perf-results/<scenario>/_sweep.md` (publication table).

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

use super::report::ScenarioReport;

/// One captured sweep point.
struct PointRecord {
    point: f64,
    report_json: Value,
    ratio: Option<f64>,
    setup_p99: Option<u64>,
    achieved: Option<f64>,
    errors_total: Option<u64>,
    rss_delta_mb: Option<f64>,
}

pub struct SweepRunner {
    scenario: String,
    point_label: String, // "CPS", "concurrent", "REG/s" — for the markdown header
    achieved_key: String, // "achieved_cps", "achieved_concurrent", "achieved_rps"
    /// Industry KPI tag for this scenario's success ratio — "ASR"
    /// (Answer-Seizure Ratio) for call scenarios, "RSR" (Register-Success
    /// Ratio) for the REG scenario. The lowercase form is also the
    /// `results.<key>` we pull from each per-point report.
    ratio_label: String,
    points: Vec<f64>,
    environment: Option<Value>, // kept as JSON Value to avoid deserialise round-trip
    records: Vec<PointRecord>,
    baseline_setup_p99: Option<u64>,
    knee: Option<(f64, &'static str)>,
}

impl SweepRunner {
    /// `point_label` is the column header in the markdown sweep table
    /// (e.g. `"CPS target"`, `"Concurrent target"`, `"REG/s target"`).
    ///
    /// `achieved_key` is the `results.*` key the scenario will emit
    /// containing the achieved load (used to fill the "Achieved" column
    /// in the markdown table without rebuilding it from scratch).
    ///
    /// `ratio_label` is the industry KPI tag — `"ASR"` for call
    /// scenarios (Answer-Seizure Ratio, ITU E.411), `"RSR"` for the
    /// REGISTER scenario (Register-Success Ratio). The lowercase form
    /// is also the `results.<key>` the runner pulls from each report.
    pub fn new(
        scenario: impl Into<String>,
        points: Vec<f64>,
        point_label: impl Into<String>,
        achieved_key: impl Into<String>,
        ratio_label: impl Into<String>,
    ) -> Self {
        Self {
            scenario: scenario.into(),
            point_label: point_label.into(),
            achieved_key: achieved_key.into(),
            ratio_label: ratio_label.into(),
            points,
            environment: None,
            records: Vec::new(),
            baseline_setup_p99: None,
            knee: None,
        }
    }

    pub fn points(&self) -> &[f64] {
        &self.points
    }

    pub fn is_sweep(&self) -> bool {
        self.points.len() > 1
    }

    /// Hand the runner a built `ScenarioReport` for the just-finished
    /// point. The runner captures the JSON, extracts the key metrics
    /// for the sweep table, and prints a per-point summary line.
    pub fn add_point(&mut self, point: f64, report: ScenarioReport) {
        let report_json = report.to_json();

        // Capture environment from the first point — kept as a JSON
        // Value so we don't have to round-trip through `EnvironmentBlock`
        // (which carries `&'static str` fields).
        if self.environment.is_none() {
            if let Some(env) = report_json.get("environment").cloned() {
                self.environment = Some(env);
            }
        }

        // Extract sweep-relevant scalars for the aggregated table.
        // `ratio_label` is upper-case ("ASR" / "RSR"); the report's key
        // is the lower-case form.
        let ratio_key = self.ratio_label.to_ascii_lowercase();
        let ratio = report_json
            .pointer(&format!("/results/{ratio_key}"))
            .and_then(|v| v.as_f64());
        // Pick the primary latency histogram per scenario: setup for
        // call scenarios, register for the REG scenario. Knee detection
        // applies to whichever the scenario emits.
        let setup_p99 = report_json
            .pointer("/latency_ns/setup_latency/p99")
            .or_else(|| report_json.pointer("/latency_ns/register_latency/p99"))
            .and_then(|v| v.as_u64());
        let achieved = report_json
            .pointer(&format!("/results/{}", self.achieved_key))
            .and_then(|v| v.as_f64());
        let errors_total = report_json
            .pointer("/results/errors")
            .and_then(|v| v.as_object())
            .map(|m| m.values().filter_map(|x| x.as_u64()).sum::<u64>());
        // Prefer the resources-block computation (post-Phase 1.5) and
        // fall back to the legacy `results.rss_delta_mb` for older
        // scenarios that haven't been migrated.
        let rss_delta_mb = match (
            report_json
                .pointer("/resources/peak_rss_mb")
                .and_then(|v| v.as_f64()),
            report_json
                .pointer("/resources/baseline_rss_mb")
                .and_then(|v| v.as_f64()),
        ) {
            (Some(peak), Some(base)) => Some((peak - base).max(0.0)),
            _ => report_json
                .pointer("/results/rss_delta_mb")
                .and_then(|v| v.as_f64()),
        };

        // Baseline setup_p99 from the first point lets us flag tail
        // blow-up as a knee trigger.
        if self.baseline_setup_p99.is_none() {
            self.baseline_setup_p99 = setup_p99;
        }

        // Knee detection — first triggering point wins, later points
        // continue to run so the operator can see the full degradation.
        if self.knee.is_none() {
            if let Some(r) = ratio {
                if r < 0.95 {
                    self.knee = Some((point, "ratio<0.95"));
                }
            }
            if self.knee.is_none() {
                if let (Some(p99), Some(base)) = (setup_p99, self.baseline_setup_p99) {
                    if base > 0 && p99 > base.saturating_mul(5) {
                        self.knee = Some((point, "setup_p99>5×baseline"));
                    }
                }
            }
            if self.knee.is_none() {
                if let Some(errs) = errors_total {
                    let attempts = report_json
                        .pointer("/results/calls_offered")
                        .or_else(|| report_json.pointer("/results/registers_offered"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if attempts > 0 {
                        let err_rate = errs as f64 / attempts as f64;
                        if err_rate > 0.005 {
                            self.knee = Some((point, "errors>0.5%"));
                        }
                    }
                }
            }
        }

        print_point_line(
            &self.scenario,
            point,
            &self.point_label,
            &self.ratio_label,
            achieved,
            ratio,
            setup_p99,
            errors_total,
        );

        self.records.push(PointRecord {
            point,
            report_json,
            ratio,
            setup_p99,
            achieved,
            errors_total,
            rss_delta_mb,
        });
    }

    /// Write all outputs. Returns the list of files produced (always
    /// `>= 1`; single-point mode produces exactly one).
    pub fn finalize(self) -> Vec<PathBuf> {
        let dir_base = target_dir().join("perf-results");
        fs::create_dir_all(&dir_base).expect("create perf-results dir");

        let mut written = Vec::new();
        if self.records.len() <= 1 {
            // Single-point: keep the legacy flat layout.
            let path = dir_base.join(format!("{}.json", self.scenario));
            if let Some(rec) = self.records.first() {
                let pretty = serde_json::to_string_pretty(&rec.report_json).expect("serialize");
                fs::write(&path, pretty).expect("write perf JSON");
                written.push(path);
            }
            return written;
        }

        // Multi-point: per-scenario subdirectory.
        let scenario_dir = dir_base.join(&self.scenario);
        fs::create_dir_all(&scenario_dir).expect("create scenario perf dir");

        for rec in &self.records {
            let fname = format!("{}.json", point_to_filename(rec.point));
            let path = scenario_dir.join(fname);
            let pretty = serde_json::to_string_pretty(&rec.report_json).expect("serialize");
            fs::write(&path, pretty).expect("write per-point JSON");
            written.push(path);
        }

        // Compute the 80%-of-knee headline operating point.
        // ChatGPT VoIP-guidance: "stable p99 at 80% load is more
        // impressive than peak CPS." If we found a knee at P_k, the
        // headline is the largest sweep point ≤ 0.8 * P_k. If no knee
        // was detected, the highest point in the sweep is the headline
        // (we ran the whole curve and stayed in the safe band).
        let headline = self.compute_headline();

        // _sweep.json (aggregated)
        let sweep_json_path = scenario_dir.join("_sweep.json");
        let sweep_json = json!({
            "scenario": self.scenario,
            "environment": self.environment,
            "sweep_summary": {
                "points": self.records.iter().map(|r| r.point).collect::<Vec<_>>(),
                "point_label": self.point_label,
                "knee_point": self.knee.map(|(p, _)| p),
                "knee_reason": self.knee.map(|(_, r)| r),
            },
            "headline": headline,
            "points": self.records.iter().map(|r| &r.report_json).collect::<Vec<_>>(),
        });
        fs::write(
            &sweep_json_path,
            serde_json::to_string_pretty(&sweep_json).expect("serialize sweep"),
        )
        .expect("write _sweep.json");
        written.push(sweep_json_path.clone());

        // _sweep.md (publication-ready)
        let md = self.render_markdown();
        let sweep_md_path = scenario_dir.join("_sweep.md");
        fs::write(&sweep_md_path, md).expect("write _sweep.md");
        written.push(sweep_md_path.clone());

        // Final aggregated stdout summary.
        self.print_aggregated_summary(&sweep_md_path);
        written
    }

    /// 80%-of-knee headline: returns a JSON Value or `Null`.
    ///
    /// - Knee at P_k → headline is the largest point ≤ 0.8 * P_k.
    /// - No knee detected → headline is the highest sweep point (we
    ///   ran the whole curve without saturating, that's the result we
    ///   publish).
    /// - Fewer than 2 points → Null (no curve to size against).
    fn compute_headline(&self) -> Value {
        if self.records.len() < 2 {
            return Value::Null;
        }
        let target_point = match self.knee {
            Some((kp, _)) => {
                // Largest sweep point ≤ 0.8 * knee.
                let threshold = kp * 0.8;
                self.records
                    .iter()
                    .filter(|r| r.point <= threshold)
                    .map(|r| r.point)
                    .fold(f64::NEG_INFINITY, f64::max)
            }
            None => self
                .records
                .iter()
                .map(|r| r.point)
                .fold(f64::NEG_INFINITY, f64::max),
        };
        if !target_point.is_finite() {
            return Value::Null;
        }
        let rec = self
            .records
            .iter()
            .find(|r| (r.point - target_point).abs() < f64::EPSILON);
        let Some(rec) = rec else { return Value::Null };

        let framing = if self.knee.is_some() {
            "sustained_at_80pct_of_knee"
        } else {
            "max_point_no_knee_detected"
        };
        json!({
            "operating_point": rec.point,
            "achieved": rec.achieved,
            "ratio_label": self.ratio_label,
            "ratio": rec.ratio,
            "setup_p99_ns": rec.setup_p99,
            "framing": framing,
        })
    }

    /// Format the headline callout (markdown). Empty string if no
    /// headline (single-point or undefined).
    fn render_headline_callout(&self) -> String {
        let headline = self.compute_headline();
        if headline.is_null() {
            return String::new();
        }
        let point = headline.get("operating_point").and_then(|v| v.as_f64());
        let achieved = headline.get("achieved").and_then(|v| v.as_f64());
        let ratio = headline.get("ratio").and_then(|v| v.as_f64());
        let setup_p99 = headline.get("setup_p99_ns").and_then(|v| v.as_u64());
        let framing = headline
            .get("framing")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let point_str = point.map(fmt_point).unwrap_or_else(|| "?".to_string());
        let achieved_str = achieved
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "?".to_string());
        let ratio_str = ratio
            .map(|v| format!("{v:.4}"))
            .unwrap_or_else(|| "?".to_string());
        let p99_str = setup_p99.map(fmt_ns).unwrap_or_else(|| "?".to_string());

        let framing_label = match framing {
            "sustained_at_80pct_of_knee" => "80% of knee",
            "max_point_no_knee_detected" => "max of sweep range, knee not reached",
            other => other,
        };
        format!(
            "**Headline: rvoip-sip sustains {achieved_str} {achieved_unit} at p99 {p99_str} with {ratio_label} {ratio_str} ({framing_label}).**\n\nOperating point: {target_label} = {point_str}.\n\n",
            achieved_unit = self.achieved_unit(),
            target_label = self.point_label,
            ratio_label = self.ratio_label,
        )
    }

    fn achieved_unit(&self) -> &'static str {
        // Best-effort label tied to which scenario we're rendering.
        // The exact column header is `achieved_key` (e.g. achieved_cps,
        // achieved_concurrent, achieved_rps); these are the matching
        // human units.
        if self.achieved_key == "achieved_cps" {
            "CPS"
        } else if self.achieved_key == "achieved_rps" {
            "REGs/sec"
        } else if self.achieved_key == "achieved_concurrent" {
            "concurrent calls"
        } else {
            ""
        }
    }

    fn render_markdown(&self) -> String {
        let mut out = String::new();
        let host_line = match &self.environment {
            Some(env) => {
                let cpu_model = env.get("cpu_model").and_then(|v| v.as_str()).unwrap_or("?");
                let phys = env
                    .get("cpu_count_physical")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let logi = env
                    .get("cpu_count_logical")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let ram_gb = env
                    .get("total_ram_gb")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let rustc = env.get("rustc").and_then(|v| v.as_str()).unwrap_or("?");
                let rustc_ver = rustc.split_whitespace().nth(1).unwrap_or(rustc);
                let version = env
                    .get("rvoip_sip_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let git_rev = env.get("git_rev").and_then(|v| v.as_str()).unwrap_or("?");
                format!(
                    " on {cpu_model} ({phys}P / {logi}L cores, {ram_gb:.0} GB RAM, rustc {rustc_ver}, rvoip-sip {version} @ {git_rev})",
                )
            }
            None => String::new(),
        };
        out.push_str(&format!("### {} — sweep{}\n\n", self.scenario, host_line));

        // 80%-of-knee headline callout, if applicable.
        out.push_str(&self.render_headline_callout());

        // Header — `ratio_label` is "ASR" / "RSR"; "Latency" is the
        // primary per-call histogram (setup or register, picked per
        // scenario by `first_latency_percentiles`). RSS Δ MB +
        // RSS Δ /min + CPU% come from the Phase 1.5 resource block.
        out.push_str(&format!(
            "| {} | Achieved | per-core | {} | Latency p50 | Latency p95 | Latency p99 | Full-cycle p99 | RSS Δ MB | RSS slope MB/min | CPU% | Errors |\n",
            self.point_label, self.ratio_label,
        ));
        out.push_str("| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n");

        for rec in &self.records {
            let (p50_lat, p95_lat, p99_lat) = first_latency_percentiles(&rec.report_json);
            let p99_full = pluck_ns(&rec.report_json, "/latency_ns/full_cycle/p99");
            let per_core = first_per_core_value(&rec.report_json);
            let rss_growth = rec
                .report_json
                .pointer("/resources/rss_growth_mb_per_min")
                .and_then(|v| v.as_f64());
            let cpu_pct = rec
                .report_json
                .pointer("/resources/avg_cpu_pct")
                .and_then(|v| v.as_f64());
            let knee_marker = match self.knee {
                Some((kp, reason)) if (kp - rec.point).abs() < f64::EPSILON => {
                    format!(" — **knee ({reason})**")
                }
                _ => String::new(),
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {}{} |\n",
                fmt_point(rec.point),
                rec.achieved
                    .map(|v| format!("{v:.1}"))
                    .unwrap_or_else(|| "n/a".to_string()),
                per_core,
                rec.ratio
                    .map(|v| format!("{v:.4}"))
                    .unwrap_or_else(|| "n/a".to_string()),
                p50_lat,
                p95_lat,
                p99_lat,
                p99_full,
                rec.rss_delta_mb
                    .map(|v| format!("{v:.1}"))
                    .unwrap_or_else(|| "n/a".to_string()),
                rss_growth
                    .map(|v| format!("{v:+.1}"))
                    .unwrap_or_else(|| "n/a".to_string()),
                cpu_pct
                    .map(|v| format!("{v:.0}%"))
                    .unwrap_or_else(|| "n/a".to_string()),
                rec.errors_total
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                knee_marker,
            ));
        }

        if let Some((point, reason)) = self.knee {
            out.push_str(&format!(
                "\nKnee detected at point={} ({}). See methodology §3.5 for interpretation.\n",
                fmt_point(point),
                reason,
            ));
        } else {
            out.push_str("\nNo knee detected within sweep range.\n");
        }

        out
    }

    fn print_aggregated_summary(&self, md_path: &PathBuf) {
        println!();
        println!("════════════════════════════════════════════════════════════════════════");
        println!(" SWEEP COMPLETE: {}", self.scenario);
        println!("────────────────────────────────────────────────────────────────────────");
        if let Some(env) = &self.environment {
            let cpu_model = env.get("cpu_model").and_then(|v| v.as_str()).unwrap_or("?");
            let phys = env
                .get("cpu_count_physical")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let logi = env
                .get("cpu_count_logical")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let ram_gb = env
                .get("total_ram_gb")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            println!(" host    : {cpu_model}  ({phys}P / {logi}L cores, {ram_gb:.1} GB RAM)",);
        }
        println!(" points  : {} sweep points", self.records.len());
        match self.knee {
            Some((p, reason)) => println!(" knee    : at point={} ({})", fmt_point(p), reason),
            None => println!(" knee    : not detected within range"),
        }
        // Headline operating point: 80% of knee, or max of sweep range.
        let headline = self.compute_headline();
        if let (Some(pt), Some(ach), Some(p99)) = (
            headline.get("operating_point").and_then(|v| v.as_f64()),
            headline.get("achieved").and_then(|v| v.as_f64()),
            headline.get("setup_p99_ns").and_then(|v| v.as_u64()),
        ) {
            println!(
                " headline: {ratio_label} sustained at {ach:.1} {unit} (operating point = {pt_label} {pt_str}, p99 {p99_str})",
                ratio_label = self.ratio_label,
                unit = self.achieved_unit(),
                pt_label = self.point_label,
                pt_str = fmt_point(pt),
                p99_str = fmt_ns(p99),
            );
        }
        println!(" markdown: {}", md_path.display());
        println!("════════════════════════════════════════════════════════════════════════");
        println!();
    }
}

fn print_point_line(
    scenario: &str,
    point: f64,
    point_label: &str,
    ratio_label: &str,
    achieved: Option<f64>,
    ratio: Option<f64>,
    setup_p99: Option<u64>,
    errors_total: Option<u64>,
) {
    println!(
        "  [{scenario}] {point_label}={pt:>8}  achieved={ach:>10}  {rl}={r:>7}  p99={p99:>10}  errors={err:>4}",
        pt = fmt_point(point),
        ach = achieved
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "n/a".to_string()),
        rl = ratio_label.to_ascii_lowercase(),
        r = ratio
            .map(|v| format!("{v:.4}"))
            .unwrap_or_else(|| "n/a".to_string()),
        p99 = setup_p99.map(fmt_ns).unwrap_or_else(|| "n/a".to_string()),
        err = errors_total
            .map(|v| v.to_string())
            .unwrap_or_else(|| "n/a".to_string()),
    );
}

/// Find the per-core normalisation field, whatever the scenario calls
/// it (cps_per_core / dialogs_per_core / regs_per_core_per_sec / etc).
/// Returns a formatted string for the markdown cell or `"n/a"` if no
/// such key is present.
fn first_per_core_value(report: &Value) -> String {
    let results = report.pointer("/results").and_then(|v| v.as_object());
    let candidate = results.and_then(|m| {
        for k in [
            "cps_per_core",
            "dialogs_per_core",
            "regs_per_core_per_sec",
            "streams_per_core",
            "packets_per_core_per_sec",
        ] {
            if let Some(v) = m.get(k).and_then(|v| v.as_f64()) {
                return Some(v);
            }
        }
        None
    });
    candidate
        .map(|v| format!("{v:.2}"))
        .unwrap_or_else(|| "n/a".to_string())
}

/// Pull the p50/p95/p99 of the primary "call-setup or register"
/// latency histogram. We can't just take the first entry — the
/// underlying `serde_json::Map` is alphabetically ordered, which would
/// give us `full_cycle` ahead of `setup_latency`. Look up the known
/// names explicitly so the sweep markdown shows the setup-side number.
fn first_latency_percentiles(report: &Value) -> (String, String, String) {
    // Try the call-scenario name first, then the REG-scenario name.
    // If neither is present, fall back to the first entry to stay
    // useful for hypothetical new scenarios.
    let lat = report.pointer("/latency_ns").and_then(|v| v.as_object());
    let obj = lat.and_then(|m| {
        m.get("setup_latency")
            .or_else(|| m.get("register_latency"))
            .or_else(|| m.values().next())
    });
    let p50 = obj
        .and_then(|v| v.get("p50"))
        .and_then(|v| v.as_u64())
        .map(fmt_ns);
    let p95 = obj
        .and_then(|v| v.get("p95"))
        .and_then(|v| v.as_u64())
        .map(fmt_ns);
    let p99 = obj
        .and_then(|v| v.get("p99"))
        .and_then(|v| v.as_u64())
        .map(fmt_ns);
    (
        p50.unwrap_or_else(|| "n/a".to_string()),
        p95.unwrap_or_else(|| "n/a".to_string()),
        p99.unwrap_or_else(|| "n/a".to_string()),
    )
}

fn pluck_ns(value: &Value, ptr: &str) -> String {
    value
        .pointer(ptr)
        .and_then(|v| v.as_u64())
        .map(fmt_ns)
        .unwrap_or_else(|| "n/a".to_string())
}

fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2}s", ns as f64 / 1_000_000_000.0)
    } else if ns >= 1_000_000 {
        format!("{:.1}ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.1}µs", ns as f64 / 1_000.0)
    } else {
        format!("{}ns", ns)
    }
}

fn fmt_point(p: f64) -> String {
    if (p.fract()).abs() < f64::EPSILON {
        format!("{}", p as u64)
    } else {
        format!("{p:.3}")
    }
}

fn point_to_filename(p: f64) -> String {
    if (p.fract()).abs() < f64::EPSILON {
        format!("{}", p as u64)
    } else {
        // Replace '.' with '_' so the file name stays portable.
        format!("{p:.3}").replace('.', "_")
    }
}

/// Parse the sweep env var (`RVOIP_PERF_SWEEP_CPS=10,50,100`). Returns
/// `Some(points)` if the var is set; `None` if not (caller uses the
/// scenario default in that case).
pub fn parse_sweep_env(name: &str) -> Option<Vec<f64>> {
    let raw = std::env::var(name).ok()?;
    let mut points: Vec<f64> = raw
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();
    points.retain(|p| *p > 0.0);
    if points.is_empty() {
        None
    } else {
        Some(points)
    }
}

fn target_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("RVOIP_PERF_OUTPUT_ROOT") {
        return PathBuf::from(path);
    }
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set under cargo"),
    );
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.join("target"))
        .unwrap_or_else(|| PathBuf::from("target"))
}
