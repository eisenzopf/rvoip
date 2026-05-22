//! Standalone `CallbackPeer<AutoAccept>` that listens on a fixed UDP
//! SIP port. Used for sipp-driven benchmarks where we want
//! apples-to-apples comparisons against Asterisk / FreeSWITCH on the
//! same loopback (or VM-routed) sipp setup.
//!
//! Run via:
//! ```text
//! cargo run -p rvoip-sip --release --example perf_listener -- 5060
//! ```
//!
//! The process runs forever; SIGINT to terminate.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_sip::adapters::media_adapter::cleanup_session_diag;
use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::Config;

#[derive(Clone)]
struct CountingAccept {
    accepted: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl CallHandler for CountingAccept {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        let _ = call.accept().await;
        self.accepted.fetch_add(1, Ordering::Relaxed);
        CallHandlerDecision::Accept
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    // NEXT_STEPS B.1 diag — bring tracing online so the
    // `Action::CleanupMedia` info! log is visible alongside the
    // `accepted_total` / `cleaned_total` poll output. Default to
    // `rvoip_sip::state_machine::actions=info` so we don't drown in
    // dialog/transaction chatter at 100 CPS; full debug is one
    // `RUST_LOG=debug` away.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,rvoip_sip::state_machine::actions=info".into()),
        )
        .with_target(false)
        .try_init();

    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5060);

    let accepted = Arc::new(AtomicU64::new(0));
    let handler = CountingAccept {
        accepted: Arc::clone(&accepted),
    };
    let peer = CallbackPeer::new(handler, Config::local("rvoip-perf-listener", port))
        .await
        .expect("CallbackPeer::new");

    println!("rvoip-sip perf_listener: listening on 0.0.0.0:{port} (UDP SIP)");
    println!("Accepts every inbound INVITE via auto-accept handler. SIGINT to stop.");

    // Hold the shutdown signal before `run()` consumes the peer.
    let shutdown = peer.shutdown_handle();

    // Report accepted-call count every 5 s so the operator can watch
    // throughput while sipp drives load.
    // NEXT_STEPS B.1 — log `cleaned_total` (poll of the process-global
    // counter in `media_adapter::cleanup_session_diag`) alongside
    // `accepted_total`. The gap between the two at wedge time is the
    // load-bearing diagnostic: if cleanup keeps pace with accept the
    // wedge is not a leak; if cleanup is far behind we have a missing
    // state-machine row or an allocator stall.
    let reporter = {
        let accepted = Arc::clone(&accepted);
        tokio::spawn(async move {
            let mut last_acc = 0u64;
            let mut last_cln = 0u64;
            let mut last_t = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let now_acc = accepted.load(Ordering::Relaxed);
                let now_cln = cleanup_session_diag::cleaned_total();
                let now_t = Instant::now();
                let dt = now_t.duration_since(last_t).as_secs_f64();
                let cps = if dt > 0.0 {
                    (now_acc - last_acc) as f64 / dt
                } else {
                    0.0
                };
                let cln_rate = if dt > 0.0 {
                    (now_cln - last_cln) as f64 / dt
                } else {
                    0.0
                };
                println!(
                    "[perf_listener] accepted_total={now_acc}  delta={}  cps_5s={cps:.1}  \
                     cleaned_total={now_cln}  cleaned_delta={}  cleaned_rate={cln_rate:.1}  \
                     in_flight={}",
                    now_acc - last_acc,
                    now_cln - last_cln,
                    now_acc.saturating_sub(now_cln),
                );
                last_acc = now_acc;
                last_cln = now_cln;
                last_t = now_t;
            }
        })
    };

    // Drive the peer's event loop on a task so we can listen for
    // SIGINT on the main task and trigger graceful shutdown.
    let run_task = tokio::spawn(async move {
        let _ = peer.run().await;
    });

    let _ = tokio::signal::ctrl_c().await;
    println!(
        "[perf_listener] stopping. final accepted_total={} cleaned_total={}",
        accepted.load(Ordering::Relaxed),
        cleanup_session_diag::cleaned_total(),
    );
    shutdown.shutdown();
    reporter.abort();
    let _ = tokio::time::timeout(Duration::from_secs(3), run_task).await;
}
