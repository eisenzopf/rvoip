//! The DUT process: rvoip configuration, run files and the drain.

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rvoip_sip::api::callback_peer::CallbackPeer;
use rvoip_sip::api::unified::{Config, SipTlsMode};
use rvoip_sip::{
    MediaExecutorConfig, MediaLanes, MediaRuntimeConfig, PerformanceConfig, UnifiedCoordinator,
};
use rvoip_sip_dialog::transaction::NewCallAdmission;
use serde_json::json;

use crate::admission;
use crate::calls::{BenchHandler, CallTable, TOMBSTONE_TTL};
use crate::config::{DutConfig, LoadedConfig, Media, Transport};
use crate::drain::{self, DrainOutcome};
use crate::metrics_http;
use crate::process;
use crate::stats::{Counters, External, Snapshot};

/// Exit code: the drain finished with every gauge at zero.
pub const EXIT_OK: i32 = 0;
/// Exit code: the process failed (configuration, bind, I/O).
pub const EXIT_ERROR: i32 = 1;
/// Exit code: the configuration was rejected.
pub const EXIT_CONFIG: i32 = 2;
/// Exit code: the drain timed out with calls or resources left.
pub const EXIT_DRAIN_TIMEOUT: i32 = 3;

/// SDP port advertised for signaling-only calls (the discard port, RFC 863
/// convention used by rvoip's signaling-only recipe).
const SIGNALING_ONLY_SDP_PORT: u16 = 9;
/// How long the SIP stack gets to stop after the drain.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
/// SIP user part of the DUT's Contact.
const SIP_USER: &str = "rvoip-bench-uas";

/// Package version, reported in `ready.json`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Git commit the binary was built from, reported in `ready.json`.
pub const GIT_SHA: &str = env!("RVOIP_BENCH_UAS_GIT_SHA");

/// Seconds since the Unix epoch.
pub fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

fn host(ip: std::net::IpAddr) -> String {
    match ip {
        std::net::IpAddr::V4(ip) => ip.to_string(),
        std::net::IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

/// The rvoip configuration for `dut` with `media_lanes` dedicated media lanes.
///
/// It starts from rvoip's bundled auto-answer server recipes
/// (`pbx-media-server` with media, `signaling-only-server-high-performance`
/// without): fast auto-accept (the 200 OK is sent on the INVITE's own path,
/// before any application callback), no automatic 100/180, sharded UDP
/// parsing and transaction/dialog/session dispatch, large socket buffers.
/// On top of the recipe it removes the soft-limit pacing (which would delay
/// INVITEs and distort setup delay), turns `max_concurrent_calls` into a
/// stateless transaction-layer 503 with the lock-free admission check, drops
/// per-session transition history, and for media selects executor v2 on
/// dedicated lanes so RTP is received, decrypted, echoed and sent on the
/// socket's lane.
pub fn rvoip_config(dut: &DutConfig, media_lanes: usize) -> Result<Config, String> {
    let advertised = SocketAddr::new(dut.advertise_ip, dut.bind.port());
    let contact_host = format!("{}:{}", host(dut.advertise_ip), dut.bind.port());
    let mut config = Config::lan_pbx(SIP_USER, dut.bind, advertised);

    let performance = match dut.media {
        Media::None => {
            PerformanceConfig::signaling_only_server_high_performance(dut.queue_capacity())
                .with_signaling_only_rtp_port(SIGNALING_ONLY_SDP_PORT)
        }
        Media::Rtp | Media::Srtp => PerformanceConfig::pbx_media_server(dut.queue_capacity()),
    };
    config = config
        .try_with_performance_config(performance)
        .map_err(|error| format!("performance recipe: {error}"))?;

    // Capacity and admission.
    let retry_after = admission::drain_retry_after(dut.admission);
    config.server_call_capacity = Some(dut.max_concurrent_calls);
    config.server_retained_lifecycle_capacity = Some(dut.retained_lifecycle_capacity());
    config.server_call_admission_limit = Some(dut.max_concurrent_calls);
    config.server_call_admission_soft_limit = None;
    config.server_call_admission_pacing_delay_ms = None;
    config.server_call_admission_early_reject = true;
    config.server_call_admission_lock_free = true;
    config.server_overload_retry_after_secs = Some(retry_after);
    config.sip_new_call_admission = admission::gate_config(dut.admission);
    config.inbound_session_history = false;

    // Media.
    if dut.media.has_rtp() {
        let [start, end] = dut.rtp_port_range;
        config = config.with_media_ports(start, end);
        config.media_session_capacity = Some(dut.max_concurrent_calls.min(dut.rtp_port_count()));
        config.offer_srtp = dut.media == Media::Srtp;
        config.srtp_required = dut.media == Media::Srtp;
        // rvoip answers `a=rtcp-mux` whenever the offer carries it; without
        // mux it sends RTCP from RTP port + 1 to a peer that declines it.
        config.offer_rtcp_mux = dut.rtcp_mux;
        config.rtcp_non_mux = !dut.rtcp_mux;
        let lanes = if dut.tuning.pin_media_lanes {
            let first = dut.tuning.media_lane_first_cpu;
            MediaLanes::pinned(first..first + media_lanes.max(1))
        } else {
            MediaLanes::dedicated(media_lanes.max(1))
        };
        config.media_runtime = Some(MediaRuntimeConfig::new(lanes));
        config.media_executor = Some(MediaExecutorConfig::v2());
    } else {
        // No media, but the answer keeps the offer's profile (RFC 3264 §6):
        // an SDES offer (the TLS rows offer RTP/SAVP) is answered RTP/SAVP
        // with an answer key on the signaling-only port; RTP/AVP stays
        // RTP/AVP. Without this rvoip declines an SRTP m-line (port 0).
        config.offer_srtp = true;
        config.srtp_required = false;
    }

    // Addressing.
    config.local_uri = format!("sip:{SIP_USER}@{contact_host}");
    match dut.transport {
        Transport::Udp => {
            config.contact_uri = Some(format!("sip:{SIP_USER}@{contact_host}"));
        }
        Transport::Tls => {
            // rvoip always binds UDP and TCP on `bind_addr`; TLS must have
            // the TCP port to itself, so those take an ephemeral port.
            config.bind_addr = SocketAddr::new(dut.bind.ip(), 0);
            config.sip_port = 0;
            config.sip_tls_mode = SipTlsMode::ServerOnly;
            config.tls_bind_addr = Some(dut.bind);
            config.tls_advertised_addr = Some(advertised);
            config.tls_cert_path = dut.tls_cert.clone();
            config.tls_key_path = dut.tls_key.clone();
            config.contact_uri = Some(format!("sip:{SIP_USER}@{contact_host};transport=tls"));
        }
    }
    config
        .validate()
        .map_err(|error| format!("rvoip configuration: {error}"))?;
    Ok(config)
}

/// Everything a snapshot reads.
#[derive(Clone)]
struct Probe {
    counters: Arc<Counters>,
    table: Arc<CallTable>,
    coordinator: Arc<UnifiedCoordinator>,
    gate: Arc<NewCallAdmission>,
    transport: Transport,
    sip_port: u16,
    draining: Arc<AtomicBool>,
}

impl Probe {
    async fn snapshot(&self) -> Snapshot {
        let gate = self.gate.stats();
        let slots = self.coordinator.sip_slot_cleanup_stats();
        let cleanup = rvoip_sip::cleanup_diag::session_cleanup_counters();
        let media_ports = self.coordinator.allocated_media_port_count().await as u64;
        let process = process::process_stats();
        let (tls_connections, udp_flows) = match self.transport {
            Transport::Tls => (process::established_tcp_connections(self.sip_port), 0),
            Transport::Udp => (0, self.table.live_remote_count()),
        };
        let external = External {
            gate_rejected: gate.rejected,
            gate_reject_sends_dropped: gate.reject_sends_dropped,
            capacity_rejections: slots.new_invite_capacity_rejections,
            draining_rejections: slots.draining_invite_rejections,
            media_port_rejections: cleanup.media_port_admission_rejects,
            setup_failure_rejections: cleanup.setup_failure_rejects,
            setup_failure_unanswered: cleanup.setup_failure_unanswered,
            live_dialogs: rvoip_sip_dialog::live_dialog_count() as u64,
            media_ports,
            srtp_unprotect_failures: rvoip_rtp_core::transport::srtp_unprotect_failures_total(),
            tls_connections,
            udp_flows,
            process_cpu_seconds: process.cpu_seconds,
            process_resident_memory_bytes: process.resident_bytes,
            process_threads: process.threads,
        };
        Snapshot::compute(
            &self.counters,
            &external,
            self.draining.load(Ordering::Relaxed),
        )
    }
}

/// Write `value` to `path` atomically (temporary file, then rename).
fn write_json(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut text = serde_json::to_string_pretty(value).expect("JSON value serializes");
    text.push('\n');
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

/// Append snapshots to `stats.jsonl` every `interval` until aborted.
async fn stats_loop(probe: Probe, path: PathBuf, interval: Duration) {
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) => {
            tracing::error!("cannot open {}: {error}", path.display());
            return;
        }
    };
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        probe.table.sweep_tombstones(TOMBSTONE_TTL);
        let line = probe.snapshot().await.json_line(unix_now());
        if let Err(error) = writeln!(file, "{line}").and_then(|()| file.flush()) {
            tracing::error!("cannot append to {}: {error}", path.display());
        }
    }
}

/// Run the DUT until the drain ends; returns the process exit code.
pub async fn run(loaded: LoadedConfig, media_lanes: usize) -> Result<i32, String> {
    let dut = &loaded.config;
    let run_dir = &dut.run_dir;
    std::fs::create_dir_all(run_dir)
        .map_err(|error| format!("cannot create {}: {error}", run_dir.display()))?;
    for stale in ["ready.json", "summary.json"] {
        let _ = std::fs::remove_file(run_dir.join(stale));
    }
    let started_at = unix_now();

    let counters = Arc::new(Counters::default());
    let table = CallTable::new(Arc::clone(&counters), dut.media.has_rtp());
    let config = rvoip_config(dut, media_lanes)?;
    let peer = CallbackPeer::new(BenchHandler::new(Arc::clone(&table)), config)
        .await
        .map_err(|error| format!("cannot start the SIP stack on {}: {error}", dut.bind))?;
    // rvoip logs (and survives) a TLS listener that failed to bind; the DUT
    // must not report ready without its listener.
    if dut.transport == Transport::Tls && process::tcp_listening(dut.bind.port()) == Some(false) {
        return Err(format!(
            "the SIP TLS listener on {} did not start (see the log)",
            dut.bind
        ));
    }
    let coordinator = Arc::clone(peer.coordinator());
    table.attach(&coordinator);
    let gate = coordinator.new_call_admission();
    let shutdown = peer.shutdown_handle();
    let peer_task = tokio::spawn(peer.run());

    let draining = Arc::new(AtomicBool::new(false));
    let probe = Probe {
        counters,
        table,
        coordinator,
        gate: Arc::clone(&gate),
        transport: dut.transport,
        sip_port: dut.bind.port(),
        draining: Arc::clone(&draining),
    };

    let metrics_listen = match dut.metrics_listen {
        Some(listen) => {
            let probe = probe.clone();
            let bound = metrics_http::serve(listen, move || {
                let probe = probe.clone();
                async move { probe.snapshot().await.render_prometheus() }
            })
            .await
            .map_err(|error| format!("cannot bind metrics on {listen}: {error}"))?;
            Some(bound)
        }
        None => None,
    };

    write_json(
        &run_dir.join("ready.json"),
        &json!({
            "pid": std::process::id(),
            "started_at": started_at,
            "transport": dut.transport.as_str(),
            "listen": dut.bind.to_string(),
            "media": dut.media.as_str(),
            "metrics_listen": metrics_listen.map(|address| address.to_string()),
            "version": VERSION,
            "git_sha": GIT_SHA,
            "config_sha256": loaded.sha256,
        }),
    )
    .map_err(|error| format!("cannot write ready.json: {error}"))?;
    tracing::info!(
        listen = %dut.bind,
        transport = dut.transport.as_str(),
        media = dut.media.as_str(),
        media_lanes,
        "rvoip-bench-uas ready"
    );

    let stats_path = run_dir.join("stats.jsonl");
    let stats = tokio::spawn(stats_loop(
        probe.clone(),
        stats_path.clone(),
        Duration::from_millis(dut.stats_interval_ms),
    ));

    let finish = run_dir.join("finish");
    tokio::select! {
        () = drain::wait_for_file(&finish, drain::POLL_INTERVAL) => {
            tracing::info!("finish file found; draining");
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("interrupted; draining");
        }
    }

    // Drain: refuse new calls, wait for the rest to end and be cleaned up.
    draining.store(true, Ordering::Relaxed);
    admission::close_for_drain(&gate, dut.admission);
    let outcome: DrainOutcome = drain::wait_for_true_zero(
        || probe.snapshot(),
        Duration::from_secs(dut.drain_timeout_secs),
        drain::POLL_INTERVAL,
    )
    .await;
    stats.abort();
    let _ = stats.await;
    let ended_at = unix_now();
    // The final stats line is the drained state.
    if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(&stats_path) {
        let _ = writeln!(file, "{}", outcome.last.json_line(ended_at));
    }

    shutdown.shutdown();
    let stopped = matches!(
        tokio::time::timeout(SHUTDOWN_TIMEOUT, peer_task).await,
        Ok(Ok(Ok(())))
    );
    let clean_shutdown = outcome.drained && stopped;
    if !stopped {
        tracing::warn!("the SIP stack did not stop cleanly");
    }

    write_json(
        &run_dir.join("summary.json"),
        &json!({
            "started_at": started_at,
            "ended_at": ended_at,
            "drain_seconds": outcome.seconds,
            "drain_timed_out": !outcome.drained,
            "true_zero": outcome.last.is_true_zero(),
            "clean_shutdown": clean_shutdown,
            "counters": &outcome.last,
            "version": VERSION,
            "git_sha": GIT_SHA,
            "config_sha256": loaded.sha256,
        }),
    )
    .map_err(|error| format!("cannot write summary.json: {error}"))?;
    tracing::info!(
        drained = outcome.drained,
        drain_seconds = outcome.seconds,
        clean_shutdown,
        "rvoip-bench-uas finished"
    );
    Ok(if outcome.drained {
        EXIT_OK
    } else {
        EXIT_DRAIN_TIMEOUT
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dut(media: &str, transport: &str, extra: &str) -> DutConfig {
        let dir = std::env::temp_dir().join(format!("bench-uas-dut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("c.pem");
        let key = dir.join("k.pem");
        std::fs::write(&cert, "c").unwrap();
        std::fs::write(&key, "k").unwrap();
        DutConfig::from_json(
            format!(
                r#"{{"run_dir": "/tmp/r", "transport": "{transport}", "bind": "10.1.2.3:5061",
                    "advertise_ip": "10.1.2.3", "media": "{media}",
                    "tls_cert": "{}", "tls_key": "{}"{extra}}}"#,
                cert.display(),
                key.display()
            )
            .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn signaling_only_udp_uses_the_signaling_recipe_without_pacing() {
        let config = rvoip_config(&dut("none", "udp", ""), 0).unwrap();
        assert!(config.fast_auto_accept_incoming_calls);
        assert!(!config.auto_180_ringing && !config.auto_100_trying);
        assert!(matches!(
            config.media_mode,
            rvoip_sip::MediaMode::SignalingOnly { sdp_rtp_port: 9 }
        ));
        assert_eq!(config.server_call_admission_limit, Some(60_000));
        assert_eq!(config.server_call_admission_soft_limit, None);
        assert_eq!(config.server_call_admission_pacing_delay_ms, None);
        assert!(config.server_call_admission_early_reject);
        assert!(config.server_call_admission_lock_free);
        assert!(!config.inbound_session_history);
        assert_eq!(config.media_executor, None);
        // An SDES offer is answered RTP/SAVP; plain offers stay RTP/AVP.
        assert!(config.offer_srtp && !config.srtp_required);
        assert_eq!(config.sip_tls_mode, SipTlsMode::Disabled);
        assert_eq!(
            config.contact_uri.as_deref(),
            Some("sip:rvoip-bench-uas@10.1.2.3:5061")
        );
        assert_eq!(
            config.sip_new_call_admission.mode,
            rvoip_sip_dialog::transaction::AdmissionMode::Enforce
        );
        assert_eq!(config.sip_new_call_admission.rate_limit, None);
    }

    #[test]
    fn srtp_over_tls_runs_media_on_executor_v2_lanes() {
        let config = rvoip_config(
            &dut(
                "srtp",
                "tls",
                r#", "admission": {"mode": "max-cps", "max_cps": 4200, "retry_after_secs": 2},
                    "rtp_port_range": [20000, 30000]"#,
            ),
            4,
        )
        .unwrap();
        assert!(config.offer_srtp && config.srtp_required);
        assert!(config.offer_rtcp_mux && !config.rtcp_non_mux);
        assert_eq!(
            (config.media_port_start, config.media_port_end),
            (20_000, 30_000)
        );
        assert_eq!(config.media_port_capacity, None);
        assert_eq!(config.media_executor, Some(MediaExecutorConfig::v2()));
        assert_eq!(
            config.media_runtime,
            Some(MediaRuntimeConfig::new(MediaLanes::dedicated(4)))
        );
        assert_eq!(config.sip_tls_mode, SipTlsMode::ServerOnly);
        assert_eq!(config.tls_bind_addr, Some("10.1.2.3:5061".parse().unwrap()));
        assert_eq!(
            config.bind_addr,
            "10.1.2.3:0".parse().unwrap(),
            "UDP/TCP off the TLS port"
        );
        assert_eq!(
            config.contact_uri.as_deref(),
            Some("sip:rvoip-bench-uas@10.1.2.3:5061;transport=tls")
        );
        let limit = config
            .sip_new_call_admission
            .rate_limit
            .expect("max-cps sets a rate limit");
        assert_eq!(limit.max_cps, 4_200.0);
        assert_eq!(limit.burst, 4_200.0, "a one-second burst");
        assert_eq!(limit.retry_after_secs, Some(2));
        assert_eq!(config.server_overload_retry_after_secs, Some(2));
    }

    #[test]
    fn plain_rtp_without_mux_and_pinned_lanes() {
        let config = rvoip_config(
            &dut(
                "rtp",
                "udp",
                r#", "rtcp_mux": false, "tuning": {"pin_media_lanes": true, "media_lane_first_cpu": 2}"#,
            ),
            3,
        )
        .unwrap();
        assert!(!config.offer_srtp && !config.srtp_required);
        assert!(!config.offer_rtcp_mux && config.rtcp_non_mux);
        assert_eq!(
            config.media_runtime,
            Some(MediaRuntimeConfig::new(MediaLanes::pinned([2, 3, 4])))
        );
    }
}
