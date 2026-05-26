//! Config-backed performance recipe loading and application.
//!
//! The recipe values live in YAML so deployments and release gates can tune
//! server shapes without changing library source. The library owns parsing,
//! validation, and application to [`Config`].

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::api::unified::{Config, MediaMode};
use crate::errors::{Result, SessionError};

const DEFAULT_RECIPE_BOOK: &str = include_str!("../../config/performance-recipes.yaml");

/// Parameterized performance recipe selection.
///
/// `profile` is resolved from a YAML recipe book. When `recipe_path` is
/// omitted, rvoip-sip uses its bundled default recipe book. When `recipe_path`
/// is set, that YAML file is loaded instead.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceConfig {
    /// Recipe name from the selected YAML recipe book.
    pub profile: String,
    /// Expected burst or active-call capacity for capacity-driven recipes.
    pub capacity: Option<usize>,
    /// SDP RTP port used by signaling-only recipes.
    pub signaling_only_rtp_port: Option<u16>,
    /// Optional YAML recipe book path.
    pub recipe_path: Option<PathBuf>,
}

impl PerformanceConfig {
    /// Create a performance config for a named recipe profile.
    pub fn profile(profile: impl Into<String>) -> Self {
        Self {
            profile: profile.into(),
            capacity: None,
            signaling_only_rtp_port: None,
            recipe_path: None,
        }
    }

    /// Endpoint recipe. This is also the implicit endpoint default.
    pub fn endpoint() -> Self {
        Self::profile("endpoint")
    }

    /// PBX-style media server recipe.
    pub fn pbx_media_server(capacity: usize) -> Self {
        Self::profile("pbx-media-server").with_capacity(capacity)
    }

    /// High-performance signaling-only server recipe.
    pub fn signaling_only_server_high_performance(capacity: usize) -> Self {
        Self::profile("signaling-only-server-high-performance")
            .with_capacity(capacity)
            .with_signaling_only_rtp_port(9)
    }

    /// Set the capacity parameter.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = Some(capacity);
        self
    }

    /// Set the signaling-only SDP RTP port parameter.
    pub fn with_signaling_only_rtp_port(mut self, port: u16) -> Self {
        self.signaling_only_rtp_port = Some(port);
        self
    }

    /// Load recipes from this YAML path instead of the bundled default book.
    pub fn with_recipe_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.recipe_path = Some(path.into());
        self
    }
}

/// YAML performance recipe book.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceRecipeBook {
    /// Recipe schema version.
    pub version: u32,
    /// Named performance profiles.
    pub performance_profiles: BTreeMap<String, PerformanceRecipe>,
}

impl PerformanceRecipeBook {
    /// Parse the bundled default recipe book.
    pub fn bundled() -> Result<Self> {
        Self::from_yaml_str(DEFAULT_RECIPE_BOOK, "bundled default performance recipes")
    }

    /// Load a recipe book from a YAML file.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|e| {
            SessionError::ConfigError(format!(
                "failed to read performance recipe file '{}': {e}",
                path.display()
            ))
        })?;
        Self::from_yaml_str(&text, &path.display().to_string())
    }

    /// Parse a YAML recipe book string.
    pub fn from_yaml_str(text: &str, source: &str) -> Result<Self> {
        let book: Self = serde_yaml::from_str(text).map_err(|e| {
            SessionError::ConfigError(format!(
                "failed to parse performance recipe book {source}: {e}"
            ))
        })?;
        if book.version != 1 {
            return Err(SessionError::ConfigError(format!(
                "unsupported performance recipe book version {}; expected 1",
                book.version
            )));
        }
        Ok(book)
    }

    /// Apply a named performance config to an existing [`Config`].
    pub fn apply(&self, config: Config, performance: &PerformanceConfig) -> Result<Config> {
        let recipe = self
            .performance_profiles
            .get(&performance.profile)
            .ok_or_else(|| {
                let mut names = self
                    .performance_profiles
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>();
                names.sort();
                SessionError::ConfigError(format!(
                    "unknown performance profile '{}'; available profiles: {}",
                    performance.profile,
                    names.join(", ")
                ))
            })?;
        recipe.apply(config, performance)
    }
}

/// One named performance recipe from YAML.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceRecipe {
    /// Human-readable recipe description.
    pub description: Option<String>,
    /// Whether `PerformanceConfig.capacity` is required.
    pub requires_capacity: Option<bool>,
    /// Config field values to apply.
    pub config: PerformanceRecipeConfig,
}

impl PerformanceRecipe {
    fn apply(&self, mut config: Config, performance: &PerformanceConfig) -> Result<Config> {
        if self.requires_capacity.unwrap_or(false) {
            let capacity = performance.capacity.ok_or_else(|| {
                SessionError::ConfigError(format!(
                    "performance profile '{}' requires capacity",
                    performance.profile
                ))
            })?;
            if capacity == 0 {
                return Err(SessionError::ConfigError(
                    "performance capacity must be at least 1".to_string(),
                ));
            }
        }

        let params = RecipeParams {
            capacity: performance.capacity,
            signaling_only_rtp_port: performance.signaling_only_rtp_port,
        };
        self.config.apply(&mut config, &params)?;
        config.validate()?;
        Ok(config)
    }
}

/// Config mutations supported by YAML performance recipes.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceRecipeConfig {
    /// Capacity for the standard SIP signaling queues.
    pub channel_capacity: Option<RecipeUsize>,
    /// Whether automatic `180 Ringing` is sent for inbound INVITEs.
    pub auto_180_ringing: Option<bool>,
    /// Whether automatic `100 Trying` timer tasks are armed.
    pub auto_100_trying: Option<bool>,
    /// Whether inbound INVITEs are accepted before app callbacks.
    pub fast_auto_accept_incoming_calls: Option<bool>,
    /// SIP UDP receive socket buffer size in bytes.
    pub sip_udp_recv_buffer_size: Option<usize>,
    /// SIP UDP send socket buffer size in bytes.
    pub sip_udp_send_buffer_size: Option<usize>,
    /// UDP parse worker count.
    pub sip_udp_parse_workers: Option<usize>,
    /// Per-worker UDP parse queue capacity.
    pub sip_udp_parse_queue_capacity: Option<RecipeUsize>,
    /// UDP parse dispatch strategy.
    pub sip_udp_parse_dispatch: Option<RecipeUdpParseDispatch>,
    /// Transaction-manager ingress dispatch worker count.
    pub sip_transaction_dispatch_workers: Option<usize>,
    /// Transaction-manager ingress dispatch queue capacity.
    pub sip_transaction_dispatch_queue_capacity: Option<RecipeUsize>,
    /// Per-transaction command channel capacity.
    pub sip_transaction_command_channel_capacity: Option<usize>,
    /// Dialog-core transaction-event dispatch worker count.
    pub sip_dialog_dispatch_workers: Option<usize>,
    /// Dialog-core transaction-event dispatch queue capacity.
    pub sip_dialog_dispatch_queue_capacity: Option<RecipeUsize>,
    /// App-session event dispatcher worker count.
    pub session_event_dispatcher_workers: Option<usize>,
    /// Per-worker app-session event dispatcher queue capacity.
    pub session_event_dispatcher_channel_capacity: Option<RecipeUsize>,
    /// Media behavior.
    pub media_mode: Option<RecipeMediaMode>,
    /// SDP RTP port for signaling-only media mode.
    pub signaling_only_rtp_port: Option<RecipeU16>,
    /// RTP media port range by start and requested capacity.
    pub media_port_capacity: Option<RecipeMediaPortCapacity>,
    /// Media-core session and RTP allocator capacity hint.
    pub media_session_capacity: Option<RecipeUsize>,
    /// Server-side active call capacity hint.
    pub server_call_capacity: Option<RecipeUsize>,
    /// Server-side inbound call admission limit.
    pub server_call_admission_limit: Option<RecipeUsize>,
    /// Server-side inbound call admission soft pacing threshold.
    pub server_call_admission_soft_limit: Option<RecipeUsize>,
    /// Delay in milliseconds while above the soft admission threshold.
    pub server_call_admission_pacing_delay_ms: Option<u64>,
    /// Retry-After seconds for server overload rejections.
    pub server_overload_retry_after_secs: Option<u32>,
}

impl PerformanceRecipeConfig {
    fn apply(&self, config: &mut Config, params: &RecipeParams) -> Result<()> {
        if let Some(capacity) = &self.channel_capacity {
            *config = config
                .clone()
                .with_channel_capacity(capacity.resolve(params, "channelCapacity")?);
        }
        if let Some(enabled) = self.auto_180_ringing {
            config.auto_180_ringing = enabled;
        }
        if let Some(enabled) = self.auto_100_trying {
            config.auto_100_trying = enabled;
        }
        if let Some(enabled) = self.fast_auto_accept_incoming_calls {
            config.fast_auto_accept_incoming_calls = enabled;
        }
        if let Some(size) = self.sip_udp_recv_buffer_size {
            config.sip_udp_recv_buffer_size = Some(size);
        }
        if let Some(size) = self.sip_udp_send_buffer_size {
            config.sip_udp_send_buffer_size = Some(size);
        }
        if let Some(workers) = self.sip_udp_parse_workers {
            config.sip_udp_parse_workers = Some(workers);
        }
        if let Some(capacity) = &self.sip_udp_parse_queue_capacity {
            config.sip_udp_parse_queue_capacity =
                Some(capacity.resolve(params, "sipUdpParseQueueCapacity")?);
        }
        if let Some(dispatch) = self.sip_udp_parse_dispatch {
            config.sip_udp_parse_dispatch = Some(dispatch.into());
        }
        if let Some(workers) = self.sip_transaction_dispatch_workers {
            config.sip_transaction_dispatch_workers = Some(workers);
        }
        if let Some(capacity) = &self.sip_transaction_dispatch_queue_capacity {
            config.sip_transaction_dispatch_queue_capacity =
                Some(capacity.resolve(params, "sipTransactionDispatchQueueCapacity")?);
        }
        if let Some(capacity) = self.sip_transaction_command_channel_capacity {
            config.sip_transaction_command_channel_capacity = Some(capacity);
        }
        if let Some(workers) = self.sip_dialog_dispatch_workers {
            config.sip_dialog_dispatch_workers = Some(workers);
        }
        if let Some(capacity) = &self.sip_dialog_dispatch_queue_capacity {
            config.sip_dialog_dispatch_queue_capacity =
                Some(capacity.resolve(params, "sipDialogDispatchQueueCapacity")?);
        }
        if let Some(workers) = self.session_event_dispatcher_workers {
            config.session_event_dispatcher_workers = workers;
        }
        if let Some(capacity) = &self.session_event_dispatcher_channel_capacity {
            config.session_event_dispatcher_channel_capacity =
                capacity.resolve(params, "sessionEventDispatcherChannelCapacity")?;
        }
        if let Some(capacity) = &self.server_call_capacity {
            config.server_call_capacity = Some(capacity.resolve(params, "serverCallCapacity")?);
        }
        if let Some(limit) = &self.server_call_admission_limit {
            config.server_call_admission_limit =
                Some(limit.resolve(params, "serverCallAdmissionLimit")?);
        }
        if let Some(limit) = &self.server_call_admission_soft_limit {
            config.server_call_admission_soft_limit =
                Some(limit.resolve(params, "serverCallAdmissionSoftLimit")?);
        }
        if let Some(delay_ms) = self.server_call_admission_pacing_delay_ms {
            config.server_call_admission_pacing_delay_ms = Some(delay_ms);
        }
        if let Some(seconds) = self.server_overload_retry_after_secs {
            config.server_overload_retry_after_secs = Some(seconds);
        }
        if let Some(port_capacity) = &self.media_port_capacity {
            *config = config.clone().with_media_port_capacity(
                port_capacity.start,
                port_capacity
                    .capacity
                    .resolve(params, "mediaPortCapacity.capacity")?,
            );
        }
        if let Some(capacity) = &self.media_session_capacity {
            config.media_session_capacity = Some(capacity.resolve(params, "mediaSessionCapacity")?);
        }
        if let Some(mode) = self.media_mode {
            config.media_mode = match mode {
                RecipeMediaMode::Enabled => MediaMode::Enabled,
                RecipeMediaMode::SignalingOnly => {
                    let port = self
                        .signaling_only_rtp_port
                        .as_ref()
                        .map(|p| p.resolve(params, "signalingOnlyRtpPort"))
                        .transpose()?
                        .unwrap_or(9);
                    MediaMode::SignalingOnly { sdp_rtp_port: port }
                }
            };
        }
        Ok(())
    }
}

struct RecipeParams {
    capacity: Option<usize>,
    signaling_only_rtp_port: Option<u16>,
}

/// Recipe value that can be a literal integer or `$capacity`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RecipeUsize {
    /// Literal value.
    Literal(usize),
    /// Variable reference.
    Variable(String),
}

impl RecipeUsize {
    fn resolve(&self, params: &RecipeParams, field: &str) -> Result<usize> {
        match self {
            Self::Literal(value) => Ok(*value),
            Self::Variable(name) if name == "$capacity" || name == "capacity" => {
                params.capacity.ok_or_else(|| {
                    SessionError::ConfigError(format!("{field} requires performance capacity"))
                })
            }
            Self::Variable(name) if name == "$capacity90Percent" || name == "capacity90Percent" => {
                params
                    .capacity
                    .map(|capacity| capacity.saturating_mul(90).div_ceil(100).max(1))
                    .ok_or_else(|| {
                        SessionError::ConfigError(format!("{field} requires performance capacity"))
                    })
            }
            Self::Variable(name) => Err(SessionError::ConfigError(format!(
                "unsupported variable '{name}' in performance recipe field {field}"
            ))),
        }
    }
}

/// Recipe value that can be a literal port or `$signalingOnlyRtpPort`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RecipeU16 {
    /// Literal value.
    Literal(u16),
    /// Variable reference.
    Variable(String),
}

impl RecipeU16 {
    fn resolve(&self, params: &RecipeParams, field: &str) -> Result<u16> {
        match self {
            Self::Literal(value) => Ok(*value),
            Self::Variable(name)
                if name == "$signalingOnlyRtpPort" || name == "signalingOnlyRtpPort" =>
            {
                Ok(params.signaling_only_rtp_port.unwrap_or(9))
            }
            Self::Variable(name) => Err(SessionError::ConfigError(format!(
                "unsupported variable '{name}' in performance recipe field {field}"
            ))),
        }
    }
}

/// UDP parse dispatch strategy in recipe YAML.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecipeUdpParseDispatch {
    /// Preserve per-source ordering.
    SourceHash,
    /// Spread received datagrams across workers.
    RoundRobin,
}

impl From<RecipeUdpParseDispatch> for rvoip_sip_transport::UdpParseDispatch {
    fn from(value: RecipeUdpParseDispatch) -> Self {
        match value {
            RecipeUdpParseDispatch::SourceHash => Self::SourceHash,
            RecipeUdpParseDispatch::RoundRobin => Self::RoundRobin,
        }
    }
}

/// Media mode in recipe YAML.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecipeMediaMode {
    /// Use real media-core RTP allocation.
    Enabled,
    /// Generate SDP without media-core RTP allocation.
    SignalingOnly,
}

/// Media port capacity recipe value.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeMediaPortCapacity {
    /// RTP media port range start.
    pub start: u16,
    /// Requested number of ports.
    pub capacity: RecipeUsize,
}
