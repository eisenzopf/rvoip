//! SIP_API_DESIGN_2 §10 verification #28 — OPTIONS timeout honors
//! `OptionsBuilder::with_timeout(Duration)`.
//!
//! Aims an OPTIONS at a black-hole address (no listener) with a short
//! per-request timeout. The dialog-core transaction layer must time
//! out within the configured duration and bubble a `SessionError`
//! up to the caller.

use std::time::{Duration, Instant};

use rvoip_sip::api::unified::{Config, UnifiedCoordinator};

const TEST_PORT: u16 = 17090;
// A port nothing is listening on. Loopback IP keeps DNS resolution
// instant; we want the timer to be exercised, not name resolution.
const BLACKHOLE_PORT: u16 = 17091;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn options_with_timeout_returns_within_configured_duration() {
    let _ = tracing_subscriber::fmt::try_init();

    let coord = UnifiedCoordinator::new(Config::local("opt-timeout", TEST_PORT))
        .await
        .expect("coordinator");
    tokio::time::sleep(Duration::from_millis(50)).await;

    let started = Instant::now();
    let result = coord
        .options(format!("sip:nobody@127.0.0.1:{BLACKHOLE_PORT}"))
        .with_timeout(Duration::from_millis(600))
        .send()
        .await;
    let elapsed = started.elapsed();

    // The send must fail with a timeout-flavored error. We don't pin a
    // specific variant — dialog-core wraps the timer in a network /
    // dialog error — but the message must mention "timed out".
    let err = result.expect_err("OPTIONS to a black-hole address must not return Ok");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("timed out") || msg.contains("timeout"),
        "expected timeout in error message; got: {err}"
    );

    // The whole call must complete within ~timeout + small slack
    // budget. Without the per-request timeout, dialog-core defaults
    // to 8 seconds (see `send_options_out_of_dialog_with_options`),
    // which would blow this assertion.
    assert!(
        elapsed < Duration::from_secs(3),
        "with_timeout(600ms) must not stall for 8s default; elapsed = {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(500),
        "must not return earlier than the configured timeout; elapsed = {elapsed:?}"
    );
}
