//! General-purpose terminal SIP client built directly on session-core.
//!
//! This example intentionally avoids the legacy `client-core` / `sip-client`
//! wrappers. SIP signalling, registration, call state, SDP, RTP, codecs, SRTP,
//! DTMF, hold/resume, and transfer all go through `Endpoint` and
//! `SessionHandle`; this file only owns terminal UI state and CPAL device I/O.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Stdout};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{
    event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use serde::Deserialize;
use tokio::sync::mpsc;

use rvoip_media_core::types::AudioFrame;
use rvoip_session_core::{
    CallId, Config, Endpoint, EndpointProfile, Event, RegistrationHandle, Result as SessionResult,
    SessionError, SessionHandle, SipTlsMode,
};

const SAMPLE_RATE: u32 = 8_000;
const FRAME_MS: u32 = 20;
const FRAME_SAMPLES: usize = (SAMPLE_RATE as usize * FRAME_MS as usize) / 1_000;
const MAX_LOGS: usize = 200;

#[derive(Parser, Debug)]
#[command(name = "sip_client")]
#[command(about = "Interactive terminal SIP client built directly on session-core")]
struct Cli {
    /// Path to a TOML config file.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Profile name inside the config file.
    #[arg(long)]
    config_profile: Option<String>,

    /// List audio devices and exit.
    #[arg(long)]
    list_devices: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Direct peer-to-peer client on a local network.
    Lan(CommonArgs),
    /// Receive direct SIP calls without registration.
    Listen(CommonArgs),
    /// Register to Asterisk, FreeSWITCH, or another SIP registrar.
    Register(RegisterArgs),
}

#[derive(Args, Debug, Clone, Default)]
struct CommonArgs {
    /// Local SIP display/configuration name.
    #[arg(long)]
    name: Option<String>,

    /// SIP UDP/TCP bind address.
    #[arg(long)]
    bind: Option<SocketAddr>,

    /// Public/reachable SIP address to advertise in Contact/Via.
    #[arg(long)]
    advertise: Option<SocketAddr>,

    /// Public media address to advertise in SDP. Accepts IP or ip:port.
    #[arg(long)]
    media_public: Option<String>,

    /// STUN server used for best-effort media public address discovery.
    #[arg(long)]
    stun: Option<String>,

    /// Outbound proxy URI, usually with ;lr.
    #[arg(long)]
    outbound_proxy: Option<String>,

    /// Signalling transport preference.
    #[arg(long, value_enum)]
    transport: Option<TransportMode>,

    /// SRTP negotiation mode.
    #[arg(long, value_enum)]
    srtp: Option<SrtpMode>,

    /// Deployment shortcut.
    #[arg(long, value_enum)]
    profile: Option<PbxProfile>,

    /// Input device name substring or device index from --list-devices.
    #[arg(long)]
    input_device: Option<String>,

    /// Output device name substring or device index from --list-devices.
    #[arg(long)]
    output_device: Option<String>,

    /// Dial this target after startup.
    #[arg(long)]
    target: Option<String>,
}

#[derive(Args, Debug, Clone, Default)]
struct RegisterArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// SIP account username or extension.
    #[arg(long)]
    username: Option<String>,

    /// Digest auth username when it differs from --username.
    #[arg(long)]
    auth_username: Option<String>,

    /// Digest auth password.
    #[arg(long)]
    password: Option<String>,

    /// Registrar URI, e.g. sip:192.168.1.50:5060 or sips:pbx.example.com:5061.
    #[arg(long)]
    registrar: Option<String>,

    /// Requested registration expiry in seconds.
    #[arg(long)]
    expires: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupMode {
    Lan,
    Listen,
    Register,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TransportMode {
    Udp,
    Tcp,
    Tls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SrtpMode {
    Off,
    Offer,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum PbxProfile {
    LanPbx,
    AsteriskUdp,
    AsteriskTlsSrtp,
    FreeswitchInternal,
    CarrierSbc,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    default_profile: Option<String>,
    profiles: Option<BTreeMap<String, ProfileConfig>>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ProfileConfig {
    mode: Option<String>,
    name: Option<String>,
    bind: Option<String>,
    advertise: Option<String>,
    media_public: Option<String>,
    stun: Option<String>,
    outbound_proxy: Option<String>,
    transport: Option<TransportMode>,
    srtp: Option<SrtpMode>,
    profile: Option<PbxProfile>,
    input_device: Option<String>,
    output_device: Option<String>,
    target: Option<String>,
    username: Option<String>,
    auth_username: Option<String>,
    password: Option<String>,
    registrar: Option<String>,
    expires: Option<u32>,
}

#[derive(Debug, Clone)]
struct RuntimeOptions {
    mode: StartupMode,
    name: String,
    bind: SocketAddr,
    advertise: Option<SocketAddr>,
    media_public: Option<SocketAddr>,
    stun: Option<String>,
    outbound_proxy: Option<String>,
    transport: TransportMode,
    srtp: SrtpMode,
    profile: PbxProfile,
    input_device: Option<String>,
    output_device: Option<String>,
    target: Option<String>,
    username: Option<String>,
    auth_username: Option<String>,
    password: Option<String>,
    registrar: Option<String>,
    expires: u32,
}

#[derive(Debug)]
enum RuntimeCommand {
    Dial(String),
    Answer,
    Reject,
    Hangup,
    HoldResume,
    ToggleMute,
    SendDtmf(char),
    Transfer(String),
    Shutdown,
}

#[derive(Debug)]
enum RuntimeEvent {
    Ready {
        local_uri: String,
        reachable: String,
    },
    Registration(String),
    Incoming {
        call_id: String,
        from: String,
        to: String,
    },
    State(AppState),
    ActiveCall {
        call_id: String,
        peer: String,
    },
    AudioStarted(String),
    AudioStopped,
    Muted(bool),
    Log(String),
    Error(String),
    ShutdownComplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AppState {
    Idle,
    Registering,
    Registered,
    Incoming,
    Calling,
    InCall,
    OnHold,
    Transferring,
    ShuttingDown,
    Error,
}

impl AppState {
    fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Registering => "Registering",
            Self::Registered => "Registered",
            Self::Incoming => "Incoming",
            Self::Calling => "Calling",
            Self::InCall => "In Call",
            Self::OnHold => "On Hold",
            Self::Transferring => "Transferring",
            Self::ShuttingDown => "Shutting Down",
            Self::Error => "Error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Dial,
    Transfer,
}

struct TuiApp {
    options: RuntimeOptions,
    state: AppState,
    registration: String,
    local_uri: String,
    reachable: String,
    incoming: Option<(String, String, String)>,
    active_call: Option<(String, String)>,
    audio: String,
    muted: bool,
    input_mode: InputMode,
    input: String,
    logs: VecDeque<String>,
    should_quit: bool,
}

impl TuiApp {
    fn new(options: RuntimeOptions) -> Self {
        let registration = match options.mode {
            StartupMode::Register => "Not registered".to_string(),
            _ => "Not used".to_string(),
        };

        Self {
            options,
            state: AppState::Idle,
            registration,
            local_uri: String::new(),
            reachable: String::new(),
            incoming: None,
            active_call: None,
            audio: "Inactive".to_string(),
            muted: false,
            input_mode: InputMode::Normal,
            input: String::new(),
            logs: VecDeque::new(),
            should_quit: false,
        }
    }

    fn apply_event(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Ready {
                local_uri,
                reachable,
            } => {
                self.local_uri = local_uri;
                self.reachable = reachable;
                self.push_log("endpoint ready");
            }
            RuntimeEvent::Registration(status) => {
                self.registration = status.clone();
                self.push_log(status);
            }
            RuntimeEvent::Incoming { call_id, from, to } => {
                self.incoming = Some((call_id.clone(), from.clone(), to));
                self.state = AppState::Incoming;
                self.push_log(format!("incoming call from {from} ({call_id})"));
            }
            RuntimeEvent::State(state) => {
                self.state = state;
            }
            RuntimeEvent::ActiveCall { call_id, peer } => {
                self.active_call = Some((call_id, peer));
                self.incoming = None;
                self.state = AppState::InCall;
            }
            RuntimeEvent::AudioStarted(device) => {
                self.audio = format!("Active ({device})");
            }
            RuntimeEvent::AudioStopped => {
                self.audio = "Inactive".to_string();
            }
            RuntimeEvent::Muted(muted) => {
                self.muted = muted;
                self.push_log(if muted {
                    "microphone muted"
                } else {
                    "microphone unmuted"
                });
            }
            RuntimeEvent::Log(message) => self.push_log(message),
            RuntimeEvent::Error(message) => {
                self.state = AppState::Error;
                self.push_log(format!("error: {message}"));
            }
            RuntimeEvent::ShutdownComplete => {
                self.push_log("shutdown complete");
                self.should_quit = true;
            }
        }
    }

    fn push_log(&mut self, message: impl Into<String>) {
        let timestamp = chrono::Local::now().format("%H:%M:%S");
        self.logs
            .push_back(format!("[{timestamp}] {}", message.into()));
        while self.logs.len() > MAX_LOGS {
            self.logs.pop_front();
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "rvoip_session_core=info,warn".into()),
        )
        .with_writer(io::sink)
        .init();

    let cli = Cli::parse();
    if cli.list_devices {
        list_audio_devices()?;
        return Ok(());
    }

    let file_config = load_file_config(cli.config.clone())?;
    let options = build_runtime_options(&cli, &file_config)?;

    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let runtime_options = options.clone();
    let runtime = std::thread::spawn(move || {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt.block_on(run_runtime(runtime_options, command_rx, event_tx)),
            Err(err) => eprintln!("failed to start SIP runtime: {err}"),
        }
    });

    let mut terminal = TerminalSession::enter()?;
    let mut app = TuiApp::new(options);
    app.push_log("press d to dial, q to quit");

    loop {
        while let Ok(event) = event_rx.try_recv() {
            app.apply_event(event);
        }

        terminal.draw(|frame| draw_ui(frame, &app))?;

        if app.should_quit {
            break;
        }

        if event::poll(Duration::from_millis(50))? {
            if let CrosstermEvent::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key(key, &mut app, &command_tx);
                }
            }
        }
    }

    let _ = command_tx.send(RuntimeCommand::Shutdown);
    let _ = runtime.join();
    Ok(())
}

fn handle_key(key: KeyEvent, app: &mut TuiApp, command_tx: &mpsc::UnboundedSender<RuntimeCommand>) {
    match app.input_mode {
        InputMode::Dial | InputMode::Transfer => match key.code {
            KeyCode::Esc => {
                app.input_mode = InputMode::Normal;
                app.input.clear();
            }
            KeyCode::Enter => {
                let value = app.input.trim().to_string();
                if !value.is_empty() {
                    let command = if app.input_mode == InputMode::Dial {
                        RuntimeCommand::Dial(value)
                    } else {
                        RuntimeCommand::Transfer(value)
                    };
                    let _ = command_tx.send(command);
                }
                app.input_mode = InputMode::Normal;
                app.input.clear();
            }
            KeyCode::Backspace => {
                app.input.pop();
            }
            KeyCode::Char(ch) => {
                app.input.push(ch);
            }
            _ => {}
        },
        InputMode::Normal => match key.code {
            KeyCode::Char('q') => {
                app.state = AppState::ShuttingDown;
                let _ = command_tx.send(RuntimeCommand::Shutdown);
            }
            KeyCode::Char('d') => {
                app.input_mode = InputMode::Dial;
                app.input.clear();
            }
            KeyCode::Char('a') => {
                let _ = command_tx.send(RuntimeCommand::Answer);
            }
            KeyCode::Char('r') => {
                let _ = command_tx.send(RuntimeCommand::Reject);
            }
            KeyCode::Char('h') => {
                let _ = command_tx.send(RuntimeCommand::Hangup);
            }
            KeyCode::Char('m') => {
                let _ = command_tx.send(RuntimeCommand::ToggleMute);
            }
            KeyCode::Char('o') => {
                let _ = command_tx.send(RuntimeCommand::HoldResume);
            }
            KeyCode::Char('t') => {
                app.input_mode = InputMode::Transfer;
                app.input.clear();
            }
            KeyCode::Char(ch) if ch.is_ascii_digit() || ch == '*' || ch == '#' => {
                let _ = command_tx.send(RuntimeCommand::SendDtmf(ch));
            }
            _ => {}
        },
    }
}

async fn run_runtime(
    options: RuntimeOptions,
    mut command_rx: mpsc::UnboundedReceiver<RuntimeCommand>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
) {
    if let Err(err) = run_runtime_inner(options, &mut command_rx, &event_tx).await {
        let _ = event_tx.send(RuntimeEvent::Error(err.to_string()));
    }
    let _ = event_tx.send(RuntimeEvent::ShutdownComplete);
}

async fn run_runtime_inner(
    options: RuntimeOptions,
    command_rx: &mut mpsc::UnboundedReceiver<RuntimeCommand>,
    event_tx: &mpsc::UnboundedSender<RuntimeEvent>,
) -> SessionResult<()> {
    let mut endpoint = build_endpoint(&options).await?;
    let mut registration_handle: Option<RegistrationHandle> = None;

    if options.mode == StartupMode::Register {
        let _ = event_tx.send(RuntimeEvent::State(AppState::Registering));
        match endpoint.register().await {
            Ok(handle) => {
                registration_handle = Some(handle);
                let _ = event_tx.send(RuntimeEvent::State(AppState::Registered));
                let _ = event_tx.send(RuntimeEvent::Registration("Registered".into()));
            }
            Err(err) => {
                let _ = event_tx.send(RuntimeEvent::Registration(format!(
                    "Registration failed: {err}"
                )));
                return Err(err);
            }
        }
    }

    let local_uri = local_uri_for_options(&options);
    let reachable = reachable_address(&options);
    let _ = event_tx.send(RuntimeEvent::Ready {
        local_uri,
        reachable,
    });

    let (control, mut events) = endpoint.into_stream_peer().split();
    let audio_bridge = AudioBridge::new(
        options.input_device.clone(),
        options.output_device.clone(),
        event_tx.clone(),
    );
    let mut active_handle: Option<SessionHandle> = None;
    let mut pending_incoming: Option<(CallId, String, String)> = None;
    let mut _running_audio: Option<RunningAudio> = None;
    let mut on_hold = false;

    if let Some(target) = options.target.as_ref() {
        match dial_target(&options, &control, target).await {
            Ok(handle) => {
                let _ = event_tx.send(RuntimeEvent::State(AppState::Calling));
                let _ = event_tx.send(RuntimeEvent::Log(format!(
                    "calling {} ({})",
                    target,
                    handle.id()
                )));
                active_handle = Some(handle);
            }
            Err(err) => {
                let _ = event_tx.send(RuntimeEvent::Error(format!("dial failed: {err}")));
            }
        }
    } else if options.mode != StartupMode::Register {
        let _ = event_tx.send(RuntimeEvent::State(AppState::Idle));
    }

    loop {
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else { break; };
                match command {
                    RuntimeCommand::Dial(target) => {
                        if active_handle.is_some() {
                            let _ = event_tx.send(RuntimeEvent::Log("already in a call".into()));
                            continue;
                        }
                        match dial_target(&options, &control, &target).await {
                            Ok(handle) => {
                                let _ = event_tx.send(RuntimeEvent::State(AppState::Calling));
                                let _ = event_tx.send(RuntimeEvent::Log(format!("calling {target} ({})", handle.id())));
                                active_handle = Some(handle);
                                on_hold = false;
                            }
                            Err(err) => {
                                let _ = event_tx.send(RuntimeEvent::Error(format!("dial failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::Answer => {
                        let Some((call_id, from, _to)) = pending_incoming.take() else {
                            let _ = event_tx.send(RuntimeEvent::Log("no incoming call to answer".into()));
                            continue;
                        };
                        match control.accept(&call_id).await {
                            Ok(handle) => {
                                let _ = event_tx.send(RuntimeEvent::ActiveCall {
                                    call_id: handle.id().to_string(),
                                    peer: from,
                                });
                                _running_audio = start_audio(&audio_bridge, handle.clone(), event_tx).await;
                                active_handle = Some(handle);
                                on_hold = false;
                            }
                            Err(err) => {
                                let _ = event_tx.send(RuntimeEvent::Error(format!("answer failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::Reject => {
                        let Some((call_id, from, _to)) = pending_incoming.take() else {
                            let _ = event_tx.send(RuntimeEvent::Log("no incoming call to reject".into()));
                            continue;
                        };
                        if let Err(err) = control.reject(&call_id, 486, "Busy Here").await {
                            let _ = event_tx.send(RuntimeEvent::Error(format!("reject failed: {err}")));
                        } else {
                            let _ = event_tx.send(RuntimeEvent::Log(format!("rejected call from {from}")));
                            let _ = event_tx.send(RuntimeEvent::State(if options.mode == StartupMode::Register {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        }
                    }
                    RuntimeCommand::Hangup => {
                        if let Some(handle) = active_handle.take() {
                            let _ = event_tx.send(RuntimeEvent::Log(format!("hanging up {}", handle.id())));
                            let _ = handle.hangup_and_wait(Some(Duration::from_secs(5))).await;
                            _running_audio = None;
                            let _ = event_tx.send(RuntimeEvent::AudioStopped);
                            let _ = event_tx.send(RuntimeEvent::State(if options.mode == StartupMode::Register {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        } else if let Some((call_id, _from, _to)) = pending_incoming.take() {
                            let _ = control.reject(&call_id, 486, "Busy Here").await;
                            let _ = event_tx.send(RuntimeEvent::State(if options.mode == StartupMode::Register {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        } else {
                            let _ = event_tx.send(RuntimeEvent::Log("no call to hang up".into()));
                        }
                    }
                    RuntimeCommand::HoldResume => {
                        let Some(handle) = active_handle.as_ref() else {
                            let _ = event_tx.send(RuntimeEvent::Log("no active call".into()));
                            continue;
                        };
                        let result = if on_hold { handle.resume().await } else { handle.hold().await };
                        match result {
                            Ok(()) => {
                                on_hold = !on_hold;
                                let _ = event_tx.send(RuntimeEvent::State(if on_hold {
                                    AppState::OnHold
                                } else {
                                    AppState::InCall
                                }));
                            }
                            Err(err) => {
                                let _ = event_tx.send(RuntimeEvent::Error(format!("hold/resume failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::ToggleMute => {
                        let muted = audio_bridge.toggle_muted();
                        if let Some(handle) = active_handle.as_ref() {
                            let _ = if muted { handle.mute().await } else { handle.unmute().await };
                        }
                        let _ = event_tx.send(RuntimeEvent::Muted(muted));
                    }
                    RuntimeCommand::SendDtmf(digit) => {
                        let Some(handle) = active_handle.as_ref() else {
                            let _ = event_tx.send(RuntimeEvent::Log("no active call".into()));
                            continue;
                        };
                        match handle.send_dtmf(digit).await {
                            Ok(()) => {
                                let _ = event_tx.send(RuntimeEvent::Log(format!("sent DTMF {digit}")));
                            }
                            Err(err) => {
                                let _ = event_tx.send(RuntimeEvent::Error(format!("DTMF failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::Transfer(target) => {
                        let Some(handle) = active_handle.as_ref() else {
                            let _ = event_tx.send(RuntimeEvent::Log("no active call".into()));
                            continue;
                        };
                        let _ = event_tx.send(RuntimeEvent::State(AppState::Transferring));
                        match handle.transfer_blind(&resolve_target(&options, &target)?).await {
                            Ok(()) => {
                                let _ = event_tx.send(RuntimeEvent::Log(format!("transfer requested to {target}")));
                            }
                            Err(err) => {
                                let _ = event_tx.send(RuntimeEvent::Error(format!("transfer failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::Shutdown => break,
                }
            }
            event = events.next() => {
                let Some(event) = event else {
                    let _ = event_tx.send(RuntimeEvent::Log("session event stream closed".into()));
                    break;
                };
                match event {
                    Event::IncomingCall { call_id, from, to, .. } => {
                        if active_handle.is_some() || pending_incoming.is_some() {
                            let _ = control.reject(&call_id, 486, "Busy Here").await;
                            let _ = event_tx.send(RuntimeEvent::Log(format!("auto-rejected call from {from}; already busy")));
                        } else {
                            pending_incoming = Some((call_id.clone(), from.clone(), to.clone()));
                            let _ = event_tx.send(RuntimeEvent::Incoming {
                                call_id: call_id.to_string(),
                                from,
                                to,
                            });
                        }
                    }
                    Event::CallProgress { call_id, status_code, reason, .. } => {
                        let _ = event_tx.send(RuntimeEvent::Log(format!("{call_id}: {status_code} {reason}")));
                    }
                    Event::CallAnswered { call_id, .. } => {
                        if let Some(handle) = active_handle.as_ref().filter(|h| *h.id() == call_id) {
                            let peer = active_peer_label(&options);
                            let _ = event_tx.send(RuntimeEvent::ActiveCall {
                                call_id: call_id.to_string(),
                                peer,
                            });
                            _running_audio = start_audio(&audio_bridge, handle.clone(), event_tx).await;
                        }
                    }
                    Event::CallEnded { call_id, reason } => {
                        if active_handle.as_ref().is_some_and(|h| *h.id() == call_id) {
                            active_handle = None;
                            _running_audio = None;
                            on_hold = false;
                            let _ = event_tx.send(RuntimeEvent::AudioStopped);
                            let _ = event_tx.send(RuntimeEvent::Log(format!("call ended: {reason}")));
                            let _ = event_tx.send(RuntimeEvent::State(if options.mode == StartupMode::Register {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        }
                    }
                    Event::CallFailed { call_id, status_code, reason } => {
                        if active_handle.as_ref().is_some_and(|h| *h.id() == call_id) {
                            active_handle = None;
                            _running_audio = None;
                            on_hold = false;
                            let _ = event_tx.send(RuntimeEvent::AudioStopped);
                            let _ = event_tx.send(RuntimeEvent::Error(format!("call failed: {status_code} {reason}")));
                            let _ = event_tx.send(RuntimeEvent::State(if options.mode == StartupMode::Register {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        }
                    }
                    Event::CallCancelled { call_id } => {
                        if pending_incoming.as_ref().is_some_and(|(id, _, _)| *id == call_id) {
                            pending_incoming = None;
                            let _ = event_tx.send(RuntimeEvent::Log("incoming call cancelled".into()));
                            let _ = event_tx.send(RuntimeEvent::State(if options.mode == StartupMode::Register {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        }
                    }
                    Event::CallOnHold { call_id } => {
                        if active_handle.as_ref().is_some_and(|h| *h.id() == call_id) {
                            on_hold = true;
                            let _ = event_tx.send(RuntimeEvent::State(AppState::OnHold));
                        }
                    }
                    Event::CallResumed { call_id } => {
                        if active_handle.as_ref().is_some_and(|h| *h.id() == call_id) {
                            on_hold = false;
                            let _ = event_tx.send(RuntimeEvent::State(AppState::InCall));
                        }
                    }
                    Event::RemoteCallOnHold { call_id } => {
                        let _ = event_tx.send(RuntimeEvent::Log(format!("{call_id}: remote hold")));
                    }
                    Event::RemoteCallResumed { call_id } => {
                        let _ = event_tx.send(RuntimeEvent::Log(format!("{call_id}: remote resumed")));
                    }
                    Event::DtmfReceived { call_id, digit } => {
                        let _ = event_tx.send(RuntimeEvent::Log(format!("{call_id}: received DTMF {digit}")));
                    }
                    Event::RegistrationSuccess { registrar, expires, contact } => {
                        let _ = event_tx.send(RuntimeEvent::Registration(format!(
                            "Registered to {registrar}, expires {expires}s, contact {contact}"
                        )));
                    }
                    Event::RegistrationFailed { registrar, status_code, reason } => {
                        let _ = event_tx.send(RuntimeEvent::Registration(format!(
                            "Registration failed at {registrar}: {status_code} {reason}"
                        )));
                    }
                    Event::UnregistrationSuccess { registrar } => {
                        let _ = event_tx.send(RuntimeEvent::Registration(format!("Unregistered from {registrar}")));
                    }
                    Event::NetworkError { error, .. } => {
                        let _ = event_tx.send(RuntimeEvent::Error(format!("network error: {error}")));
                    }
                    other => {
                        if let Some(call_id) = other.call_id() {
                            if other.is_transfer_event() || other.is_media_event() {
                                let _ = event_tx.send(RuntimeEvent::Log(format!("{call_id}: {other:?}")));
                            }
                        }
                    }
                }
            }
        }
    }

    let _ = event_tx.send(RuntimeEvent::State(AppState::ShuttingDown));
    _running_audio = None;
    let _ = event_tx.send(RuntimeEvent::AudioStopped);

    if let Some(handle) = active_handle.take() {
        let _ = handle.hangup_and_wait(Some(Duration::from_secs(3))).await;
    }
    if let Some((call_id, _from, _to)) = pending_incoming.take() {
        let _ = control.reject(&call_id, 486, "Busy Here").await;
    }
    if let Some(handle) = registration_handle.as_ref() {
        let _ = control
            .coordinator()
            .unregister_and_wait(handle, Some(Duration::from_secs(3)))
            .await;
    }
    control
        .coordinator()
        .shutdown_gracefully(Some(Duration::from_secs(3)))
        .await?;
    Ok(())
}

async fn build_endpoint(options: &RuntimeOptions) -> SessionResult<Endpoint> {
    let mut registrar = options
        .registrar
        .clone()
        .map(|uri| apply_transport_to_uri(&uri, options.transport, true));

    let config = build_session_config(options)?;
    let mut builder = Endpoint::builder()
        .name(&options.name)
        .bind_addr(options.bind)
        .profile(EndpointProfile::Custom(config));

    if let Some(advertise) = options.advertise {
        builder = builder.advertised_addr(advertise);
    }
    if let Some(media_public) = options.media_public {
        builder = builder.media_public_addr(media_public);
    }
    if let Some(outbound_proxy) = options.outbound_proxy.as_ref() {
        builder = builder.outbound_proxy(outbound_proxy);
    }

    if options.mode == StartupMode::Register {
        let username = options.username.as_ref().ok_or_else(|| {
            SessionError::ConfigError("--username is required for register mode".into())
        })?;
        let password = options.password.as_ref().ok_or_else(|| {
            SessionError::ConfigError("--password is required for register mode".into())
        })?;
        let registrar_value = registrar.take().ok_or_else(|| {
            SessionError::ConfigError("--registrar is required for register mode".into())
        })?;

        builder = builder
            .account(username)
            .password(password)
            .registrar(registrar_value)
            .expires(options.expires);

        if let Some(auth_username) = options.auth_username.as_ref() {
            builder = builder.auth_username(auth_username);
        }
    }

    builder.build().await
}

fn build_session_config(options: &RuntimeOptions) -> SessionResult<Config> {
    let sip_name = options
        .username
        .as_deref()
        .filter(|_| options.mode == StartupMode::Register)
        .unwrap_or(&options.name);

    let mut config = match options.profile {
        PbxProfile::AsteriskTlsSrtp => Config::asterisk_tls_registered_flow(
            sip_name,
            options.bind,
            format!("urn:uuid:{}", uuid::Uuid::new_v4()),
        ),
        PbxProfile::FreeswitchInternal => Config::freeswitch_internal(sip_name, options.bind),
        PbxProfile::CarrierSbc => {
            let public = options.advertise.ok_or_else(|| {
                SessionError::ConfigError("--advertise is required for carrier-sbc profile".into())
            })?;
            let outbound = options.outbound_proxy.clone().ok_or_else(|| {
                SessionError::ConfigError(
                    "--outbound-proxy is required for carrier-sbc profile".into(),
                )
            })?;
            Config::carrier_sbc(
                sip_name,
                options.bind,
                public,
                outbound,
                format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            )
        }
        PbxProfile::LanPbx | PbxProfile::AsteriskUdp => base_lan_config(sip_name, options)?,
    };

    if let Some(advertise) = options.advertise {
        config.sip_advertised_addr = Some(advertise);
        if config.media_public_addr.is_none() {
            config.media_public_addr = Some(SocketAddr::new(advertise.ip(), 0));
        }
    }
    if let Some(media_public) = options.media_public {
        config.media_public_addr = Some(media_public);
    }
    if let Some(stun) = options.stun.as_ref() {
        config.stun_server = Some(stun.clone());
    }
    if let Some(outbound_proxy) = options.outbound_proxy.as_ref() {
        config.outbound_proxy_uri = Some(outbound_proxy.clone());
    }
    match options.srtp {
        SrtpMode::Off => {
            config.offer_srtp = false;
            config.srtp_required = false;
        }
        SrtpMode::Offer => {
            config.offer_srtp = true;
            config.srtp_required = false;
        }
        SrtpMode::Required => {
            config.offer_srtp = true;
            config.srtp_required = true;
        }
    }
    if options.transport == TransportMode::Tls {
        config.sip_tls_mode = SipTlsMode::ClientOnly;
    }
    Ok(config)
}

fn base_lan_config(name: &str, options: &RuntimeOptions) -> SessionResult<Config> {
    if let Some(advertise) = options.advertise {
        Ok(Config::lan_pbx(name, options.bind, advertise))
    } else if options.bind.ip().is_unspecified() {
        Err(SessionError::ConfigError(
            "--advertise is required when --bind uses 0.0.0.0 or ::".into(),
        ))
    } else if options.bind.ip().is_loopback() {
        let mut config = Config::local(name, options.bind.port());
        config.bind_addr = options.bind;
        Ok(config)
    } else {
        let mut config = Config::on(name, options.bind.ip(), options.bind.port());
        config.bind_addr = options.bind;
        Ok(config)
    }
}

async fn dial_target(
    options: &RuntimeOptions,
    control: &rvoip_session_core::PeerControl,
    target: &str,
) -> SessionResult<SessionHandle> {
    let target = resolve_target(options, target)?;
    control.call(&target).await
}

fn resolve_target(options: &RuntimeOptions, target: &str) -> SessionResult<String> {
    let target = target.trim();
    if target.is_empty() {
        return Err(SessionError::InvalidInput("dial target is empty".into()));
    }
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("sip:") || lower.starts_with("sips:") || lower.starts_with("tel:") {
        return Ok(apply_transport_to_uri(
            target,
            options.transport,
            options.mode == StartupMode::Register,
        ));
    }

    if let Some(registrar) = options.registrar.as_ref() {
        let registrar = apply_transport_to_uri(registrar, options.transport, true);
        let (scheme, host) = split_sip_uri_host(&registrar)?;
        let user = if target.contains('@') {
            target.to_string()
        } else {
            format!("{target}@{host}")
        };
        return Ok(format!("{scheme}:{user}"));
    }

    if target.contains('@') {
        Ok(format!("sip:{target}"))
    } else {
        Err(SessionError::InvalidInput(
            "bare targets need registration mode or a full SIP URI".into(),
        ))
    }
}

fn split_sip_uri_host(uri: &str) -> SessionResult<(&str, String)> {
    let (scheme, rest) = uri
        .split_once(':')
        .ok_or_else(|| SessionError::InvalidInput(format!("invalid SIP URI: {uri}")))?;
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
    Ok((scheme, host))
}

fn apply_transport_to_uri(uri: &str, transport: TransportMode, registrar_or_proxy: bool) -> String {
    match transport {
        TransportMode::Udp => uri.to_string(),
        TransportMode::Tcp => {
            if uri.contains(";transport=") {
                uri.to_string()
            } else {
                format!("{uri};transport=tcp")
            }
        }
        TransportMode::Tls => {
            let tls_uri = if uri.to_ascii_lowercase().starts_with("sip:") {
                format!("sips:{}", &uri[4..])
            } else {
                uri.to_string()
            };
            if registrar_or_proxy || tls_uri.contains(";transport=") {
                tls_uri
            } else {
                format!("{tls_uri};transport=tls")
            }
        }
    }
}

async fn start_audio(
    bridge: &AudioBridge,
    handle: SessionHandle,
    event_tx: &mpsc::UnboundedSender<RuntimeEvent>,
) -> Option<RunningAudio> {
    match bridge.start(handle).await {
        Ok(audio) => Some(audio),
        Err(err) => {
            let _ = event_tx.send(RuntimeEvent::Error(format!("audio failed: {err}")));
            None
        }
    }
}

fn active_peer_label(options: &RuntimeOptions) -> String {
    options
        .target
        .clone()
        .or_else(|| options.registrar.clone())
        .unwrap_or_else(|| "remote".into())
}

fn local_uri_for_options(options: &RuntimeOptions) -> String {
    if options.mode == StartupMode::Register {
        if let (Some(username), Some(registrar)) = (&options.username, &options.registrar) {
            if let Ok((scheme, host)) = split_sip_uri_host(registrar) {
                return format!("{scheme}:{username}@{host}");
            }
        }
    }
    let host = options
        .advertise
        .map(|addr| addr.to_string())
        .unwrap_or_else(|| options.bind.to_string());
    format!("sip:{}@{host}", options.name)
}

fn reachable_address(options: &RuntimeOptions) -> String {
    options
        .advertise
        .map(|addr| format!("sip:{}@{addr}", options.name))
        .unwrap_or_else(|| format!("sip:{}@{}", options.name, options.bind))
}

#[derive(Clone)]
struct AudioBridge {
    input_device: Option<String>,
    output_device: Option<String>,
    muted: Arc<AtomicBool>,
    event_tx: mpsc::UnboundedSender<RuntimeEvent>,
}

impl AudioBridge {
    fn new(
        input_device: Option<String>,
        output_device: Option<String>,
        event_tx: mpsc::UnboundedSender<RuntimeEvent>,
    ) -> Self {
        Self {
            input_device,
            output_device,
            muted: Arc::new(AtomicBool::new(false)),
            event_tx,
        }
    }

    fn toggle_muted(&self) -> bool {
        let next = !self.muted.load(Ordering::SeqCst);
        self.muted.store(next, Ordering::SeqCst);
        next
    }

    async fn start(&self, handle: SessionHandle) -> anyhow::Result<RunningAudio> {
        let audio = handle.audio().await?;
        let (sender, mut receiver) = audio.split();

        let host = cpal::default_host();
        let input = choose_device(&host, true, self.input_device.as_deref())?;
        let output = choose_device(&host, false, self.output_device.as_deref())?;
        let input_name = input.name().unwrap_or_else(|_| "input".into());
        let output_name = output.name().unwrap_or_else(|_| "output".into());

        let input_config = input.default_input_config()?;
        let output_config = output.default_output_config()?;
        let input_sample_rate = input_config.sample_rate().0;
        let output_sample_rate = output_config.sample_rate().0;
        let input_channels = input_config.channels() as usize;
        let output_channels = output_config.channels() as usize;

        let (mic_tx, mut mic_rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let playback_buffer = Arc::new(Mutex::new(VecDeque::<f32>::with_capacity(
            output_sample_rate as usize,
        )));
        let muted = self.muted.clone();
        let event_tx = self.event_tx.clone();

        let input_stream = build_input_stream(
            &input,
            &input_config.into(),
            input_channels,
            mic_tx,
            muted.clone(),
        )?;
        let output_stream = build_output_stream(
            &output,
            &output_config.into(),
            output_channels,
            playback_buffer.clone(),
        )?;

        input_stream.play()?;
        output_stream.play()?;

        let input_task = tokio::spawn(async move {
            let mut mono_buffer = Vec::<f32>::new();
            let mut timestamp = 0u32;
            while let Some(samples) = mic_rx.recv().await {
                let resampled = resample_linear(&samples, input_sample_rate, SAMPLE_RATE);
                mono_buffer.extend(resampled);
                while mono_buffer.len() >= FRAME_SAMPLES {
                    let chunk = mono_buffer.drain(..FRAME_SAMPLES).collect::<Vec<_>>();
                    let pcm = chunk
                        .into_iter()
                        .map(|sample| float_to_i16(sample))
                        .collect::<Vec<_>>();
                    let frame = AudioFrame::new(pcm, SAMPLE_RATE, 1, timestamp);
                    timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);
                    if sender.send(frame).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(FRAME_MS as u64)).await;
                }
            }
        });

        let output_task = tokio::spawn(async move {
            while let Some(frame) = receiver.recv().await {
                let mono = frame
                    .samples
                    .iter()
                    .map(|sample| *sample as f32 / i16::MAX as f32)
                    .collect::<Vec<_>>();
                let resampled = resample_linear(&mono, frame.sample_rate, output_sample_rate);
                if let Ok(mut buffer) = playback_buffer.lock() {
                    buffer.extend(resampled);
                    let max_len = output_sample_rate as usize * 2;
                    while buffer.len() > max_len {
                        buffer.pop_front();
                    }
                } else {
                    let _ = event_tx.send(RuntimeEvent::Error("playback buffer poisoned".into()));
                    break;
                }
            }
        });

        let _ = self.event_tx.send(RuntimeEvent::AudioStarted(format!(
            "{input_name} -> {output_name}"
        )));

        Ok(RunningAudio {
            input_stream,
            output_stream,
            input_task,
            output_task,
        })
    }
}

struct RunningAudio {
    input_stream: cpal::Stream,
    output_stream: cpal::Stream,
    input_task: tokio::task::JoinHandle<()>,
    output_task: tokio::task::JoinHandle<()>,
}

impl Drop for RunningAudio {
    fn drop(&mut self) {
        let _ = &self.input_stream;
        let _ = &self.output_stream;
        self.input_task.abort();
        self.output_task.abort();
    }
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    tx: mpsc::UnboundedSender<Vec<f32>>,
    muted: Arc<AtomicBool>,
) -> anyhow::Result<cpal::Stream> {
    let err_fn = |err| eprintln!("input stream error: {err}");
    let sample_format = device.default_input_config()?.sample_format();
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _| {
                if muted.load(Ordering::SeqCst) {
                    let _ = tx.send(vec![0.0; data.len() / channels.max(1)]);
                } else {
                    let _ = tx.send(mix_to_mono(data, channels));
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _| {
                if muted.load(Ordering::SeqCst) {
                    let _ = tx.send(vec![0.0; data.len() / channels.max(1)]);
                } else {
                    let converted = data
                        .iter()
                        .map(|sample| *sample as f32 / i16::MAX as f32)
                        .collect::<Vec<_>>();
                    let _ = tx.send(mix_to_mono(&converted, channels));
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            config,
            move |data: &[u16], _| {
                if muted.load(Ordering::SeqCst) {
                    let _ = tx.send(vec![0.0; data.len() / channels.max(1)]);
                } else {
                    let converted = data
                        .iter()
                        .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0)
                        .collect::<Vec<_>>();
                    let _ = tx.send(mix_to_mono(&converted, channels));
                }
            },
            err_fn,
            None,
        )?,
        other => anyhow::bail!("unsupported input sample format {other:?}"),
    };
    Ok(stream)
}

fn build_output_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    playback_buffer: Arc<Mutex<VecDeque<f32>>>,
) -> anyhow::Result<cpal::Stream> {
    let err_fn = |err| eprintln!("output stream error: {err}");
    let sample_format = device.default_output_config()?.sample_format();
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            config,
            move |data: &mut [f32], _| fill_output(data, channels, &playback_buffer, |s| s),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream(
            config,
            move |data: &mut [i16], _| fill_output(data, channels, &playback_buffer, float_to_i16),
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_output_stream(
            config,
            move |data: &mut [u16], _| {
                fill_output(data, channels, &playback_buffer, |sample| {
                    ((sample.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16
                })
            },
            err_fn,
            None,
        )?,
        other => anyhow::bail!("unsupported output sample format {other:?}"),
    };
    Ok(stream)
}

fn fill_output<T: Copy>(
    data: &mut [T],
    channels: usize,
    playback_buffer: &Arc<Mutex<VecDeque<f32>>>,
    convert: impl Fn(f32) -> T,
) {
    let zero = convert(0.0);
    if let Ok(mut buffer) = playback_buffer.lock() {
        for frame in data.chunks_mut(channels.max(1)) {
            let sample = buffer.pop_front().unwrap_or(0.0);
            let converted = convert(sample);
            for out in frame {
                *out = converted;
            }
        }
    } else {
        for out in data {
            *out = zero;
        }
    }
}

fn mix_to_mono(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
        .collect()
}

fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if input.is_empty() || from_rate == to_rate {
        return input.to_vec();
    }
    let out_len = ((input.len() as u64 * to_rate as u64) / from_rate as u64).max(1) as usize;
    let ratio = from_rate as f32 / to_rate as f32;
    let mut output = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f32 * ratio;
        let idx = pos.floor() as usize;
        let frac = pos - idx as f32;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        output.push(a + (b - a) * frac);
    }
    output
}

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

fn choose_device(
    host: &cpal::Host,
    input: bool,
    selector: Option<&str>,
) -> anyhow::Result<cpal::Device> {
    if let Some(selector) = selector {
        let devices = if input {
            host.input_devices()?
        } else {
            host.output_devices()?
        }
        .collect::<Vec<_>>();

        if let Ok(index) = selector.parse::<usize>() {
            return devices
                .into_iter()
                .nth(index)
                .ok_or_else(|| anyhow::anyhow!("audio device index {index} not found"));
        }

        let needle = selector.to_ascii_lowercase();
        return devices
            .into_iter()
            .find(|device| {
                device
                    .name()
                    .map(|name| name.to_ascii_lowercase().contains(&needle))
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow::anyhow!("audio device matching '{selector}' not found"));
    }

    if input {
        host.default_input_device()
            .ok_or_else(|| anyhow::anyhow!("no default input device"))
    } else {
        host.default_output_device()
            .ok_or_else(|| anyhow::anyhow!("no default output device"))
    }
}

fn list_audio_devices() -> anyhow::Result<()> {
    let host = cpal::default_host();
    println!("Input devices:");
    for (idx, device) in host.input_devices()?.enumerate() {
        println!(
            "  {idx}: {}",
            device.name().unwrap_or_else(|_| "<unknown>".into())
        );
    }
    println!();
    println!("Output devices:");
    for (idx, device) in host.output_devices()?.enumerate() {
        println!(
            "  {idx}: {}",
            device.name().unwrap_or_else(|_| "<unknown>".into())
        );
    }
    Ok(())
}

fn draw_ui(frame: &mut Frame<'_>, app: &TuiApp) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(5),
        ])
        .split(frame.size());

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(root[0]);

    let account = vec![
        Line::from(vec![
            Span::styled("Mode: ", Style::default().fg(Color::Gray)),
            Span::raw(match app.options.mode {
                StartupMode::Lan => "lan",
                StartupMode::Listen => "listen",
                StartupMode::Register => "register",
            }),
        ]),
        Line::from(format!("State: {}", app.state.label())),
        Line::from(format!("Registration: {}", app.registration)),
        Line::from(format!("Local URI: {}", app.local_uri)),
        Line::from(format!("Reachable: {}", app.reachable)),
    ];
    frame.render_widget(
        Paragraph::new(account)
            .block(Block::default().title("Account").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        top[0],
    );

    let call = vec![
        Line::from(format!(
            "Incoming: {}",
            app.incoming
                .as_ref()
                .map(|(_, from, _)| from.as_str())
                .unwrap_or("-")
        )),
        Line::from(format!(
            "Active call: {}",
            app.active_call
                .as_ref()
                .map(|(id, peer)| format!("{id} / {peer}"))
                .unwrap_or_else(|| "-".into())
        )),
        Line::from(format!("Audio: {}", app.audio)),
        Line::from(format!("Mic muted: {}", app.muted)),
        Line::from(input_status(app)),
    ];
    frame.render_widget(
        Paragraph::new(call)
            .block(Block::default().title("Call").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        top[1],
    );

    let input_title = match app.input_mode {
        InputMode::Normal => "Input",
        InputMode::Dial => "Dial Target (Enter to dial, Esc to cancel)",
        InputMode::Transfer => "Transfer Target (Enter to transfer, Esc to cancel)",
    };
    frame.render_widget(
        Paragraph::new(app.input.as_str())
            .block(Block::default().title(input_title).borders(Borders::ALL)),
        root[1],
    );

    let logs = app
        .logs
        .iter()
        .rev()
        .take(root[2].height.saturating_sub(2) as usize)
        .rev()
        .map(|line| ListItem::new(Line::from(line.clone())))
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(logs).block(Block::default().title("Events").borders(Borders::ALL)),
        root[2],
    );

    let help = Line::from(vec![
        Span::styled("d", key_style()),
        Span::raw(" dial  "),
        Span::styled("a", key_style()),
        Span::raw(" answer  "),
        Span::styled("r", key_style()),
        Span::raw(" reject  "),
        Span::styled("h", key_style()),
        Span::raw(" hangup  "),
        Span::styled("m", key_style()),
        Span::raw(" mute  "),
        Span::styled("o", key_style()),
        Span::raw(" hold/resume  "),
        Span::styled("0-9*#", key_style()),
        Span::raw(" DTMF  "),
        Span::styled("t", key_style()),
        Span::raw(" transfer  "),
        Span::styled("q", key_style()),
        Span::raw(" quit"),
    ]);
    frame.render_widget(
        Paragraph::new(help)
            .block(Block::default().title("Keys").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        root[3],
    );
}

fn key_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn input_status(app: &TuiApp) -> String {
    match app.input_mode {
        InputMode::Normal => "Input: normal".into(),
        InputMode::Dial => "Input: dial target".into(),
        InputMode::Transfer => "Input: transfer target".into(),
    }
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    fn draw<F>(&mut self, f: F) -> io::Result<()>
    where
        F: FnOnce(&mut Frame<'_>),
    {
        self.terminal.draw(f).map(|_| ())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

fn load_file_config(path: Option<PathBuf>) -> anyhow::Result<FileConfig> {
    let path = path.unwrap_or_else(default_config_path);
    if !path.exists() {
        return Ok(FileConfig::default());
    }
    let text = fs::read_to_string(&path)?;
    let config = toml::from_str::<FileConfig>(&text)?;
    Ok(config)
}

fn default_config_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".config/rvoip/sip-client.toml")
    } else {
        PathBuf::from("sip-client.toml")
    }
}

fn build_runtime_options(cli: &Cli, file_config: &FileConfig) -> anyhow::Result<RuntimeOptions> {
    let profile_name = cli
        .config_profile
        .as_deref()
        .or(file_config.default_profile.as_deref());
    let profile = profile_name
        .and_then(|name| file_config.profiles.as_ref()?.get(name))
        .cloned()
        .unwrap_or_default();

    let mut options = RuntimeOptions::from_profile(profile)?;

    match cli.command.clone() {
        Some(Command::Lan(args)) => {
            options.mode = StartupMode::Lan;
            apply_common_args(&mut options, args)?;
        }
        Some(Command::Listen(args)) => {
            options.mode = StartupMode::Listen;
            apply_common_args(&mut options, args)?;
        }
        Some(Command::Register(args)) => {
            options.mode = StartupMode::Register;
            apply_common_args(&mut options, args.common)?;
            if let Some(value) = args.username {
                options.username = Some(value);
            }
            if let Some(value) = args.auth_username {
                options.auth_username = Some(value);
            }
            if let Some(value) = args.password {
                options.password = Some(value);
            }
            if let Some(value) = args.registrar {
                options.registrar = Some(value);
            }
            if let Some(value) = args.expires {
                options.expires = value;
            }
        }
        None => {
            if profile_name.is_none() {
                anyhow::bail!("provide a startup mode or --config-profile");
            }
        }
    }

    if options.mode == StartupMode::Register {
        if options.username.is_none() {
            anyhow::bail!("register mode requires --username or config username");
        }
        if options.password.is_none() {
            anyhow::bail!("register mode requires --password or config password");
        }
        if options.registrar.is_none() {
            anyhow::bail!("register mode requires --registrar or config registrar");
        }
    }

    Ok(options)
}

impl RuntimeOptions {
    fn from_profile(profile: ProfileConfig) -> anyhow::Result<Self> {
        let mode = match profile.mode.as_deref().unwrap_or("lan") {
            "lan" => StartupMode::Lan,
            "listen" => StartupMode::Listen,
            "register" => StartupMode::Register,
            other => anyhow::bail!("unknown config mode '{other}'"),
        };

        Ok(Self {
            mode,
            name: profile.name.unwrap_or_else(|| "endpoint".into()),
            bind: parse_socket(profile.bind.as_deref())?
                .unwrap_or_else(|| SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 5060)),
            advertise: parse_socket(profile.advertise.as_deref())?,
            media_public: parse_media_public(profile.media_public.as_deref())?,
            stun: profile.stun,
            outbound_proxy: profile.outbound_proxy,
            transport: profile.transport.unwrap_or(TransportMode::Udp),
            srtp: profile.srtp.unwrap_or(SrtpMode::Off),
            profile: profile.profile.unwrap_or(PbxProfile::LanPbx),
            input_device: profile.input_device,
            output_device: profile.output_device,
            target: profile.target,
            username: profile.username,
            auth_username: profile.auth_username,
            password: profile.password,
            registrar: profile.registrar,
            expires: profile.expires.unwrap_or(3600),
        })
    }
}

fn apply_common_args(options: &mut RuntimeOptions, args: CommonArgs) -> anyhow::Result<()> {
    if let Some(value) = args.name {
        options.name = value;
    }
    if let Some(value) = args.bind {
        options.bind = value;
    }
    if let Some(value) = args.advertise {
        options.advertise = Some(value);
    }
    if let Some(value) = args.media_public {
        options.media_public = Some(parse_media_public(Some(&value))?.ok_or_else(|| {
            anyhow::anyhow!("--media-public was provided but could not be parsed")
        })?);
    }
    if let Some(value) = args.stun {
        options.stun = Some(value);
    }
    if let Some(value) = args.outbound_proxy {
        options.outbound_proxy = Some(value);
    }
    if let Some(value) = args.transport {
        options.transport = value;
    }
    if let Some(value) = args.srtp {
        options.srtp = value;
    }
    if let Some(value) = args.profile {
        options.profile = value;
    }
    if let Some(value) = args.input_device {
        options.input_device = Some(value);
    }
    if let Some(value) = args.output_device {
        options.output_device = Some(value);
    }
    if let Some(value) = args.target {
        options.target = Some(value);
    }
    Ok(())
}

fn parse_socket(value: Option<&str>) -> anyhow::Result<Option<SocketAddr>> {
    value
        .map(SocketAddr::from_str)
        .transpose()
        .map_err(|err| anyhow::anyhow!(err))
}

fn parse_media_public(value: Option<&str>) -> anyhow::Result<Option<SocketAddr>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Ok(Some(addr));
    }
    let ip = value.parse::<IpAddr>()?;
    Ok(Some(SocketAddr::new(ip, 0)))
}
