#![allow(dead_code)]

//! Carrier-style burst scenario definitions and helpers.

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const DEFAULT_SCENARIO_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/config/perf-burst-scenarios.yaml"
);

/// Same-host burst media topology.
///
/// The two media pools are deliberately disjoint. The gap from 26000 through
/// 26999 is reserved for the receiver SIP socket and every sharded caller SIP
/// socket used by the canonical matrix. Keeping the upper media pool below
/// 49152 also avoids the default IANA dynamic/private port range. A real bind
/// remains authoritative, so unrelated processes occupying a candidate are
/// handled by allocator quarantine and retry.
pub const BURST_BOB_MEDIA_START: u16 = 4_000;
pub const BURST_BOB_MEDIA_END: u16 = 25_999;
pub const BURST_ALICE_MEDIA_START: u16 = 27_000;
pub const BURST_ALICE_MEDIA_END: u16 = 49_151;

pub fn validate_same_host_burst_port_layout(
    bob_sip_port: u16,
    alice_base_sip_port: u16,
    alice_shards: usize,
    required_media_sessions: usize,
) -> Result<(), String> {
    if alice_shards == 0 {
        return Err("Alice shard count must be greater than zero".to_string());
    }

    let bob_capacity = media_port_range_capacity(BURST_BOB_MEDIA_START, BURST_BOB_MEDIA_END);
    if bob_capacity < required_media_sessions {
        return Err(format!(
            "Bob media range {}-{} has capacity {}, below the required {} sessions",
            BURST_BOB_MEDIA_START, BURST_BOB_MEDIA_END, bob_capacity, required_media_sessions
        ));
    }

    let alice_capacity = media_port_range_capacity(BURST_ALICE_MEDIA_START, BURST_ALICE_MEDIA_END);
    if alice_capacity < required_media_sessions {
        return Err(format!(
            "Alice media range {}-{} has capacity {}, below the required {} sessions",
            BURST_ALICE_MEDIA_START, BURST_ALICE_MEDIA_END, alice_capacity, required_media_sessions
        ));
    }

    let mut signaling_ports = Vec::with_capacity(alice_shards + 1);
    signaling_ports.push(("Bob", bob_sip_port));
    for shard in 0..alice_shards {
        let offset = u16::try_from(shard)
            .map_err(|_| format!("Alice shard index {shard} does not fit in a UDP port"))?
            .checked_mul(2)
            .ok_or_else(|| format!("Alice shard index {shard} overflows its SIP port offset"))?;
        let port = alice_base_sip_port.checked_add(offset).ok_or_else(|| {
            format!(
                "Alice SIP port range overflows u16 for base {} and shard {}",
                alice_base_sip_port, shard
            )
        })?;
        signaling_ports.push(("Alice", port));
    }

    for (role, port) in signaling_ports {
        if port_in_range(port, BURST_BOB_MEDIA_START, BURST_BOB_MEDIA_END)
            || port_in_range(port, BURST_ALICE_MEDIA_START, BURST_ALICE_MEDIA_END)
        {
            return Err(format!(
                "{role} SIP port {port} overlaps a same-host RTP media range"
            ));
        }
    }

    Ok(())
}

fn media_port_range_capacity(start: u16, end: u16) -> usize {
    usize::from(end.saturating_sub(start)) + 1
}

fn port_in_range(port: u16, start: u16, end: u16) -> bool {
    (start..=end).contains(&port)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BurstScenarioBook {
    pub version: u32,
    pub scenarios: Vec<BurstScenario>,
}

impl BurstScenarioBook {
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let text = fs::read_to_string(path).unwrap_or_else(|err| {
            panic!(
                "failed to read burst scenario file '{}': {err}",
                path.display()
            )
        });
        Self::from_yaml_str(&text, &path.display().to_string())
    }

    pub fn from_yaml_str(text: &str, source: &str) -> Self {
        let book: Self = serde_yaml::from_str(text)
            .unwrap_or_else(|err| panic!("failed to parse burst scenario file {source}: {err}"));
        assert_eq!(
            book.version, 1,
            "unsupported burst scenario book version {}; expected 1",
            book.version
        );
        assert!(
            !book.scenarios.is_empty(),
            "burst scenario book must contain at least one scenario"
        );
        for scenario in &book.scenarios {
            scenario.validate();
        }
        book
    }

    pub fn load_default_or_env() -> Self {
        let path = std::env::var("RVOIP_PERF_BURST_SCENARIO_FILE")
            .or_else(|_| std::env::var("BETA_BURST_SCENARIO_FILE"))
            .unwrap_or_else(|_| DEFAULT_SCENARIO_FILE.to_string());
        Self::from_path(path)
    }

    pub fn scenario(&self, name: &str) -> BurstScenario {
        self.scenarios
            .iter()
            .find(|scenario| scenario.name == name)
            .unwrap_or_else(|| {
                let names = self
                    .scenarios
                    .iter()
                    .map(|scenario| scenario.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                panic!("unknown burst scenario '{name}'; available scenarios: {names}")
            })
            .clone()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BurstScenario {
    pub name: String,
    pub description: Option<String>,
    pub phases: Vec<BurstPhase>,
    #[serde(default = "default_hold_distribution")]
    pub hold_distribution: Vec<HoldBucket>,
    #[serde(default)]
    pub answer_delay: AnswerDelay,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_server_profile")]
    pub server_profile: String,
    #[serde(default = "default_client_profile")]
    pub client_profile: String,
    #[serde(default = "default_capacity")]
    pub capacity: usize,
    #[serde(default = "default_alice_shards")]
    pub alice_shards: usize,
    #[serde(default)]
    pub acceptance: BurstAcceptance,
}

impl BurstScenario {
    pub fn validate(&self) {
        assert!(
            !self.name.trim().is_empty(),
            "burst scenario name is required"
        );
        assert!(
            !self.phases.is_empty(),
            "burst scenario '{}' must contain at least one phase",
            self.name
        );
        assert!(
            self.capacity > 0,
            "burst scenario '{}' capacity must be greater than 0",
            self.name
        );
        assert!(
            self.alice_shards > 0,
            "burst scenario '{}' aliceShards must be greater than 0",
            self.name
        );
        for phase in &self.phases {
            phase.validate(&self.name);
        }
        assert!(
            !self.hold_distribution.is_empty(),
            "burst scenario '{}' holdDistribution must not be empty",
            self.name
        );
        let mut total_weight = 0u64;
        for bucket in &self.hold_distribution {
            bucket.validate(&self.name);
            total_weight += u64::from(bucket.weight);
        }
        assert!(
            total_weight > 0,
            "burst scenario '{}' holdDistribution total weight must be greater than 0",
            self.name
        );
        self.answer_delay.validate(&self.name);
        self.acceptance.validate(&self.name);
        if self.acceptance.min_recovery_asr.is_some() {
            let recovery = self
                .phases
                .iter()
                .rev()
                .find(|phase| phase.label.to_ascii_lowercase().contains("recovery"))
                .unwrap_or_else(|| {
                    panic!(
                        "burst scenario '{}' acceptance.minRecoveryAsr requires a recovery phase",
                        self.name
                    )
                });
            let recovery_budget = self.acceptance.max_recovery_secs.unwrap_or_else(|| {
                panic!(
                    "burst scenario '{}' acceptance.minRecoveryAsr requires maxRecoverySecs",
                    self.name
                )
            });
            assert!(
                recovery_budget < recovery.duration_secs,
                "burst scenario '{}' maxRecoverySecs must leave a non-empty stable recovery window",
                self.name
            );
        }
    }

    pub fn duration_secs(&self) -> u64 {
        self.phases.iter().map(|phase| phase.duration_secs).sum()
    }

    pub fn total_offered_calls(&self) -> u64 {
        self.phases.iter().map(|phase| phase.expected_calls()).sum()
    }

    /// Bound the exact-lifecycle authority for this finite burst workload.
    ///
    /// Active call admission and retained SIP anti-reuse fences are separate
    /// dimensions. A short-call scenario can retire far more identifiers than
    /// its active-call limit during the 64-second anti-reuse horizon. Covering
    /// the active limit plus every offered call is deliberately conservative,
    /// independent of answer rate, and avoids turning retained fence pressure
    /// into a false live-call overload signal during recovery.
    pub fn retained_lifecycle_capacity(&self) -> usize {
        let offered = usize::try_from(self.total_offered_calls()).unwrap_or(usize::MAX);
        self.capacity.saturating_add(offered)
    }

    pub fn phase_start_secs(&self, phase_index: usize) -> u64 {
        self.phases
            .iter()
            .take(phase_index)
            .map(|phase| phase.duration_secs)
            .sum()
    }

    pub fn hold_duration(&self, call_seq: u64) -> Duration {
        let total_weight = self
            .hold_distribution
            .iter()
            .map(|bucket| u64::from(bucket.weight))
            .sum::<u64>();
        let mut choice =
            deterministic_u64(self.seed ^ call_seq.wrapping_mul(0x9E37_79B9)) % total_weight;
        let bucket = self
            .hold_distribution
            .iter()
            .find(|bucket| {
                if choice < u64::from(bucket.weight) {
                    true
                } else {
                    choice -= u64::from(bucket.weight);
                    false
                }
            })
            .unwrap_or_else(|| {
                self.hold_distribution
                    .last()
                    .expect("validated hold distribution")
            });
        let span = bucket.max_secs - bucket.min_secs + 1;
        let offset = if span <= 1 {
            0
        } else {
            deterministic_u64(self.seed ^ call_seq.wrapping_mul(0xBF58_476D_1CE4_E5B9)) % span
        };
        Duration::from_secs(bucket.min_secs + offset)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BurstPhase {
    pub label: String,
    pub cps: f64,
    pub duration_secs: u64,
}

impl BurstPhase {
    fn validate(&self, scenario: &str) {
        assert!(
            !self.label.trim().is_empty(),
            "burst scenario '{scenario}' phase label is required"
        );
        assert!(
            self.cps.is_finite() && self.cps >= 0.0,
            "burst scenario '{scenario}' phase '{}' cps must be finite and >= 0",
            self.label
        );
        assert!(
            self.duration_secs > 0,
            "burst scenario '{scenario}' phase '{}' durationSecs must be greater than 0",
            self.label
        );
    }

    pub fn expected_calls(&self) -> u64 {
        (self.cps * self.duration_secs as f64).round() as u64
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldBucket {
    pub label: String,
    pub weight: u32,
    pub min_secs: u64,
    pub max_secs: u64,
}

impl HoldBucket {
    fn validate(&self, scenario: &str) {
        assert!(
            !self.label.trim().is_empty(),
            "burst scenario '{scenario}' hold bucket label is required"
        );
        assert!(
            self.weight > 0,
            "burst scenario '{scenario}' hold bucket '{}' weight must be greater than 0",
            self.label
        );
        assert!(
            self.min_secs > 0 && self.max_secs >= self.min_secs,
            "burst scenario '{scenario}' hold bucket '{}' minSecs/maxSecs are invalid",
            self.label
        );
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerDelay {
    #[serde(default)]
    pub min_millis: u64,
    #[serde(default)]
    pub max_millis: u64,
}

impl AnswerDelay {
    fn validate(&self, scenario: &str) {
        assert!(
            self.max_millis >= self.min_millis,
            "burst scenario '{scenario}' answerDelay maxMillis must be >= minMillis"
        );
    }

    pub fn duration_for(&self, call_seq: u64, seed: u64) -> Duration {
        let span = self.max_millis - self.min_millis + 1;
        let offset = if span <= 1 {
            0
        } else {
            deterministic_u64(seed ^ call_seq.wrapping_mul(0x94D0_49BB_1331_11EB)) % span
        };
        Duration::from_millis(self.min_millis + offset)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BurstAcceptance {
    #[serde(default = "default_min_asr")]
    pub min_asr: f64,
    #[serde(default)]
    pub allow_overload_rejections: bool,
    #[serde(default)]
    pub max_media_setup_failed: u64,
    #[serde(default)]
    pub max_teardown_failed: u64,
    #[serde(default)]
    pub max_retained_after_drain: u64,
    #[serde(default)]
    pub max_active_audio_receivers_after_drain: u64,
    #[serde(default)]
    pub max_rss_growth_mb_per_hr: Option<f64>,
    #[serde(default = "default_min_rss_gate_window_secs")]
    pub min_rss_gate_window_secs: f64,
    #[serde(default)]
    pub max_recovery_secs: Option<u64>,
    #[serde(default)]
    pub min_recovery_asr: Option<f64>,
}

impl Default for BurstAcceptance {
    fn default() -> Self {
        Self {
            min_asr: default_min_asr(),
            allow_overload_rejections: false,
            max_media_setup_failed: 0,
            max_teardown_failed: 0,
            max_retained_after_drain: 0,
            max_active_audio_receivers_after_drain: 0,
            max_rss_growth_mb_per_hr: None,
            min_rss_gate_window_secs: default_min_rss_gate_window_secs(),
            max_recovery_secs: None,
            min_recovery_asr: None,
        }
    }
}

impl BurstAcceptance {
    fn validate(&self, scenario: &str) {
        assert!(
            self.min_asr.is_finite() && (0.0..=1.0).contains(&self.min_asr),
            "burst scenario '{scenario}' acceptance.minAsr must be between 0 and 1"
        );
        if let Some(limit) = self.max_rss_growth_mb_per_hr {
            assert!(
                limit.is_finite() && limit > 0.0,
                "burst scenario '{scenario}' acceptance.maxRssGrowthMbPerHr must be > 0"
            );
        }
        assert!(
            self.min_rss_gate_window_secs.is_finite() && self.min_rss_gate_window_secs >= 0.0,
            "burst scenario '{scenario}' acceptance.minRssGateWindowSecs must be >= 0"
        );
        if let Some(min_recovery_asr) = self.min_recovery_asr {
            assert!(
                min_recovery_asr.is_finite() && (0.0..=1.0).contains(&min_recovery_asr),
                "burst scenario '{scenario}' acceptance.minRecoveryAsr must be between 0 and 1"
            );
            assert!(
                self.allow_overload_rejections,
                "burst scenario '{scenario}' acceptance.minRecoveryAsr requires allowOverloadRejections"
            );
            assert!(
                self.max_recovery_secs.is_some(),
                "burst scenario '{scenario}' acceptance.minRecoveryAsr requires maxRecoverySecs"
            );
        }
    }
}

fn default_hold_distribution() -> Vec<HoldBucket> {
    vec![
        HoldBucket {
            label: "short".to_string(),
            weight: 40,
            min_secs: 10,
            max_secs: 30,
        },
        HoldBucket {
            label: "medium".to_string(),
            weight: 40,
            min_secs: 31,
            max_secs: 180,
        },
        HoldBucket {
            label: "long".to_string(),
            weight: 20,
            min_secs: 181,
            max_secs: 360,
        },
    ]
}

fn default_seed() -> u64 {
    0x5256_4f49_505f_4255
}

fn default_server_profile() -> String {
    "pbx-media-server".to_string()
}

fn default_client_profile() -> String {
    "endpoint".to_string()
}

fn default_capacity() -> usize {
    1_000
}

fn default_alice_shards() -> usize {
    4
}

fn default_min_asr() -> f64 {
    0.999
}

fn default_min_rss_gate_window_secs() -> f64 {
    120.0
}

fn deterministic_u64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod port_layout_tests {
    use super::*;

    #[test]
    fn canonical_same_host_layout_covers_high_density_without_sip_overlap() {
        validate_same_host_burst_port_layout(26_060, 26_062, 16, 12_500)
            .expect("canonical burst layout");
    }

    #[test]
    fn same_host_layout_rejects_runtime_sip_port_inside_media_pool() {
        let error = validate_same_host_burst_port_layout(BURST_BOB_MEDIA_START, 26_062, 16, 12_500)
            .expect_err("overlap must fail closed");
        assert!(error.contains("overlaps"));
    }

    #[test]
    fn same_host_layout_rejects_insufficient_media_capacity() {
        let error = validate_same_host_burst_port_layout(26_060, 26_062, 16, 30_000)
            .expect_err("capacity must fail closed");
        assert!(error.contains("below the required"));
    }
}
