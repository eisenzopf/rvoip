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

    // Report accepted-call count every 5 s so the operator can watch
    // throughput while sipp drives load.
    let reporter = {
        let accepted = Arc::clone(&accepted);
        tokio::spawn(async move {
            let mut last = 0u64;
            let mut last_t = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let now = accepted.load(Ordering::Relaxed);
                let now_t = Instant::now();
                let dt = now_t.duration_since(last_t).as_secs_f64();
                let cps = if dt > 0.0 {
                    (now - last) as f64 / dt
                } else {
                    0.0
                };
                println!(
                    "[perf_listener] accepted_total={now}  delta={}  cps_5s={cps:.1}",
                    now - last
                );
                last = now;
                last_t = now_t;
            }
        })
    };

    let _ = tokio::signal::ctrl_c().await;
    println!(
        "[perf_listener] stopping. final accepted_total={}",
        accepted.load(Ordering::Relaxed)
    );
    reporter.abort();
    let _ = peer.shutdown_handle();
}
