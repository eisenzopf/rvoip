// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

mod api_coordinator;
mod file_coordinator;

use std::sync::Arc;
use std::{net, path::PathBuf};

use clap::Parser;
use tokio_util::sync::CancellationToken;
use url::Url;

use api_coordinator::{ApiCoordinator, ApiCoordinatorConfig};
use file_coordinator::{FileCoordinator, FileCoordinatorLimits};
use moq_relay_ietf::{
    CertificateFingerprintAdmission, Coordinator, DevelopmentAllowAllAdmission,
    ListenerSecurityPolicy, Relay, RelayCapacityLimitSet, RelayCapacityLimits, RelayConfig,
    RemoteManagerLimits, SessionAdmission, SetupTokenAdmission, Web, WebConfig,
};

#[derive(clap::Args, Clone, Debug)]
pub struct RelayCapacityArgs {
    #[arg(long, default_value_t = 20_000)]
    pub max_relay_resources: usize,
    #[arg(long, default_value_t = 4_096)]
    pub max_publish_namespaces: usize,
    #[arg(long, default_value_t = 8_192)]
    pub max_publish_tracks: usize,
    #[arg(long, default_value_t = 10_000)]
    pub max_subscribes: usize,
    #[arg(long, default_value_t = 2_048)]
    pub max_track_statuses: usize,
    #[arg(long, default_value_t = 4_096)]
    pub max_fetches: usize,

    #[arg(long, default_value_t = 2_048)]
    pub max_relay_resources_per_principal: usize,
    #[arg(long, default_value_t = 256)]
    pub max_publish_namespaces_per_principal: usize,
    #[arg(long, default_value_t = 1_024)]
    pub max_publish_tracks_per_principal: usize,
    #[arg(long, default_value_t = 1_024)]
    pub max_subscribes_per_principal: usize,
    #[arg(long, default_value_t = 256)]
    pub max_track_statuses_per_principal: usize,
    #[arg(long, default_value_t = 512)]
    pub max_fetches_per_principal: usize,

    #[arg(long, default_value_t = 8_192)]
    pub max_relay_resources_per_scope: usize,
    #[arg(long, default_value_t = 2_048)]
    pub max_publish_namespaces_per_scope: usize,
    #[arg(long, default_value_t = 4_096)]
    pub max_publish_tracks_per_scope: usize,
    #[arg(long, default_value_t = 4_096)]
    pub max_subscribes_per_scope: usize,
    #[arg(long, default_value_t = 1_024)]
    pub max_track_statuses_per_scope: usize,
    #[arg(long, default_value_t = 2_048)]
    pub max_fetches_per_scope: usize,
}

impl RelayCapacityArgs {
    fn limits(&self) -> RelayCapacityLimits {
        RelayCapacityLimits {
            process: RelayCapacityLimitSet {
                total: self.max_relay_resources,
                publish_namespaces: self.max_publish_namespaces,
                publish_tracks: self.max_publish_tracks,
                subscribes: self.max_subscribes,
                track_statuses: self.max_track_statuses,
                fetches: self.max_fetches,
            },
            per_principal: RelayCapacityLimitSet {
                total: self.max_relay_resources_per_principal,
                publish_namespaces: self.max_publish_namespaces_per_principal,
                publish_tracks: self.max_publish_tracks_per_principal,
                subscribes: self.max_subscribes_per_principal,
                track_statuses: self.max_track_statuses_per_principal,
                fetches: self.max_fetches_per_principal,
            },
            per_scope: RelayCapacityLimitSet {
                total: self.max_relay_resources_per_scope,
                publish_namespaces: self.max_publish_namespaces_per_scope,
                publish_tracks: self.max_publish_tracks_per_scope,
                subscribes: self.max_subscribes_per_scope,
                track_statuses: self.max_track_statuses_per_scope,
                fetches: self.max_fetches_per_scope,
            },
        }
    }
}

#[derive(Parser, Clone)]
pub struct Cli {
    /// Listen on this address
    #[arg(long, default_value = "[::]:443")]
    pub bind: net::SocketAddr,

    /// The TLS configuration.
    #[command(flatten)]
    pub tls: moq_native_ietf::tls::Args,

    /// Directory to write qlog files (one per connection)
    #[arg(long)]
    pub qlog_dir: Option<PathBuf>,

    /// Directory to write mlog files (one per connection)
    #[arg(long)]
    pub mlog_dir: Option<PathBuf>,

    /// Forward all PUBLISH_NAMESPACE messages to the provided server for auth/routing.
    /// If not provided, the relay accepts every unique namespace publish.
    #[arg(long)]
    pub announce: Option<Url>,

    /// The URL of the moq-api server in order to run a cluster.
    /// Must be used in conjunction with --node to advertise the origin
    #[arg(long)]
    pub api: Option<Url>,

    /// The hostname that we advertise to other origins.
    /// The provided certificate must be valid for this address.
    #[arg(long)]
    pub node: Option<Url>,

    /// Enable insecure local-only development mode.
    ///
    /// This weakens production security posture and also serves certificate
    /// fingerprints/diagnostics over HTTPS. Never enable it on a public relay.
    #[arg(long = "insecure-development", visible_alias = "dev")]
    pub dev: bool,

    /// Bind an mTLS leaf fingerprint to one publisher scope as SHA256=/path.
    /// Repeat for additional principals or scopes.
    #[arg(long = "admit-publisher")]
    pub admitted_publishers: Vec<String>,

    /// Maximum active sessions for each admitted publisher fingerprint.
    #[arg(long, default_value_t = 100)]
    pub publisher_session_cap: usize,

    /// Bind an mTLS relay client fingerprint to one subscribe-only upstream
    /// scope as SHA256=/path. Repeat for additional relay principals/scopes.
    #[arg(long = "admit-relay-subscriber")]
    pub admitted_relay_subscribers: Vec<String>,

    /// Maximum active sessions for each admitted relay-subscriber fingerprint.
    #[arg(long, default_value_t = 100)]
    pub relay_subscriber_session_cap: usize,

    /// SHA-256 digest of a SETUP bearer token admitted for subscribe-only listeners.
    #[arg(long = "admit-subscribe-token-sha256")]
    pub admitted_subscribe_token_sha256: Vec<String>,

    /// Security role for this relay process's inbound listener.
    #[arg(long, value_enum)]
    pub listener_security: Option<ListenerSecurityPolicy>,

    #[arg(long, default_value_t = 5_000)]
    pub setup_timeout_ms: u64,

    #[arg(long, default_value_t = 2_000)]
    pub admission_timeout_ms: u64,

    #[arg(long, default_value_t = 1_000)]
    pub pre_admission_cleanup_timeout_ms: u64,

    /// Maximum wait for replay tombstoning and distributed admission-lease
    /// release after an admitted session stops.
    #[arg(
        long,
        default_value_t = 5_000,
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    pub session_close_timeout_ms: u64,

    #[arg(long, default_value_t = 128)]
    pub max_pending_admissions: usize,

    /// Maximum concurrently admitted sessions per listener.
    #[arg(long, default_value_t = 10_000)]
    pub max_active_sessions: usize,

    #[arg(long, default_value_t = 30_000)]
    pub token_revalidation_interval_ms: u64,

    /// Hierarchical limits for long-lived relay request and media state.
    #[command(flatten)]
    pub relay_capacity: RelayCapacityArgs,

    /// Maximum retained authenticated upstream relay connections.
    #[arg(long, default_value_t = 128)]
    pub max_upstream_connections: usize,

    /// Maximum retained tracks across all upstream relay connections.
    #[arg(long, default_value_t = 4_096)]
    pub max_upstream_tracks: usize,

    #[arg(long, default_value_t = 30_000)]
    pub upstream_track_idle_timeout_ms: u64,

    #[arg(long, default_value_t = 60_000)]
    pub upstream_connection_idle_timeout_ms: u64,

    /// Maximum cached tracks in one published namespace.
    #[arg(long, default_value_t = 4_096)]
    pub max_tracks_per_namespace: usize,

    /// Maximum pending track requests in one published namespace.
    #[arg(long, default_value_t = 1_024)]
    pub max_pending_track_requests_per_namespace: usize,

    /// Maximum retained FETCH payload bytes in one MoQT session.
    #[arg(long, default_value_t = 64 * 1024 * 1024)]
    pub max_retained_bytes_per_session: usize,

    /// Maximum retained FETCH payload bytes across this relay process.
    #[arg(long, default_value_t = 512 * 1024 * 1024)]
    pub max_retained_bytes_process: usize,

    /// Serve qlog files over HTTPS at /qlog/:cid
    /// Requires --dev to enable the web server. Only serves files by exact CID - no index.
    #[arg(long)]
    pub qlog_serve: bool,

    /// Serve mlog files over HTTPS at /mlog/:cid
    /// Requires --dev to enable the web server. Only serves files by exact CID - no index.
    #[arg(long)]
    pub mlog_serve: bool,

    /// Path to the shared coordinator file for multi-relay coordination.
    /// Multiple relay instances can share namespace/track registration via this file.
    /// User doesn't have to explicitly create and populate anything. This path will be
    /// used by file coordinator to store namespace/track registration information.
    /// User need to make sure if multiple relay's are being used all of them have same path
    /// to this file.
    #[arg(long, default_value = "/tmp/moq-coordinator.json")]
    pub coordinator_file: PathBuf,

    /// Maximum namespace entries accepted in the file coordinator.
    #[arg(long, default_value_t = 100_000)]
    pub coordinator_file_max_entries: usize,

    /// Maximum serialized file coordinator size.
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    pub coordinator_file_max_bytes: usize,

    /// URL of the moq-api server for coordination (e.g., "http://localhost:8080").
    /// When specified, uses moq-api HTTP server instead of file-based coordination.
    /// This is useful when running a cluster of relays with a centralized API server.
    #[arg(long)]
    pub api_url: Option<Url>,

    /// TTL in seconds for namespace registrations in the API.
    /// Only used when --api-url is specified.
    #[arg(long, default_value = "600")]
    pub api_ttl: u64,

    /// Maximum supervised refresh/cleanup tasks for API registrations.
    #[arg(long, default_value_t = 4_096)]
    pub api_max_background_tasks: usize,

    /// Maximum duration of one API coordinator request.
    #[arg(long, default_value_t = 2_000)]
    pub api_request_timeout_ms: u64,

    /// Maximum API coordinator cleanup and shutdown wait.
    #[arg(long, default_value_t = 2_000)]
    pub api_cleanup_timeout_ms: u64,

    /// Address to expose Prometheus metrics on (e.g., "127.0.0.1:9090").
    /// Requires the `metrics-prometheus` feature to be enabled.
    /// When set, serves metrics at http://<addr>/metrics
    #[arg(long)]
    pub metrics_addr: Option<net::SocketAddr>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing with env filter (respects RUST_LOG environment variable)
    // Default to info level, but suppress quinn's verbose output
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,quinn=warn")),
        )
        .init();

    let cli = Cli::parse();

    // Initialize Prometheus metrics exporter if --metrics-addr is provided
    #[cfg(feature = "metrics-prometheus")]
    if let Some(metrics_addr) = cli.metrics_addr {
        use metrics_exporter_prometheus::PrometheusBuilder;

        // Configure histogram buckets for subscribe latency (1ms to 10s)
        let subscribe_latency_buckets = vec![
            0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0, 10.0,
        ];

        PrometheusBuilder::new()
            .with_http_listener(metrics_addr)
            .set_buckets_for_metric(
                metrics_exporter_prometheus::Matcher::Full(
                    "moq_relay_subscribe_latency_seconds".to_string(),
                ),
                &subscribe_latency_buckets,
            )?
            .install()
            .expect("failed to install Prometheus metrics exporter");

        // Register metric descriptions (shows as # HELP in Prometheus output)
        moq_relay_ietf::metrics::describe_metrics();

        tracing::info!(
            "metrics exporter listening on http://{}/metrics",
            metrics_addr
        );
    }

    #[cfg(not(feature = "metrics-prometheus"))]
    if cli.metrics_addr.is_some() {
        tracing::warn!(
            "--metrics-addr was provided but the metrics-prometheus feature is not enabled. \
             Rebuild with --features metrics-prometheus to enable the Prometheus exporter."
        );
    }

    let tls = cli.tls.load()?;

    if !tls.has_server() {
        anyhow::bail!("missing TLS certificates");
    }

    let listener_security = cli.listener_security.unwrap_or(if cli.dev {
        ListenerSecurityPolicy::Development
    } else {
        ListenerSecurityPolicy::MutualTlsPublisher
    });
    let admission: Arc<dyn SessionAdmission> = match listener_security {
        ListenerSecurityPolicy::MutualTlsPublisher => {
            anyhow::ensure!(
                cli.admitted_subscribe_token_sha256.is_empty(),
                "subscribe token digests are only valid for token-subscriber listeners"
            );
            anyhow::ensure!(
                cli.admitted_relay_subscribers.is_empty(),
                "relay-subscriber bindings are only valid for mTLS relay-subscriber listeners"
            );
            CertificateFingerprintAdmission::new_bindings_with_limit(
                cli.admitted_publishers.clone(),
                cli.publisher_session_cap,
            )?
        }
        ListenerSecurityPolicy::MutualTlsRelaySubscriber => {
            anyhow::ensure!(
                cli.admitted_subscribe_token_sha256.is_empty(),
                "subscribe token digests are only valid for token-subscriber listeners"
            );
            anyhow::ensure!(
                cli.admitted_publishers.is_empty(),
                "publisher bindings are only valid for mTLS publisher listeners"
            );
            CertificateFingerprintAdmission::new_relay_subscriber_bindings_with_limit(
                cli.admitted_relay_subscribers.clone(),
                cli.relay_subscriber_session_cap,
            )?
        }
        ListenerSecurityPolicy::TokenSubscriber
        | ListenerSecurityPolicy::RawQuicTokenSubscriber => {
            anyhow::ensure!(
                cli.dev,
                "the built-in static token allowlist is non-production; embed Relay with an external replay- and lease-aware SessionAdmission policy"
            );
            anyhow::ensure!(
                cli.admitted_publishers.is_empty() && cli.admitted_relay_subscribers.is_empty(),
                "mTLS certificate bindings are only valid for mTLS listeners"
            );
            SetupTokenAdmission::new(cli.admitted_subscribe_token_sha256.clone())?
        }
        ListenerSecurityPolicy::Development => {
            anyhow::ensure!(cli.dev, "development listener policy requires --dev");
            anyhow::ensure!(
                cli.admitted_publishers.is_empty()
                    && cli.admitted_relay_subscribers.is_empty()
                    && cli.admitted_subscribe_token_sha256.is_empty(),
                "development allow-all cannot be combined with production identity allowlists"
            );
            DevelopmentAllowAllAdmission::explicitly_enabled()
        }
    };

    // Determine qlog directory for both relay and web server
    let qlog_dir_for_relay = cli.qlog_dir.clone();
    let qlog_dir_for_web = if cli.qlog_serve {
        cli.qlog_dir.clone()
    } else {
        None
    };

    // Determine mlog directory for both relay and web server
    let mlog_dir_for_relay = cli.mlog_dir.clone();
    let mlog_dir_for_web = if cli.mlog_serve {
        cli.mlog_dir.clone()
    } else {
        None
    };

    // Build the relay URL from the node or bind address
    let relay_url = cli
        .node
        .clone()
        .unwrap_or_else(|| Url::parse(&format!("https://{}", cli.bind)).unwrap());

    // Create the coordinator based on CLI arguments
    // Priority: api-url > file coordinator
    let coordinator: Arc<dyn Coordinator> = if let Some(api_url) = &cli.api_url {
        let config = ApiCoordinatorConfig::new(api_url.clone(), relay_url)
            .with_ttl(cli.api_ttl)
            .with_background_task_limit(cli.api_max_background_tasks)
            .with_request_timeout(std::time::Duration::from_millis(cli.api_request_timeout_ms))
            .with_cleanup_timeout(std::time::Duration::from_millis(cli.api_cleanup_timeout_ms));
        let api_coordinator = ApiCoordinator::new(config)?;
        tracing::info!(
            api_url = %moq_relay_ietf::redact_url_for_logging(api_url),
            "using API coordinator"
        );
        Arc::new(api_coordinator)
    } else {
        tracing::info!("using file coordinator: {}", cli.coordinator_file.display());
        Arc::new(FileCoordinator::with_limits(
            &cli.coordinator_file,
            relay_url,
            FileCoordinatorLimits {
                max_entries: cli.coordinator_file_max_entries,
                max_bytes: cli.coordinator_file_max_bytes,
            },
        )?)
    };

    // Create a QUIC server for media.
    let relay = Relay::new(RelayConfig {
        tls: tls.clone(),
        bind: Some(cli.bind),
        endpoints: vec![],
        qlog_dir: qlog_dir_for_relay,
        mlog_dir: mlog_dir_for_relay,
        node: cli.node,
        announce: cli.announce,
        coordinator,
        admission,
        development: cli.dev,
        listener_security,
        setup_timeout: std::time::Duration::from_millis(cli.setup_timeout_ms),
        admission_timeout: std::time::Duration::from_millis(cli.admission_timeout_ms),
        cleanup_timeout: std::time::Duration::from_millis(cli.pre_admission_cleanup_timeout_ms),
        session_close_timeout: std::time::Duration::from_millis(cli.session_close_timeout_ms),
        max_pending_admissions: cli.max_pending_admissions,
        max_active_sessions: cli.max_active_sessions,
        token_revalidation_interval: std::time::Duration::from_millis(
            cli.token_revalidation_interval_ms,
        ),
        capacity_limits: cli.relay_capacity.limits(),
        remote_limits: RemoteManagerLimits {
            max_connections: cli.max_upstream_connections,
            max_tracks: cli.max_upstream_tracks,
            track_idle_timeout: std::time::Duration::from_millis(
                cli.upstream_track_idle_timeout_ms,
            ),
            connection_idle_timeout: std::time::Duration::from_millis(
                cli.upstream_connection_idle_timeout_ms,
            ),
        },
        tracks_limits: moq_transport::serve::TracksLimits {
            max_cached_tracks: cli.max_tracks_per_namespace,
            max_pending_requests: cli.max_pending_track_requests_per_namespace,
        },
        request_limits: moq_transport::session::RequestLimits {
            retention: moq_transport::serve::RetentionBudgetLimits {
                max_session_bytes: cli.max_retained_bytes_per_session,
                max_process_bytes: cli.max_retained_bytes_process,
            },
            ..Default::default()
        },
    })?;

    if cli.dev {
        // Create a web server too.
        // Currently this only contains the certificate fingerprint (for development only).
        let web = Web::new(WebConfig {
            bind: cli.bind,
            tls,
            qlog_dir: qlog_dir_for_web,
            mlog_dir: mlog_dir_for_web,
        });

        tokio::spawn(async move {
            web.run().await.expect("failed to run web server");
        });
    }

    let shutdown = CancellationToken::new();
    let signal_shutdown = shutdown.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("shutdown signal received; draining admitted sessions");
            signal_shutdown.cancel();
        }
    });

    relay.run_until(shutdown).await
}

#[cfg(test)]
mod cli_tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn insecure_development_is_explicit_in_cli_help() {
        let cli = Cli::try_parse_from(["moq-relay-ietf", "--insecure-development"]).unwrap();
        assert!(cli.dev);
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("--insecure-development"));
        assert!(help.contains("Never enable it on a public relay"));
    }

    #[test]
    fn raw_quic_token_subscriber_is_a_distinct_cli_policy() {
        let cli = Cli::try_parse_from([
            "moq-relay-ietf",
            "--listener-security",
            "raw-quic-token-subscriber",
        ])
        .unwrap();
        assert_eq!(
            cli.listener_security,
            Some(ListenerSecurityPolicy::RawQuicTokenSubscriber)
        );

        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("raw-quic-token-subscriber"));
        assert!(help.contains("token-subscriber"));
    }

    #[test]
    fn mtls_relay_subscriber_has_distinct_bindings_and_capacity_flags() {
        let binding = format!("{}=/tenant/live", "42".repeat(32));
        let cli = Cli::try_parse_from([
            "moq-relay-ietf",
            "--listener-security",
            "mutual-tls-relay-subscriber",
            "--admit-relay-subscriber",
            &binding,
            "--relay-subscriber-session-cap",
            "7",
        ])
        .unwrap();
        assert_eq!(
            cli.listener_security,
            Some(ListenerSecurityPolicy::MutualTlsRelaySubscriber)
        );
        assert_eq!(cli.admitted_relay_subscribers, vec![binding]);
        assert_eq!(cli.relay_subscriber_session_cap, 7);
        assert!(cli.admitted_publishers.is_empty());

        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("mutual-tls-relay-subscriber"));
        assert!(help.contains("--admit-relay-subscriber"));
        assert!(help.contains("--relay-subscriber-session-cap"));
    }

    #[test]
    fn relay_capacity_defaults_are_valid_and_match_library_defaults() {
        let cli = Cli::try_parse_from(["moq-relay-ietf"]).unwrap();
        let limits = cli.relay_capacity.limits();
        assert_eq!(limits, RelayCapacityLimits::default());
        limits.validate().unwrap();
    }

    #[test]
    fn admitted_session_close_timeout_is_positive_and_documented() {
        let cli = Cli::try_parse_from(["moq-relay-ietf"]).unwrap();
        assert_eq!(cli.session_close_timeout_ms, 5_000);
        assert!(
            Cli::try_parse_from(["moq-relay-ietf", "--session-close-timeout-ms", "0"]).is_err()
        );

        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("--session-close-timeout-ms"));
        assert!(help.contains("replay tombstoning"));
    }

    #[test]
    fn relay_capacity_rejects_zero_during_relay_validation() {
        let cli = Cli::try_parse_from(["moq-relay-ietf", "--max-subscibes", "0"]);
        assert!(
            cli.is_err(),
            "misspelled capacity flags must not be ignored"
        );

        let cli = Cli::try_parse_from(["moq-relay-ietf", "--max-subscribes", "0"]).unwrap();
        assert!(cli.relay_capacity.limits().validate().is_err());
    }

    #[test]
    fn upstream_retention_defaults_match_library_defaults() {
        let cli = Cli::try_parse_from(["moq-relay-ietf"]).unwrap();
        assert_eq!(
            RemoteManagerLimits {
                max_connections: cli.max_upstream_connections,
                max_tracks: cli.max_upstream_tracks,
                track_idle_timeout: std::time::Duration::from_millis(
                    cli.upstream_track_idle_timeout_ms,
                ),
                connection_idle_timeout: std::time::Duration::from_millis(
                    cli.upstream_connection_idle_timeout_ms,
                ),
            },
            RemoteManagerLimits::default()
        );
    }

    #[test]
    fn per_namespace_track_defaults_match_transport_defaults() {
        let cli = Cli::try_parse_from(["moq-relay-ietf"]).unwrap();
        let limits = moq_transport::serve::TracksLimits {
            max_cached_tracks: cli.max_tracks_per_namespace,
            max_pending_requests: cli.max_pending_track_requests_per_namespace,
        };
        assert_eq!(limits, moq_transport::serve::TracksLimits::default());
        limits.validate().unwrap();
    }

    #[test]
    fn coordinator_retention_defaults_match_config_defaults() {
        let cli = Cli::try_parse_from(["moq-relay-ietf"]).unwrap();
        assert_eq!(
            FileCoordinatorLimits {
                max_entries: cli.coordinator_file_max_entries,
                max_bytes: cli.coordinator_file_max_bytes,
            },
            FileCoordinatorLimits::default()
        );
        let config = ApiCoordinatorConfig::new(
            Url::parse("http://localhost:8080").unwrap(),
            Url::parse("https://relay.example.com").unwrap(),
        );
        assert_eq!(cli.api_ttl, config.registration_ttl_secs);
        assert_eq!(cli.api_max_background_tasks, config.max_background_tasks);
        assert_eq!(
            std::time::Duration::from_millis(cli.api_request_timeout_ms),
            config.request_timeout
        );
        assert_eq!(
            std::time::Duration::from_millis(cli.api_cleanup_timeout_ms),
            config.cleanup_timeout
        );
    }

    #[test]
    fn retained_byte_defaults_match_transport_defaults() {
        let cli = Cli::try_parse_from(["moq-relay-ietf"]).unwrap();
        assert_eq!(
            moq_transport::serve::RetentionBudgetLimits {
                max_session_bytes: cli.max_retained_bytes_per_session,
                max_process_bytes: cli.max_retained_bytes_process,
            },
            moq_transport::serve::RetentionBudgetLimits::default()
        );
    }
}
