//! Unified PBX interop harness shared by the `pbx_endpoint`, `pbx_stream_peer`,
//! `pbx_callback_builder`, and `pbx_analyze` Cargo examples in this directory.
//!
//! The same scenario suite — registration/unregistration, basic call,
//! hold/resume, ring/cancel, DTMF, reject/busy, and blind transfer — is
//! exercised against both Asterisk and FreeSWITCH and through all three
//! public API surfaces ([`Endpoint`](rvoip_sip::Endpoint),
//! [`StreamPeer`](rvoip_sip::StreamPeer), and
//! [`CallbackPeer::builder`](rvoip_sip::CallbackPeerBuilder)) so provider
//! behaviour and surface ergonomics are validated in the same matrix.
//!
//! The runner (`examples/pbx/run.sh`) controls behaviour via these env vars:
//!
//! - `PBX_PROVIDER` (`asterisk`|`freeswitch`) — selects PBX defaults and SRTP
//!   policy
//! - `PBX_SCENARIO` (e.g. `registration`, `basic_call`, `hold_resume`,
//!   `ring_cancel`, `dtmf`, `reject`, `blind_transfer`) — chooses the scenario
//! - `PBX_TRANSPORT` (`udp`|`tls`) — selects the transport leg
//! - `PBX_ROLE` — selects the participant (caller/callee/transfer-target/etc.)
//!
//! Per-provider tunables (`SIP_PORT`, `SIP_TLS_PORT`, `SIP_PASSWORD`,
//! `ASTERISK_TLS_CONTACT_MODE`, `FREESWITCH_UDP_ADDR`, etc.) come from the
//! `env/asterisk.env` and `env/freeswitch.env` files loaded by `run.sh`.
//!
//! See `examples/pbx/README.md` for the full scenario matrix, evidence layout,
//! and provider differences.

#![allow(dead_code)]

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use rvoip_media_core::types::AudioFrame;
use rvoip_sip::{
    types::Credentials, AudioSender, CallHandlerDecision, CallId, CallbackPeer,
    CallbackPeerControl, Config, Endpoint, EndpointAccount, EndpointProfile, Event, EventReceiver,
    MediaSecurityKeying, MediaSecurityProfile, MediaSecurityState, Registration,
    RegistrationHandle, SessionHandle, SipContactMode, SrtpSuitePolicy, StreamPeer,
    TransferOutcome, TransferWaitMode,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

pub type ExampleResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub const SAMPLE_RATE: u32 = 8000;
pub const FRAME_SIZE: usize = 160;
pub const TONE_FRAMES: usize = 150;
pub const ENDPOINT_2001_TONE_HZ: f32 = 440.0;
pub const ENDPOINT_2002_TONE_HZ: f32 = 880.0;
pub const ENDPOINT_1001_TONE_HZ: f32 = ENDPOINT_2001_TONE_HZ;
pub const ENDPOINT_1002_TONE_HZ: f32 = ENDPOINT_2002_TONE_HZ;
pub const ENDPOINT_1003_TONE_HZ: f32 = 660.0;
pub const MIN_RECEIVED_SAMPLES: usize = 12_000;
pub const DOMINANCE_RATIO: f32 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PbxProvider {
    Asterisk,
    FreeSwitch,
}

impl PbxProvider {
    pub fn from_env_or_args() -> ExampleResult<Self> {
        let mut value = std::env::var("PBX_PROVIDER")
            .or_else(|_| std::env::var("PBX"))
            .unwrap_or_else(|_| "asterisk".to_string());
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--pbx" | "--provider" => {
                    value = args
                        .next()
                        .ok_or_else(|| format!("{} requires a value", arg))?;
                }
                _ => {}
            }
        }
        match value.trim().to_ascii_lowercase().as_str() {
            "asterisk" | "ast" => Ok(Self::Asterisk),
            "freeswitch" | "free-switch" | "fs" => Ok(Self::FreeSwitch),
            other => Err(format!("unknown PBX provider '{}'", other).into()),
        }
    }

    pub fn env_name(self) -> &'static str {
        match self {
            Self::Asterisk => "asterisk",
            Self::FreeSwitch => "freeswitch",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Asterisk => "Asterisk",
            Self::FreeSwitch => "FreeSWITCH",
        }
    }

    fn default_settle_secs(self) -> u64 {
        match self {
            Self::Asterisk => 5,
            Self::FreeSwitch => 2,
        }
    }

    fn default_retry_attempts(self) -> usize {
        match self {
            Self::Asterisk => 8,
            Self::FreeSwitch => 4,
        }
    }

    fn expects_target_cancel(self) -> bool {
        match self {
            Self::Asterisk => env_bool("ASTERISK_EXPECT_TARGET_CANCEL", false).unwrap_or(false),
            Self::FreeSwitch => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    Udp,
    TlsSrtp,
}

impl TransportMode {
    pub fn from_env_or_args() -> ExampleResult<Self> {
        let mut value = std::env::var("PBX_TRANSPORT")
            .or_else(|_| std::env::var("SIP_TRANSPORT"))
            .unwrap_or_else(|_| "udp".to_string());
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--transport" => {
                    value = args
                        .next()
                        .ok_or_else(|| "--transport requires a value".to_string())?;
                }
                _ => {}
            }
        }
        match value.trim().to_ascii_lowercase().as_str() {
            "udp" | "rtp" => Ok(Self::Udp),
            "tls" | "tls-srtp" | "srtp" => Ok(Self::TlsSrtp),
            other => Err(format!("unknown PBX transport '{}'", other).into()),
        }
    }

    pub fn is_tls(self) -> bool {
        self == Self::TlsSrtp
    }

    pub fn env_value(self) -> &'static str {
        match self {
            Self::Udp => "UDP",
            Self::TlsSrtp => "TLS",
        }
    }

    pub fn scenario_prefix(self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::TlsSrtp => "tls",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Registration,
    BasicCall,
    HoldResume,
    RingCancel,
    Dtmf,
    Reject,
    BlindTransfer,
}

impl Scenario {
    pub fn from_env_or_args() -> ExampleResult<Self> {
        let mut value = std::env::var("PBX_SCENARIO")
            .or_else(|_| std::env::var("CALLBACK_SCENARIO"))
            .unwrap_or_else(|_| "registration".to_string());
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--scenario" => {
                    value = args
                        .next()
                        .ok_or_else(|| "--scenario requires a value".to_string())?;
                }
                _ => {}
            }
        }
        let normalized = value
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
            .replace("tls_", "")
            .replace("udp_", "");
        match normalized.as_str() {
            "registration" | "registration_tls" | "registration_udp" => Ok(Self::Registration),
            "basic" | "basic_call" | "call" | "udp_call" => Ok(Self::BasicCall),
            "hold" | "hold_resume" => Ok(Self::HoldResume),
            "ring" | "ring_cancel" | "ring_remote" => Ok(Self::RingCancel),
            "dtmf" => Ok(Self::Dtmf),
            "reject" | "busy" => Ok(Self::Reject),
            "blind_transfer" | "blind_transfer_remote" | "transfer" => Ok(Self::BlindTransfer),
            other => Err(format!("unknown PBX scenario '{}'", other).into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Registration,
    Caller,
    Callee,
    Target,
    Transferor,
    Transferee,
}

impl Role {
    pub fn from_env_or_args() -> ExampleResult<Self> {
        let mut value = std::env::var("PBX_ROLE").unwrap_or_else(|_| "registration".to_string());
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--role" => {
                    value = args
                        .next()
                        .ok_or_else(|| "--role requires a value".to_string())?;
                }
                _ => {}
            }
        }
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "registration" | "register" => Ok(Self::Registration),
            "caller" | "uac" => Ok(Self::Caller),
            "callee" | "uas" => Ok(Self::Callee),
            "target" | "transfer_target" | "ring_target" => Ok(Self::Target),
            "transferor" => Ok(Self::Transferor),
            "transferee" => Ok(Self::Transferee),
            other => Err(format!("unknown PBX role '{}'", other).into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsContactMode {
    ReachableContact,
    RegisteredFlowRfc5626,
    RegisteredFlowSymmetric,
}

impl TlsContactMode {
    fn from_env(provider: PbxProvider) -> ExampleResult<Self> {
        if provider == PbxProvider::Asterisk && env_bool("ASTERISK_TLS_FLOW_REUSE", false)? {
            return Ok(Self::RegisteredFlowSymmetric);
        }
        let key = match provider {
            PbxProvider::Asterisk => "ASTERISK_TLS_CONTACT_MODE",
            PbxProvider::FreeSwitch => "FREESWITCH_TLS_CONTACT_MODE",
        };
        match env_string(key, "reachable-contact")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "reachable-contact" | "reachable" | "listener" | "uas" => Ok(Self::ReachableContact),
            "registered-flow" | "registered-flow-rfc5626" | "rfc5626" | "outbound" => {
                Ok(Self::RegisteredFlowRfc5626)
            }
            "registered-flow-symmetric" | "symmetric" | "symmetric-transport"
            | "flow-reuse" | "client-only" => Ok(Self::RegisteredFlowSymmetric),
            other => Err(format!(
                "{} must be reachable-contact, registered-flow-rfc5626, or registered-flow-symmetric, got '{}'",
                key, other
            )
            .into()),
        }
    }

    fn uses_listener(self) -> bool {
        self == Self::ReachableContact
    }

    fn sip_contact_mode(self) -> SipContactMode {
        match self {
            Self::ReachableContact => SipContactMode::ReachableContact,
            Self::RegisteredFlowRfc5626 => SipContactMode::RegisteredFlowRfc5626,
            Self::RegisteredFlowSymmetric => SipContactMode::RegisteredFlowSymmetric,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EndpointConfig {
    pub provider: PbxProvider,
    pub username: String,
    pub auth_username: String,
    pub password: String,
    pub sip_server: String,
    pub sip_port: u16,
    pub transport: TransportMode,
    pub local_ip: IpAddr,
    pub advertised_ip: IpAddr,
    pub media_advertised_ip: IpAddr,
    pub local_port: u16,
    pub tls_local_port: Option<u16>,
    pub tls_contact_mode: TlsContactMode,
    pub media_port_start: u16,
    pub media_port_end: u16,
    pub output_dir: PathBuf,
}

impl EndpointConfig {
    pub fn new(
        provider: PbxProvider,
        username: &str,
        transport: TransportMode,
    ) -> ExampleResult<Self> {
        let defaults = endpoint_defaults(provider, username, transport);
        let prefix = format!("ENDPOINT_{}", username);
        let (sip_server, sip_port) = match provider {
            PbxProvider::Asterisk => {
                let server = env_string("SIP_SERVER", "192.168.1.103");
                let port = if transport.is_tls() {
                    env_u16("SIP_TLS_PORT", 5061)?
                } else {
                    env_u16("SIP_PORT", 5060)?
                };
                (server, port)
            }
            PbxProvider::FreeSwitch => {
                let addr_key = if transport.is_tls() {
                    "FREESWITCH_TLS_ADDR"
                } else {
                    "FREESWITCH_UDP_ADDR"
                };
                let default_addr = if transport.is_tls() {
                    "127.0.0.1:5063"
                } else {
                    "127.0.0.1:5062"
                };
                split_host_port(&env_string(addr_key, default_addr))?
            }
        };
        let auth_username = auth_username_for(&prefix, username);
        let password = match provider {
            PbxProvider::Asterisk => std::env::var(format!("{}_PASSWORD", prefix))
                .or_else(|_| std::env::var("SIP_PASSWORD"))
                .unwrap_or_else(|_| "password123".to_string()),
            PbxProvider::FreeSwitch => std::env::var(format!("{}_PASSWORD", prefix))
                .or_else(|_| std::env::var("FREESWITCH_PASSWORD"))
                .or_else(|_| std::env::var("SIP_PASSWORD"))
                .unwrap_or_else(|_| "1234".to_string()),
        };
        let local_ip: IpAddr = match provider {
            PbxProvider::Asterisk => env_string("LOCAL_IP", "0.0.0.0").parse()?,
            PbxProvider::FreeSwitch => std::env::var("RVOIP_LOCAL_IP")
                .or_else(|_| std::env::var("LOCAL_IP"))
                .unwrap_or_else(|_| "127.0.0.1".to_string())
                .parse()?,
        };
        let advertised_ip = advertised_ip(provider, local_ip)?;
        let media_advertised_ip = media_advertised_ip(provider, advertised_ip)?;
        let local_port = env_u16(&format!("{}_LOCAL_PORT", prefix), defaults.local_port)?;
        let tls_contact_mode = TlsContactMode::from_env(provider)?;
        let tls_local_port = if transport.is_tls() {
            Some(env_u16(
                &format!("{}_TLS_LOCAL_PORT", prefix),
                defaults
                    .tls_local_port
                    .unwrap_or(defaults.local_port.saturating_add(1)),
            )?)
        } else {
            None
        };
        let media_port_start = env_u16(
            &format!("{}_MEDIA_PORT_START", prefix),
            defaults.media_port_start,
        )?;
        let media_port_end = env_u16(
            &format!("{}_MEDIA_PORT_END", prefix),
            defaults.media_port_end,
        )?;
        let output_dir = std::env::var("AUDIO_OUTPUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("examples/pbx/output")
                    .join(provider.env_name())
            });

        Ok(Self {
            provider,
            username: username.to_string(),
            auth_username,
            password,
            sip_server,
            sip_port,
            transport,
            local_ip,
            advertised_ip,
            media_advertised_ip,
            local_port,
            tls_local_port,
            tls_contact_mode,
            media_port_start,
            media_port_end,
            output_dir,
        })
    }

    pub fn registrar_uri(&self) -> String {
        format!(
            "{}:{}:{}{}",
            self.uri_scheme(),
            self.sip_server,
            self.sip_port,
            transport_suffix(self.transport)
        )
    }

    pub fn aor_uri(&self) -> String {
        format!(
            "{}:{}@{}",
            self.uri_scheme(),
            self.username,
            self.sip_server
        )
    }

    pub fn contact_uri(&self) -> String {
        format!(
            "{}:{}@{}:{}{}",
            self.uri_scheme(),
            self.username,
            self.advertised_ip,
            self.contact_port(),
            transport_suffix(self.transport)
        )
    }

    pub fn call_uri(&self, target: &str) -> String {
        if self.transport.is_tls() || self.sip_port != default_pbx_port(self.transport) {
            format!(
                "{}:{}@{}:{}{}",
                self.uri_scheme(),
                target,
                self.sip_server,
                self.sip_port,
                transport_suffix(self.transport)
            )
        } else {
            format!("sip:{}@{}", target, self.sip_server)
        }
    }

    pub fn outbound_call_uri(&self, target: &str) -> String {
        let key = format!("ENDPOINT_{}_CALL_URI", self.username);
        std::env::var(&key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.call_uri(target))
    }

    pub fn remote_user(&self) -> &'static str {
        if self.transport.is_tls() {
            "1003"
        } else {
            "2003"
        }
    }

    pub fn remote_call_uri(&self) -> String {
        let override_key = if self.transport.is_tls() {
            "REMOTE_TLS_CALL_URI"
        } else {
            "REMOTE_UDP_CALL_URI"
        };
        std::env::var(override_key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.call_uri(self.remote_user()))
    }

    pub fn stream_config(&self) -> Config {
        let mut config = match self.provider {
            PbxProvider::Asterisk => Config::on(&self.username, self.local_ip, self.local_port),
            PbxProvider::FreeSwitch => Config::freeswitch_internal(
                &self.username,
                SocketAddr::new(self.local_ip, self.local_port),
            ),
        };
        config.local_uri = self.aor_uri();
        config.contact_uri = Some(self.contact_uri());
        config.sip_advertised_addr = Some(SocketAddr::new(self.advertised_ip, self.local_port));
        if self.transport.is_tls() {
            config.tls_advertised_addr =
                Some(SocketAddr::new(self.advertised_ip, self.contact_port()));
        }
        config.sip_contact_mode = if self.transport.is_tls() {
            self.tls_contact_mode.sip_contact_mode()
        } else {
            SipContactMode::ReachableContact
        };
        config.credentials = Some(Credentials::new(&self.auth_username, &self.password));
        config.media_port_start = self.media_port_start;
        config.media_port_end = self.media_port_end;
        config.media_public_addr = Some(SocketAddr::new(self.media_advertised_ip, 0));
        config
    }

    pub fn session_config(&self) -> ExampleResult<Config> {
        if !self.transport.is_tls() {
            return Ok(self.stream_config());
        }

        let mut config = self.stream_config();
        match self.tls_contact_mode {
            TlsContactMode::ReachableContact => {
                let tls_port = self.tls_local_port.ok_or_else(|| {
                    "TLS reachable-contact mode requires ENDPOINT_<user>_TLS_LOCAL_PORT".to_string()
                })?;
                config = config.tls_reachable_contact(
                    SocketAddr::new(self.local_ip, tls_port),
                    required_path("TLS_CERT_PATH")?,
                    required_path("TLS_KEY_PATH")?,
                );
            }
            TlsContactMode::RegisteredFlowRfc5626 => {
                config = config.tls_registered_flow_rfc5626(self.sip_instance_urn());
            }
            TlsContactMode::RegisteredFlowSymmetric => {
                config = config.tls_registered_flow_symmetric(self.sip_instance_urn());
            }
        }
        config.tls_extra_ca_path = optional_path("TLS_CA_PATH");
        config.tls_client_cert_path = optional_path("TLS_CLIENT_CERT_PATH");
        config.tls_client_key_path = optional_path("TLS_CLIENT_KEY_PATH");
        #[cfg(feature = "dev-insecure-tls")]
        {
            let default_insecure = self.provider == PbxProvider::FreeSwitch;
            config.tls_insecure_skip_verify = env_bool("TLS_INSECURE", default_insecure)?;
        }
        config.offer_srtp = true;
        config.srtp_required = match self.provider {
            PbxProvider::Asterisk => env_bool("ASTERISK_TLS_SRTP_REQUIRED", true)?,
            PbxProvider::FreeSwitch => env_bool("FREESWITCH_TLS_SRTP_REQUIRED", true)?,
        };
        if self.provider == PbxProvider::FreeSwitch {
            config = config.with_srtp_suite_policy(SrtpSuitePolicy::FreeSwitchCompatible);
        }
        Ok(config)
    }

    pub fn registration(&self) -> Registration {
        Registration::new(self.registrar_uri(), &self.auth_username, &self.password)
            .from_uri(self.aor_uri())
            .contact_uri(self.contact_uri())
    }

    pub fn endpoint_account(&self) -> EndpointAccount {
        EndpointAccount::new(self.registrar_uri(), &self.username, &self.password)
            .auth_username(&self.auth_username)
            .from_uri(self.aor_uri())
            .contact_uri(self.contact_uri())
    }

    fn uri_scheme(&self) -> &'static str {
        if self.transport.is_tls() {
            "sips"
        } else {
            "sip"
        }
    }

    fn contact_port(&self) -> u16 {
        if self.transport.is_tls() && self.tls_contact_mode.uses_listener() {
            self.tls_local_port
                .unwrap_or(self.local_port.saturating_add(1))
        } else {
            self.local_port
        }
    }

    fn sip_instance_urn(&self) -> String {
        std::env::var(format!("ENDPOINT_{}_SIP_INSTANCE", self.username))
            .or_else(|_| std::env::var("SIP_INSTANCE"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| deterministic_sip_instance(&self.username))
    }
}

#[derive(Debug, Clone, Copy)]
struct EndpointDefaults {
    local_port: u16,
    tls_local_port: Option<u16>,
    media_port_start: u16,
    media_port_end: u16,
}

pub struct ToneAnalysis {
    pub samples: usize,
    pub expected_hz: f32,
    pub rejected_hz: f32,
    pub expected_magnitude: f32,
    pub rejected_magnitude: f32,
    pub ratio: f32,
}

pub struct ToneRecorder {
    running: Arc<AtomicBool>,
    send_task: JoinHandle<()>,
    recv_task: JoinHandle<()>,
    received_buf: Arc<Mutex<Vec<i16>>>,
}

#[derive(Debug, Clone, Copy)]
pub enum IncomingMode {
    Accept,
    RejectBusy,
    Defer(Duration),
}

pub enum CallbackEvent {
    Incoming {
        call_id: CallId,
        from: String,
        to: String,
    },
    Established(SessionHandle),
    Progress {
        call_id: CallId,
        status_code: u16,
        reason: String,
        sdp: Option<String>,
    },
    Ended {
        call_id: CallId,
        reason: String,
    },
    Failed {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
    Cancelled {
        call_id: CallId,
    },
    Dtmf {
        call_id: CallId,
        digit: char,
    },
    MediaSecurity {
        call_id: CallId,
        state: MediaSecurityState,
    },
    LocalHold {
        call_id: CallId,
    },
    LocalResume {
        call_id: CallId,
    },
    RemoteHold {
        call_id: CallId,
    },
    RemoteResume {
        call_id: CallId,
    },
    TransferAccepted {
        call_id: CallId,
        refer_to: String,
    },
    ReferProgress {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
    ReferCompleted {
        call_id: CallId,
        target: String,
        status_code: u16,
        reason: String,
    },
    TransferFailed {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
    RegistrationSuccess {
        registrar: String,
        expires: u32,
        contact: String,
    },
    UnregistrationSuccess {
        registrar: String,
    },
}

pub struct CallbackRuntime {
    pub cfg: EndpointConfig,
    pub control: CallbackPeerControl,
    pub events: mpsc::UnboundedReceiver<CallbackEvent>,
    run_task: JoinHandle<rvoip_sip::Result<()>>,
}

impl CallbackRuntime {
    pub async fn shutdown(self) -> ExampleResult<()> {
        self.control.shutdown();
        let _ = timeout(Duration::from_secs(3), self.run_task).await;
        Ok(())
    }
}

pub fn load_env(provider: PbxProvider) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Ok(home) = std::env::var("HOME") {
        match provider {
            PbxProvider::Asterisk => {
                let _ = dotenvy::from_filename(
                    Path::new(&home)
                        .join("Developer")
                        .join("asterisk")
                        .join("rvoip-local.env"),
                );
            }
            PbxProvider::FreeSwitch => {
                let _ = dotenvy::from_filename(
                    Path::new(&home)
                        .join("Developer")
                        .join("freeswitch")
                        .join("freeswitch-local.env"),
                );
            }
        }
    }
    let _ = dotenvy::from_filename(
        manifest
            .join("examples/pbx/env")
            .join(format!("{}.env", provider.env_name())),
    );
    let _ = dotenvy::from_filename(manifest.join("examples/pbx/.env.local"));
    let _ = dotenvy::dotenv();
}

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,rvoip_sip_dialog=warn".into()),
        )
        .try_init();
}

pub fn context() -> ExampleResult<(PbxProvider, Scenario, TransportMode, Role)> {
    let provider = PbxProvider::from_env_or_args()?;
    load_env(provider);
    init_tracing();
    Ok((
        provider,
        Scenario::from_env_or_args()?,
        TransportMode::from_env_or_args()?,
        Role::from_env_or_args()?,
    ))
}

pub fn username_for(transport: TransportMode, role: Role) -> &'static str {
    match (transport, role) {
        (TransportMode::TlsSrtp, Role::Caller | Role::Transferor | Role::Registration) => "1001",
        (TransportMode::TlsSrtp, Role::Callee | Role::Transferee) => "1002",
        (TransportMode::TlsSrtp, Role::Target) => "1003",
        (TransportMode::Udp, Role::Caller | Role::Transferor | Role::Registration) => "2001",
        (TransportMode::Udp, Role::Callee | Role::Transferee) => "2002",
        (TransportMode::Udp, Role::Target) => "2003",
    }
}

pub fn endpoint_config_for(
    provider: PbxProvider,
    transport: TransportMode,
    role: Role,
) -> ExampleResult<EndpointConfig> {
    EndpointConfig::new(provider, username_for(transport, role), transport)
}

pub async fn new_stream_peer(cfg: &EndpointConfig) -> ExampleResult<StreamPeer> {
    Ok(StreamPeer::with_config(cfg.session_config()?).await?)
}

pub async fn new_endpoint(cfg: &EndpointConfig) -> ExampleResult<Endpoint> {
    Ok(Endpoint::builder()
        .name(&cfg.username)
        .endpoint_account(cfg.endpoint_account())
        .profile(EndpointProfile::Custom(cfg.session_config()?))
        .build()
        .await?)
}

pub async fn callback_runtime(
    provider: PbxProvider,
    transport: TransportMode,
    role: Role,
    mode: IncomingMode,
) -> ExampleResult<CallbackRuntime> {
    let cfg = endpoint_config_for(provider, transport, role)?;
    let (tx, events) = mpsc::unbounded_channel();
    let incoming_tx = tx.clone();
    let established_tx = tx.clone();
    let progress_tx = tx.clone();
    let ended_tx = tx.clone();
    let failed_tx = tx.clone();
    let cancelled_tx = tx.clone();
    let dtmf_tx = tx.clone();
    let media_security_tx = tx.clone();
    let local_hold_tx = tx.clone();
    let local_resume_tx = tx.clone();
    let remote_hold_tx = tx.clone();
    let remote_resume_tx = tx.clone();
    let transfer_accepted_tx = tx.clone();
    let refer_progress_tx = tx.clone();
    let refer_completed_tx = tx.clone();
    let transfer_failed_tx = tx.clone();
    let registration_tx = tx.clone();
    let unregistration_tx = tx;

    let peer = CallbackPeer::builder(cfg.session_config()?)
        .on_incoming(move |call| {
            let tx = incoming_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Incoming {
                    call_id: call.call_id.clone(),
                    from: call.from.clone(),
                    to: call.to.clone(),
                });
                match mode {
                    IncomingMode::Accept => CallHandlerDecision::Accept,
                    IncomingMode::RejectBusy => CallHandlerDecision::Reject {
                        status: 486,
                        reason: "Busy Here".to_string(),
                    },
                    IncomingMode::Defer(duration) => {
                        CallHandlerDecision::Defer(call.defer(duration))
                    }
                }
            }
        })
        .on_established(move |handle| {
            let tx = established_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Established(handle));
                Ok(())
            }
        })
        .on_progress(move |handle, status_code, reason, sdp| {
            let tx = progress_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Progress {
                    call_id: handle.id().clone(),
                    status_code,
                    reason,
                    sdp,
                });
                Ok(())
            }
        })
        .on_ended(move |call_id, reason| {
            let tx = ended_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Ended {
                    call_id,
                    reason: format!("{reason:?}"),
                });
                Ok(())
            }
        })
        .on_failed(move |call_id, status_code, reason| {
            let tx = failed_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Failed {
                    call_id,
                    status_code,
                    reason,
                });
                Ok(())
            }
        })
        .on_cancelled(move |call_id| {
            let tx = cancelled_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Cancelled { call_id });
                Ok(())
            }
        })
        .on_dtmf(move |handle, digit| {
            let tx = dtmf_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::Dtmf {
                    call_id: handle.id().clone(),
                    digit,
                });
                Ok(())
            }
        })
        .on_media_security(move |handle, state| {
            let tx = media_security_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::MediaSecurity {
                    call_id: handle.id().clone(),
                    state,
                });
                Ok(())
            }
        })
        .on_transfer_request(|handle, target| async move {
            println!(
                "[callback-transfer] accepting REFER on call {} to {}",
                handle.id(),
                target
            );
            Ok(true)
        })
        .on_local_hold(move |handle| {
            let tx = local_hold_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::LocalHold {
                    call_id: handle.id().clone(),
                });
                Ok(())
            }
        })
        .on_local_resume(move |handle| {
            let tx = local_resume_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::LocalResume {
                    call_id: handle.id().clone(),
                });
                Ok(())
            }
        })
        .on_remote_hold(move |handle| {
            let tx = remote_hold_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::RemoteHold {
                    call_id: handle.id().clone(),
                });
                Ok(())
            }
        })
        .on_remote_resume(move |handle| {
            let tx = remote_resume_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::RemoteResume {
                    call_id: handle.id().clone(),
                });
                Ok(())
            }
        })
        .on_transfer_accepted(move |handle, refer_to| {
            let tx = transfer_accepted_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::TransferAccepted {
                    call_id: handle.id().clone(),
                    refer_to,
                });
                Ok(())
            }
        })
        .on_refer_progress(move |handle, status_code, reason| {
            let tx = refer_progress_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::ReferProgress {
                    call_id: handle.id().clone(),
                    status_code,
                    reason,
                });
                Ok(())
            }
        })
        .on_refer_completed(move |handle, target, status_code, reason| {
            let tx = refer_completed_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::ReferCompleted {
                    call_id: handle.id().clone(),
                    target,
                    status_code,
                    reason,
                });
                Ok(())
            }
        })
        .on_transfer_failed(move |handle, status_code, reason| {
            let tx = transfer_failed_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::TransferFailed {
                    call_id: handle.id().clone(),
                    status_code,
                    reason,
                });
                Ok(())
            }
        })
        .on_registration_success(move |registrar, expires, contact| {
            let tx = registration_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::RegistrationSuccess {
                    registrar,
                    expires,
                    contact,
                });
                Ok(())
            }
        })
        .on_unregistration_success(move |registrar| {
            let tx = unregistration_tx.clone();
            async move {
                let _ = tx.send(CallbackEvent::UnregistrationSuccess { registrar });
                Ok(())
            }
        })
        .build()
        .await?;
    let control = peer.control();
    let run_task = tokio::spawn(async move { peer.run().await });
    sleep(Duration::from_millis(100)).await;
    Ok(CallbackRuntime {
        cfg,
        control,
        events,
        run_task,
    })
}

pub async fn register_stream_peer(
    peer: &mut StreamPeer,
    cfg: &EndpointConfig,
) -> ExampleResult<RegistrationHandle> {
    print_registration_context(cfg);
    let handle = peer.register_with(cfg.registration()).await?;
    wait_for_stream_registration(peer, &handle, &cfg.username).await?;
    println!("[{}] Registered.", cfg.username);
    Ok(handle)
}

pub async fn register_endpoint_api(
    endpoint: &mut Endpoint,
    cfg: &EndpointConfig,
) -> ExampleResult<RegistrationHandle> {
    print_registration_context(cfg);
    let handle = endpoint.register().await?;
    for _ in 0..50 {
        if endpoint
            .control()
            .coordinator()
            .is_registered(&handle)
            .await?
        {
            println!("[{}] Registered.", cfg.username);
            return Ok(handle);
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!("endpoint {} did not register within 10s", cfg.username).into())
}

pub async fn register_callback_endpoint(
    runtime: &mut CallbackRuntime,
) -> ExampleResult<RegistrationHandle> {
    print_registration_context(&runtime.cfg);
    let handle = runtime
        .control
        .register_with(runtime.cfg.registration())
        .await?;
    for _ in 0..50 {
        if runtime.control.is_registered(&handle).await? {
            wait_for_registration_success(&mut runtime.events, Duration::from_secs(10)).await?;
            println!("[{}] Registered.", runtime.cfg.username);
            return Ok(handle);
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!(
        "callback endpoint {} did not register within 10s",
        runtime.cfg.username
    )
    .into())
}

pub async fn unregister_callback_endpoint(
    runtime: &mut CallbackRuntime,
    handle: &RegistrationHandle,
) -> ExampleResult<()> {
    runtime.control.unregister(handle).await?;
    wait_for_unregistration_success(&mut runtime.events, Duration::from_secs(10)).await?;
    println!("[{}] Unregistered.", runtime.cfg.username);
    Ok(())
}

pub async fn run_stream_peer_surface() -> ExampleResult<()> {
    let (provider, scenario, transport, role) = context()?;
    run_stream_peer(provider, scenario, transport, role).await
}

pub async fn run_endpoint_surface() -> ExampleResult<()> {
    let (provider, scenario, transport, role) = context()?;
    run_endpoint(provider, scenario, transport, role).await
}

pub async fn run_callback_builder_surface() -> ExampleResult<()> {
    let (provider, scenario, transport, role) = context()?;
    run_callback(provider, scenario, transport, role).await
}

async fn run_stream_peer(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
) -> ExampleResult<()> {
    let cfg = endpoint_config_for(provider, transport, role)?;
    let mut peer = new_stream_peer(&cfg).await?;
    let registration = register_stream_peer(&mut peer, &cfg).await?;
    match scenario {
        Scenario::Registration => {
            sleep(idle_duration()).await;
        }
        Scenario::BasicCall
        | Scenario::HoldResume
        | Scenario::RingCancel
        | Scenario::Dtmf
        | Scenario::Reject => {
            run_stream_peer_two_party(provider, scenario, transport, role, &cfg, &mut peer).await?;
        }
        Scenario::BlindTransfer => {
            run_stream_peer_transfer(provider, transport, role, &cfg, &mut peer).await?;
        }
    }
    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    Ok(())
}

async fn run_endpoint(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
) -> ExampleResult<()> {
    let cfg = endpoint_config_for(provider, transport, role)?;
    let mut endpoint = new_endpoint(&cfg).await?;
    register_endpoint_api(&mut endpoint, &cfg).await?;
    match scenario {
        Scenario::Registration => {
            sleep(idle_duration()).await;
        }
        Scenario::BasicCall
        | Scenario::HoldResume
        | Scenario::RingCancel
        | Scenario::Dtmf
        | Scenario::Reject => {
            run_endpoint_two_party(provider, scenario, transport, role, &cfg, &mut endpoint)
                .await?;
        }
        Scenario::BlindTransfer => {
            run_endpoint_transfer(provider, transport, role, &cfg, &mut endpoint).await?;
        }
    }
    endpoint.unregister().await.ok();
    endpoint.shutdown().await.ok();
    Ok(())
}

async fn run_callback(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
) -> ExampleResult<()> {
    let mode = match (scenario, role) {
        (Scenario::Reject, Role::Callee) => IncomingMode::RejectBusy,
        (Scenario::RingCancel, Role::Target) => IncomingMode::Defer(Duration::from_secs(30)),
        (_, Role::Callee | Role::Target | Role::Transferee) => IncomingMode::Accept,
        _ => IncomingMode::RejectBusy,
    };
    let mut runtime = callback_runtime(provider, transport, role, mode).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    match scenario {
        Scenario::Registration => {
            sleep(idle_duration()).await;
        }
        Scenario::BasicCall
        | Scenario::HoldResume
        | Scenario::RingCancel
        | Scenario::Dtmf
        | Scenario::Reject => {
            run_callback_two_party(provider, scenario, transport, role, &mut runtime).await?;
        }
        Scenario::BlindTransfer => {
            run_callback_transfer(transport, role, &mut runtime).await?;
        }
    }
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_stream_peer_two_party(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    peer: &mut StreamPeer,
) -> ExampleResult<()> {
    match (scenario, role) {
        (Scenario::BasicCall, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = call_with_answer_retry(
                peer,
                &cfg.outbound_call_uri("2002"),
                remote_test_timeout(provider)?,
            )
            .await?;
            sleep(Duration::from_secs(2)).await;
            handle.hangup_and_wait(Some(Duration::from_secs(8))).await?;
        }
        (Scenario::BasicCall, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            handle
                .wait_for_end(Some(remote_test_timeout(provider)?))
                .await
                .ok();
        }
        (Scenario::HoldResume, Role::Caller) => {
            settle_after_register(provider).await;
            let target = cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                call_with_answer_retry(peer, &target, remote_test_timeout(provider)?).await?;
            run_hold_on_handle(provider, cfg, &handle, transport).await?;
        }
        (Scenario::HoldResume, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_answering_tone_role(
                cfg,
                &handle,
                tone_for_callee(transport),
                hold_resume_callee_wav(transport),
                transport,
            )
            .await?;
        }
        (Scenario::RingCancel, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = call_with_ringing_retry(
                peer,
                &cfg.remote_call_uri(),
                remote_test_timeout(provider)?,
            )
            .await?;
            let mut events = handle.events().await?;
            handle
                .hangup_and_wait(Some(Duration::from_secs(12)))
                .await?;
            wait_for_call_cancelled_on_events(&mut events, Duration::from_secs(12))
                .await
                .ok();
        }
        (Scenario::RingCancel, Role::Target) => run_deferred_target(provider, peer, cfg).await?,
        (Scenario::Dtmf, Role::Caller) => {
            settle_after_register(provider).await;
            let target = target_user_for(transport);
            let handle = call_with_answer_retry(
                peer,
                &cfg.outbound_call_uri(target),
                remote_test_timeout(provider)?,
            )
            .await?;
            run_dtmf_caller(cfg, &handle, transport).await?;
        }
        (Scenario::Dtmf, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_dtmf_callee(provider, cfg, &handle, transport).await?;
        }
        (Scenario::Reject, Role::Caller) => {
            settle_after_register(provider).await;
            let target = target_user_for(transport);
            let call_id = peer.invite(cfg.outbound_call_uri(target)).send().await?;
            let handle = peer.coordinator().session(&call_id);
            let mut events = handle.events().await?;
            let (status, _) =
                wait_for_call_failed_on_events(&mut events, remote_test_timeout(provider)?).await?;
            if status != 486 {
                return Err(format!("expected 486 Busy Here, got {}", status).into());
            }
        }
        (Scenario::Reject, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            incoming.reject(486, "Busy Here");
            sleep(Duration::from_secs(1)).await;
        }
        _ => {
            return Err(format!("unsupported StreamPeer role {:?} for {:?}", role, scenario).into())
        }
    }
    Ok(())
}

async fn run_endpoint_two_party(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    endpoint: &mut Endpoint,
) -> ExampleResult<()> {
    match (scenario, role) {
        (Scenario::BasicCall, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait("2002", Some(remote_test_timeout(provider)?))
                .await?;
            sleep(Duration::from_secs(2)).await;
            handle.hangup_and_wait(Some(Duration::from_secs(8))).await?;
        }
        (Scenario::BasicCall, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            handle
                .wait_for_end(Some(remote_test_timeout(provider)?))
                .await
                .ok();
        }
        (Scenario::HoldResume, Role::Caller) => {
            settle_after_register(provider).await;
            let target = cfg.outbound_call_uri(target_user_for(transport));
            let call_id = endpoint.invite(&target)?.send().await?;
            let handle = endpoint
                .wrap_call(call_id)
                .wait_for_answered(Some(remote_test_timeout(provider)?))
                .await?;
            run_hold_on_handle(provider, cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::HoldResume, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_answering_tone_role(
                cfg,
                handle.as_session_handle(),
                tone_for_callee(transport),
                hold_resume_callee_wav(transport),
                transport,
            )
            .await?;
        }
        (Scenario::RingCancel, Role::Caller) => {
            settle_after_register(provider).await;
            let call_id = endpoint.invite(&cfg.remote_call_uri())?.send().await?;
            let handle = endpoint.wrap_call(call_id);
            handle
                .as_session_handle()
                .wait_for_progress(
                    |event| {
                        matches!(
                            event,
                            Event::CallProgress {
                                status_code: 180 | 183,
                                ..
                            }
                        )
                    },
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            let mut events = handle.as_session_handle().events().await?;
            handle
                .hangup_and_wait(Some(Duration::from_secs(12)))
                .await?;
            wait_for_call_cancelled_on_events(&mut events, Duration::from_secs(12))
                .await
                .ok();
        }
        (Scenario::RingCancel, Role::Target) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let guard = incoming.defer(Duration::from_secs(30));
            let result = guard
                .wait_for_cancelled(Some(Duration::from_secs(12)))
                .await;
            if provider.expects_target_cancel() {
                result?;
            }
        }
        (Scenario::Dtmf, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait(
                    target_user_for(transport),
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            run_dtmf_caller(cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::Dtmf, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_dtmf_callee(provider, cfg, handle.as_session_handle(), transport).await?;
        }
        (Scenario::Reject, Role::Caller) => {
            settle_after_register(provider).await;
            let call_id = endpoint.invite(target_user_for(transport))?.send().await?;
            let handle = endpoint.wrap_call(call_id);
            let mut events = handle.as_session_handle().events().await?;
            let (status, _) =
                wait_for_call_failed_on_events(&mut events, remote_test_timeout(provider)?).await?;
            if status != 486 {
                return Err(format!("expected 486 Busy Here, got {}", status).into());
            }
        }
        (Scenario::Reject, Role::Callee) => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            incoming.reject(486, "Busy Here").await?;
            sleep(Duration::from_secs(1)).await;
        }
        _ => return Err(format!("unsupported Endpoint role {:?} for {:?}", role, scenario).into()),
    }
    Ok(())
}

async fn run_callback_two_party(
    provider: PbxProvider,
    scenario: Scenario,
    transport: TransportMode,
    role: Role,
    runtime: &mut CallbackRuntime,
) -> ExampleResult<()> {
    match (scenario, role) {
        (Scenario::BasicCall, Role::Caller) => {
            settle_after_register(provider).await;
            let handle = callback_call_with_answer_retry(
                runtime,
                &runtime.cfg.outbound_call_uri("2002"),
                remote_test_timeout(provider)?,
            )
            .await?;
            sleep(Duration::from_secs(2)).await;
            handle.hangup_and_wait(Some(Duration::from_secs(8))).await?;
        }
        (Scenario::BasicCall, Role::Callee) => {
            let handle =
                wait_for_next_established(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            handle
                .wait_for_end(Some(remote_test_timeout(provider)?))
                .await
                .ok();
        }
        (Scenario::HoldResume, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                callback_call_with_answer_retry(runtime, &target, remote_test_timeout(provider)?)
                    .await?;
            run_hold_on_handle(provider, &runtime.cfg, &handle, transport).await?;
            wait_for_local_hold_resume(&mut runtime.events, Duration::from_secs(15)).await?;
        }
        (Scenario::HoldResume, Role::Callee) => {
            let handle =
                wait_for_next_established(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            run_answering_tone_role(
                &runtime.cfg,
                &handle,
                tone_for_callee(transport),
                hold_resume_callee_wav(transport),
                transport,
            )
            .await?;
        }
        (Scenario::RingCancel, Role::Caller) => {
            settle_after_register(provider).await;
            let call_id = runtime
                .control
                .invite(runtime.cfg.remote_call_uri())
                .send()
                .await?;
            let handle = runtime.control.coordinator().session(&call_id);
            wait_for_callback_progress(
                &mut runtime.events,
                handle.id(),
                remote_test_timeout(provider)?,
            )
            .await?;
            handle
                .hangup_and_wait(Some(Duration::from_secs(12)))
                .await?;
            wait_for_cancelled(
                &mut runtime.events,
                Some(handle.id()),
                Duration::from_secs(12),
            )
            .await
            .ok();
        }
        (Scenario::RingCancel, Role::Target) => {
            let call_id =
                wait_for_incoming_notice(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            let result =
                wait_for_cancelled(&mut runtime.events, Some(&call_id), Duration::from_secs(12))
                    .await;
            if provider.expects_target_cancel() {
                result?;
            }
        }
        (Scenario::Dtmf, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle =
                callback_call_with_answer_retry(runtime, &target, remote_test_timeout(provider)?)
                    .await?;
            run_dtmf_caller(&runtime.cfg, &handle, transport).await?;
        }
        (Scenario::Dtmf, Role::Callee) => {
            let handle =
                wait_for_next_established(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            let recorder = if transport.is_tls() {
                Some(start_tone_recorder(&handle, tone_for_callee(transport)).await?)
            } else {
                None
            };
            wait_for_dtmf_sequence(
                &mut runtime.events,
                &remote_test_digits(provider),
                remote_test_timeout(provider)?,
            )
            .await?;
            handle
                .wait_for_end(Some(Duration::from_secs(15)))
                .await
                .ok();
            if let Some(recorder) = recorder {
                recorder
                    .stop_and_save(&runtime.cfg.output_dir, dtmf_callee_wav(transport))
                    .await?;
            }
        }
        (Scenario::Reject, Role::Caller) => {
            settle_after_register(provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let call_id = runtime.control.invite(target).send().await?;
            let handle = runtime.control.coordinator().session(&call_id);
            wait_for_call_failed(
                &mut runtime.events,
                handle.id(),
                486,
                remote_test_timeout(provider)?,
            )
            .await?;
        }
        (Scenario::Reject, Role::Callee) => {
            let _call_id =
                wait_for_incoming_notice(&mut runtime.events, remote_test_timeout(provider)?)
                    .await?;
            sleep(Duration::from_secs(1)).await;
        }
        _ => return Err(format!("unsupported Callback role {:?} for {:?}", role, scenario).into()),
    }
    Ok(())
}

async fn run_stream_peer_transfer(
    provider: PbxProvider,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    peer: &mut StreamPeer,
) -> ExampleResult<()> {
    match role {
        Role::Transferor => {
            settle_after_register(provider).await;
            let handle = call_with_answer_retry(
                peer,
                &cfg.outbound_call_uri(target_user_for(transport)),
                remote_test_timeout(provider)?,
            )
            .await?;
            run_transferor(provider, cfg, &handle, transport).await?;
        }
        Role::Transferee => {
            let incoming =
                timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_transfer_answering_role(cfg, &handle, transport, true).await?;
        }
        Role::Target => {
            let incoming = timeout(Duration::from_secs(90), peer.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_transfer_answering_role(cfg, &handle, transport, false).await?;
        }
        _ => return Err(format!("unsupported transfer role {:?}", role).into()),
    }
    Ok(())
}

async fn run_endpoint_transfer(
    provider: PbxProvider,
    transport: TransportMode,
    role: Role,
    cfg: &EndpointConfig,
    endpoint: &mut Endpoint,
) -> ExampleResult<()> {
    match role {
        Role::Transferor => {
            settle_after_register(provider).await;
            let handle = endpoint
                .call_and_wait(
                    target_user_for(transport),
                    Some(remote_test_timeout(provider)?),
                )
                .await?;
            run_transferor(provider, cfg, handle.as_session_handle(), transport).await?;
        }
        Role::Transferee => {
            let incoming =
                timeout(remote_test_timeout(provider)?, endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_transfer_answering_role(cfg, handle.as_session_handle(), transport, true).await?;
        }
        Role::Target => {
            let incoming = timeout(Duration::from_secs(90), endpoint.wait_for_incoming()).await??;
            let handle = incoming.accept().await?;
            run_transfer_answering_role(cfg, handle.as_session_handle(), transport, false).await?;
        }
        _ => return Err(format!("unsupported transfer role {:?}", role).into()),
    }
    Ok(())
}

async fn run_callback_transfer(
    transport: TransportMode,
    role: Role,
    runtime: &mut CallbackRuntime,
) -> ExampleResult<()> {
    match role {
        Role::Transferor => {
            settle_after_register(runtime.cfg.provider).await;
            let target = runtime.cfg.outbound_call_uri(target_user_for(transport));
            let handle = callback_call_with_answer_retry(
                runtime,
                &target,
                remote_test_timeout(runtime.cfg.provider)?,
            )
            .await?;
            run_transferor(runtime.cfg.provider, &runtime.cfg, &handle, transport).await?;
        }
        Role::Transferee => {
            let handle = wait_for_next_established(
                &mut runtime.events,
                remote_test_timeout(runtime.cfg.provider)?,
            )
            .await?;
            run_transfer_answering_role(&runtime.cfg, &handle, transport, true).await?;
        }
        Role::Target => {
            let handle =
                wait_for_next_established(&mut runtime.events, Duration::from_secs(90)).await?;
            run_transfer_answering_role(&runtime.cfg, &handle, transport, false).await?;
        }
        _ => return Err(format!("unsupported transfer role {:?}", role).into()),
    }
    Ok(())
}

async fn run_hold_on_handle(
    _provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let mut call_events = handle.events().await?;
    let audio = handle.audio().await?;
    let (sender, mut receiver) = audio.split();
    let received_buf = Arc::new(Mutex::new(Vec::<i16>::new()));
    let recv_buf = received_buf.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if let Ok(mut buf) = recv_buf.lock() {
                buf.extend_from_slice(&frame.samples);
            }
        }
    });

    let mut frame_index = 0usize;
    send_tone_segment(&sender, ENDPOINT_1001_TONE_HZ, 100, &mut frame_index).await?;
    handle.hold().await?;
    wait_for_local_hold_on_events(&mut call_events, Duration::from_secs(8)).await?;
    send_tone_segment(&sender, 550.0, 50, &mut frame_index).await?;
    sleep(Duration::from_millis(500)).await;
    handle.resume().await?;
    wait_for_local_resume_on_events(&mut call_events, Duration::from_secs(8)).await?;
    send_tone_segment(&sender, ENDPOINT_1003_TONE_HZ, 100, &mut frame_index).await?;
    sleep(Duration::from_secs(1)).await;
    drop(sender);
    handle
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await
        .ok();
    stop_recv_task(recv_task).await;
    let received = received_buf.lock().map(|g| g.clone()).unwrap_or_default();
    save_wav(
        &cfg.output_dir,
        hold_resume_caller_wav(transport),
        &received,
    )?;
    Ok(())
}

async fn run_answering_tone_role(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    tone_hz: f32,
    wav_name: &str,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder = start_tone_recorder(handle, tone_hz).await?;
    handle
        .wait_for_end(Some(Duration::from_secs(45)))
        .await
        .ok();
    recorder.stop_and_save(&cfg.output_dir, wav_name).await?;
    Ok(())
}

async fn run_deferred_target(
    provider: PbxProvider,
    peer: &mut StreamPeer,
    _cfg: &EndpointConfig,
) -> ExampleResult<()> {
    let incoming = timeout(remote_test_timeout(provider)?, peer.wait_for_incoming()).await??;
    let guard = incoming.defer(Duration::from_secs(30));
    let result = guard
        .wait_for_cancelled(Some(Duration::from_secs(12)))
        .await;
    if provider.expects_target_cancel() {
        result?;
    }
    Ok(())
}

async fn run_dtmf_caller(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder = if transport.is_tls() {
        Some(start_tone_recorder(handle, ENDPOINT_1001_TONE_HZ).await?)
    } else {
        None
    };
    for digit in remote_test_digits(cfg.provider) {
        sleep(Duration::from_millis(500)).await;
        handle.send_dtmf(digit).await?;
    }
    sleep(Duration::from_secs(1)).await;
    handle.hangup_and_wait(Some(Duration::from_secs(8))).await?;
    if let Some(recorder) = recorder {
        recorder
            .stop_and_save(&cfg.output_dir, dtmf_caller_wav(transport))
            .await?;
    }
    Ok(())
}

async fn run_dtmf_callee(
    provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder = if transport.is_tls() {
        Some(start_tone_recorder(handle, tone_for_callee(transport)).await?)
    } else {
        None
    };
    let mut events = handle.events().await?;
    wait_for_dtmf_sequence_on_events(
        &mut events,
        &remote_test_digits(provider),
        remote_test_timeout(provider)?,
    )
    .await?;
    handle
        .wait_for_end(Some(Duration::from_secs(15)))
        .await
        .ok();
    if let Some(recorder) = recorder {
        recorder
            .stop_and_save(&cfg.output_dir, dtmf_callee_wav(transport))
            .await?;
    }
    Ok(())
}

async fn run_transferor(
    provider: PbxProvider,
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder = if transport.is_tls() {
        Some(start_tone_recorder(handle, ENDPOINT_1001_TONE_HZ).await?)
    } else {
        None
    };
    sleep(transfer_settle_duration(provider)).await;
    let transfer_outcome = handle
        .transfer_blind_and_wait_for_outcome(
            &cfg.remote_call_uri(),
            TransferWaitMode::NotifyFinal,
            Some(remote_test_timeout(provider)?),
        )
        .await?;
    match transfer_outcome {
        TransferOutcome::ReferCompleted {
            status_code,
            reason,
            ..
        } => {
            println!("[transfer] REFER completed: {} {}", status_code, reason);
        }
        TransferOutcome::Failed {
            status_code,
            reason,
            ..
        } => {
            return Err(format!("REFER failed: {} {}", status_code, reason).into());
        }
        other => return Err(format!("unexpected transfer outcome: {:?}", other).into()),
    }
    if let Some(recorder) = recorder {
        recorder
            .stop_and_save(&cfg.output_dir, transferor_wav(transport))
            .await?;
    }
    Ok(())
}

async fn run_transfer_answering_role(
    cfg: &EndpointConfig,
    handle: &SessionHandle,
    transport: TransportMode,
    transferee: bool,
) -> ExampleResult<()> {
    if transport.is_tls() {
        assert_srtp_media_security(handle, Duration::from_secs(5)).await?;
    }
    let recorder = if transport.is_tls() {
        let tone = if transferee {
            ENDPOINT_1002_TONE_HZ
        } else {
            ENDPOINT_1003_TONE_HZ
        };
        Some(start_tone_recorder(handle, tone).await?)
    } else {
        None
    };
    sleep(if transferee {
        Duration::from_secs(12)
    } else {
        Duration::from_secs(4)
    })
    .await;
    handle
        .hangup_and_wait(Some(Duration::from_secs(8)))
        .await
        .ok();
    if let Some(recorder) = recorder {
        let name = if transferee {
            transferee_wav(transport)
        } else {
            transfer_target_wav(transport)
        };
        recorder.stop_and_save(&cfg.output_dir, name).await?;
    }
    Ok(())
}

pub async fn run_analyze() -> ExampleResult<()> {
    let provider = PbxProvider::from_env_or_args()?;
    load_env(provider);
    init_tracing();
    let scenario = Scenario::from_env_or_args()?;
    let transport = TransportMode::from_env_or_args()?;
    let cfg = EndpointConfig::new(provider, username_for(transport, Role::Caller), transport)?;
    match scenario {
        Scenario::HoldResume => analyze_hold(&cfg, transport),
        Scenario::Dtmf if transport.is_tls() => analyze_dtmf(&cfg, transport),
        Scenario::BlindTransfer if transport.is_tls() => analyze_transfer(&cfg, transport),
        _ => {
            println!(
                "No WAV analysis is required for {:?} over {:?}.",
                scenario, transport
            );
            Ok(())
        }
    }
}

fn analyze_hold(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    let caller_wav = cfg.output_dir.join(hold_resume_caller_wav(transport));
    let callee_wav = cfg.output_dir.join(hold_resume_callee_wav(transport));
    let caller = assert_audio_path(
        &caller_wav,
        tone_for_callee(transport),
        ENDPOINT_1001_TONE_HZ,
    )?;
    let callee_samples = read_wav(&callee_wav)?;
    let pre_hold = assert_samples_tone(
        "callee pre-hold caller tone",
        first_half(&callee_samples),
        ENDPOINT_1001_TONE_HZ,
        ENDPOINT_1003_TONE_HZ,
    )?;
    let post_resume = assert_samples_tone(
        "callee post-resume caller tone",
        second_half(&callee_samples),
        ENDPOINT_1003_TONE_HZ,
        ENDPOINT_1002_TONE_HZ,
    )?;
    print_analysis(
        "caller received callee reference tone",
        &caller_wav,
        &caller,
    );
    print_analysis("callee pre-hold caller tone", &callee_wav, &pre_hold);
    print_analysis("callee post-resume caller tone", &callee_wav, &post_resume);
    Ok(())
}

fn analyze_dtmf(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    let caller_wav = cfg.output_dir.join(dtmf_caller_wav(transport));
    let callee_wav = cfg.output_dir.join(dtmf_callee_wav(transport));
    let caller = assert_audio_path(&caller_wav, ENDPOINT_1002_TONE_HZ, ENDPOINT_1001_TONE_HZ)?;
    let callee = assert_audio_path(&callee_wav, ENDPOINT_1001_TONE_HZ, ENDPOINT_1002_TONE_HZ)?;
    print_analysis("1001 received 1002 reference tone", &caller_wav, &caller);
    print_analysis("1002 received 1001 reference tone", &callee_wav, &callee);
    Ok(())
}

fn analyze_transfer(cfg: &EndpointConfig, transport: TransportMode) -> ExampleResult<()> {
    const WINDOW_SAMPLES: usize = SAMPLE_RATE as usize;
    const MIN_TRANSFEREE_SAMPLES: usize = WINDOW_SAMPLES * 2;

    let transferor_wav = cfg.output_dir.join(transferor_wav(transport));
    let transferee_wav = cfg.output_dir.join(transferee_wav(transport));
    let target_wav = cfg.output_dir.join(transfer_target_wav(transport));
    let transferor = assert_audio_path(
        &transferor_wav,
        ENDPOINT_1002_TONE_HZ,
        ENDPOINT_1001_TONE_HZ,
    )?;
    let target = assert_audio_path(&target_wav, ENDPOINT_1002_TONE_HZ, ENDPOINT_1003_TONE_HZ)?;
    let transferee_samples = read_wav(&transferee_wav)?;
    if transferee_samples.len() < MIN_TRANSFEREE_SAMPLES {
        return Err(format!(
            "{} too short: {} samples (expected at least {})",
            transferee_wav.display(),
            transferee_samples.len(),
            MIN_TRANSFEREE_SAMPLES
        )
        .into());
    }
    let first_window = &transferee_samples[..WINDOW_SAMPLES];
    let last_window = &transferee_samples[transferee_samples.len() - WINDOW_SAMPLES..];
    let initial = assert_samples_tone(
        "1002 initial leg received 1001 tone",
        first_window,
        ENDPOINT_1001_TONE_HZ,
        ENDPOINT_1003_TONE_HZ,
    )?;
    let transferred = assert_samples_tone(
        "1002 transferred leg received 1003 tone",
        last_window,
        ENDPOINT_1003_TONE_HZ,
        ENDPOINT_1001_TONE_HZ,
    )?;
    print_analysis(
        "1001 received 1002 initial-leg tone",
        &transferor_wav,
        &transferor,
    );
    print_analysis(
        "1003 received 1002 transferred-leg tone",
        &target_wav,
        &target,
    );
    print_analysis(
        "1002 initial window received 1001 tone",
        &transferee_wav,
        &initial,
    );
    print_analysis(
        "1002 final window received 1003 tone",
        &transferee_wav,
        &transferred,
    );
    Ok(())
}

pub fn generate_tone(freq: f32, frame_num: usize) -> Vec<i16> {
    (0..FRAME_SIZE)
        .map(|j| {
            let t = (frame_num * FRAME_SIZE + j) as f32 / SAMPLE_RATE as f32;
            (0.3 * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
        })
        .collect()
}

pub async fn send_tone_segment(
    sender: &AudioSender,
    tone_hz: f32,
    frames: usize,
    frame_index: &mut usize,
) -> ExampleResult<()> {
    for _ in 0..frames {
        let frame = AudioFrame::new(
            generate_tone(tone_hz, *frame_index),
            SAMPLE_RATE,
            1,
            (*frame_index * FRAME_SIZE) as u32,
        );
        sender.send(frame).await?;
        *frame_index += 1;
        sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

pub async fn start_tone_recorder(
    handle: &SessionHandle,
    tone_hz: f32,
) -> ExampleResult<ToneRecorder> {
    let audio = handle.audio().await?;
    let (sender, mut receiver) = audio.split();
    let received_buf = Arc::new(Mutex::new(Vec::<i16>::new()));
    let recv_buf = received_buf.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if let Ok(mut buf) = recv_buf.lock() {
                buf.extend_from_slice(&frame.samples);
            }
        }
    });
    let running = Arc::new(AtomicBool::new(true));
    let send_running = running.clone();
    let send_task = tokio::spawn(async move {
        let mut frame_index = 0usize;
        while send_running.load(Ordering::Relaxed) && sender.is_open() {
            let frame = AudioFrame::new(
                generate_tone(tone_hz, frame_index),
                SAMPLE_RATE,
                1,
                (frame_index * FRAME_SIZE) as u32,
            );
            if sender.send(frame).await.is_err() {
                break;
            }
            frame_index += 1;
            sleep(Duration::from_millis(20)).await;
        }
    });
    Ok(ToneRecorder {
        running,
        send_task,
        recv_task,
        received_buf,
    })
}

impl ToneRecorder {
    pub async fn stop_and_save(
        self,
        output_dir: &Path,
        output_name: &str,
    ) -> ExampleResult<PathBuf> {
        let ToneRecorder {
            running,
            send_task,
            recv_task,
            received_buf,
        } = self;
        running.store(false, Ordering::Relaxed);
        let _ = timeout(Duration::from_secs(2), send_task).await;
        stop_recv_task(recv_task).await;
        let received = received_buf.lock().map(|g| g.clone()).unwrap_or_default();
        save_wav(output_dir, output_name, &received)
    }
}

pub async fn call_with_answer_retry(
    peer: &mut StreamPeer,
    target: &str,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    let attempts =
        call_retry_attempts(PbxProvider::from_env_or_args().unwrap_or(PbxProvider::Asterisk))
            .max(1);
    let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    for attempt in 1..=attempts {
        let call_id = peer.invite(target).send().await?;
        let handle = peer.coordinator().session(&call_id);
        match handle.wait_for_answered(Some(timeout_duration)).await {
            Ok(answered) => return Ok(answered),
            Err(e) => {
                println!(
                    "[call] Attempt {}/{} to {} was not answered: {}",
                    attempt, attempts, target, e
                );
                last_error = Some(Box::new(e));
            }
        }
        if attempt < attempts {
            sleep(Duration::from_secs(2)).await;
        }
    }
    Err(last_error.unwrap_or_else(|| "call was not answered".into()))
}

pub async fn call_with_ringing_retry(
    peer: &mut StreamPeer,
    target: &str,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    let attempts =
        call_retry_attempts(PbxProvider::from_env_or_args().unwrap_or(PbxProvider::Asterisk))
            .max(1);
    let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    for attempt in 1..=attempts {
        let call_id = peer.invite(target).send().await?;
        let handle = peer.coordinator().session(&call_id);
        match handle
            .wait_for_progress(
                |event| {
                    matches!(
                        event,
                        Event::CallProgress {
                            status_code: 180 | 183,
                            ..
                        }
                    )
                },
                Some(timeout_duration),
            )
            .await
        {
            Ok(_) => return Ok(handle),
            Err(e) => {
                println!(
                    "[call] Attempt {}/{} to {} did not ring: {}",
                    attempt, attempts, target, e
                );
                last_error = Some(Box::new(e));
            }
        }
        if attempt < attempts {
            sleep(Duration::from_secs(2)).await;
        }
    }
    Err(last_error.unwrap_or_else(|| "call did not reach ringing".into()))
}

pub async fn callback_call_with_answer_retry(
    runtime: &mut CallbackRuntime,
    target: &str,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    let attempts = call_retry_attempts(runtime.cfg.provider).max(1);
    let mut last_error: Option<String> = None;
    for attempt in 1..=attempts {
        let call_id = runtime.control.invite(target).send().await?;
        let handle = runtime.control.coordinator().session(&call_id);
        match wait_for_established(&mut runtime.events, handle.id(), timeout_duration).await {
            Ok(answered) => return Ok(answered),
            Err(e) => {
                println!(
                    "[call] Attempt {}/{} to {} was not answered: {}",
                    attempt, attempts, target, e
                );
                last_error = Some(e.to_string());
            }
        }
        if attempt < attempts {
            sleep(Duration::from_secs(2)).await;
        }
    }
    Err(last_error
        .unwrap_or_else(|| "call was not answered".into())
        .into())
}

pub async fn assert_srtp_media_security(
    handle: &SessionHandle,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    let security = handle
        .wait_for_media_security(Some(timeout_duration))
        .await?;
    if security.keying != MediaSecurityKeying::Sdes {
        return Err(format!("expected SDES keying, got {:?}", security.keying).into());
    }
    if security.profile != MediaSecurityProfile::RtpSavp {
        return Err(format!("expected RTP/SAVP profile, got {:?}", security.profile).into());
    }
    if !security.contexts_installed {
        return Err("SRTP media security exists but contexts_installed=false".into());
    }
    println!(
        "[security] SRTP negotiated: keying=SDES suite={} profile=RTP/SAVP contexts_installed={}",
        security.suite, security.contexts_installed
    );
    Ok(())
}

pub async fn wait_for_stream_registration(
    peer: &StreamPeer,
    handle: &RegistrationHandle,
    username: &str,
) -> ExampleResult<()> {
    for _ in 0..50 {
        if peer.is_registered(handle).await? {
            return Ok(());
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!("endpoint {} did not register within 10s", username).into())
}

pub async fn wait_for_local_hold_on_events(
    events: &mut EventReceiver,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    wait_for_named_event(events, timeout_duration, "CallOnHold", |event| {
        matches!(event, Event::CallOnHold { .. })
    })
    .await
}

pub async fn wait_for_local_resume_on_events(
    events: &mut EventReceiver,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    wait_for_named_event(events, timeout_duration, "CallResumed", |event| {
        matches!(event, Event::CallResumed { .. })
    })
    .await
}

pub async fn wait_for_dtmf_sequence_on_events(
    events: &mut EventReceiver,
    expected: &[char],
    timeout_duration: Duration,
) -> ExampleResult<()> {
    let expected = expected.to_vec();
    timeout(timeout_duration, async {
        let mut index = 0usize;
        while index < expected.len() {
            match events.next().await {
                Some(Event::DtmfReceived { digit, .. }) if digit == expected[index] => {
                    index += 1;
                }
                Some(Event::DtmfReceived { digit, .. }) => {
                    return Err(format!(
                        "DTMF sequence mismatch at index {}: expected '{}', got '{}'",
                        index, expected[index], digit
                    )
                    .into());
                }
                Some(Event::CallEnded { reason, .. }) => {
                    return Err(format!("call ended before DTMF completed: {}", reason).into());
                }
                Some(Event::CallFailed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(format!(
                        "call failed before DTMF completed: {} {}",
                        status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("event stream closed while waiting for DTMF".into()),
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| format!("timed out after {:?} waiting for DTMF", timeout_duration))?
}

pub async fn wait_for_call_cancelled_on_events(
    events: &mut EventReceiver,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.next().await {
                Some(Event::CallCancelled { .. }) => return Ok(()),
                Some(Event::CallEnded { reason, .. }) => {
                    return Err(
                        format!("call ended while waiting for CallCancelled: {}", reason).into(),
                    );
                }
                Some(Event::CallFailed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(format!(
                        "call failed while waiting for CallCancelled: {} {}",
                        status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("event stream closed while waiting for CallCancelled".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for CallCancelled",
            timeout_duration
        )
    })?
}

pub async fn wait_for_call_failed_on_events(
    events: &mut EventReceiver,
    timeout_duration: Duration,
) -> ExampleResult<(u16, String)> {
    timeout(timeout_duration, async {
        loop {
            match events.next().await {
                Some(Event::CallFailed {
                    status_code,
                    reason,
                    ..
                }) => return Ok((status_code, reason)),
                Some(Event::CallEnded { reason, .. }) => {
                    return Err(
                        format!("call ended while waiting for CallFailed: {}", reason).into(),
                    );
                }
                Some(_) => {}
                None => return Err("event stream closed while waiting for CallFailed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for CallFailed",
            timeout_duration
        )
    })?
}

async fn wait_for_named_event<F>(
    events: &mut EventReceiver,
    timeout_duration: Duration,
    event_name: &str,
    mut predicate: F,
) -> ExampleResult<()>
where
    F: FnMut(&Event) -> bool,
{
    timeout(timeout_duration, async {
        loop {
            match events.next().await {
                Some(event) if predicate(&event) => return Ok(()),
                Some(Event::CallEnded { reason, .. }) => {
                    return Err(format!(
                        "call ended before {} was observed: {}",
                        event_name, reason
                    )
                    .into());
                }
                Some(Event::CallFailed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(format!(
                        "call failed before {} was observed: {} {}",
                        event_name, status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => {
                    return Err(
                        format!("event stream closed while waiting for {}", event_name).into(),
                    )
                }
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for {}",
            timeout_duration, event_name
        )
    })?
}

pub async fn wait_for_established(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    call_id: &CallId,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Established(handle)) if handle.id() == call_id => {
                    return Ok(handle)
                }
                Some(CallbackEvent::Failed {
                    call_id: failed_id,
                    status_code,
                    reason,
                }) if &failed_id == call_id => {
                    return Err(format!("call failed with {} {}", status_code, reason).into());
                }
                Some(CallbackEvent::Cancelled {
                    call_id: cancelled_id,
                }) if &cancelled_id == call_id => {
                    return Err("call cancelled before answer".into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| format!("timed out after {:?} waiting for answer", timeout_duration))?
}

pub async fn wait_for_next_established(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Established(handle)) => return Ok(handle),
                Some(CallbackEvent::Failed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(format!("call failed with {} {}", status_code, reason).into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for established call",
            timeout_duration
        )
    })?
}

pub async fn wait_for_call_failed(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    call_id: &CallId,
    expected_status: u16,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Failed {
                    call_id: failed_id,
                    status_code,
                    reason,
                }) if &failed_id == call_id => {
                    if status_code == expected_status {
                        return Ok(());
                    }
                    return Err(format!(
                        "expected failure {}, got {} {}",
                        expected_status, status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for CallFailed",
            timeout_duration
        )
    })?
}

pub async fn wait_for_cancelled(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    call_id: Option<&CallId>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Cancelled {
                    call_id: cancelled_id,
                }) if call_id.map_or(true, |expected| expected == &cancelled_id) => return Ok(()),
                Some(CallbackEvent::Failed {
                    call_id: failed_id,
                    status_code,
                    reason,
                }) if call_id.map_or(true, |expected| expected == &failed_id) => {
                    return Err(format!(
                        "call failed while waiting for cancellation: {} {}",
                        status_code, reason
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for CallCancelled",
            timeout_duration
        )
    })?
}

pub async fn wait_for_callback_progress(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    call_id: &CallId,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Progress {
                    call_id: progress_id,
                    status_code: 180 | 183,
                    ..
                }) if &progress_id == call_id => return Ok(()),
                Some(CallbackEvent::Failed {
                    call_id: failed_id,
                    status_code,
                    reason,
                }) if &failed_id == call_id => {
                    return Err(format!("call failed with {} {}", status_code, reason).into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for callback call progress",
            timeout_duration
        )
    })?
}

pub async fn wait_for_dtmf_sequence(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    expected: &[char],
    timeout_duration: Duration,
) -> ExampleResult<()> {
    let expected = expected.to_vec();
    timeout(timeout_duration, async {
        let mut index = 0usize;
        while index < expected.len() {
            match events.recv().await {
                Some(CallbackEvent::Dtmf { digit, .. }) if digit == expected[index] => index += 1,
                Some(CallbackEvent::Dtmf { digit, .. }) => {
                    return Err(format!(
                        "DTMF sequence mismatch at index {}: expected '{}', got '{}'",
                        index, expected[index], digit
                    )
                    .into());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| format!("timed out after {:?} waiting for DTMF", timeout_duration))?
}

pub async fn wait_for_registration_success(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::RegistrationSuccess { registrar, .. }) => {
                    println!("[callback-registration] registered with {}", registrar);
                    return Ok(());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for registration",
            timeout_duration
        )
    })?
}

pub async fn wait_for_unregistration_success(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::UnregistrationSuccess { registrar }) => {
                    println!("[callback-registration] unregistered from {}", registrar);
                    return Ok(());
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for unregistration",
            timeout_duration
        )
    })?
}

pub async fn wait_for_local_hold_resume(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        let mut saw_hold = false;
        loop {
            match events.recv().await {
                Some(CallbackEvent::LocalHold { .. }) => saw_hold = true,
                Some(CallbackEvent::LocalResume { .. }) if saw_hold => return Ok(()),
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for hold/resume",
            timeout_duration
        )
    })?
}

async fn wait_for_incoming_notice(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<CallId> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Incoming { call_id, from, to }) => {
                    println!("[callback] incoming call {} -> {}", from, to);
                    return Ok(call_id);
                }
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for incoming call",
            timeout_duration
        )
    })?
}

pub fn save_wav(out_dir: &Path, name: &str, samples: &[i16]) -> ExampleResult<PathBuf> {
    std::fs::create_dir_all(out_dir)?;
    let path = out_dir.join(name);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for &s in samples {
        writer.write_sample(s)?;
    }
    writer.finalize()?;
    println!("Saved {} ({} samples)", path.display(), samples.len());
    Ok(path)
}

pub fn read_wav(path: &Path) -> ExampleResult<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)?;
    let samples = reader.samples::<i16>().collect::<Result<Vec<_>, _>>()?;
    Ok(samples)
}

pub fn analyze_samples(
    samples: &[i16],
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<ToneAnalysis> {
    let expected_magnitude = goertzel_magnitude(samples, SAMPLE_RATE as f32, expected_hz);
    let rejected_magnitude = goertzel_magnitude(samples, SAMPLE_RATE as f32, rejected_hz);
    let ratio = if rejected_magnitude > 1.0 {
        expected_magnitude / rejected_magnitude
    } else {
        f32::INFINITY
    };
    Ok(ToneAnalysis {
        samples: samples.len(),
        expected_hz,
        rejected_hz,
        expected_magnitude,
        rejected_magnitude,
        ratio,
    })
}

pub fn assert_audio_path(
    path: &Path,
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<ToneAnalysis> {
    let analysis = analyze_samples(&read_wav(path)?, expected_hz, rejected_hz)?;
    if analysis.samples < MIN_RECEIVED_SAMPLES {
        return Err(format!(
            "{} too short: {} samples (expected at least {})",
            path.display(),
            analysis.samples,
            MIN_RECEIVED_SAMPLES
        )
        .into());
    }
    if analysis.ratio < DOMINANCE_RATIO {
        return Err(format!(
            "{}: {:.0}Hz magnitude {:.1} vs {:.0}Hz magnitude {:.1}, ratio {:.2} (expected at least {:.2})",
            path.display(),
            analysis.expected_hz,
            analysis.expected_magnitude,
            analysis.rejected_hz,
            analysis.rejected_magnitude,
            analysis.ratio,
            DOMINANCE_RATIO
        )
        .into());
    }
    Ok(analysis)
}

pub fn assert_samples_tone(
    label: &str,
    samples: &[i16],
    expected_hz: f32,
    rejected_hz: f32,
) -> ExampleResult<ToneAnalysis> {
    let analysis = analyze_samples(samples, expected_hz, rejected_hz)?;
    if analysis.ratio < DOMINANCE_RATIO {
        return Err(format!(
            "{}: {:.0}Hz magnitude {:.1} vs {:.0}Hz magnitude {:.1}, ratio {:.2} (expected at least {:.2})",
            label,
            analysis.expected_hz,
            analysis.expected_magnitude,
            analysis.rejected_hz,
            analysis.rejected_magnitude,
            analysis.ratio,
            DOMINANCE_RATIO
        )
        .into());
    }
    Ok(analysis)
}

pub fn print_analysis(label: &str, path: &Path, analysis: &ToneAnalysis) {
    println!(
        "{}: {} samples, {:.0}Hz magnitude {:.1}, {:.0}Hz magnitude {:.1}, ratio {:.2}",
        label,
        analysis.samples,
        analysis.expected_hz,
        analysis.expected_magnitude,
        analysis.rejected_hz,
        analysis.rejected_magnitude,
        analysis.ratio
    );
    println!("{} WAV: {}", label, path.display());
}

pub fn goertzel_magnitude(samples: &[i16], sample_rate: f32, target_hz: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let n = samples.len() as f32;
    let k = (0.5 + (n * target_hz) / sample_rate).floor();
    let omega = (2.0 * std::f32::consts::PI * k) / n;
    let coeff = 2.0 * omega.cos();
    let (mut q1, mut q2) = (0.0f32, 0.0f32);
    for &s in samples {
        let q0 = coeff * q1 - q2 + (s as f32);
        q2 = q1;
        q1 = q0;
    }
    (q1 * q1 + q2 * q2 - q1 * q2 * coeff).sqrt()
}

fn endpoint_defaults(
    provider: PbxProvider,
    username: &str,
    transport: TransportMode,
) -> EndpointDefaults {
    let base = match provider {
        PbxProvider::Asterisk => 0,
        PbxProvider::FreeSwitch => 10_000,
    };
    match (transport, username) {
        (TransportMode::TlsSrtp, "1001") => EndpointDefaults {
            local_port: 5070 + base,
            tls_local_port: Some(5071 + base),
            media_port_start: 16000,
            media_port_end: 16100,
        },
        (TransportMode::TlsSrtp, "1002") => EndpointDefaults {
            local_port: 5072 + base,
            tls_local_port: Some(5073 + base),
            media_port_start: 16120,
            media_port_end: 16220,
        },
        (TransportMode::TlsSrtp, "1003") => EndpointDefaults {
            local_port: 5074 + base,
            tls_local_port: Some(5075 + base),
            media_port_start: 16240,
            media_port_end: 16340,
        },
        (TransportMode::Udp, "2001") => EndpointDefaults {
            local_port: 5080 + base,
            tls_local_port: None,
            media_port_start: 17000,
            media_port_end: 17100,
        },
        (TransportMode::Udp, "2002") => EndpointDefaults {
            local_port: 5082 + base,
            tls_local_port: None,
            media_port_start: 17120,
            media_port_end: 17220,
        },
        (TransportMode::Udp, "2003") => EndpointDefaults {
            local_port: 5084 + base,
            tls_local_port: None,
            media_port_start: 17240,
            media_port_end: 17340,
        },
        _ => EndpointDefaults {
            local_port: 5090 + base,
            tls_local_port: Some(5091 + base),
            media_port_start: 18000,
            media_port_end: 18100,
        },
    }
}

fn print_registration_context(cfg: &EndpointConfig) {
    println!("[{}] Provider:   {}", cfg.username, cfg.provider.label());
    println!(
        "[{}] Transport:  {}",
        cfg.username,
        cfg.transport.env_value()
    );
    println!("[{}] AOR:        {}", cfg.username, cfg.aor_uri());
    println!("[{}] Contact:    {}", cfg.username, cfg.contact_uri());
    println!("[{}] Registrar:  {}", cfg.username, cfg.registrar_uri());
    println!("[{}] Media SDP:  {}", cfg.username, cfg.media_advertised_ip);
}

async fn settle_after_register(provider: PbxProvider) {
    let secs = std::env::var(match provider {
        PbxProvider::Asterisk => "ASTERISK_POST_REGISTER_SETTLE_SECS",
        PbxProvider::FreeSwitch => "FREESWITCH_POST_REGISTER_SETTLE_SECS",
    })
    .or_else(|_| std::env::var("POST_REGISTER_SETTLE_SECS"))
    .ok()
    .and_then(|value| value.parse().ok())
    .unwrap_or(provider.default_settle_secs());
    if secs > 0 {
        sleep(Duration::from_secs(secs)).await;
    }
}

fn idle_duration() -> Duration {
    env_duration_secs("IDLE_SECS", 2)
}

fn remote_test_timeout(provider: PbxProvider) -> ExampleResult<Duration> {
    let key = match provider {
        PbxProvider::Asterisk => "ASTERISK_TEST_TIMEOUT_SECS",
        PbxProvider::FreeSwitch => "FREESWITCH_TEST_TIMEOUT_SECS",
    };
    let secs = std::env::var(key)
        .or_else(|_| std::env::var("REMOTE_TEST_TIMEOUT_SECS"))
        .unwrap_or_else(|_| "60".to_string())
        .parse()?;
    Ok(Duration::from_secs(secs))
}

fn transfer_settle_duration(provider: PbxProvider) -> Duration {
    let key = match provider {
        PbxProvider::Asterisk => "ASTERISK_TRANSFER_SETTLE_SECS",
        PbxProvider::FreeSwitch => "FREESWITCH_TRANSFER_SETTLE_SECS",
    };
    env_duration_secs(key, 3)
}

fn call_retry_attempts(provider: PbxProvider) -> usize {
    let key = match provider {
        PbxProvider::Asterisk => "ASTERISK_CALL_RETRY_ATTEMPTS",
        PbxProvider::FreeSwitch => "FREESWITCH_CALL_RETRY_ATTEMPTS",
    };
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(provider.default_retry_attempts())
}

fn remote_test_digits(provider: PbxProvider) -> Vec<char> {
    let key = match provider {
        PbxProvider::Asterisk => "ASTERISK_TEST_DIGITS",
        PbxProvider::FreeSwitch => "FREESWITCH_TEST_DIGITS",
    };
    std::env::var(key)
        .or_else(|_| std::env::var("REMOTE_TEST_DIGITS"))
        .unwrap_or_else(|_| "1234#".to_string())
        .chars()
        .collect()
}

fn target_user_for(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "1002"
    } else {
        "2002"
    }
}

fn tone_for_callee(transport: TransportMode) -> f32 {
    if transport.is_tls() {
        ENDPOINT_1002_TONE_HZ
    } else {
        ENDPOINT_2002_TONE_HZ
    }
}

fn hold_resume_caller_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_hold_resume_1001_received.wav"
    } else {
        "hold_resume_2001_received.wav"
    }
}

fn hold_resume_callee_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_hold_resume_1002_received.wav"
    } else {
        "hold_resume_2002_received.wav"
    }
}

fn dtmf_caller_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_dtmf_1001_received.wav"
    } else {
        "dtmf_2001_received.wav"
    }
}

fn dtmf_callee_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_dtmf_1002_received.wav"
    } else {
        "dtmf_2002_received.wav"
    }
}

fn transferor_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_blind_transfer_1001_received.wav"
    } else {
        "blind_transfer_2001_received.wav"
    }
}

fn transferee_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_blind_transfer_1002_received.wav"
    } else {
        "blind_transfer_2002_received.wav"
    }
}

fn transfer_target_wav(transport: TransportMode) -> &'static str {
    if transport.is_tls() {
        "tls_srtp_blind_transfer_1003_received.wav"
    } else {
        "blind_transfer_2003_received.wav"
    }
}

fn first_half(samples: &[i16]) -> &[i16] {
    &samples[..samples.len() / 2]
}

fn second_half(samples: &[i16]) -> &[i16] {
    &samples[samples.len() / 2..]
}

async fn stop_recv_task(task: JoinHandle<()>) {
    let _ = timeout(Duration::from_secs(2), async {
        loop {
            if task.is_finished() {
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    task.abort();
}

fn advertised_ip(provider: PbxProvider, local_ip: IpAddr) -> ExampleResult<IpAddr> {
    let value = match provider {
        PbxProvider::Asterisk => std::env::var("ADVERTISED_IP"),
        PbxProvider::FreeSwitch => {
            std::env::var("RVOIP_ADVERTISED_IP").or_else(|_| std::env::var("ADVERTISED_IP"))
        }
    };
    match value {
        Ok(value) => Ok(value.parse()?),
        Err(_) if !local_ip.is_unspecified() => Ok(local_ip),
        Err(_) => Err("advertised IP is required when local IP is unspecified".into()),
    }
}

fn media_advertised_ip(provider: PbxProvider, advertised_ip: IpAddr) -> ExampleResult<IpAddr> {
    let value = match provider {
        PbxProvider::Asterisk => std::env::var("MEDIA_ADVERTISED_IP"),
        PbxProvider::FreeSwitch => std::env::var("RVOIP_MEDIA_ADVERTISED_IP")
            .or_else(|_| std::env::var("MEDIA_ADVERTISED_IP")),
    };
    match value {
        Ok(value) if !value.trim().is_empty() => Ok(value.parse()?),
        _ => Ok(advertised_ip),
    }
}

fn auth_username_for(prefix: &str, username: &str) -> String {
    let endpoint_auth = non_empty_env(&format!("{}_AUTH_USERNAME", prefix));
    let sip_username = non_empty_env("SIP_USERNAME");
    let sip_auth_username = non_empty_env("SIP_AUTH_USERNAME");
    select_auth_username(
        username,
        endpoint_auth.as_deref(),
        sip_username.as_deref(),
        sip_auth_username.as_deref(),
    )
}

fn select_auth_username(
    username: &str,
    endpoint_auth: Option<&str>,
    sip_username: Option<&str>,
    sip_auth_username: Option<&str>,
) -> String {
    if let Some(value) = endpoint_auth {
        return value.trim().to_string();
    }
    match (sip_username, sip_auth_username) {
        (Some(sip_username), Some(auth_username)) if sip_username.trim() == username => {
            auth_username.trim().to_string()
        }
        (None, Some(auth_username)) => auth_username.trim().to_string(),
        _ => username.to_string(),
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_string(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u16(key: &str, default: u16) -> ExampleResult<u16> {
    Ok(std::env::var(key)
        .unwrap_or_else(|_| default.to_string())
        .parse()?)
}

fn env_bool(key: &str, default: bool) -> ExampleResult<bool> {
    let value = match std::env::var(key) {
        Ok(value) => value,
        Err(_) => return Ok(default),
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("{} must be a boolean value", key).into()),
    }
}

fn env_duration_secs(key: &str, default: u64) -> Duration {
    Duration::from_secs(
        std::env::var(key)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default),
    )
}

fn split_host_port(value: &str) -> ExampleResult<(String, u16)> {
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Ok((addr.ip().to_string(), addr.port()));
    }
    let (host, port) = value
        .rsplit_once(':')
        .ok_or_else(|| format!("expected host:port PBX address, got '{}'", value))?;
    Ok((host.to_string(), port.parse()?))
}

fn deterministic_sip_instance(username: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in username.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!(
        "urn:uuid:00000000-0000-4000-8000-{:012x}",
        hash & 0xffff_ffff_ffff
    )
}

fn required_path(key: &str) -> ExampleResult<PathBuf> {
    let value =
        std::env::var(key).map_err(|_| format!("{} must be set for SIP_TRANSPORT=TLS", key))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{} must not be empty", key).into());
    }
    Ok(PathBuf::from(value))
}

fn optional_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn default_pbx_port(transport: TransportMode) -> u16 {
    if transport.is_tls() {
        5061
    } else {
        5060
    }
}

fn transport_suffix(transport: TransportMode) -> &'static str {
    match transport {
        TransportMode::TlsSrtp => ";transport=tls",
        TransportMode::Udp => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freeswitch_defaults_use_local_high_ports() {
        let udp = endpoint_defaults(PbxProvider::FreeSwitch, "2001", TransportMode::Udp);
        assert_eq!(udp.local_port, 15080);
        let tls = endpoint_defaults(PbxProvider::FreeSwitch, "1001", TransportMode::TlsSrtp);
        assert_eq!(tls.local_port, 15070);
        assert_eq!(tls.tls_local_port, Some(15071));
    }

    #[test]
    fn asterisk_defaults_preserve_existing_lab_ports() {
        let udp = endpoint_defaults(PbxProvider::Asterisk, "2001", TransportMode::Udp);
        assert_eq!(udp.local_port, 5080);
        let tls = endpoint_defaults(PbxProvider::Asterisk, "1001", TransportMode::TlsSrtp);
        assert_eq!(tls.local_port, 5070);
        assert_eq!(tls.tls_local_port, Some(5071));
    }

    #[test]
    fn split_host_port_accepts_ipv4_socket_addr() {
        let (host, port) = split_host_port("127.0.0.1:5062").unwrap();
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 5062);
    }

    #[test]
    fn canonical_user_sets_are_transport_specific() {
        let roles = [
            Role::Registration,
            Role::Caller,
            Role::Transferor,
            Role::Callee,
            Role::Transferee,
            Role::Target,
        ];
        let mut tls_users = roles
            .iter()
            .map(|role| username_for(TransportMode::TlsSrtp, *role))
            .collect::<Vec<_>>();
        tls_users.sort_unstable();
        tls_users.dedup();
        assert_eq!(tls_users, vec!["1001", "1002", "1003"]);

        let mut udp_users = roles
            .iter()
            .map(|role| username_for(TransportMode::Udp, *role))
            .collect::<Vec<_>>();
        udp_users.sort_unstable();
        udp_users.dedup();
        assert_eq!(udp_users, vec!["2001", "2002", "2003"]);
    }

    #[test]
    fn auth_username_ignores_global_for_other_endpoint_users() {
        assert_eq!(
            select_auth_username("2001", None, Some("1001"), Some("1001")),
            "2001"
        );
        assert_eq!(
            select_auth_username("1001", None, Some("1001"), Some("1001")),
            "1001"
        );
        assert_eq!(
            select_auth_username("2001", Some("auth2001"), Some("1001"), Some("1001")),
            "auth2001"
        );
    }
}
