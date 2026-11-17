//! `dut.json`: the DUT configuration (RFC7502_BENCHMARK.md §3).
//!
//! Unknown fields are rejected at every level, so a typo in an orchestrator
//! template fails the trial before it starts instead of silently running the
//! default.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// SIP signaling transport.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// SIP over UDP.
    Udp,
    /// SIP over TLS (RFC 3261 §26.2, RFC 5630); the server certificate and
    /// key come from `tls_cert` / `tls_key`.
    Tls,
}

impl Transport {
    /// The name used in `ready.json` and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::Tls => "tls",
        }
    }
}

/// What the SDP answer offers and what happens to RTP.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Media {
    /// Signaling only: the answer carries `RTP/AVP` SDP, no RTP socket is
    /// allocated and nothing is echoed.
    None,
    /// Plain RTP (`RTP/AVP`), echoed back to its source.
    Rtp,
    /// SDES-SRTP (`RTP/SAVP` + `a=crypto`, RFC 4568), decrypted and echoed
    /// back re-encrypted with the answer key.
    Srtp,
}

impl Media {
    /// The name used in `ready.json` and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Rtp => "rtp",
            Self::Srtp => "srtp",
        }
    }

    /// Whether calls allocate RTP sockets and echo media.
    pub fn has_rtp(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// New-call admission.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Admission {
    /// Never shed: every INVITE is answered 200 (subject only to
    /// `max_concurrent_calls`).
    #[default]
    Off,
    /// GCRA rate limit with a one-second burst; excess INVITEs get a
    /// stateless `503 Service Unavailable` with `Retry-After`.
    MaxCps {
        /// Sustained admitted INVITEs per second.
        max_cps: f64,
        /// `Retry-After` seconds on the 503.
        #[serde(default = "default_retry_after_secs")]
        retry_after_secs: u32,
    },
}

fn default_retry_after_secs() -> u32 {
    2
}

/// Optional performance knobs. Every field has a default chosen for the
/// release lanes (c8i.4xlarge DUT); none of them changes protocol behaviour.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tuning {
    /// Dedicated media lanes (one single-threaded runtime each) for
    /// `media: rtp|srtp`. Default: half the CPUs the process may use, at
    /// least 1. Ignored for `media: none`.
    #[serde(default)]
    pub media_lanes: Option<usize>,
    /// Pin media lane `i` to CPU `i + media_lane_first_cpu` (Linux only).
    /// Default `false`: lanes float.
    #[serde(default)]
    pub pin_media_lanes: bool,
    /// First CPU for pinned media lanes. Default 0.
    #[serde(default)]
    pub media_lane_first_cpu: usize,
    /// Event queue capacity of the SIP stack (UDP parse queue, transaction,
    /// dialog and session event channels). Default 20,000.
    #[serde(default)]
    pub queue_capacity: Option<usize>,
    /// Lifecycle records the session authority retains for the 64 s SIP
    /// anti-reuse horizon. Default: `max(8 x max_concurrent_calls,
    /// max_concurrent_calls + 10,000 x 64)`.
    #[serde(default)]
    pub retained_lifecycle_capacity: Option<usize>,
}

/// The DUT configuration file.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DutConfig {
    /// Directory for `ready.json`, `stats.jsonl`, `summary.json` and the
    /// `finish` trigger. Created if missing.
    pub run_dir: PathBuf,
    /// SIP transport.
    pub transport: Transport,
    /// SIP listen address (UDP, or TLS over TCP).
    pub bind: SocketAddr,
    /// Address advertised in Contact and in SDP `c=`/`o=`.
    pub advertise_ip: IpAddr,
    /// PEM server certificate chain (required for `tls`).
    #[serde(default)]
    pub tls_cert: Option<PathBuf>,
    /// PEM private key (required for `tls`).
    #[serde(default)]
    pub tls_key: Option<PathBuf>,
    /// Media handling.
    pub media: Media,
    /// Answer `a=rtcp-mux` when the offer carries it. Default `true`.
    #[serde(default = "default_true")]
    pub rtcp_mux: bool,
    /// Inclusive RTP port range for media sockets. Default `[10000, 64999]`.
    #[serde(default = "default_rtp_port_range")]
    pub rtp_port_range: [u16; 2],
    /// New-call admission. Default `{"mode": "off"}`.
    #[serde(default)]
    pub admission: Admission,
    /// Hard cap on concurrent calls; INVITEs beyond it get a stateless 503.
    /// Default 60,000.
    #[serde(default = "default_max_concurrent_calls")]
    pub max_concurrent_calls: usize,
    /// Prometheus listen address (`GET /metrics`). `null` disables it.
    /// Port 0 binds an ephemeral port, reported in `ready.json`.
    /// Default `0.0.0.0:9464`.
    #[serde(default = "default_metrics_listen")]
    pub metrics_listen: Option<SocketAddr>,
    /// `stats.jsonl` interval. Default 1,000 ms.
    #[serde(default = "default_stats_interval_ms")]
    pub stats_interval_ms: u64,
    /// Longest wait for calls to end after `finish` appears. Default 120 s.
    #[serde(default = "default_drain_timeout_secs")]
    pub drain_timeout_secs: u64,
    /// Signaling (Tokio) worker threads; `null` sizes them from the host.
    #[serde(default)]
    pub worker_threads: Option<usize>,
    /// Optional performance knobs (an extension to the contract).
    #[serde(default)]
    pub tuning: Tuning,
}

fn default_true() -> bool {
    true
}

fn default_rtp_port_range() -> [u16; 2] {
    [10_000, 64_999]
}

fn default_max_concurrent_calls() -> usize {
    60_000
}

fn default_metrics_listen() -> Option<SocketAddr> {
    Some(SocketAddr::from(([0, 0, 0, 0], 9464)))
}

fn default_stats_interval_ms() -> u64 {
    1_000
}

fn default_drain_timeout_secs() -> u64 {
    120
}

/// Default SIP stack queue capacity (`Tuning::queue_capacity`).
pub const DEFAULT_QUEUE_CAPACITY: usize = 20_000;

/// Call-arrival rate the retained-lifecycle default is sized for.
const RETAINED_LIFECYCLE_CPS: usize = 10_000;
/// RFC 3261 anti-reuse horizon the session authority fences identifiers for.
const LIFECYCLE_ANTI_REUSE_HORIZON_SECS: usize = 64;

/// A configuration error, reported before anything binds.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError(message.into())
}

/// A validated configuration and the SHA-256 of the file it came from.
#[derive(Clone, Debug)]
pub struct LoadedConfig {
    /// The configuration.
    pub config: DutConfig,
    /// Lowercase hex SHA-256 of the raw file bytes (`config_sha256`).
    pub sha256: String,
}

impl DutConfig {
    /// Parse and validate `bytes` (the content of `dut.json`).
    pub fn from_json(bytes: &[u8]) -> Result<Self, ConfigError> {
        let config: Self =
            serde_json::from_slice(bytes).map_err(|error| invalid(format!("dut.json: {error}")))?;
        config.validate()?;
        Ok(config)
    }

    /// Read, parse and validate the file at `path`.
    pub fn load(path: &Path) -> Result<LoadedConfig, ConfigError> {
        let bytes = std::fs::read(path)
            .map_err(|error| invalid(format!("cannot read {}: {error}", path.display())))?;
        let config = Self::from_json(&bytes)?;
        Ok(LoadedConfig {
            config,
            sha256: sha256_hex(&bytes),
        })
    }

    /// Check every cross-field rule.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.run_dir.as_os_str().is_empty() {
            return Err(invalid("run_dir must not be empty"));
        }
        if self.bind.port() == 0 {
            return Err(invalid("bind must name a port"));
        }
        if self.advertise_ip.is_unspecified() {
            return Err(invalid("advertise_ip must be a reachable address"));
        }
        if self.bind.is_ipv4() != self.advertise_ip.is_ipv4() {
            return Err(invalid("bind and advertise_ip must be the same IP family"));
        }
        if self.transport == Transport::Tls {
            for (name, path) in [("tls_cert", &self.tls_cert), ("tls_key", &self.tls_key)] {
                match path {
                    None => return Err(invalid(format!("{name} is required for transport tls"))),
                    Some(path) if !path.is_file() => {
                        return Err(invalid(format!(
                            "{name} {} is not a readable file",
                            path.display()
                        )))
                    }
                    Some(_) => {}
                }
            }
        }
        let [start, end] = self.rtp_port_range;
        if start < 1024 || start >= end {
            return Err(invalid(format!(
                "rtp_port_range [{start}, {end}] must satisfy 1024 <= start < end"
            )));
        }
        if self.media.has_rtp() && self.bind.port() >= start && self.bind.port() <= end {
            return Err(invalid(format!(
                "bind port {} lies inside rtp_port_range [{start}, {end}]",
                self.bind.port()
            )));
        }
        if let Admission::MaxCps { max_cps, .. } = self.admission {
            if !max_cps.is_finite() || max_cps < 1.0 {
                return Err(invalid("admission.max_cps must be a number >= 1"));
            }
        }
        if self.max_concurrent_calls == 0 {
            return Err(invalid("max_concurrent_calls must be at least 1"));
        }
        if self.stats_interval_ms < 50 {
            return Err(invalid("stats_interval_ms must be at least 50"));
        }
        if matches!(self.worker_threads, Some(0)) || self.worker_threads.unwrap_or(1) > 1024 {
            return Err(invalid("worker_threads must be from 1 to 1024"));
        }
        if matches!(self.tuning.media_lanes, Some(0)) || self.tuning.media_lanes.unwrap_or(1) > 256
        {
            return Err(invalid("tuning.media_lanes must be from 1 to 256"));
        }
        if matches!(self.tuning.queue_capacity, Some(0)) {
            return Err(invalid("tuning.queue_capacity must be at least 1"));
        }
        if self
            .tuning
            .retained_lifecycle_capacity
            .is_some_and(|retained| retained < self.max_concurrent_calls)
        {
            return Err(invalid(
                "tuning.retained_lifecycle_capacity must be at least max_concurrent_calls",
            ));
        }
        Ok(())
    }

    /// The SIP stack queue capacity in effect.
    pub fn queue_capacity(&self) -> usize {
        self.tuning.queue_capacity.unwrap_or(DEFAULT_QUEUE_CAPACITY)
    }

    /// Retained lifecycle records in effect (see
    /// [`Tuning::retained_lifecycle_capacity`]).
    pub fn retained_lifecycle_capacity(&self) -> usize {
        self.tuning.retained_lifecycle_capacity.unwrap_or_else(|| {
            let active = self.max_concurrent_calls;
            active.saturating_mul(8).max(active.saturating_add(
                RETAINED_LIFECYCLE_CPS.saturating_mul(LIFECYCLE_ANTI_REUSE_HORIZON_SECS),
            ))
        })
    }

    /// The `(media lanes, signaling worker threads)` split for a host with
    /// `cpus` usable CPUs.
    pub fn thread_plan(&self, cpus: usize) -> (usize, usize) {
        let cpus = cpus.max(1);
        let lanes = if self.media.has_rtp() {
            self.tuning.media_lanes.unwrap_or((cpus / 2).max(1))
        } else {
            0
        };
        let workers = self
            .worker_threads
            .unwrap_or_else(|| cpus.saturating_sub(lanes).max(2));
        (lanes, workers)
    }

    /// Port count of `rtp_port_range` (inclusive).
    pub fn rtp_port_count(&self) -> usize {
        usize::from(self.rtp_port_range[1] - self.rtp_port_range[0]) + 1
    }
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal(extra: &str) -> String {
        format!(
            r#"{{"run_dir": "/tmp/run", "transport": "udp", "bind": "10.0.0.1:5060",
                "advertise_ip": "10.0.0.1", "media": "none"{extra}}}"#
        )
    }

    fn parse(json: &str) -> Result<DutConfig, ConfigError> {
        DutConfig::from_json(json.as_bytes())
    }

    #[test]
    fn contract_example_parses_with_every_field() {
        let dir = std::env::temp_dir().join(format!("bench-uas-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("dut.crt");
        let key = dir.join("dut.key");
        std::fs::write(&cert, "cert").unwrap();
        std::fs::write(&key, "key").unwrap();
        let json = format!(
            r#"{{
              "run_dir": "/var/lib/rvoip-bench/run",
              "transport": "tls",
              "bind": "10.90.200.10:5061",
              "advertise_ip": "10.90.200.10",
              "tls_cert": "{}",
              "tls_key": "{}",
              "media": "srtp",
              "rtcp_mux": true,
              "rtp_port_range": [10000, 64999],
              "admission": {{"mode": "max-cps", "max_cps": 4200, "retry_after_secs": 2}},
              "max_concurrent_calls": 60000,
              "metrics_listen": "0.0.0.0:9464",
              "stats_interval_ms": 1000,
              "drain_timeout_secs": 120,
              "worker_threads": null
            }}"#,
            cert.display(),
            key.display()
        );
        let config = parse(&json).expect("the contract example is valid");
        assert_eq!(config.transport, Transport::Tls);
        assert_eq!(config.media, Media::Srtp);
        assert_eq!(
            config.admission,
            Admission::MaxCps {
                max_cps: 4200.0,
                retry_after_secs: 2
            }
        );
        assert_eq!(
            config.bind,
            "10.90.200.10:5061".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(config.worker_threads, None);
        assert_eq!(config.tuning, Tuning::default());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn defaults_follow_the_contract_example() {
        let config = parse(&minimal("")).unwrap();
        assert!(config.rtcp_mux);
        assert_eq!(config.rtp_port_range, [10_000, 64_999]);
        assert_eq!(config.admission, Admission::Off);
        assert_eq!(config.max_concurrent_calls, 60_000);
        assert_eq!(config.metrics_listen, Some("0.0.0.0:9464".parse().unwrap()));
        assert_eq!(config.stats_interval_ms, 1_000);
        assert_eq!(config.drain_timeout_secs, 120);
        assert_eq!(config.queue_capacity(), DEFAULT_QUEUE_CAPACITY);
        assert_eq!(config.rtp_port_count(), 55_000);
        // Default retained lifecycle: the 10k-CPS, 64 s horizon dominates
        // 8 x 60k.
        assert_eq!(config.retained_lifecycle_capacity(), 60_000 + 640_000);
        let null_metrics = parse(&minimal(r#", "metrics_listen": null"#)).unwrap();
        assert_eq!(null_metrics.metrics_listen, None);
    }

    #[test]
    fn unknown_fields_are_rejected_at_every_level() {
        let top = parse(&minimal(r#", "colour": "blue""#)).unwrap_err();
        assert!(top.0.contains("unknown field `colour`"), "{top}");
        let admission = parse(&minimal(
            r#", "admission": {"mode": "max-cps", "max_cps": 10, "burst": 5}"#,
        ))
        .unwrap_err();
        assert!(admission.0.contains("burst"), "{admission}");
        let tuning = parse(&minimal(r#", "tuning": {"turbo": true}"#)).unwrap_err();
        assert!(tuning.0.contains("turbo"), "{tuning}");
        let mode = parse(&minimal(r#", "admission": {"mode": "sometimes"}"#)).unwrap_err();
        assert!(mode.0.contains("sometimes"), "{mode}");
        let transport = parse(&minimal("").replace("\"udp\"", "\"tcp\"")).unwrap_err();
        assert!(transport.0.contains("tcp"), "{transport}");
    }

    #[test]
    fn admission_modes_parse() {
        let off = parse(&minimal(r#", "admission": {"mode": "off"}"#)).unwrap();
        assert_eq!(off.admission, Admission::Off);
        let default_retry = parse(&minimal(
            r#", "admission": {"mode": "max-cps", "max_cps": 1500.5}"#,
        ))
        .unwrap();
        assert_eq!(
            default_retry.admission,
            Admission::MaxCps {
                max_cps: 1500.5,
                retry_after_secs: 2
            }
        );
    }

    #[test]
    fn cross_field_rules_are_enforced() {
        let cases = [
            (
                r#", "admission": {"mode": "max-cps", "max_cps": 0}"#,
                "max_cps",
            ),
            (r#", "max_concurrent_calls": 0"#, "max_concurrent_calls"),
            (r#", "rtp_port_range": [20000, 20000]"#, "rtp_port_range"),
            (r#", "rtp_port_range": [80, 20000]"#, "rtp_port_range"),
            (r#", "stats_interval_ms": 10"#, "stats_interval_ms"),
            (r#", "worker_threads": 0"#, "worker_threads"),
            (r#", "tuning": {"media_lanes": 0}"#, "media_lanes"),
            (r#", "tuning": {"queue_capacity": 0}"#, "queue_capacity"),
            (
                r#", "tuning": {"retained_lifecycle_capacity": 10}"#,
                "retained_lifecycle_capacity",
            ),
        ];
        for (extra, needle) in cases {
            let error = parse(&minimal(extra)).unwrap_err();
            assert!(error.0.contains(needle), "{extra}: {error}");
        }
        let unspecified = parse(&minimal("").replace(
            r#""advertise_ip": "10.0.0.1""#,
            r#""advertise_ip": "0.0.0.0""#,
        ))
        .unwrap_err();
        assert!(unspecified.0.contains("advertise_ip"), "{unspecified}");
        let no_port = parse(&minimal("").replace("10.0.0.1:5060", "10.0.0.1:0")).unwrap_err();
        assert!(no_port.0.contains("bind"), "{no_port}");
        let family = parse(&minimal("").replace(r#""10.0.0.1""#, r#""::1""#)).unwrap_err();
        assert!(family.0.contains("family"), "{family}");
        // Media sockets must not collide with the SIP port.
        let overlap =
            parse(&minimal(r#", "rtp_port_range": [5000, 6000]"#).replace("\"none\"", "\"rtp\""))
                .unwrap_err();
        assert!(overlap.0.contains("inside rtp_port_range"), "{overlap}");
        // Signaling only allocates no RTP, so the same range is harmless.
        parse(&minimal(r#", "rtp_port_range": [5000, 6000]"#)).unwrap();
    }

    #[test]
    fn tls_requires_readable_cert_and_key() {
        let tls = minimal("").replace("\"udp\"", "\"tls\"");
        let missing = parse(&tls).unwrap_err();
        assert!(missing.0.contains("tls_cert is required"), "{missing}");
        let absent = parse(
            &minimal(r#", "tls_cert": "/nonexistent/c.pem", "tls_key": "/nonexistent/k.pem""#)
                .replace("\"udp\"", "\"tls\""),
        )
        .unwrap_err();
        assert!(absent.0.contains("not a readable file"), "{absent}");
        // UDP ignores TLS paths entirely.
        parse(&minimal(
            r#", "tls_cert": "/nonexistent/c.pem", "tls_key": "/nonexistent/k.pem""#,
        ))
        .unwrap();
    }

    #[test]
    fn thread_plan_splits_media_lanes_from_signaling() {
        let signaling = parse(&minimal("")).unwrap();
        assert_eq!(signaling.thread_plan(16), (0, 16));
        let media = parse(&minimal("").replace("\"none\"", "\"srtp\"")).unwrap();
        assert_eq!(media.thread_plan(16), (8, 8));
        assert_eq!(media.thread_plan(1), (1, 2));
        let tuned = parse(
            &minimal(r#", "worker_threads": 6, "tuning": {"media_lanes": 10}"#)
                .replace("\"none\"", "\"rtp\""),
        )
        .unwrap();
        assert_eq!(tuned.thread_plan(16), (10, 6));
    }

    #[test]
    fn config_hash_is_the_sha256_of_the_file_bytes() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let dir = std::env::temp_dir().join(format!("bench-uas-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dut.json");
        let json = minimal("");
        std::fs::write(&path, &json).unwrap();
        let loaded = DutConfig::load(&path).unwrap();
        assert_eq!(loaded.sha256, sha256_hex(json.as_bytes()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
