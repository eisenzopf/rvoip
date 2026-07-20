//! Clap-based CLI argument parsing plus preset and JSON config loading for
//! the `sip_client` example. Translates `--preset` / `--config` choices into
//! an [`EndpointConfig`](rvoip_sip::EndpointConfig) the runtime can hand to
//! [`Endpoint::builder`](rvoip_sip::Endpoint::builder).
//!
//! Implementation detail of the `sip_client` example; see [`super::main`].

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use rvoip_sip::{
    EndpointAccountConfig, EndpointConfig, EndpointMediaConfig, EndpointNetworkConfig,
    EndpointProfileName, EndpointRegistrationInfo, EndpointRegistrationStatus, EndpointSrtpMode,
    EndpointTransport,
};

#[derive(Parser, Debug)]
#[command(name = "sip_client")]
#[command(about = "RVoIP terminal softphone and Endpoint smoke-test client")]
pub(crate) struct Cli {
    /// JSON endpoint config path.
    #[arg(long)]
    pub(crate) config: Option<PathBuf>,

    /// Built-in endpoint configuration preset.
    #[arg(long, value_enum)]
    pub(crate) preset: Option<ConfigPreset>,

    /// List audio devices and exit.
    #[arg(long)]
    pub(crate) list_devices: bool,

    /// Display/configuration name.
    #[arg(long)]
    pub(crate) name: Option<String>,

    /// SIP username or extension.
    #[arg(long)]
    pub(crate) username: Option<String>,

    /// Digest auth username when it differs from username.
    #[arg(long)]
    pub(crate) auth_username: Option<String>,

    /// Digest auth password.
    #[arg(long)]
    pub(crate) password: Option<String>,

    /// SIP registrar URI.
    #[arg(long)]
    pub(crate) registrar: Option<String>,

    /// Register on startup.
    #[arg(long)]
    pub(crate) register: bool,

    /// Dial this target after startup.
    #[arg(long)]
    pub(crate) dial: Option<String>,

    /// SIP bind address.
    #[arg(long)]
    pub(crate) bind: Option<SocketAddr>,

    /// SIP advertised address.
    #[arg(long)]
    pub(crate) advertise: Option<SocketAddr>,

    /// Preferred signalling transport.
    #[arg(long, value_enum)]
    pub(crate) transport: Option<CliTransport>,

    /// STUN server for media public address discovery.
    #[arg(long)]
    pub(crate) stun: Option<String>,

    /// Outbound proxy URI.
    #[arg(long)]
    pub(crate) outbound_proxy: Option<String>,

    /// Deployment profile.
    #[arg(long, value_enum)]
    pub(crate) profile: Option<CliProfile>,

    /// Public media address as IP or ip:port.
    #[arg(long)]
    pub(crate) media_public: Option<String>,

    /// SRTP negotiation mode.
    #[arg(long, value_enum)]
    pub(crate) srtp: Option<CliSrtp>,

    /// Input device name substring or index from --list-devices.
    #[arg(long)]
    pub(crate) input_device: Option<String>,

    /// Output device name substring or index from --list-devices.
    #[arg(long)]
    pub(crate) output_device: Option<String>,

    /// Emit SIP message trace events into the TUI.
    #[arg(long)]
    pub(crate) sip_trace: bool,

    /// Also append SIP trace messages to this file.
    #[arg(long)]
    pub(crate) sip_trace_file: Option<PathBuf>,

    /// Do not redact auth-bearing SIP headers in trace output.
    #[arg(long)]
    pub(crate) sip_trace_no_redact: bool,

    /// Number of SIP trace messages to retain in the TUI.
    #[arg(long)]
    pub(crate) sip_trace_capacity: Option<usize>,

    /// Run noninteractive smoke mode.
    #[arg(long, value_enum)]
    pub(crate) test: Option<TestRole>,

    /// Smoke test duration after answer, in seconds.
    #[arg(long, default_value_t = 5)]
    pub(crate) test_duration: u64,

    /// Smoke test operation timeout, in seconds.
    #[arg(long, default_value_t = 30)]
    pub(crate) test_timeout: u64,

    /// DTMF digit to send in smoke mode.
    #[arg(long, default_value_t = '5')]
    pub(crate) test_dtmf: char,

    /// Smoke-test audio backend.
    #[arg(long, value_enum, default_value_t = TestAudio::Synthetic)]
    pub(crate) test_audio: TestAudio,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ConfigPreset {
    #[value(name = "alice-loopback")]
    AliceLoopback,
    #[value(name = "bob-loopback")]
    BobLoopback,
    #[value(name = "asterisk-2001")]
    Asterisk2001,
    #[value(name = "asterisk-2002")]
    Asterisk2002,
    #[value(name = "freeswitch-1001")]
    Freeswitch1001,
    #[value(name = "freeswitch-1002")]
    Freeswitch1002,
}

impl ConfigPreset {
    fn load(self) -> anyhow::Result<EndpointConfig> {
        let text = match self {
            Self::AliceLoopback => include_str!("alice.loopback.json"),
            Self::BobLoopback => include_str!("bob.loopback.json"),
            Self::Asterisk2001 => include_str!("pbx-2001.asterisk-udp.json"),
            Self::Asterisk2002 => include_str!("pbx-2002.asterisk-udp.json"),
            Self::Freeswitch1001 => include_str!("pbx-1001.freeswitch.json"),
            Self::Freeswitch1002 => include_str!("pbx-1002.freeswitch.json"),
        };
        Ok(serde_json::from_str::<EndpointConfig>(text)?)
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum CliTransport {
    Udp,
    Tcp,
    Tls,
}

impl From<CliTransport> for EndpointTransport {
    fn from(value: CliTransport) -> Self {
        match value {
            CliTransport::Udp => Self::Udp,
            CliTransport::Tcp => Self::Tcp,
            CliTransport::Tls => Self::Tls,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum CliSrtp {
    Off,
    Offer,
    Required,
}

impl From<CliSrtp> for EndpointSrtpMode {
    fn from(value: CliSrtp) -> Self {
        match value {
            CliSrtp::Off => Self::Off,
            CliSrtp::Offer => Self::Offer,
            CliSrtp::Required => Self::Required,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum CliProfile {
    Local,
    LanPbx,
    AsteriskUdp,
    AsteriskTlsSrtp,
    FreeswitchInternal,
    FreeswitchTlsSrtp,
    CarrierSbc,
}

impl From<CliProfile> for EndpointProfileName {
    fn from(value: CliProfile) -> Self {
        match value {
            CliProfile::Local => Self::Local,
            CliProfile::LanPbx => Self::LanPbx,
            CliProfile::AsteriskUdp => Self::AsteriskUdp,
            CliProfile::AsteriskTlsSrtp => Self::AsteriskTlsSrtp,
            CliProfile::FreeswitchInternal => Self::FreeswitchInternal,
            CliProfile::FreeswitchTlsSrtp => Self::FreeswitchTlsSrtp,
            CliProfile::CarrierSbc => Self::CarrierSbc,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TestRole {
    Caller,
    Callee,
    PbxCaller,
    PbxCallee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TestAudio {
    Synthetic,
    Cpal,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeOptions {
    pub(crate) endpoint: EndpointConfig,
    pub(crate) register_on_start: bool,
    pub(crate) dial: Option<String>,
    pub(crate) input_device: Option<String>,
    pub(crate) output_device: Option<String>,
    pub(crate) sip_trace: SipTraceOptions,
    pub(crate) test_duration: Duration,
    pub(crate) test_timeout: Duration,
    pub(crate) test_dtmf: char,
    pub(crate) test_audio: TestAudio,
}

#[derive(Debug, Clone)]
pub(crate) struct SipTraceOptions {
    pub(crate) enabled: bool,
    pub(crate) redact_sensitive_headers: bool,
    pub(crate) capacity: usize,
    pub(crate) file: Option<PathBuf>,
}

pub(crate) fn build_runtime_options(cli: &Cli) -> anyhow::Result<RuntimeOptions> {
    let mut endpoint = load_endpoint_config(cli.config.clone(), cli.preset)?;
    apply_cli_overrides(&mut endpoint, cli);
    let sip_trace = apply_sip_trace_overrides(&mut endpoint, cli)?;
    let register_on_start = cli.register
        || endpoint.register_on_start.unwrap_or(false)
        || matches!(cli.test, Some(TestRole::PbxCaller | TestRole::PbxCallee));
    Ok(RuntimeOptions {
        endpoint,
        register_on_start,
        dial: cli.dial.clone(),
        input_device: cli.input_device.clone(),
        output_device: cli.output_device.clone(),
        sip_trace,
        test_duration: Duration::from_secs(cli.test_duration),
        test_timeout: Duration::from_secs(cli.test_timeout),
        test_dtmf: cli.test_dtmf,
        test_audio: cli.test_audio,
    })
}

fn load_endpoint_config(
    path: Option<PathBuf>,
    preset: Option<ConfigPreset>,
) -> anyhow::Result<EndpointConfig> {
    match (path, preset) {
        (Some(_), Some(_)) => anyhow::bail!("--preset and --config cannot be used together"),
        (Some(path), None) => {
            let text = fs::read_to_string(&path)?;
            Ok(serde_json::from_str::<EndpointConfig>(&text)?)
        }
        (None, Some(preset)) => preset.load(),
        (None, None) => Ok(EndpointConfig::default()),
    }
}

fn apply_sip_trace_overrides(
    config: &mut EndpointConfig,
    cli: &Cli,
) -> anyhow::Result<SipTraceOptions> {
    if matches!(cli.sip_trace_capacity, Some(0)) {
        anyhow::bail!("--sip-trace-capacity must be greater than zero");
    }

    let mut trace = config.sip_trace.clone().unwrap_or_default();
    if cli.sip_trace || cli.sip_trace_file.is_some() {
        trace.enabled = true;
    }
    if cli.sip_trace_no_redact {
        trace.redact_sensitive_headers = false;
    }
    if let Some(capacity) = cli.sip_trace_capacity {
        trace.capacity = capacity;
    }

    let enabled = trace.enabled;
    let options = SipTraceOptions {
        enabled,
        redact_sensitive_headers: trace.redact_sensitive_headers,
        capacity: trace.capacity.max(1),
        file: cli.sip_trace_file.clone(),
    };

    config.sip_trace = if enabled { Some(trace) } else { None };
    Ok(options)
}

fn apply_cli_overrides(config: &mut EndpointConfig, cli: &Cli) {
    if let Some(name) = cli.name.clone() {
        config.name = Some(name);
    }
    if let Some(profile) = cli.profile {
        config.profile = Some(profile.into());
    }
    if cli.username.is_some() || cli.password.is_some() || cli.registrar.is_some() {
        let account = config.account.get_or_insert_with(|| EndpointAccountConfig {
            registrar: String::new(),
            username: String::new(),
            auth_username: None,
            password: String::new(),
            expires: None,
            from_uri: None,
            contact_uri: None,
        });
        if let Some(username) = cli.username.clone() {
            account.username = username;
        }
        if let Some(auth_username) = cli.auth_username.clone() {
            account.auth_username = Some(auth_username);
        }
        if let Some(password) = cli.password.clone() {
            account.password = password;
        }
        if let Some(registrar) = cli.registrar.clone() {
            account.registrar = registrar;
        }
    }
    if cli.bind.is_some()
        || cli.advertise.is_some()
        || cli.transport.is_some()
        || cli.stun.is_some()
        || cli.outbound_proxy.is_some()
    {
        let network = config
            .network
            .get_or_insert_with(EndpointNetworkConfig::default);
        if let Some(bind) = cli.bind {
            network.bind = Some(bind);
        }
        if let Some(advertise) = cli.advertise {
            network.advertise = Some(advertise);
        }
        if let Some(transport) = cli.transport {
            network.transport = Some(transport.into());
        }
        if let Some(stun) = cli.stun.clone() {
            network.stun = Some(stun);
        }
        if let Some(proxy) = cli.outbound_proxy.clone() {
            network.outbound_proxy = Some(proxy);
        }
    }
    if cli.media_public.is_some() || cli.srtp.is_some() {
        let media = config
            .media
            .get_or_insert_with(EndpointMediaConfig::default);
        if let Some(public) = cli.media_public.clone() {
            media.public_address = Some(public);
        }
        if let Some(srtp) = cli.srtp {
            media.srtp = Some(srtp.into());
        }
    }
    if cli.register {
        config.register_on_start = Some(true);
    }
}

pub(crate) fn local_label(config: &EndpointConfig) -> String {
    if let Some(account) = config.account.as_ref() {
        if let Some((scheme, host)) = split_sip_uri_host(&account.registrar) {
            return format!("{scheme}:{}@{host}", account.username);
        }
    }
    let name = config.name.as_deref().unwrap_or("endpoint");
    let bind = config
        .bind
        .or(config.network.as_ref().and_then(|network| network.bind))
        .map(|addr| addr.to_string())
        .unwrap_or_else(|| "127.0.0.1:5060".into());
    format!("sip:{name}@{bind}")
}

fn split_sip_uri_host(uri: &str) -> Option<(&str, String)> {
    let (scheme, rest) = uri.split_once(':')?;
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let authority = rest
        .split(';')
        .next()
        .unwrap_or(rest)
        .split('?')
        .next()
        .unwrap_or(rest);
    let host = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority)
        .to_string();
    Some((scheme, host))
}

pub(crate) fn format_registration(info: &EndpointRegistrationInfo) -> String {
    match info.status {
        EndpointRegistrationStatus::Registered => format!(
            "registered to {}{}",
            info.registrar.as_deref().unwrap_or("registrar"),
            info.contact
                .as_ref()
                .map(|contact| format!(" as {contact}"))
                .unwrap_or_default()
        ),
        EndpointRegistrationStatus::Registering => "registering".into(),
        EndpointRegistrationStatus::Unregistering => "unregistering".into(),
        EndpointRegistrationStatus::Unregistered => "unregistered".into(),
        EndpointRegistrationStatus::Failed => format!(
            "registration failed: {}",
            info.last_failure.as_deref().unwrap_or("unknown error")
        ),
    }
}
