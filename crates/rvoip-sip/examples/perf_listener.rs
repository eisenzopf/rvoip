//! Standalone `CallbackPeer<AutoAccept>` that listens on a fixed UDP
//! SIP port. Used for sipp-driven benchmarks where we want
//! apples-to-apples comparisons against Asterisk / FreeSWITCH on the
//! same loopback (or VM-routed) sipp setup.
//!
//! Run via:
//! ```text
//! cargo run -p rvoip-sip --release --example perf_listener -- 5060
//! cargo run -p rvoip-sip --release --example perf_listener -- 35060 host.docker.internal
//! ```
//!
//! The optional second argument, or `RVOIP_PERF_ADVERTISED_ADDR`, sets
//! the SIP Contact/Via fallback address and SDP `o=`/`c=` public media
//! address. Use it for Docker-sidecar SIPp runs so the 200 OK does not
//! advertise `127.0.0.1` back to the container.
//!
//! Perf sizing knobs:
//! - `RVOIP_PERF_CHANNEL_CAPACITY` (default 20000)
//! - `RVOIP_PERF_SERVER_CAPACITY` (optional hot-index preallocation)
//! - `RVOIP_PERF_RTP_PORT_CAPACITY` / `RVOIP_PERF_MEDIA_PORT_CAPACITY`
//!   (default follows `RVOIP_PERF_CHANNEL_CAPACITY`)
//! - `RVOIP_PERF_RTP_PORT_START` / `RVOIP_PERF_MEDIA_PORT_START`
//! - `RVOIP_PERF_RTP_PORT_END` / `RVOIP_PERF_MEDIA_PORT_END`
//! - `RVOIP_PERF_SESSION_EVENT_WORKERS`
//! - `RVOIP_PERF_SESSION_EVENT_CHANNEL_CAPACITY`
//! - `RVOIP_PERF_SIP_UDP_RECV_BUFFER` / `RVOIP_PERF_SIP_UDP_SEND_BUFFER`
//! - `RVOIP_PERF_CLEANUP_DIAG=1` (periodic cleanup-stage summary)
//! - `RVOIP_PERF_CLEANUP_DIAG_EVENTS=1` (per-operation cleanup timestamps)
//! - `RVOIP_PERF_NO_MEDIA=1` (SIP+SDP only; skip RTP/media-core allocation)
//!
//! The process runs forever; SIGINT to terminate.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rvoip_sip::adapters::media_adapter::cleanup_session_diag;
use rvoip_sip::api::callback_peer::{CallHandler, CallHandlerDecision, CallbackPeer};
use rvoip_sip::api::incoming::IncomingCall;
use rvoip_sip::api::unified::Config;
use rvoip_sip::cleanup_diag;

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
    // NEXT_STEPS B.1 diag — bring tracing online so the
    // `Action::CleanupMedia` info! log is visible alongside the
    // `accepted_total` / `cleaned_total` poll output. Default to
    // `rvoip_sip::state_machine::actions=info` so we don't drown in
    // dialog/transaction chatter at 100 CPS; full debug is one
    // `RUST_LOG=debug` away.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                if cleanup_diag::enabled() {
                    "warn,rvoip_sip::state_machine::actions=info,rvoip_sip::cleanup_diag=info"
                        .into()
                } else {
                    "warn,rvoip_sip::state_machine::actions=info".into()
                }
            }),
        )
        .with_target(false)
        .try_init();

    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5060);
    let advertised_arg = args
        .next()
        .or_else(|| std::env::var("RVOIP_PERF_ADVERTISED_ADDR").ok());

    let accepted = Arc::new(AtomicU64::new(0));
    let handler = CountingAccept {
        accepted: Arc::clone(&accepted),
    };

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
    let config = apply_perf_config(config);

    let peer = CallbackPeer::new(handler, config)
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
                if cleanup_diag::enabled() {
                    let snapshot = cleanup_diag::snapshot();
                    println!("{}", cleanup_diag::format_summary(&snapshot));
                }
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
    if cleanup_diag::enabled() {
        let snapshot = cleanup_diag::snapshot();
        println!("{}", cleanup_diag::format_summary(&snapshot));
    }
    shutdown.shutdown();
    reporter.abort();
    let _ = tokio::time::timeout(Duration::from_secs(3), run_task).await;
}

fn apply_perf_config(config: Config) -> Config {
    let channel_capacity = env_usize("RVOIP_PERF_CHANNEL_CAPACITY", 20_000).max(1);
    let mut config = config.with_channel_capacity(channel_capacity);
    if let Some(server_capacity) = env_usize_opt("RVOIP_PERF_SERVER_CAPACITY") {
        let server_capacity = server_capacity.max(1);
        println!(
            "rvoip-sip perf_listener: server hot-index capacity {} (channel capacity {})",
            server_capacity, channel_capacity
        );
        config = config.with_server_capacity(server_capacity);
    }
    config = apply_perf_media_ports(config, channel_capacity);
    if let Some(workers) = env_usize_opt("RVOIP_PERF_SESSION_EVENT_WORKERS") {
        config = config.with_session_event_dispatcher_workers(workers);
    }
    if let Some(capacity) = env_usize_opt("RVOIP_PERF_SESSION_EVENT_CHANNEL_CAPACITY") {
        config = config.with_session_event_dispatcher_channel_capacity(capacity);
    }
    if let Some(recv_buffer) = env_usize_opt("RVOIP_PERF_SIP_UDP_RECV_BUFFER") {
        config = config.with_sip_udp_recv_buffer_size(recv_buffer);
    }
    if let Some(send_buffer) = env_usize_opt("RVOIP_PERF_SIP_UDP_SEND_BUFFER") {
        config = config.with_sip_udp_send_buffer_size(send_buffer);
    }
    config
}

fn apply_perf_media_ports(config: Config, channel_capacity: usize) -> Config {
    let media_port_start =
        env_u16_any(&["RVOIP_PERF_RTP_PORT_START", "RVOIP_PERF_MEDIA_PORT_START"])
            .unwrap_or(config.media_port_start);
    let explicit_end = env_u16_any(&["RVOIP_PERF_RTP_PORT_END", "RVOIP_PERF_MEDIA_PORT_END"]);
    let media_port_capacity = env_usize_any(&[
        "RVOIP_PERF_RTP_PORT_CAPACITY",
        "RVOIP_PERF_MEDIA_PORT_CAPACITY",
    ])
    .unwrap_or(channel_capacity)
    .max(1);

    let capacity_end = port_range_end_for_capacity(media_port_start, media_port_capacity);
    let media_port_end = explicit_end.unwrap_or(config.media_port_end.max(capacity_end));
    if media_port_start > media_port_end {
        panic!(
            "invalid RTP media port range: start {} is greater than end {}",
            media_port_start, media_port_end
        );
    }

    let available_ports = media_port_end as usize - media_port_start as usize + 1;
    if available_ports < media_port_capacity {
        eprintln!(
            "rvoip-sip perf_listener: RTP media port range {}-{} provides {} ports, below requested capacity {}",
            media_port_start, media_port_end, available_ports, media_port_capacity
        );
    } else {
        println!(
            "rvoip-sip perf_listener: RTP media port range {}-{} ({} ports, requested capacity {})",
            media_port_start, media_port_end, available_ports, media_port_capacity
        );
    }

    config.with_media_ports(media_port_start, media_port_end)
}

fn port_range_end_for_capacity(start: u16, capacity: usize) -> u16 {
    let end = start as usize + capacity.saturating_sub(1);
    end.min(u16::MAX as usize) as u16
}

fn env_usize(name: &str, default: usize) -> usize {
    env_usize_opt(name).unwrap_or(default)
}

fn env_usize_opt(name: &str) -> Option<usize> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

fn env_usize_any(names: &[&str]) -> Option<usize> {
    names.iter().find_map(|name| env_usize_opt(name))
}

fn env_u16_opt(name: &str) -> Option<u16> {
    std::env::var(name).ok().and_then(|s| s.parse().ok())
}

fn env_u16_any(names: &[&str]) -> Option<u16> {
    names.iter().find_map(|name| env_u16_opt(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perf_media_capacity_expands_default_port_range() {
        let start = Config::DEFAULT_MEDIA_PORT_START;
        assert_eq!(
            port_range_end_for_capacity(start, 20_000),
            start.saturating_add(19_999)
        );
    }

    #[test]
    fn perf_media_capacity_caps_at_u16_max() {
        assert_eq!(port_range_end_for_capacity(60_000, 20_000), u16::MAX);
    }
}
