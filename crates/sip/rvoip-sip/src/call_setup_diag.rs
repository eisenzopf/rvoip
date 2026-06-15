//! Feature-gated call setup timing diagnostics for perf investigations.

use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

const ENV: &str = "RVOIP_PERF_CALL_SETUP_DIAGNOSTICS";
const MAX_RECORDS: usize = 20_000;

static ENABLED: OnceLock<bool> = OnceLock::new();
static RECORDS: LazyLock<Mutex<Vec<StageRecord>>> = LazyLock::new(|| Mutex::new(Vec::new()));

#[derive(Clone, Serialize)]
struct StageRecord {
    call_id: String,
    stage: &'static str,
    elapsed_ns: u64,
}

pub(crate) fn record_stage(call_id: impl ToString, stage: &'static str, elapsed: Duration) {
    if !enabled() {
        return;
    }
    let Ok(mut records) = RECORDS.lock() else {
        return;
    };
    if records.len() >= MAX_RECORDS {
        return;
    }
    records.push(StageRecord {
        call_id: call_id.to_string(),
        stage,
        elapsed_ns: elapsed.as_nanos().try_into().unwrap_or(u64::MAX),
    });
}

/// Snapshot call setup stage timings for perf report JSON.
pub fn snapshot() -> Value {
    let records = RECORDS
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    json!({
        "enabled": enabled(),
        "env": ENV,
        "max_records": MAX_RECORDS,
        "records": records,
    })
}

fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var(ENV)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
            .unwrap_or(false)
    })
}
