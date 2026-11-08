//! Size the media lanes and the signaling runtime from the host.
//!
//! [`ExecutorPlan::probe`] reads what the kernel exposes about the CPUs,
//! cgroup limits and memory available to this process and turns it into an
//! immutable plan: how many dedicated media lanes to run and on which CPUs,
//! and how many signaling (Tokio multi-thread) worker threads to run and
//! where. The plan only describes a layout; nothing changes until the
//! application applies it:
//!
//! - [`ExecutorPlan::media_runtime_config`] is the media executor layout, one
//!   [`MediaLanes::Dedicated`] lane per media core, for
//!   [`install_media_runtime_config`](crate::task_runtime::install_media_runtime_config).
//! - [`ExecutorPlan::signaling_runtime_builder`] is a Tokio runtime builder
//!   for the application's signaling runtime, sized and placed by the plan.
//!
//! ```no_run
//! use rvoip_rtp_core::host_plan::{ExecutorPlan, HostPlanConfig};
//! use rvoip_rtp_core::task_runtime::{install_media_runtime_config, start_media_runtime};
//!
//! // Run before any runtime exists, then build the signaling runtime.
//! let plan = ExecutorPlan::probe(&HostPlanConfig::sibling_signaling())?;
//! install_media_runtime_config(plan.media_runtime_config())?;
//! let lanes = start_media_runtime();
//! let signaling = plan.signaling_runtime_builder().enable_all().build()?;
//! # drop((lanes, signaling));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Inputs
//!
//! Only std and `/proc` / `/sys` are read ([`HostFacts::read`]); a missing
//! or malformed input leaves its fact unknown and adds a note to the plan.
//! On platforms other than Linux the plan falls back to
//! [`std::thread::available_parallelism`] and never pins.
//!
//! - `L`: scheduler affinity (`/proc/self/status` `Cpus_allowed_list`)
//!   intersected with the cgroup v2 `cpuset.cpus.effective`.
//! - `P`: physical cores in `L`, from `topology/core_cpus_list`, then
//!   `topology/thread_siblings_list`, then one core per CPU.
//! - `Q`: `cpu.max`, walked up the cgroup hierarchy (the smallest quota wins).
//! - `B`: min(`memory.max`, `memory.high`, `MemTotal`), plus `pids.max`,
//!   `RLIMIT_NOFILE` and the NUMA nodes, which are reported only.
//!
//! # Sizing
//!
//! - Effective cores `E = min(P, ⌊Q⌋)` when the quota is the binding limit
//!   (`Q < |L|`), otherwise `P`.
//! - Housekeeping `I = 1` core when `E` reaches
//!   [`HostPlanConfig::housekeeping_min_cores`], otherwise 0.
//! - SMT (every core has two or more siblings, [`SmtPolicy::Siblings`]):
//!   `M = E − I` media lanes pinned to first siblings, and
//!   `S = clamp(⌈σ_smt·E⌉, 1, E − I)` signaling threads floating over the
//!   second siblings of the media cores. Signaling threads take otherwise
//!   idle siblings, so `σ_smt = 1.0` gives one signaling thread per media
//!   core without taking a media lane away.
//! - No SMT: `S = clamp(round(σ·(E − I)), 1, E − I − 1)` and `M = E − I − S`,
//!   each on dedicated cores; every signaling thread costs a media core.
//! - Quota only: no pinning, `⌊Q⌋` threads split by `σ`.
//! - `max_blocking_threads = clamp(pids.max / 8, 1, cap)`.
//!
//! [`HostPlanConfig::signaling_threads`] replaces the `σ` rule with an
//! explicit count, clamped to the usable CPUs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::task_runtime::{pin_current_thread, MediaLanes, MediaRuntimeConfig};

const DEFAULT_SIGMA: f64 = 0.40;
const DEFAULT_HOUSEKEEPING_MIN_CORES: usize = 12;
const DEFAULT_LANE_UTILIZATION: f64 = 0.70;
const DEFAULT_PACKET_COST_US: f64 = 9.5;
const DEFAULT_SESSION_PPS: f64 = 105.0;
const DEFAULT_MAX_BLOCKING_THREADS: usize = 64;
const MAX_SIGNALING_THREADS: usize = 1024;
const MAX_CPU_ID: usize = 65_535;
const EPSILON: f64 = 1e-9;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// How SMT siblings are used when every core has them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SmtPolicy {
    /// Media lanes on first siblings, signaling threads on second siblings.
    #[default]
    Siblings,
    /// Plan as if SMT were absent: first siblings only, dedicated cores.
    Ignore,
}

/// Parameters of the host plan. The defaults are the planner's original
/// recording-call shares; every value is a plain field so it can come from
/// the application's own configuration.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HostPlanConfig {
    /// `σ`: signaling share of the cores on hosts without SMT and under a
    /// CPU quota, from 0.05 to 0.95 (default 0.40).
    pub sigma: f64,
    /// `σ_smt`: signaling share used to size the threads placed on second
    /// SMT siblings, from 0.05 to 1.0; `None` follows [`Self::sigma`].
    pub sigma_smt: Option<f64>,
    /// Reserve one housekeeping core once the effective core count reaches
    /// this value (at least 3; default 12).
    pub housekeeping_min_cores: usize,
    /// How SMT siblings are used.
    pub smt_policy: SmtPolicy,
    /// Explicit signaling thread count (1 to 1024), replacing the `σ` rule;
    /// clamped to the usable CPUs.
    pub signaling_threads: Option<usize>,
    /// Target media-lane utilization for the per-lane session estimate, from
    /// 0.05 to 1.0 (default 0.70).
    pub lane_utilization: f64,
    /// Media CPU microseconds per packet for the session estimate, from 0.1
    /// to 1000 (default 9.5).
    pub packet_cost_us: f64,
    /// Packets per second per session, both directions, for the session
    /// estimate, from 1 to 10000 (default 105).
    pub session_pps: f64,
    /// Upper bound for the signaling runtime's blocking pool, from 1 to 512
    /// (default 64).
    pub max_blocking_threads_cap: usize,
}

impl Default for HostPlanConfig {
    fn default() -> Self {
        Self {
            sigma: DEFAULT_SIGMA,
            sigma_smt: None,
            housekeeping_min_cores: DEFAULT_HOUSEKEEPING_MIN_CORES,
            smt_policy: SmtPolicy::Siblings,
            signaling_threads: None,
            lane_utilization: DEFAULT_LANE_UTILIZATION,
            packet_cost_us: DEFAULT_PACKET_COST_US,
            session_pps: DEFAULT_SESSION_PPS,
            max_blocking_threads_cap: DEFAULT_MAX_BLOCKING_THREADS,
        }
    }
}

impl HostPlanConfig {
    /// The default parameters.
    pub fn new() -> Self {
        Self::default()
    }

    /// One signaling thread per media core on SMT hosts (`σ_smt = 1.0`).
    ///
    /// Every media lane keeps its first sibling and the signaling runtime
    /// gets the full set of second siblings, so call setup scales with the
    /// media lanes instead of with 40% of them. Hosts without SMT keep the
    /// `σ` rule.
    pub fn sibling_signaling() -> Self {
        Self::default().with_sigma_smt(1.0)
    }

    /// Set `σ`.
    pub fn with_sigma(mut self, sigma: f64) -> Self {
        self.sigma = sigma;
        self
    }

    /// Set `σ_smt`.
    pub fn with_sigma_smt(mut self, sigma_smt: f64) -> Self {
        self.sigma_smt = Some(sigma_smt);
        self
    }

    /// Set the effective core count that reserves a housekeeping core.
    pub fn with_housekeeping_min_cores(mut self, cores: usize) -> Self {
        self.housekeeping_min_cores = cores;
        self
    }

    /// Set the SMT policy.
    pub fn with_smt_policy(mut self, policy: SmtPolicy) -> Self {
        self.smt_policy = policy;
        self
    }

    /// Use an explicit signaling thread count instead of the `σ` rule.
    pub fn with_signaling_threads(mut self, threads: usize) -> Self {
        self.signaling_threads = Some(threads);
        self
    }

    /// Set the cap of the signaling runtime's blocking pool.
    pub fn with_max_blocking_threads_cap(mut self, cap: usize) -> Self {
        self.max_blocking_threads_cap = cap;
        self
    }

    /// `σ_smt` in effect.
    pub fn effective_sigma_smt(&self) -> f64 {
        self.sigma_smt.unwrap_or(self.sigma)
    }

    /// Check every parameter against its documented range.
    pub fn validate(&self) -> Result<(), HostPlanError> {
        check_f64("sigma", self.sigma, 0.05, 0.95)?;
        if let Some(sigma_smt) = self.sigma_smt {
            check_f64("sigma_smt", sigma_smt, 0.05, 1.0)?;
        }
        check_usize(
            "housekeeping_min_cores",
            self.housekeeping_min_cores,
            3,
            MAX_CPU_ID + 1,
        )?;
        if let Some(threads) = self.signaling_threads {
            check_usize("signaling_threads", threads, 1, MAX_SIGNALING_THREADS)?;
        }
        check_f64("lane_utilization", self.lane_utilization, 0.05, 1.0)?;
        check_f64("packet_cost_us", self.packet_cost_us, 0.1, 1000.0)?;
        check_f64("session_pps", self.session_pps, 1.0, 10_000.0)?;
        check_usize(
            "max_blocking_threads_cap",
            self.max_blocking_threads_cap,
            1,
            512,
        )?;
        Ok(())
    }
}

fn check_f64(name: &str, value: f64, min: f64, max: f64) -> Result<(), HostPlanError> {
    if value.is_finite() && (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(HostPlanError(format!(
            "host plan {name} must be from {min} to {max}, got {value}"
        )))
    }
}

fn check_usize(name: &str, value: usize, min: usize, max: usize) -> Result<(), HostPlanError> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(HostPlanError(format!(
            "host plan {name} must be from {min} to {max}, got {value}"
        )))
    }
}

/// A [`HostPlanConfig`] value outside its documented range.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct HostPlanError(String);

// ---------------------------------------------------------------------------
// Pure parsers
// ---------------------------------------------------------------------------

/// Parse a kernel CPU list such as `0-3,8-11` (trailing newline allowed).
/// An empty list parses to an empty vector. The result is sorted and unique.
fn parse_cpu_list(text: &str) -> Result<Vec<usize>, String> {
    let mut cpus = BTreeSet::new();
    for part in text.trim().split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (start, end) = match part.split_once('-') {
            Some((start, end)) => (start.trim(), end.trim()),
            None => (part, part),
        };
        let start = start
            .parse::<usize>()
            .map_err(|_| format!("invalid CPU list entry {part:?}"))?;
        let end = end
            .parse::<usize>()
            .map_err(|_| format!("invalid CPU list entry {part:?}"))?;
        if start > end || end > MAX_CPU_ID {
            return Err(format!("invalid CPU range {part:?}"));
        }
        cpus.extend(start..=end);
    }
    Ok(cpus.into_iter().collect())
}

/// `Cpus_allowed_list` from `/proc/self/status`.
fn parse_status_cpus_allowed(status: &str) -> Option<Result<Vec<usize>, String>> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .map(parse_cpu_list)
}

/// The cgroup v2 path (`0::<path>`) from `/proc/self/cgroup`.
fn parse_proc_cgroup_v2(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(|path| path.trim().to_string())
}

/// cgroup v2 `cpu.max` (`<quota|max> <period>`) as a CPU count; `None` when
/// unlimited.
fn parse_cpu_max(content: &str) -> Result<Option<f64>, String> {
    let mut fields = content.split_whitespace();
    let quota = fields.next().ok_or_else(|| "empty cpu.max".to_string())?;
    if quota == "max" {
        return Ok(None);
    }
    let quota = quota
        .parse::<u64>()
        .map_err(|_| format!("invalid cpu.max quota {content:?}"))?;
    let period = match fields.next() {
        Some(period) => period
            .parse::<u64>()
            .map_err(|_| format!("invalid cpu.max period {content:?}"))?,
        None => 100_000,
    };
    if period == 0 {
        return Err(format!("invalid cpu.max period {content:?}"));
    }
    Ok(Some(quota as f64 / period as f64))
}

/// A cgroup v2 limit (`memory.max`, `memory.high`, `pids.max`); `None` when
/// the file says `max`.
fn parse_limit(content: &str) -> Result<Option<u64>, String> {
    let value = content.trim();
    if value == "max" {
        return Ok(None);
    }
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| format!("invalid cgroup limit {value:?}"))
}

/// `MemTotal` from `/proc/meminfo`, in bytes.
fn parse_meminfo_total(meminfo: &str) -> Option<u64> {
    let line = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?;
    let mut fields = line.split_whitespace();
    let value = fields.next()?.parse::<u64>().ok()?;
    match fields.next() {
        Some("kB") | None => value.checked_mul(1024),
        Some(_) => None,
    }
}

/// `Max open files` soft and hard limits from `/proc/self/limits`.
/// `unlimited` is reported as `None`.
fn parse_proc_limits_nofile(limits: &str) -> Option<(Option<u64>, Option<u64>)> {
    let rest = limits
        .lines()
        .find_map(|line| line.strip_prefix("Max open files"))?;
    let mut fields = rest.split_whitespace();
    let parse = |field: Option<&str>| -> Option<Option<u64>> {
        match field? {
            "unlimited" => Some(None),
            value => value.parse::<u64>().ok().map(Some),
        }
    };
    let soft = parse(fields.next())?;
    let hard = parse(fields.next())?;
    Some((soft, hard))
}

/// Directories from the process cgroup up to (and including) `root`, leaf
/// first. A path that escapes the namespace (`..`) yields only `root`.
fn cgroup_dirs(root: &Path, relative: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let relative = relative.trim();
    if !relative.split('/').any(|segment| segment == "..") {
        let trimmed = relative.trim_start_matches('/');
        if !trimmed.is_empty() {
            let mut current = root.join(trimmed);
            while current != root && current.starts_with(root) {
                dirs.push(current.clone());
                if !current.pop() {
                    break;
                }
            }
        }
    }
    dirs.push(root.to_path_buf());
    dirs
}

// ---------------------------------------------------------------------------
// Host facts (inputs) and the plan
// ---------------------------------------------------------------------------

/// Everything the planner needs, as read from the host or built by hand.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct HostFacts {
    /// Scheduler affinity of the probing thread.
    pub affinity: Option<Vec<usize>>,
    /// cgroup v2 `cpuset.cpus.effective`.
    pub cpuset_effective: Option<Vec<usize>>,
    /// `available_parallelism`, used only when no CPU set is known.
    pub fallback_parallelism: usize,
    /// CPU id to the CPUs of its physical core (itself included).
    pub core_siblings: BTreeMap<usize, Vec<usize>>,
    /// Where `core_siblings` came from.
    pub topology_source: String,
    /// Smallest `cpu.max` quota on the hierarchy, in CPUs.
    pub cpu_quota: Option<f64>,
    /// Smallest `memory.max` on the hierarchy, bytes.
    pub memory_max: Option<u64>,
    /// Smallest `memory.high` on the hierarchy, bytes.
    pub memory_high: Option<u64>,
    /// `MemTotal`, bytes.
    pub mem_total: Option<u64>,
    /// Smallest `pids.max` on the hierarchy.
    pub pids_max: Option<u64>,
    /// `RLIMIT_NOFILE` soft limit (`None`: unlimited or unknown).
    pub nofile_soft: Option<u64>,
    /// `RLIMIT_NOFILE` hard limit (`None`: unlimited or unknown).
    pub nofile_hard: Option<u64>,
    /// NUMA node id to its CPU list.
    pub numa_nodes: BTreeMap<usize, Vec<usize>>,
    /// cgroup v2 path of the process, when known.
    pub cgroup_path: Option<String>,
    /// Whether thread CPU affinity can be applied on this platform.
    pub pinning_supported: bool,
    /// Probe diagnostics (unreadable or malformed inputs).
    pub notes: Vec<String>,
}

/// A NUMA node intersected with the usable CPUs (reported only).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NumaNode {
    /// Node id.
    pub id: usize,
    /// CPUs of this node that the process may use.
    pub cpus: Vec<usize>,
}

/// Media session capacity estimate. It is reported, never enforced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlanCapacity {
    /// `⌊u·10⁶ ÷ (pps_session × t_pkt)⌋` sessions per media lane.
    pub sessions_per_lane: u64,
    /// `M × sessions_per_lane`.
    pub media_sessions: u64,
}

/// How the signaling thread count was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignalingSizing {
    /// The `σ` / `σ_smt` share.
    Sigma,
    /// [`HostPlanConfig::signaling_threads`].
    Explicit,
}

/// The immutable executor plan.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ExecutorPlan {
    /// Parameters used.
    pub config: HostPlanConfig,
    /// `L`: usable logical CPUs.
    pub logical_cpus: Vec<usize>,
    /// `P`: physical cores in `L`.
    pub physical_cores: usize,
    /// `Q`: CPU quota in CPUs, if any.
    pub cpu_quota: Option<f64>,
    /// The quota is the binding limit (`Q < |L|`): nothing is pinned.
    pub quota_only: bool,
    /// `E`: effective cores.
    pub effective_cores: usize,
    /// Signaling threads are placed on SMT siblings of the media cores.
    pub smt: bool,
    /// `I`: dedicated housekeeping cores.
    pub housekeeping_cores: usize,
    /// CPUs of the housekeeping core(s); empty when unpinned.
    pub housekeeping_cpus: Vec<usize>,
    /// `M`: media lanes, one current-thread runtime each.
    pub media_lanes: usize,
    /// One CPU per media lane (first siblings); empty when unpinned.
    pub media_cpus: Vec<usize>,
    /// `S`: signaling runtime worker threads.
    pub signaling_threads: usize,
    /// CPUs the signaling runtime may run on; empty when unpinned.
    pub signaling_cpus: Vec<usize>,
    /// How `signaling_threads` was chosen.
    pub signaling_sizing: SignalingSizing,
    /// Signaling runtime `max_blocking_threads`.
    pub max_blocking_threads: usize,
    /// `B`: memory budget, bytes.
    pub memory_budget_bytes: Option<u64>,
    /// `memory.max`, bytes.
    pub memory_max_bytes: Option<u64>,
    /// `memory.high`, bytes.
    pub memory_high_bytes: Option<u64>,
    /// `MemTotal`, bytes.
    pub mem_total_bytes: Option<u64>,
    /// `pids.max`.
    pub pids_max: Option<u64>,
    /// `RLIMIT_NOFILE` soft limit.
    pub nofile_soft: Option<u64>,
    /// `RLIMIT_NOFILE` hard limit.
    pub nofile_hard: Option<u64>,
    /// NUMA nodes in `L` (reported only).
    pub numa_nodes: Vec<NumaNode>,
    /// Session capacity estimate (reported only).
    pub capacity: PlanCapacity,
    /// Topology source.
    pub topology_source: String,
    /// cgroup path.
    pub cgroup_path: Option<String>,
    /// Probe and planning notes.
    pub notes: Vec<String>,
}

impl ExecutorPlan {
    /// Probe the running host and plan for it.
    pub fn probe(config: &HostPlanConfig) -> Result<Self, HostPlanError> {
        config.validate()?;
        Ok(compute_plan(&HostFacts::probe(), config))
    }

    /// Plan for `facts`, which may come from [`HostFacts::read`] or be built
    /// by hand. Pure: nothing is read from the host.
    pub fn from_facts(facts: &HostFacts, config: &HostPlanConfig) -> Result<Self, HostPlanError> {
        config.validate()?;
        Ok(compute_plan(facts, config))
    }

    /// Whether media lanes and signaling threads are pinned.
    pub fn pinned(&self) -> bool {
        !self.quota_only && !self.media_cpus.is_empty()
    }

    /// The media executor layout: one dedicated lane per media core, pinned
    /// to its first sibling when the plan is pinned.
    pub fn media_runtime_config(&self) -> MediaRuntimeConfig {
        let lanes = self.media_lanes.max(1);
        let cpus = if self.pinned() && self.media_cpus.len() == lanes {
            self.media_cpus.iter().copied().map(Some).collect()
        } else {
            vec![None; lanes]
        };
        MediaRuntimeConfig::new(MediaLanes::Dedicated { cpus })
    }

    /// A multi-thread runtime builder for the signaling runtime:
    /// [`Self::signaling_threads`] workers, [`Self::max_blocking_threads`]
    /// blocking threads and, when the plan is pinned, every thread placed on
    /// [`Self::signaling_cpus`] (a thread whose placement fails logs a
    /// warning and floats). Enable the drivers and build it as usual.
    pub fn signaling_runtime_builder(&self) -> tokio::runtime::Builder {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        builder
            .worker_threads(self.signaling_threads.max(1))
            .max_blocking_threads(self.max_blocking_threads.max(1));
        if self.pinned() && !self.signaling_cpus.is_empty() {
            let cpus = self.signaling_cpus.clone();
            builder.on_thread_start(move || {
                if let Err(error) = pin_current_thread(&cpus) {
                    tracing::warn!(
                        %error,
                        ?cpus,
                        "signaling thread CPU pinning failed; the thread floats"
                    );
                }
            });
        }
        builder
    }
}

/// L = affinity ∩ cpuset.cpus.effective (falling back to whichever is known,
/// then to `0..available_parallelism`).
fn logical_cpus(facts: &HostFacts) -> Vec<usize> {
    let fallback = || (0..facts.fallback_parallelism.max(1)).collect::<Vec<_>>();
    let set: Vec<usize> = match (&facts.affinity, &facts.cpuset_effective) {
        (Some(affinity), Some(cpuset)) => {
            let cpuset: BTreeSet<usize> = cpuset.iter().copied().collect();
            let both: Vec<usize> = affinity
                .iter()
                .copied()
                .filter(|cpu| cpuset.contains(cpu))
                .collect();
            if both.is_empty() {
                affinity.clone()
            } else {
                both
            }
        }
        (Some(affinity), None) => affinity.clone(),
        (None, Some(cpuset)) => cpuset.clone(),
        (None, None) => fallback(),
    };
    let set: BTreeSet<usize> = set.into_iter().collect();
    if set.is_empty() {
        fallback()
    } else {
        set.into_iter().collect()
    }
}

/// Group `logical` into physical cores using the sibling lists. Each core is
/// its CPUs within `logical`, ascending; cores are ordered by first CPU.
fn physical_cores(logical: &[usize], siblings: &BTreeMap<usize, Vec<usize>>) -> Vec<Vec<usize>> {
    let mut by_core: BTreeMap<Vec<usize>, Vec<usize>> = BTreeMap::new();
    let mut sorted = logical.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for cpu in sorted {
        let key = siblings
            .get(&cpu)
            .filter(|list| list.contains(&cpu))
            .cloned()
            .unwrap_or_else(|| vec![cpu]);
        by_core.entry(key).or_default().push(cpu);
    }
    let mut cores: Vec<Vec<usize>> = by_core.into_values().collect();
    cores.sort_by_key(|core| core[0]);
    cores
}

fn ceil_share(share: f64, count: usize) -> usize {
    let value = (share * count as f64 - EPSILON).ceil();
    if value <= 0.0 {
        0
    } else {
        value as usize
    }
}

fn round_share(share: f64, count: usize) -> usize {
    let value = (share * count as f64).round();
    if value <= 0.0 {
        0
    } else {
        value as usize
    }
}

/// Split `available` dedicated cores/threads into (signaling, media) with
/// `signaling` threads requested.
fn split_dedicated(signaling: usize, available: usize) -> (usize, usize) {
    if available < 2 {
        // One core: signaling and the single media lane share it.
        return (1, 1);
    }
    let signaling = signaling.clamp(1, available - 1);
    (signaling, available - signaling)
}

/// Candidate `(signaling, media)` layouts.
struct Layouts {
    /// The `σ` / `σ_smt` rule.
    sigma: (usize, usize),
    /// [`HostPlanConfig::signaling_threads`], clamped to the usable CPUs.
    explicit: Option<(usize, usize)>,
}

impl Layouts {
    fn choose(&self) -> ((usize, usize), SignalingSizing) {
        match self.explicit {
            Some(layout) => (layout, SignalingSizing::Explicit),
            None => (self.sigma, SignalingSizing::Sigma),
        }
    }
}

/// Compute the plan from host facts. Pure: fixtures drive it directly.
fn compute_plan(facts: &HostFacts, config: &HostPlanConfig) -> ExecutorPlan {
    let mut notes = facts.notes.clone();
    let logical = logical_cpus(facts);
    let cores = physical_cores(&logical, &facts.core_siblings);
    let physical = cores.len().max(1);

    let quota = facts
        .cpu_quota
        .filter(|quota| quota.is_finite() && *quota > 0.0);
    let quota_only = quota.is_some_and(|quota| quota + EPSILON < logical.len() as f64);
    let quota_threads = quota.map(|quota| ((quota + EPSILON).floor() as usize).max(1));
    let effective = match (quota_only, quota_threads) {
        (true, Some(threads)) => physical.min(threads),
        _ => physical,
    };
    let housekeeping = usize::from(effective >= config.housekeeping_min_cores && effective >= 3);

    let uniform_smt = !cores.is_empty() && cores.iter().all(|core| core.len() >= 2);
    let mixed_smt = !uniform_smt && cores.iter().any(|core| core.len() >= 2);
    if mixed_smt {
        notes.push("mixed SMT in the usable CPUs: planning first siblings only".to_string());
    }
    let smt = uniform_smt && config.smt_policy == SmtPolicy::Siblings && !quota_only;
    let pin = facts.pinning_supported && !quota_only;
    if !facts.pinning_supported && !quota_only {
        notes.push("thread CPU affinity is unsupported here: lanes float".to_string());
    }

    // Candidate layouts for `available` dedicated threads (quota) or cores.
    let dedicated_layouts = |available: usize| Layouts {
        sigma: split_dedicated(round_share(config.sigma, available), available),
        explicit: config
            .signaling_threads
            .map(|threads| split_dedicated(threads, available)),
    };
    let layouts = if quota_only {
        let threads = quota_threads.unwrap_or(1);
        let available = threads.saturating_sub(housekeeping).max(1);
        notes.push(format!(
            "quota-only: {threads} threads from cpu.max, no pinning"
        ));
        dedicated_layouts(available)
    } else {
        let available = cores
            .len()
            .saturating_sub(housekeeping.min(cores.len()))
            .max(1);
        if smt {
            // Signaling threads take second siblings: every core stays a lane.
            Layouts {
                sigma: (
                    ceil_share(config.effective_sigma_smt(), effective).clamp(1, available),
                    available,
                ),
                explicit: config
                    .signaling_threads
                    .map(|threads| (threads.clamp(1, available), available)),
            }
        } else {
            dedicated_layouts(available)
        }
    };
    let ((signaling_threads, media_lanes), signaling_sizing) = layouts.choose();
    if let (Some(requested), SignalingSizing::Explicit) =
        (config.signaling_threads, signaling_sizing)
    {
        if requested != signaling_threads {
            notes.push(format!(
                "signaling_threads={requested} clamped to {signaling_threads} by the usable CPUs"
            ));
        }
    }

    let mut housekeeping_cpus = Vec::new();
    let mut media_cpus = Vec::new();
    let mut signaling_cpus = Vec::new();
    if !quota_only {
        let (housekeeping_cores, usable) = cores.split_at(housekeeping.min(cores.len()));
        if pin {
            housekeeping_cpus = housekeeping_cores.iter().flatten().copied().collect();
        }
        if smt {
            if pin {
                media_cpus = usable.iter().map(|core| core[0]).collect();
                signaling_cpus = usable
                    .iter()
                    .flat_map(|core| core[1..].iter().copied())
                    .collect();
                signaling_cpus.sort_unstable();
            }
        } else if pin && !usable.is_empty() {
            if usable.len() < 2 {
                media_cpus = vec![usable[0][0]];
                signaling_cpus = vec![usable[0][0]];
                notes.push("single usable core: signaling shares the media core".to_string());
            } else {
                media_cpus = usable[..media_lanes].iter().map(|core| core[0]).collect();
                signaling_cpus = usable[media_lanes..].iter().map(|core| core[0]).collect();
            }
        }
    }

    let max_blocking_threads = match facts.pids_max {
        Some(pids) => usize::try_from(pids / 8)
            .unwrap_or(usize::MAX)
            .clamp(1, config.max_blocking_threads_cap),
        None => config.max_blocking_threads_cap,
    };
    if let Some(pids) = facts.pids_max {
        let planned = (media_lanes + signaling_threads + max_blocking_threads) as u64;
        if planned.saturating_mul(2) > pids {
            notes.push(format!(
                "planned threads {planned} exceed half of pids.max {pids}"
            ));
        }
    }

    let memory_budget_bytes = [facts.memory_max, facts.memory_high, facts.mem_total]
        .into_iter()
        .flatten()
        .min();

    let sessions_per_lane = (config.lane_utilization * 1e6
        / (config.session_pps * config.packet_cost_us))
        .floor()
        .max(0.0) as u64;
    let capacity = PlanCapacity {
        sessions_per_lane,
        media_sessions: sessions_per_lane.saturating_mul(media_lanes as u64),
    };

    let logical_set: BTreeSet<usize> = logical.iter().copied().collect();
    let numa_nodes = facts
        .numa_nodes
        .iter()
        .filter_map(|(id, cpus)| {
            let cpus: Vec<usize> = cpus
                .iter()
                .copied()
                .filter(|cpu| logical_set.contains(cpu))
                .collect();
            (!cpus.is_empty()).then_some(NumaNode { id: *id, cpus })
        })
        .collect();

    ExecutorPlan {
        config: config.clone(),
        logical_cpus: logical,
        physical_cores: physical,
        cpu_quota: quota,
        quota_only,
        effective_cores: effective,
        smt,
        housekeeping_cores: housekeeping,
        housekeeping_cpus,
        media_lanes,
        media_cpus,
        signaling_threads,
        signaling_cpus,
        signaling_sizing,
        max_blocking_threads,
        memory_budget_bytes,
        memory_max_bytes: facts.memory_max,
        memory_high_bytes: facts.memory_high,
        mem_total_bytes: facts.mem_total,
        pids_max: facts.pids_max,
        nofile_soft: facts.nofile_soft,
        nofile_hard: facts.nofile_hard,
        numa_nodes,
        capacity,
        topology_source: facts.topology_source.clone(),
        cgroup_path: facts.cgroup_path.clone(),
        notes,
    }
}

// ---------------------------------------------------------------------------
// IO layer
// ---------------------------------------------------------------------------

/// Filesystem roots for the probe (`/proc`, `/sys`); fixtures use a temp tree.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HostRoots {
    /// procfs root.
    pub proc_root: PathBuf,
    /// sysfs root.
    pub sys_root: PathBuf,
}

impl HostRoots {
    /// The running system (`/proc`, `/sys`).
    pub fn system() -> Self {
        Self::new("/proc", "/sys")
    }

    /// procfs and sysfs mounted under the given roots.
    pub fn new(proc_root: impl Into<PathBuf>, sys_root: impl Into<PathBuf>) -> Self {
        Self {
            proc_root: proc_root.into(),
            sys_root: sys_root.into(),
        }
    }
}

fn read_text(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Smallest parsed value of `file` over `dirs`, recording malformed files.
fn min_over<T: PartialOrd + Copy>(
    dirs: &[PathBuf],
    file: &str,
    parse: fn(&str) -> Result<Option<T>, String>,
    notes: &mut Vec<String>,
) -> Option<T> {
    let mut best: Option<T> = None;
    for dir in dirs {
        let path = dir.join(file);
        let Some(text) = read_text(&path) else {
            continue;
        };
        match parse(&text) {
            Ok(Some(value)) => {
                if best.is_none_or(|current| value < current) {
                    best = Some(value);
                }
            }
            Ok(None) => {}
            Err(error) => notes.push(format!("{}: {error}", path.display())),
        }
    }
    best
}

impl HostFacts {
    /// Read the running host (`/proc`, `/sys`).
    pub fn probe() -> Self {
        Self::read(&HostRoots::system())
    }

    /// Read every planner input under `roots`. Unreadable or malformed
    /// inputs leave their fact unknown and add a note.
    pub fn read(roots: &HostRoots) -> Self {
        let mut facts = HostFacts {
            fallback_parallelism: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1),
            pinning_supported: cfg!(target_os = "linux"),
            ..HostFacts::default()
        };
        let proc_self = roots.proc_root.join("self");

        if let Some(status) = read_text(&proc_self.join("status")) {
            match parse_status_cpus_allowed(&status) {
                Some(Ok(cpus)) if !cpus.is_empty() => facts.affinity = Some(cpus),
                Some(Err(error)) => facts.notes.push(format!("Cpus_allowed_list: {error}")),
                _ => {}
            }
        }
        if let Some(limits) = read_text(&proc_self.join("limits")) {
            if let Some((soft, hard)) = parse_proc_limits_nofile(&limits) {
                facts.nofile_soft = soft;
                facts.nofile_hard = hard;
            }
        }
        if let Some(meminfo) = read_text(&roots.proc_root.join("meminfo")) {
            facts.mem_total = parse_meminfo_total(&meminfo);
        }

        facts.read_cgroup(roots, &proc_self);
        facts.read_topology(roots);
        facts
    }

    fn read_cgroup(&mut self, roots: &HostRoots, proc_self: &Path) {
        let cgroup_root = roots.sys_root.join("fs").join("cgroup");
        let cgroup_path =
            read_text(&proc_self.join("cgroup")).and_then(|text| parse_proc_cgroup_v2(&text));
        if !cgroup_root.join("cgroup.controllers").exists() {
            self.notes.push(format!(
                "cgroup v2 is not mounted at {}: cgroup limits not read",
                cgroup_root.display()
            ));
        } else if let Some(path) = cgroup_path.as_deref() {
            let mut dirs = cgroup_dirs(&cgroup_root, path);
            if dirs.len() > 1 && !dirs[0].is_dir() {
                self.notes.push(format!(
                    "cgroup {path} is not visible under {}; using its root",
                    cgroup_root.display()
                ));
                dirs = vec![cgroup_root.clone()];
            }
            for dir in &dirs {
                let path = dir.join("cpuset.cpus.effective");
                let Some(text) = read_text(&path) else {
                    continue;
                };
                match parse_cpu_list(&text) {
                    Ok(cpus) if !cpus.is_empty() => {
                        self.cpuset_effective = Some(cpus);
                        break;
                    }
                    Ok(_) => {}
                    Err(error) => self.notes.push(format!("{}: {error}", path.display())),
                }
            }
            self.cpu_quota = min_over(&dirs, "cpu.max", parse_cpu_max, &mut self.notes);
            self.memory_max = min_over(&dirs, "memory.max", parse_limit, &mut self.notes);
            self.memory_high = min_over(&dirs, "memory.high", parse_limit, &mut self.notes);
            self.pids_max = min_over(&dirs, "pids.max", parse_limit, &mut self.notes);
        } else {
            self.notes
                .push("no cgroup v2 entry in /proc/self/cgroup".to_string());
        }
        self.cgroup_path = cgroup_path;
    }

    fn read_topology(&mut self, roots: &HostRoots) {
        let cpu_root = roots.sys_root.join("devices").join("system").join("cpu");
        let mut sources = BTreeSet::new();
        for cpu in logical_cpus(self) {
            let topology = cpu_root.join(format!("cpu{cpu}")).join("topology");
            let found = ["core_cpus_list", "thread_siblings_list"]
                .into_iter()
                .find_map(|file| {
                    let list = parse_cpu_list(&read_text(&topology.join(file))?).ok()?;
                    list.contains(&cpu).then_some((list, file))
                });
            match found {
                Some((list, source)) => {
                    self.core_siblings.insert(cpu, list);
                    sources.insert(source);
                }
                None => {
                    sources.insert("one_per_cpu");
                }
            }
        }
        self.topology_source = sources.into_iter().collect::<Vec<_>>().join("+");

        let node_root = roots.sys_root.join("devices").join("system").join("node");
        if let Ok(entries) = std::fs::read_dir(&node_root) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(id) = name
                    .to_str()
                    .and_then(|name| name.strip_prefix("node"))
                    .and_then(|id| id.parse::<usize>().ok())
                else {
                    continue;
                };
                if let Some(Ok(cpus)) =
                    read_text(&entry.path().join("cpulist")).map(|text| parse_cpu_list(&text))
                {
                    self.numa_nodes.insert(id, cpus);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_runtime::MediaRuntimePool;

    const GIB: u64 = 1024 * 1024 * 1024;

    /// `cores` SMT cores with two threads each; CPU `n` and `n + cores` are
    /// siblings (the usual x86 numbering).
    fn smt_siblings(cores: usize) -> BTreeMap<usize, Vec<usize>> {
        (0..2 * cores)
            .map(|cpu| {
                let core = cpu % cores;
                (cpu, vec![core, core + cores])
            })
            .collect()
    }

    fn facts(
        affinity: &str,
        cpuset: Option<&str>,
        siblings: BTreeMap<usize, Vec<usize>>,
    ) -> HostFacts {
        HostFacts {
            affinity: Some(parse_cpu_list(affinity).unwrap()),
            cpuset_effective: cpuset.map(|text| parse_cpu_list(text).unwrap()),
            fallback_parallelism: 16,
            core_siblings: siblings,
            topology_source: "core_cpus_list".to_string(),
            pinning_supported: true,
            ..HostFacts::default()
        }
    }

    /// 8 SMT cores, 16 threads, 32 GiB.
    fn smt8(cpuset: Option<&str>) -> HostFacts {
        HostFacts {
            mem_total: Some(32 * GIB),
            ..facts("0-15", cpuset, smt_siblings(8))
        }
    }

    /// 16 cores without SMT, 64 GiB.
    fn cores16() -> HostFacts {
        let siblings = (0..16).map(|cpu| (cpu, vec![cpu])).collect();
        HostFacts {
            mem_total: Some(64 * GIB),
            ..facts("0-15", Some("0-15"), siblings)
        }
    }

    /// 24 SMT cores, 48 threads, 96 GiB.
    fn smt24() -> HostFacts {
        HostFacts {
            mem_total: Some(96 * GIB),
            ..facts("0-47", None, smt_siblings(24))
        }
    }

    fn plan(facts: &HostFacts, config: &HostPlanConfig) -> ExecutorPlan {
        ExecutorPlan::from_facts(facts, config).unwrap()
    }

    fn default_plan(facts: &HostFacts) -> ExecutorPlan {
        plan(facts, &HostPlanConfig::default())
    }

    fn assert_no_shared_media_core(plan: &ExecutorPlan, siblings: &BTreeMap<usize, Vec<usize>>) {
        let mut seen = BTreeSet::new();
        for cpu in &plan.media_cpus {
            let core = siblings.get(cpu).cloned().unwrap_or_else(|| vec![*cpu]);
            assert!(
                seen.insert(core),
                "two media lanes share a physical core: {plan:?}"
            );
        }
    }

    #[test]
    fn cpu_lists_parse_ranges_and_reject_garbage() {
        assert_eq!(
            parse_cpu_list("0-3,8-11\n").unwrap(),
            vec![0, 1, 2, 3, 8, 9, 10, 11]
        );
        assert_eq!(parse_cpu_list("0,8").unwrap(), vec![0, 8]);
        assert_eq!(parse_cpu_list("5").unwrap(), vec![5]);
        assert_eq!(parse_cpu_list("\n").unwrap(), Vec::<usize>::new());
        for bad in ["a", "3-1", "0-", "-2", "1-70000"] {
            assert!(parse_cpu_list(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cgroup_files_parse() {
        assert_eq!(parse_cpu_max("650000 100000\n").unwrap(), Some(6.5));
        assert_eq!(parse_cpu_max("max 100000\n").unwrap(), None);
        assert!(parse_cpu_max("100 0").is_err());
        assert!(parse_cpu_max("").is_err());
        assert_eq!(parse_limit("max\n").unwrap(), None);
        assert_eq!(parse_limit("30064771072\n").unwrap(), Some(28 * GIB));
        assert_eq!(parse_limit("4294967296").unwrap(), Some(4 * GIB));
        assert_eq!(parse_limit("256\n").unwrap(), Some(256));
        assert!(parse_limit("lots").is_err());
        assert_eq!(
            parse_proc_cgroup_v2("12:cpu:/x\n0::/system.slice/docker-abc.scope\n"),
            Some("/system.slice/docker-abc.scope".to_string())
        );
        assert_eq!(parse_proc_cgroup_v2("0::/\n"), Some("/".to_string()));
        assert_eq!(parse_proc_cgroup_v2("4:pids:/x\n"), None);
        assert_eq!(
            parse_meminfo_total("MemTotal:       65536000 kB\nMemFree: 1 kB\n"),
            Some(65_536_000 * 1024)
        );
        let limits = "Limit                     Soft Limit           Hard Limit           Units     \n\
                      Max open files            65536                524288               files     \n";
        assert_eq!(
            parse_proc_limits_nofile(limits),
            Some((Some(65536), Some(524_288)))
        );
        let unlimited =
            "Max open files            unlimited            unlimited            files\n";
        assert_eq!(parse_proc_limits_nofile(unlimited), Some((None, None)));
        let status = "Name:\tsip-server\nCpus_allowed:\tffff\nCpus_allowed_list:\t0-15\n";
        assert_eq!(
            parse_status_cpus_allowed(status),
            Some(Ok((0..16).collect()))
        );
    }

    #[test]
    fn cgroup_walk_is_leaf_first_and_never_escapes_root() {
        let root = Path::new("/sys/fs/cgroup");
        assert_eq!(
            cgroup_dirs(root, "/a/b"),
            vec![
                PathBuf::from("/sys/fs/cgroup/a/b"),
                PathBuf::from("/sys/fs/cgroup/a"),
                PathBuf::from("/sys/fs/cgroup"),
            ]
        );
        assert_eq!(
            cgroup_dirs(root, "/"),
            vec![PathBuf::from("/sys/fs/cgroup")]
        );
        assert_eq!(
            cgroup_dirs(root, "/../x"),
            vec![PathBuf::from("/sys/fs/cgroup")]
        );
    }

    #[test]
    fn smt_host_pins_one_lane_per_core_and_signaling_on_second_siblings() {
        let facts = HostFacts {
            memory_max: Some(28 * GIB),
            ..smt8(None)
        };
        let plan = default_plan(&facts);
        assert_eq!(plan.logical_cpus, (0..16).collect::<Vec<_>>());
        assert_eq!(plan.physical_cores, 8);
        assert_eq!(plan.effective_cores, 8);
        assert!(plan.smt && !plan.quota_only && plan.pinned());
        assert_eq!(plan.housekeeping_cores, 0);
        assert_eq!(plan.media_lanes, 8);
        assert_eq!(plan.media_cpus, (0..8).collect::<Vec<_>>());
        // ⌈0.4 × 8⌉
        assert_eq!(plan.signaling_threads, 4);
        assert_eq!(plan.signaling_sizing, SignalingSizing::Sigma);
        assert_eq!(plan.signaling_cpus, (8..16).collect::<Vec<_>>());
        assert_eq!(plan.memory_budget_bytes, Some(28 * GIB));
        assert_eq!(plan.max_blocking_threads, 64);
        assert_eq!(plan.capacity.sessions_per_lane, 701);
        assert_eq!(plan.capacity.media_sessions, 8 * 701);
        assert_no_shared_media_core(&plan, &facts.core_siblings);
    }

    #[test]
    fn sibling_signaling_on_24_smt_cores_gives_23_signaling_threads_and_23_lanes() {
        // σ_smt = 1.0 (HostPlanConfig::sibling_signaling): one signaling
        // thread per media core on its second sibling. The housekeeping core
        // keeps both of its siblings; the media lanes are unchanged.
        let facts = smt24();
        let sibling = plan(&facts, &HostPlanConfig::sibling_signaling());
        assert_eq!(sibling.config.effective_sigma_smt(), 1.0);
        assert_eq!((sibling.physical_cores, sibling.effective_cores), (24, 24));
        assert!(sibling.smt && sibling.pinned());
        assert_eq!(sibling.housekeeping_cores, 1);
        assert_eq!(sibling.housekeeping_cpus, vec![0, 24]);
        assert_eq!(sibling.media_lanes, 23);
        assert_eq!(sibling.media_cpus, (1..24).collect::<Vec<_>>());
        assert_eq!(sibling.signaling_threads, 23);
        assert_eq!(sibling.signaling_cpus, (25..48).collect::<Vec<_>>());
        assert_no_shared_media_core(&sibling, &facts.core_siblings);

        // The default share sizes 10 threads (⌈0.4 × 24⌉) for the same lanes.
        let default = default_plan(&facts);
        assert_eq!((default.signaling_threads, default.media_lanes), (10, 23));
        assert_eq!(default.media_cpus, sibling.media_cpus);
        assert_eq!(default.signaling_cpus, sibling.signaling_cpus);

        // The media layout is one pinned lane per first sibling.
        assert_eq!(
            sibling.media_runtime_config(),
            MediaRuntimeConfig::new(MediaLanes::pinned(1..24))
        );
    }

    #[test]
    fn sigma_smt_follows_sigma_unless_set_and_only_applies_with_smt() {
        let follows = plan(&smt8(None), &HostPlanConfig::new().with_sigma(0.5));
        assert_eq!(follows.config.effective_sigma_smt(), 0.5);
        assert_eq!(follows.signaling_threads, 4);
        let full = plan(&smt8(None), &HostPlanConfig::sibling_signaling());
        assert_eq!((full.signaling_threads, full.media_lanes), (8, 8));
        // Without SMT, σ_smt is ignored: every signaling thread costs a core.
        let dedicated = plan(&cores16(), &HostPlanConfig::sibling_signaling());
        let sigma = default_plan(&cores16());
        assert_eq!(
            (dedicated.signaling_threads, dedicated.media_lanes),
            (sigma.signaling_threads, sigma.media_lanes)
        );
        let quota = HostFacts {
            cpu_quota: Some(6.5),
            ..smt8(None)
        };
        let quota_plan = plan(&quota, &HostPlanConfig::sibling_signaling());
        assert!(quota_plan.quota_only && !quota_plan.smt);
        assert_eq!(
            (quota_plan.signaling_threads, quota_plan.media_lanes),
            (2, 4)
        );
    }

    #[test]
    fn cores_without_smt_reserve_housekeeping_and_exceed_the_shared_lane_cap() {
        let facts = cores16();
        let plan = default_plan(&facts);
        assert_eq!(plan.physical_cores, 16);
        assert_eq!(plan.effective_cores, 16);
        assert!(!plan.smt);
        assert_eq!(plan.housekeeping_cores, 1);
        assert_eq!(plan.housekeeping_cpus, vec![0]);
        // round(0.4 × 15)
        assert_eq!(plan.signaling_threads, 6);
        assert_eq!(plan.media_lanes, 9);
        assert_eq!(plan.media_cpus, (1..10).collect::<Vec<_>>());
        assert_eq!(plan.signaling_cpus, (10..16).collect::<Vec<_>>());
        assert_eq!(plan.memory_budget_bytes, Some(64 * GIB));
        assert_no_shared_media_core(&plan, &facts.core_siblings);
    }

    #[test]
    fn cpuset_with_smt_uses_both_siblings_of_four_cores() {
        let facts = HostFacts {
            memory_max: Some(16 * GIB),
            ..smt8(Some("0-3,8-11"))
        };
        let plan = default_plan(&facts);
        assert_eq!(plan.logical_cpus, vec![0, 1, 2, 3, 8, 9, 10, 11]);
        assert_eq!(plan.physical_cores, 4);
        assert!(plan.smt);
        assert_eq!(plan.housekeeping_cores, 0);
        assert_eq!(plan.media_lanes, 4);
        assert_eq!(plan.media_cpus, vec![0, 1, 2, 3]);
        assert_eq!(plan.signaling_threads, 2);
        assert_eq!(plan.signaling_cpus, vec![8, 9, 10, 11]);
        assert_no_shared_media_core(&plan, &facts.core_siblings);
    }

    #[test]
    fn cpuset_without_siblings_splits_dedicated_cores() {
        let facts = smt8(Some("0-7"));
        let plan = default_plan(&facts);
        assert_eq!(plan.logical_cpus, (0..8).collect::<Vec<_>>());
        assert_eq!(plan.physical_cores, 8);
        assert!(!plan.smt);
        assert_eq!(plan.housekeeping_cores, 0);
        assert_eq!(plan.signaling_threads, 3);
        assert_eq!(plan.media_lanes, 5);
        assert_eq!(plan.media_cpus, vec![0, 1, 2, 3, 4]);
        assert_eq!(plan.signaling_cpus, vec![5, 6, 7]);
        assert_no_shared_media_core(&plan, &facts.core_siblings);
    }

    #[test]
    fn two_smt_cores_give_two_lanes_and_one_signaling_thread() {
        let plan = default_plan(&smt8(Some("0-1,8-9")));
        assert_eq!(plan.physical_cores, 2);
        assert!(plan.smt);
        assert_eq!(plan.media_lanes, 2);
        assert_eq!(plan.media_cpus, vec![0, 1]);
        assert_eq!(plan.signaling_threads, 1);
        assert_eq!(plan.signaling_cpus, vec![8, 9]);
    }

    #[test]
    fn affinity_narrower_than_cpuset_is_intersected() {
        let facts = HostFacts {
            affinity: Some(parse_cpu_list("0-1,8-9").unwrap()),
            ..smt8(Some("0-3,8-11"))
        };
        let plan = default_plan(&facts);
        assert_eq!(plan.logical_cpus, vec![0, 1, 8, 9]);
        assert_eq!(plan.media_cpus, vec![0, 1]);
    }

    #[test]
    fn quota_only_disables_pinning_and_uses_the_floor_of_the_quota() {
        let facts = HostFacts {
            cpu_quota: parse_cpu_max("650000 100000").unwrap(),
            ..smt8(None)
        };
        let plan = default_plan(&facts);
        assert_eq!(plan.cpu_quota, Some(6.5));
        assert!(plan.quota_only);
        assert!(!plan.pinned());
        assert!(!plan.smt);
        assert_eq!(plan.effective_cores, 6);
        assert_eq!(plan.housekeeping_cores, 0);
        assert_eq!(plan.signaling_threads, 2);
        assert_eq!(plan.media_lanes, 4);
        assert!(plan.media_cpus.is_empty());
        assert!(plan.signaling_cpus.is_empty());
        assert!(plan.housekeeping_cpus.is_empty());
        // Unpinned dedicated lanes.
        assert_eq!(
            plan.media_runtime_config(),
            MediaRuntimeConfig::new(MediaLanes::dedicated(4))
        );
    }

    #[test]
    fn quota_at_or_above_the_cpuset_does_not_disable_pinning() {
        let facts = HostFacts {
            cpu_quota: Some(16.0),
            ..smt8(None)
        };
        let plan = default_plan(&facts);
        assert!(!plan.quota_only);
        assert!(plan.pinned());
        assert_eq!(plan.media_lanes, 8);
    }

    #[test]
    fn memory_budget_takes_the_smallest_limit() {
        let facts = HostFacts {
            memory_max: parse_limit("4294967296").unwrap(),
            memory_high: parse_limit("max").unwrap(),
            ..smt8(None)
        };
        let plan = default_plan(&facts);
        assert_eq!(plan.memory_budget_bytes, Some(4 * GIB));
        assert_eq!(plan.memory_max_bytes, Some(4 * GIB));
        assert_eq!(plan.memory_high_bytes, None);
        assert_eq!(plan.mem_total_bytes, Some(32 * GIB));
        let high = HostFacts {
            memory_max: Some(28 * GIB),
            memory_high: Some(20 * GIB),
            ..smt8(None)
        };
        assert_eq!(default_plan(&high).memory_budget_bytes, Some(20 * GIB));
    }

    #[test]
    fn pids_limits_bound_the_blocking_pool_and_total_threads() {
        for (pids, blocking) in [(256, 32), (1024, 64), (64, 8), (4, 1)] {
            let facts = HostFacts {
                pids_max: Some(pids),
                ..smt8(None)
            };
            let plan = default_plan(&facts);
            assert_eq!(plan.max_blocking_threads, blocking, "pids.max={pids}");
            if pids >= 64 {
                let threads =
                    (plan.media_lanes + plan.signaling_threads + plan.max_blocking_threads) as u64;
                assert!(
                    threads * 2 <= pids,
                    "threads {threads} > pids.max/2 for {pids}"
                );
            } else {
                assert!(
                    plan.notes.iter().any(|note| note.contains("pids.max")),
                    "{:?}",
                    plan.notes
                );
            }
        }
        let capped = plan(
            &smt8(None),
            &HostPlanConfig::new().with_max_blocking_threads_cap(16),
        );
        assert_eq!(capped.max_blocking_threads, 16);
    }

    #[test]
    fn single_cpu_hosts_still_get_one_lane_and_one_signaling_thread() {
        let plan_one = default_plan(&facts("0", None, BTreeMap::new()));
        assert_eq!((plan_one.media_lanes, plan_one.signaling_threads), (1, 1));
        assert_eq!(plan_one.media_cpus, vec![0]);
        assert_eq!(plan_one.signaling_cpus, vec![0]);
        let quota = HostFacts {
            cpu_quota: Some(0.5),
            ..smt8(None)
        };
        let plan_quota = default_plan(&quota);
        assert!(plan_quota.quota_only);
        assert_eq!(
            (plan_quota.media_lanes, plan_quota.signaling_threads),
            (1, 1)
        );
    }

    #[test]
    fn smt_ignore_policy_plans_dedicated_first_siblings() {
        let plan = plan(
            &smt8(None),
            &HostPlanConfig::new().with_smt_policy(SmtPolicy::Ignore),
        );
        assert!(!plan.smt);
        assert_eq!((plan.signaling_threads, plan.media_lanes), (3, 5));
        assert_eq!(plan.media_cpus, vec![0, 1, 2, 3, 4]);
        assert_eq!(plan.signaling_cpus, vec![5, 6, 7]);
    }

    #[test]
    fn unsupported_pinning_keeps_sizes_but_no_cpu_ids() {
        let facts = HostFacts {
            pinning_supported: false,
            ..smt8(None)
        };
        let plan = default_plan(&facts);
        assert_eq!((plan.media_lanes, plan.signaling_threads), (8, 4));
        assert!(!plan.pinned());
        assert!(plan.media_cpus.is_empty() && plan.signaling_cpus.is_empty());
        assert_eq!(
            plan.media_runtime_config(),
            MediaRuntimeConfig::new(MediaLanes::dedicated(8))
        );
    }

    #[test]
    fn explicit_signaling_threads_replace_the_share_within_usable_cpus() {
        let six = plan(
            &smt8(None),
            &HostPlanConfig::new().with_signaling_threads(6),
        );
        assert_eq!((six.signaling_threads, six.media_lanes), (6, 8));
        assert_eq!(six.signaling_sizing, SignalingSizing::Explicit);
        assert!(six.notes.iter().all(|note| !note.contains("clamped")));
        let clamped = plan(
            &smt8(None),
            &HostPlanConfig::new().with_signaling_threads(20),
        );
        assert_eq!(clamped.signaling_threads, 8);
        assert!(
            clamped
                .notes
                .iter()
                .any(|note| note.contains("clamped to 8")),
            "{:?}",
            clamped.notes
        );
        // Without SMT the explicit count takes cores from the media lanes,
        // always leaving one.
        let dedicated = plan(
            &cores16(),
            &HostPlanConfig::new().with_signaling_threads(20),
        );
        assert_eq!(
            (dedicated.signaling_threads, dedicated.media_lanes),
            (14, 1)
        );
        assert_eq!(dedicated.media_cpus, vec![1]);
        let quota = HostFacts {
            cpu_quota: Some(6.5),
            ..smt8(None)
        };
        let quota_plan = plan(&quota, &HostPlanConfig::new().with_signaling_threads(3));
        assert_eq!(
            (quota_plan.signaling_threads, quota_plan.media_lanes),
            (3, 3)
        );
    }

    #[test]
    fn configuration_defaults_and_rejects_out_of_range_values() {
        let config = HostPlanConfig::default();
        config.validate().unwrap();
        assert_eq!(config, HostPlanConfig::new());
        assert_eq!(config.sigma, 0.40);
        assert_eq!(config.sigma_smt, None);
        assert_eq!(config.housekeeping_min_cores, 12);
        assert_eq!(config.smt_policy, SmtPolicy::Siblings);
        assert_eq!(config.signaling_threads, None);
        assert_eq!(config.max_blocking_threads_cap, 64);
        HostPlanConfig::sibling_signaling().validate().unwrap();
        assert_eq!(
            HostPlanConfig::sibling_signaling().sigma_smt,
            Some(1.0),
            "the preset only changes σ_smt"
        );

        let mut cases: Vec<(&str, HostPlanConfig)> = Vec::new();
        let mut bad = |name, edit: &dyn Fn(&mut HostPlanConfig)| {
            let mut config = HostPlanConfig::default();
            edit(&mut config);
            cases.push((name, config));
        };
        bad("sigma", &|c| c.sigma = 1.5);
        bad("sigma", &|c| c.sigma = f64::NAN);
        bad("sigma", &|c| c.sigma = 0.0);
        bad("sigma_smt", &|c| c.sigma_smt = Some(0.0));
        bad("sigma_smt", &|c| c.sigma_smt = Some(1.01));
        bad("sigma_smt", &|c| c.sigma_smt = Some(f64::INFINITY));
        bad("housekeeping_min_cores", &|c| c.housekeeping_min_cores = 2);
        bad("signaling_threads", &|c| c.signaling_threads = Some(0));
        bad("signaling_threads", &|c| c.signaling_threads = Some(1025));
        bad("lane_utilization", &|c| c.lane_utilization = 2.0);
        bad("packet_cost_us", &|c| c.packet_cost_us = -1.0);
        bad("session_pps", &|c| c.session_pps = 0.0);
        bad("max_blocking_threads_cap", &|c| {
            c.max_blocking_threads_cap = 0
        });
        for (name, config) in cases {
            let error = config.validate().expect_err(name);
            assert!(error.to_string().contains(name), "{name}: {error}");
            assert_eq!(
                ExecutorPlan::from_facts(&smt8(None), &config),
                Err(error),
                "planning validates first"
            );
        }
    }

    #[test]
    fn io_layer_reads_a_fixture_tree_with_nested_cgroup_limits() {
        let base = std::env::temp_dir().join(format!(
            "rvoip-host-plan-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let write = |relative: &str, contents: &str| {
            let path = base.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        };
        write("proc/self/status", "Name:\tx\nCpus_allowed_list:\t0-15\n");
        write("proc/self/cgroup", "0::/pod/ctr\n");
        write(
            "proc/self/limits",
            "Max open files            65536                65536                files\n",
        );
        write("proc/meminfo", "MemTotal:       33554432 kB\n");
        write(
            "sys/fs/cgroup/cgroup.controllers",
            "cpuset cpu memory pids\n",
        );
        write("sys/fs/cgroup/pod/cpu.max", "650000 100000\n");
        write("sys/fs/cgroup/pod/memory.max", "30064771072\n");
        write("sys/fs/cgroup/pod/pids.max", "1024\n");
        write("sys/fs/cgroup/pod/ctr/cpu.max", "max 100000\n");
        write("sys/fs/cgroup/pod/ctr/memory.max", "max\n");
        write("sys/fs/cgroup/pod/ctr/memory.high", "max\n");
        write("sys/fs/cgroup/pod/ctr/pids.max", "256\n");
        write("sys/fs/cgroup/pod/ctr/cpuset.cpus.effective", "0-3,8-11\n");
        for cpu in 0..16 {
            let core = cpu % 8;
            // Only CPU 0 lacks core_cpus_list, exercising the fallback.
            let file = if cpu == 0 {
                "thread_siblings_list"
            } else {
                "core_cpus_list"
            };
            write(
                &format!("sys/devices/system/cpu/cpu{cpu}/topology/{file}"),
                &format!("{},{}\n", core, core + 8),
            );
        }
        write("sys/devices/system/node/node0/cpulist", "0-15\n");

        let roots = HostRoots::new(base.join("proc"), base.join("sys"));
        let facts = HostFacts::read(&roots);
        let _ = std::fs::remove_dir_all(&base);

        assert_eq!(facts.affinity, Some((0..16).collect()));
        assert_eq!(facts.cpuset_effective, Some(vec![0, 1, 2, 3, 8, 9, 10, 11]));
        assert_eq!(facts.cpu_quota, Some(6.5));
        assert_eq!(facts.memory_max, Some(28 * GIB));
        assert_eq!(facts.memory_high, None);
        assert_eq!(facts.pids_max, Some(256));
        assert_eq!(facts.mem_total, Some(32 * GIB));
        assert_eq!(
            (facts.nofile_soft, facts.nofile_hard),
            (Some(65536), Some(65536))
        );
        assert_eq!(facts.cgroup_path.as_deref(), Some("/pod/ctr"));
        assert_eq!(facts.topology_source, "core_cpus_list+thread_siblings_list");
        assert_eq!(facts.core_siblings.get(&0), Some(&vec![0, 8]));
        assert_eq!(facts.numa_nodes.get(&0), Some(&(0..16).collect::<Vec<_>>()));

        let mut facts = facts;
        facts.pinning_supported = true;
        let plan = default_plan(&facts);
        // 6.5 CPUs of quota against 8 usable logical CPUs: quota-only.
        assert!(plan.quota_only);
        assert_eq!(plan.physical_cores, 4);
        assert_eq!(plan.effective_cores, 4);
        assert_eq!((plan.signaling_threads, plan.media_lanes), (2, 4));
        assert_eq!(plan.max_blocking_threads, 32);
        assert_eq!(
            plan.numa_nodes,
            vec![NumaNode {
                id: 0,
                cpus: vec![0, 1, 2, 3, 8, 9, 10, 11]
            }]
        );
    }

    #[test]
    fn a_missing_tree_falls_back_to_available_parallelism_with_notes() {
        let base = std::env::temp_dir().join(format!(
            "rvoip-host-plan-missing-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let facts = HostFacts::read(&HostRoots::new(base.join("proc"), base.join("sys")));
        assert_eq!(facts.affinity, None);
        assert_eq!(facts.cpu_quota, None);
        assert_eq!(facts.topology_source, "one_per_cpu");
        assert!(facts.notes.iter().any(|note| note.contains("cgroup v2")));
        let plan = default_plan(&facts);
        assert_eq!(plan.logical_cpus.len(), facts.fallback_parallelism.max(1));
        assert!(plan.media_lanes >= 1 && plan.signaling_threads >= 1);
        plan.media_runtime_config().validate().unwrap();
    }

    #[test]
    fn this_host_plans_a_valid_layout_and_runs_it() {
        let plan = ExecutorPlan::probe(&HostPlanConfig::sibling_signaling()).unwrap();
        assert!(
            plan.media_lanes >= 1 && plan.signaling_threads >= 1,
            "{plan:?}"
        );
        let config = plan.media_runtime_config();
        config.validate().unwrap();
        let MediaLanes::Dedicated { cpus } = &config.lanes else {
            panic!("a plan lays out dedicated lanes: {config:?}");
        };
        assert_eq!(cpus.len(), plan.media_lanes);

        // The media lanes it describes, pinned where the host allows.
        let pool = MediaRuntimePool::from_config(&config, 1);
        let lanes = pool.snapshot();
        assert_eq!(lanes.len(), plan.media_lanes);
        for (lane, cpu) in lanes.iter().zip(cpus) {
            assert_eq!(lane.worker_threads, 1);
            assert!(
                lane.pinned_cpu == *cpu || lane.pinned_cpu.is_none(),
                "lane pinned to {:?}, planned {cpu:?}",
                lane.pinned_cpu
            );
        }
        if plan.pinned() && lanes.iter().all(|lane| lane.pinned_cpu.is_none()) {
            eprintln!("note: this host refused media lane pinning; lanes float");
        }
        drop(pool);

        // The signaling runtime the plan describes, on this host.
        let signaling = plan
            .signaling_runtime_builder()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(signaling.metrics().num_workers(), plan.signaling_threads);
        #[cfg(target_os = "linux")]
        if plan.pinned() {
            let cpu = signaling.block_on(async {
                tokio::spawn(async {
                    // SAFETY: sched_getcpu has no preconditions.
                    unsafe { libc::sched_getcpu() }
                })
                .await
                .unwrap()
            });
            assert!(
                usize::try_from(cpu).is_ok_and(|cpu| plan.signaling_cpus.contains(&cpu)),
                "signaling worker on CPU {cpu}, plan {:?}",
                plan.signaling_cpus
            );
        }
    }
}
