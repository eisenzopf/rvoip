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
    }

    pub fn duration_secs(&self) -> u64 {
        self.phases.iter().map(|phase| phase.duration_secs).sum()
    }

    pub fn total_offered_calls(&self) -> u64 {
        self.phases.iter().map(|phase| phase.expected_calls()).sum()
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
