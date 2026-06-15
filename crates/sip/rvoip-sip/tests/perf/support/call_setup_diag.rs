use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_sip::api::unified::Config;
use serde_json::{json, Value};

pub const CALL_SETUP_DIAGNOSTICS_ENV: &str = "RVOIP_PERF_CALL_SETUP_DIAGNOSTICS";
const CALL_SETUP_DIAGNOSTICS_SLOW_MS_ENV: &str = "RVOIP_PERF_CALL_SETUP_DIAGNOSTICS_SLOW_MS";
const CALL_SETUP_DIAGNOSTICS_MAX_SAMPLES_ENV: &str =
    "RVOIP_PERF_CALL_SETUP_DIAGNOSTICS_MAX_SAMPLES";

#[derive(Clone)]
pub struct CallSetupDiagnostics {
    enabled: bool,
    slow_threshold: Duration,
    max_samples: usize,
    samples: Arc<Mutex<Vec<CallSetupSample>>>,
}

#[derive(Clone)]
struct CallSetupSample {
    phase: &'static str,
    call_id: String,
    invite_send_ns: Option<u64>,
    wait_answer_ns: Option<u64>,
    elapsed_ns: u64,
}

impl CallSetupDiagnostics {
    pub fn from_env() -> Self {
        let enabled = read_bool_env(CALL_SETUP_DIAGNOSTICS_ENV);
        let slow_ms = std::env::var(CALL_SETUP_DIAGNOSTICS_SLOW_MS_ENV)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(50);
        let max_samples = std::env::var(CALL_SETUP_DIAGNOSTICS_MAX_SAMPLES_ENV)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(200);
        Self {
            enabled,
            slow_threshold: Duration::from_millis(slow_ms),
            max_samples,
            samples: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn configure(&self, config: Config) -> Config {
        if !self.enabled {
            return config;
        }
        config
            .with_sip_udp_diagnostics(true)
            .with_sip_transaction_timing_diagnostics(true)
            .with_sip_dialog_timing_diagnostics(true)
    }

    pub fn record_setup(
        &self,
        phase: &'static str,
        call_id: impl ToString,
        invite_send: Duration,
        wait_answer: Duration,
        elapsed: Duration,
    ) {
        self.record(CallSetupSample {
            phase,
            call_id: call_id.to_string(),
            invite_send_ns: Some(duration_ns(invite_send)),
            wait_answer_ns: Some(duration_ns(wait_answer)),
            elapsed_ns: duration_ns(elapsed),
        });
    }

    pub fn record_stage(&self, phase: &'static str, call_id: impl ToString, elapsed: Duration) {
        self.record(CallSetupSample {
            phase,
            call_id: call_id.to_string(),
            invite_send_ns: None,
            wait_answer_ns: None,
            elapsed_ns: duration_ns(elapsed),
        });
    }

    pub fn to_json(&self) -> Value {
        let mut samples = self
            .samples
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        samples.sort_by(|left, right| right.elapsed_ns.cmp(&left.elapsed_ns));
        json!({
            "enabled": self.enabled,
            "env": CALL_SETUP_DIAGNOSTICS_ENV,
            "slow_threshold_ms": self.slow_threshold.as_millis(),
            "max_samples": self.max_samples,
            "slow_samples": samples
                .into_iter()
                .map(|sample| {
                    json!({
                        "phase": sample.phase,
                        "call_id": sample.call_id,
                        "invite_send_ns": sample.invite_send_ns,
                        "wait_answer_ns": sample.wait_answer_ns,
                        "elapsed_ns": sample.elapsed_ns,
                    })
                })
                .collect::<Vec<_>>(),
            "rvoip_sip_setup_stages": rvoip_sip_setup_stages(),
            "sip_dialog_timing": crate::support::soak::sip_dialog_timing_diagnostics(),
        })
    }

    fn record(&self, sample: CallSetupSample) {
        if !self.enabled || sample.elapsed_ns < duration_ns(self.slow_threshold) {
            return;
        }
        let Ok(mut samples) = self.samples.lock() else {
            return;
        };
        if samples.len() < self.max_samples {
            samples.push(sample);
            return;
        }
        if let Some((min_index, min_sample)) = samples
            .iter()
            .enumerate()
            .min_by_key(|(_, existing)| existing.elapsed_ns)
        {
            if sample.elapsed_ns > min_sample.elapsed_ns {
                samples[min_index] = sample;
            }
        }
    }
}

fn read_bool_env(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
}

fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().try_into().unwrap_or(u64::MAX)
}

#[cfg(feature = "perf-call-setup-diagnostics")]
fn rvoip_sip_setup_stages() -> Value {
    rvoip_sip::call_setup_diag::snapshot()
}

#[cfg(not(feature = "perf-call-setup-diagnostics"))]
fn rvoip_sip_setup_stages() -> Value {
    json!({
        "enabled": false,
        "feature": "perf-call-setup-diagnostics",
    })
}
