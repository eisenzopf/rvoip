//! Standalone `CallbackPeer<AutoAccept>` that listens on a fixed UDP
//! SIP port. Used for sipp-driven benchmarks where we want
//! apples-to-apples comparisons against Asterisk / FreeSWITCH on the
//! same loopback (or VM-routed) sipp setup.
//!
//! Run via:
//! ```text
//! cargo run -p rvoip-sip --release --example perf_listener -- 5060
//! cargo run -p rvoip-sip --release --example perf_listener -- 35060 192.168.5.2
//! cargo run -p rvoip-sip --release --example perf_listener -- 35060 192.168.5.2 --diagnostics
//! cargo run -p rvoip-sip --release --example perf_listener -- 35060 192.168.5.2 --diagnostics --diagnostic-events
//! ```
//!
//! The optional second argument sets the SIP Contact/Via fallback address and
//! SDP `o=`/`c=` public media address. Use a container-visible host IP for
//! Docker-sidecar SIPp runs so the 200 OK does not advertise `127.0.0.1` back
//! to the container.
//!
//! `--diagnostics` enables summary counters for SIP UDP, duplicate recovery,
//! media setup, and cleanup. `--diagnostic-events` additionally enables
//! per-operation cleanup event logs. `--wire-diagnostics` enables noisy
//! SRTP/RTP/SDP diagnostic logs.
//!
//! The process runs forever; SIGINT to terminate.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_media_core::diagnostics as media_setup_diag;
use rvoip_sip::adapters::media_adapter::cleanup_session_diag;
use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::Config;
use rvoip_sip::cleanup_diag;
use rvoip_sip_dialog::diagnostics as sip_retrans_diag;
use rvoip_sip_transport::diagnostics as sip_udp_diag;

const HIGH_CPS_CAPACITY: usize = 20_000;
const HIGH_CPS_RTP_PORT_START: u16 = 16_384;
const HIGH_CPS_RTP_PORT_CAPACITY: usize = 49_152;

#[derive(Clone, Copy, Default)]
struct PerfDiagnostics {
    summary: bool,
    cleanup_events: bool,
    wire: bool,
}

#[derive(Clone)]
struct CountingAccept {
    accepted: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl CallHandler for CountingAccept {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallHandlerDecision {
        self.accepted.fetch_add(1, Ordering::Relaxed);
        CallHandlerDecision::Accept
    }
}

fn resolve_advertised_addr(raw: &str, default_port: u16) -> SocketAddr {
    if let Ok(addr) = raw.parse::<SocketAddr>() {
        return addr;
    }
    if let Ok(ip) = raw.parse::<IpAddr>() {
        return SocketAddr::new(ip, default_port);
    }
    if let Ok(mut addrs) = raw.to_socket_addrs() {
        if let Some(addr) = addrs.next() {
            return addr;
        }
    }

    let candidate = format!("{raw}:{default_port}");
    candidate
        .to_socket_addrs()
        .unwrap_or_else(|e| panic!("failed to resolve advertised address '{raw}': {e}"))
        .next()
        .unwrap_or_else(|| panic!("advertised address '{raw}' resolved to no socket addresses"))
}

fn sip_uri_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5060);
    let mut advertised_arg = None;
    let mut diagnostics = PerfDiagnostics::default();
    for arg in args {
        match arg.as_str() {
            "--diagnostics" => {
                diagnostics.summary = true;
            }
            "--diagnostic-events" => {
                diagnostics.summary = true;
                diagnostics.cleanup_events = true;
            }
            "--wire-diagnostics" => {
                diagnostics.wire = true;
            }
            _ if advertised_arg.is_none() => {
                advertised_arg = Some(arg);
            }
            _ => {
                panic!("unexpected perf_listener argument '{arg}'");
            }
        }
    }

    // NEXT_STEPS B.1 diag — bring tracing online so the
    // `Action::CleanupMedia` info! log is visible alongside the
    // `accepted_total` / `cleaned_total` poll output. Default to
    // `rvoip_sip::state_machine::actions=info` so we don't drown in
    // dialog/transaction chatter at 100 CPS; full debug is one
    // `RUST_LOG=debug` away.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                if diagnostics.cleanup_events {
                    "warn,rvoip_sip::state_machine::actions=info,rvoip_sip::cleanup_diag=info"
                        .into()
                } else {
                    "warn,rvoip_sip::state_machine::actions=info".into()
                }
            }),
        )
        .with_target(false)
        .try_init();

    let config = if let Some(raw) = advertised_arg {
        let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        let advertised = resolve_advertised_addr(&raw, port);
        let mut config = Config::lan_pbx("rvoip-perf-listener", bind, advertised);
        config.contact_uri = Some(format!(
            "sip:rvoip-perf-listener@{}:{}",
            sip_uri_host(advertised.ip()),
            advertised.port()
        ));
        println!(
            "rvoip-sip perf_listener: advertising SIP/SDP as {} (from '{}')",
            advertised, raw
        );
        config
    } else {
        Config::local("rvoip-perf-listener", port)
    };
    let config = apply_perf_config(config, diagnostics);
    print_effective_config(&config);

    let accepted = Arc::new(AtomicU64::new(0));
    let handler = CountingAccept {
        accepted: Arc::clone(&accepted),
    };

    let peer = CallbackPeer::new(handler, config)
        .await
        .expect("CallbackPeer::new");

    println!("rvoip-sip perf_listener: listening on 0.0.0.0:{port} (UDP SIP)");
    println!("Accepts every inbound INVITE via auto-accept handler. SIGINT to stop.");

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
                let now_acc = observed_answered_total(&accepted);
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
                if cleanup_diag::enabled() {
                    let snapshot = cleanup_diag::snapshot();
                    println!("{}", cleanup_diag::format_summary(&snapshot));
                }
                print_sip_udp_diagnostics();
                last_acc = now_acc;
                last_cln = now_cln;
                last_t = now_t;
            }
        })
    };

    let run_task = tokio::spawn(async move {
        let _ = peer.run().await;
    });

    let _ = tokio::signal::ctrl_c().await;
    println!(
        "[perf_listener] stopping. final accepted_total={} cleaned_total={}",
        observed_answered_total(&accepted),
        cleanup_session_diag::cleaned_total(),
    );
    if cleanup_diag::enabled() {
        let snapshot = cleanup_diag::snapshot();
        println!("{}", cleanup_diag::format_summary(&snapshot));
    }
    print_sip_udp_diagnostics();
    shutdown.shutdown();
    reporter.abort();
    let _ = tokio::time::timeout(Duration::from_secs(3), run_task).await;
}

fn observed_answered_total(accepted: &AtomicU64) -> u64 {
    let callback_count = accepted.load(Ordering::Relaxed);
    if callback_count > 0 {
        return callback_count;
    }

    if sip_retrans_diag::enabled() {
        return sip_retrans_diag::snapshot().invite_2xx_cache_insert;
    }

    callback_count
}

fn print_sip_udp_diagnostics() {
    if media_setup_diag::enabled() {
        let snapshot = media_setup_diag::snapshot();
        println!("{}", media_setup_diag::format_summary(&snapshot));
    }
    if sip_udp_diag::enabled() {
        let snapshot = sip_udp_diag::snapshot();
        println!("{}", sip_udp_diag::format_summary(&snapshot));
    }
    if sip_retrans_diag::enabled() {
        let snapshot = sip_retrans_diag::snapshot();
        println!("{}", sip_retrans_diag::format_summary(&snapshot));
    }
}

fn apply_perf_config(config: Config, diagnostics: PerfDiagnostics) -> Config {
    config
        .with_high_cps_udp_auto_answer(HIGH_CPS_CAPACITY)
        .with_media_port_capacity(HIGH_CPS_RTP_PORT_START, HIGH_CPS_RTP_PORT_CAPACITY)
        .with_media_session_capacity(HIGH_CPS_CAPACITY)
        .with_sip_udp_diagnostics(diagnostics.summary)
        .with_media_setup_diagnostics(diagnostics.summary)
        .with_cleanup_diagnostics(diagnostics.summary)
        .with_cleanup_diagnostic_events(diagnostics.cleanup_events)
        .with_srtp_diagnostics(diagnostics.wire)
        .with_rtp_diagnostics(diagnostics.wire)
        .with_media_sdp_diagnostics(diagnostics.wire)
}

fn print_effective_config(config: &Config) {
    println!(
        "rvoip-sip perf_listener: high_cps_config capacity={} auto_180_ringing={} \
         auto_100_trying={} \
         fast_auto_accept_incoming_calls={} \
         sip_udp_parse_workers={:?} sip_udp_parse_queue_capacity={:?}",
        HIGH_CPS_CAPACITY,
        config.auto_180_ringing,
        config.auto_100_trying,
        config.fast_auto_accept_incoming_calls,
        config.sip_udp_parse_workers,
        config.sip_udp_parse_queue_capacity,
    );
    println!(
        "rvoip-sip perf_listener: channels incoming={} state={} sip_transport={} \
         transaction={} global={} session_dispatch={}",
        config.incoming_call_channel_capacity,
        config.state_event_channel_capacity,
        config.sip_transport_channel_capacity,
        config.transaction_event_channel_capacity,
        config.global_event_channel_capacity,
        config.session_event_dispatcher_channel_capacity,
    );
    println!(
        "rvoip-sip perf_listener: media range {}-{} requested_capacity={:?} \
         media_session_capacity={:?} server_capacity={:?} mode={:?}",
        config.media_port_start,
        config.media_port_end,
        config.media_port_capacity,
        config.media_session_capacity,
        config.server_call_capacity,
        config.media_mode,
    );
    println!(
        "rvoip-sip perf_listener: diagnostics sip_udp={} media_setup={} cleanup={} \
         cleanup_events={} srtp={} rtp={} media_sdp={}",
        config.sip_udp_diagnostics,
        config.media_setup_diagnostics,
        config.cleanup_diagnostics,
        config.cleanup_diagnostic_events,
        config.srtp_diagnostics,
        config.rtp_diagnostics,
        config.media_sdp_diagnostics
    );
}
