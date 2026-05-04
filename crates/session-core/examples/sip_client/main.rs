//! General-purpose terminal SIP client built on the Endpoint facade.

use std::collections::VecDeque;
use std::fs;
use std::io::{self, Stdout};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{
    event::{self, Event as TerminalEvent, KeyCode, KeyEvent, KeyEventKind},
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
use tokio::sync::mpsc;

use rvoip_session_core::{
    Endpoint, EndpointAccountConfig, EndpointAudioFrame, EndpointAudioSender, EndpointCall,
    EndpointCallId, EndpointConfig, EndpointControl, EndpointEvent, EndpointEvents,
    EndpointIncomingCall, EndpointMediaConfig, EndpointNetworkConfig, EndpointProfileName,
    EndpointRegistrationInfo, EndpointRegistrationStatus, EndpointSrtpMode, EndpointTransport,
    Result as SipResult, SessionError,
};

const SAMPLE_RATE: u32 = 8_000;
const FRAME_MS: u32 = 20;
const FRAME_SAMPLES: usize = (SAMPLE_RATE as usize * FRAME_MS as usize) / 1_000;
const MAX_LOGS: usize = 200;

#[derive(Parser, Debug)]
#[command(name = "sip_client")]
#[command(about = "Endpoint-only interactive and smoke-test SIP client")]
struct Cli {
    /// JSON endpoint config path.
    #[arg(long)]
    config: Option<PathBuf>,

    /// List audio devices and exit.
    #[arg(long)]
    list_devices: bool,

    /// Display/configuration name.
    #[arg(long)]
    name: Option<String>,

    /// SIP username or extension.
    #[arg(long)]
    username: Option<String>,

    /// Digest auth username when it differs from username.
    #[arg(long)]
    auth_username: Option<String>,

    /// Digest auth password.
    #[arg(long)]
    password: Option<String>,

    /// SIP registrar URI.
    #[arg(long)]
    registrar: Option<String>,

    /// Register on startup.
    #[arg(long)]
    register: bool,

    /// Dial this target after startup.
    #[arg(long)]
    dial: Option<String>,

    /// SIP bind address.
    #[arg(long)]
    bind: Option<SocketAddr>,

    /// SIP advertised address.
    #[arg(long)]
    advertise: Option<SocketAddr>,

    /// Preferred signalling transport.
    #[arg(long, value_enum)]
    transport: Option<CliTransport>,

    /// STUN server for media public address discovery.
    #[arg(long)]
    stun: Option<String>,

    /// Outbound proxy URI.
    #[arg(long)]
    outbound_proxy: Option<String>,

    /// Deployment profile.
    #[arg(long, value_enum)]
    profile: Option<CliProfile>,

    /// Public media address as IP or ip:port.
    #[arg(long)]
    media_public: Option<String>,

    /// SRTP negotiation mode.
    #[arg(long, value_enum)]
    srtp: Option<CliSrtp>,

    /// Input device name substring or index from --list-devices.
    #[arg(long)]
    input_device: Option<String>,

    /// Output device name substring or index from --list-devices.
    #[arg(long)]
    output_device: Option<String>,

    /// Run noninteractive smoke mode.
    #[arg(long, value_enum)]
    test: Option<TestRole>,

    /// Smoke test duration after answer, in seconds.
    #[arg(long, default_value_t = 5)]
    test_duration: u64,

    /// Smoke test operation timeout, in seconds.
    #[arg(long, default_value_t = 30)]
    test_timeout: u64,

    /// DTMF digit to send in smoke mode.
    #[arg(long, default_value_t = '5')]
    test_dtmf: char,

    /// Smoke-test audio backend.
    #[arg(long, value_enum, default_value_t = TestAudio::Synthetic)]
    test_audio: TestAudio,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliTransport {
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
enum CliSrtp {
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
enum CliProfile {
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
enum TestRole {
    Caller,
    Callee,
    PbxCaller,
    PbxCallee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TestAudio {
    Synthetic,
    Cpal,
}

#[derive(Debug, Clone)]
struct RuntimeOptions {
    endpoint: EndpointConfig,
    register_on_start: bool,
    dial: Option<String>,
    input_device: Option<String>,
    output_device: Option<String>,
    test_duration: Duration,
    test_timeout: Duration,
    test_dtmf: char,
    test_audio: TestAudio,
}

#[derive(Debug)]
enum RuntimeCommand {
    Dial(String),
    Answer,
    Reject,
    Hangup,
    ToggleMute,
    HoldResume,
    SendDtmf(char),
    Transfer(String),
    Shutdown,
}

#[derive(Debug)]
enum UiEvent {
    Ready {
        local: String,
    },
    State(AppState),
    Registration(String),
    Incoming {
        id: String,
        from: String,
        to: String,
    },
    ActiveCall {
        id: String,
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
            Self::Idle => "idle",
            Self::Registering => "registering",
            Self::Registered => "registered",
            Self::Incoming => "incoming",
            Self::Calling => "calling",
            Self::InCall => "in call",
            Self::OnHold => "on hold",
            Self::Transferring => "transferring",
            Self::ShuttingDown => "shutting down",
            Self::Error => "error",
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
    local: String,
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
        Self {
            options,
            state: AppState::Idle,
            registration: "not registered".into(),
            local: "-".into(),
            incoming: None,
            active_call: None,
            audio: "stopped".into(),
            muted: false,
            input_mode: InputMode::Normal,
            input: String::new(),
            logs: VecDeque::new(),
            should_quit: false,
        }
    }

    fn push_log(&mut self, message: impl Into<String>) {
        if self.logs.len() >= MAX_LOGS {
            self.logs.pop_front();
        }
        self.logs.push_back(message.into());
    }

    fn apply_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::Ready { local } => {
                self.local = local;
                self.push_log("endpoint ready");
            }
            UiEvent::State(state) => self.state = state,
            UiEvent::Registration(status) => {
                self.registration = status.clone();
                self.push_log(status);
            }
            UiEvent::Incoming { id, from, to } => {
                self.state = AppState::Incoming;
                self.incoming = Some((id, from.clone(), to));
                self.push_log(format!("incoming call from {from}"));
            }
            UiEvent::ActiveCall { id, peer } => {
                self.state = AppState::InCall;
                self.incoming = None;
                self.active_call = Some((id, peer));
            }
            UiEvent::AudioStarted(device) => {
                self.audio = device.clone();
                self.push_log(format!("audio started: {device}"));
            }
            UiEvent::AudioStopped => {
                self.audio = "stopped".into();
            }
            UiEvent::Muted(muted) => {
                self.muted = muted;
                self.push_log(if muted {
                    "microphone muted"
                } else {
                    "microphone unmuted"
                });
            }
            UiEvent::Log(message) => self.push_log(message),
            UiEvent::Error(message) => {
                self.state = AppState::Error;
                self.push_log(format!("error: {message}"));
            }
            UiEvent::ShutdownComplete => {
                self.should_quit = true;
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let log_to_stderr = std::env::var_os("RVOIP_SIP_CLIENT_LOG").is_some();
    let subscriber = tracing_subscriber::fmt().with_env_filter(
        std::env::var("RUST_LOG").unwrap_or_else(|_| "rvoip_session_core=info,warn".into()),
    );
    if log_to_stderr {
        subscriber.with_writer(io::stderr).init();
    } else {
        subscriber.with_writer(io::sink).init();
    }

    let cli = Cli::parse();
    if cli.list_devices {
        list_audio_devices()?;
        return Ok(());
    }

    let options = build_runtime_options(&cli)?;
    if let Some(role) = cli.test {
        run_smoke(role, options).await?;
        return Ok(());
    }

    run_tui(options).await
}

async fn run_tui(options: RuntimeOptions) -> anyhow::Result<()> {
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
            if let TerminalEvent::Key(key) = event::read()? {
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
    event_tx: mpsc::UnboundedSender<UiEvent>,
) {
    if let Err(err) = run_runtime_inner(options, &mut command_rx, &event_tx).await {
        let _ = event_tx.send(UiEvent::Error(err.to_string()));
    }
    let _ = event_tx.send(UiEvent::ShutdownComplete);
}

async fn run_runtime_inner(
    options: RuntimeOptions,
    command_rx: &mut mpsc::UnboundedReceiver<RuntimeCommand>,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
) -> SipResult<()> {
    let mut endpoint = Endpoint::from_config(options.endpoint.clone()).await?;
    if options.register_on_start {
        let _ = event_tx.send(UiEvent::State(AppState::Registering));
        match endpoint.register_and_wait(Some(options.test_timeout)).await {
            Ok(info) => {
                let _ = event_tx.send(UiEvent::State(AppState::Registered));
                let _ = event_tx.send(UiEvent::Registration(format_registration(&info)));
            }
            Err(err) => {
                let _ = event_tx.send(UiEvent::Registration(format!("registration failed: {err}")));
                return Err(err);
            }
        }
    }

    let local = local_label(&options.endpoint);
    let _ = event_tx.send(UiEvent::Ready { local });
    let (control, mut events) = endpoint.split();
    let audio_bridge = AudioBridge::new(
        options.input_device.clone(),
        options.output_device.clone(),
        event_tx.clone(),
    );
    let mut active_call: Option<EndpointCall> = None;
    let mut pending_incoming: Option<EndpointIncomingCall> = None;
    let mut running_audio: Option<RunningAudio> = None;
    let mut on_hold = false;

    if let Some(target) = options.dial.as_ref() {
        match control.call(target).await {
            Ok(call) => {
                let _ = event_tx.send(UiEvent::State(AppState::Calling));
                let _ = event_tx.send(UiEvent::Log(format!("calling {target} ({})", call.id())));
                active_call = Some(call);
            }
            Err(err) => {
                let _ = event_tx.send(UiEvent::Error(format!("dial failed: {err}")));
            }
        }
    } else if !options.register_on_start {
        let _ = event_tx.send(UiEvent::State(AppState::Idle));
    }

    loop {
        let _keep_audio_streams_alive = running_audio.as_ref();
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else { break; };
                match command {
                    RuntimeCommand::Dial(target) => {
                        if active_call.is_some() {
                            let _ = event_tx.send(UiEvent::Log("already in a call".into()));
                            continue;
                        }
                        match control.call(&target).await {
                            Ok(call) => {
                                let _ = event_tx.send(UiEvent::State(AppState::Calling));
                                let _ = event_tx.send(UiEvent::Log(format!("calling {target} ({})", call.id())));
                                active_call = Some(call);
                                on_hold = false;
                            }
                            Err(err) => {
                                let _ = event_tx.send(UiEvent::Error(format!("dial failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::Answer => {
                        let Some(incoming) = pending_incoming.take() else {
                            let _ = event_tx.send(UiEvent::Log("no incoming call to answer".into()));
                            continue;
                        };
                        let peer = incoming.from().to_string();
                        match incoming.answer().await {
                            Ok(call) => {
                                let _ = event_tx.send(UiEvent::ActiveCall {
                                    id: call.id().to_string(),
                                    peer,
                                });
                                running_audio = start_cpal_audio(&audio_bridge, call.clone(), event_tx).await;
                                active_call = Some(call);
                                on_hold = false;
                            }
                            Err(err) => {
                                let _ = event_tx.send(UiEvent::Error(format!("answer failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::Reject => {
                        let Some(incoming) = pending_incoming.take() else {
                            let _ = event_tx.send(UiEvent::Log("no incoming call to reject".into()));
                            continue;
                        };
                        let from = incoming.from().to_string();
                        if let Err(err) = incoming.busy().await {
                            let _ = event_tx.send(UiEvent::Error(format!("reject failed: {err}")));
                        } else {
                            let _ = event_tx.send(UiEvent::Log(format!("rejected call from {from}")));
                            let _ = event_tx.send(UiEvent::State(if options.register_on_start {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        }
                    }
                    RuntimeCommand::Hangup => {
                        if let Some(call) = active_call.take() {
                            let _ = event_tx.send(UiEvent::Log(format!("hanging up {}", call.id())));
                            let _ = call.hangup_and_wait(Some(Duration::from_secs(5))).await;
                            running_audio = None;
                            let _ = event_tx.send(UiEvent::AudioStopped);
                            let _ = event_tx.send(UiEvent::State(if options.register_on_start {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        } else if let Some(incoming) = pending_incoming.take() {
                            let _ = incoming.busy().await;
                            let _ = event_tx.send(UiEvent::State(if options.register_on_start {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        } else {
                            let _ = event_tx.send(UiEvent::Log("no call to hang up".into()));
                        }
                    }
                    RuntimeCommand::HoldResume => {
                        let Some(call) = active_call.as_ref() else {
                            let _ = event_tx.send(UiEvent::Log("no active call".into()));
                            continue;
                        };
                        let result = if on_hold { call.resume().await } else { call.hold().await };
                        match result {
                            Ok(()) => {
                                on_hold = !on_hold;
                                let _ = event_tx.send(UiEvent::State(if on_hold {
                                    AppState::OnHold
                                } else {
                                    AppState::InCall
                                }));
                            }
                            Err(err) => {
                                let _ = event_tx.send(UiEvent::Error(format!("hold/resume failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::ToggleMute => {
                        let muted = audio_bridge.toggle_muted();
                        if let Some(call) = active_call.as_ref() {
                            let _ = if muted { call.mute().await } else { call.unmute().await };
                        }
                        let _ = event_tx.send(UiEvent::Muted(muted));
                    }
                    RuntimeCommand::SendDtmf(digit) => {
                        let Some(call) = active_call.as_ref() else {
                            let _ = event_tx.send(UiEvent::Log("no active call".into()));
                            continue;
                        };
                        match call.send_dtmf(digit).await {
                            Ok(()) => {
                                let _ = event_tx.send(UiEvent::Log(format!("sent DTMF {digit}")));
                            }
                            Err(err) => {
                                let _ = event_tx.send(UiEvent::Error(format!("DTMF failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::Transfer(target) => {
                        let Some(call) = active_call.as_ref() else {
                            let _ = event_tx.send(UiEvent::Log("no active call".into()));
                            continue;
                        };
                        let _ = event_tx.send(UiEvent::State(AppState::Transferring));
                        match call.transfer(&target).await {
                            Ok(()) => {
                                let _ = event_tx.send(UiEvent::Log(format!("transfer requested to {target}")));
                            }
                            Err(err) => {
                                let _ = event_tx.send(UiEvent::Error(format!("transfer failed: {err}")));
                            }
                        }
                    }
                    RuntimeCommand::Shutdown => break,
                }
            }
            event = events.next() => {
                let Some(event) = event? else {
                    let _ = event_tx.send(UiEvent::Log("endpoint event stream closed".into()));
                    break;
                };
                match event {
                    EndpointEvent::IncomingCall(incoming) => {
                        if active_call.is_some() || pending_incoming.is_some() {
                            let from = incoming.from().to_string();
                            let _ = incoming.busy().await;
                            let _ = event_tx.send(UiEvent::Log(format!("auto-rejected call from {from}; already busy")));
                        } else {
                            let id = incoming.id().to_string();
                            let from = incoming.from().to_string();
                            let to = incoming.to().to_string();
                            pending_incoming = Some(incoming);
                            let _ = event_tx.send(UiEvent::Incoming { id, from, to });
                        }
                    }
                    EndpointEvent::CallProgress { call_id, status_code, reason, .. } => {
                        let _ = event_tx.send(UiEvent::Log(format!("{call_id}: {status_code} {reason}")));
                    }
                    EndpointEvent::CallAnswered { call, .. } => {
                        if active_call.as_ref().is_some_and(|c| c.id() == call.id()) {
                            let peer = options.dial.clone().unwrap_or_else(|| "remote".into());
                            let _ = event_tx.send(UiEvent::ActiveCall {
                                id: call.id().to_string(),
                                peer,
                            });
                            running_audio = start_cpal_audio(&audio_bridge, call.clone(), event_tx).await;
                            active_call = Some(call);
                        }
                    }
                    EndpointEvent::CallEnded { call_id, reason } => {
                        if active_call.as_ref().is_some_and(|c| c.id() == call_id) {
                            active_call = None;
                            running_audio = None;
                            on_hold = false;
                            let _ = event_tx.send(UiEvent::AudioStopped);
                            let _ = event_tx.send(UiEvent::Log(format!("call ended: {reason}")));
                            let _ = event_tx.send(UiEvent::State(if options.register_on_start {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        }
                    }
                    EndpointEvent::CallFailed { call_id, status_code, reason } => {
                        if active_call.as_ref().is_some_and(|c| c.id() == call_id) {
                            active_call = None;
                            running_audio = None;
                            on_hold = false;
                            let _ = event_tx.send(UiEvent::AudioStopped);
                            let _ = event_tx.send(UiEvent::Error(format!("call failed: {status_code} {reason}")));
                            let _ = event_tx.send(UiEvent::State(if options.register_on_start {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        }
                    }
                    EndpointEvent::CallCancelled { call_id } => {
                        if pending_incoming.as_ref().is_some_and(|i| i.id() == call_id) {
                            pending_incoming = None;
                            let _ = event_tx.send(UiEvent::Log("incoming call cancelled".into()));
                            let _ = event_tx.send(UiEvent::State(if options.register_on_start {
                                AppState::Registered
                            } else {
                                AppState::Idle
                            }));
                        }
                    }
                    EndpointEvent::LocalHold { call_id } => {
                        if active_call.as_ref().is_some_and(|c| c.id() == call_id) {
                            on_hold = true;
                            let _ = event_tx.send(UiEvent::State(AppState::OnHold));
                        }
                    }
                    EndpointEvent::LocalResume { call_id } => {
                        if active_call.as_ref().is_some_and(|c| c.id() == call_id) {
                            on_hold = false;
                            let _ = event_tx.send(UiEvent::State(AppState::InCall));
                        }
                    }
                    EndpointEvent::RemoteHold { call_id } => {
                        let _ = event_tx.send(UiEvent::Log(format!("{call_id}: remote hold")));
                    }
                    EndpointEvent::RemoteResume { call_id } => {
                        let _ = event_tx.send(UiEvent::Log(format!("{call_id}: remote resumed")));
                    }
                    EndpointEvent::DtmfReceived { call_id, digit } => {
                        let _ = event_tx.send(UiEvent::Log(format!("{call_id}: received DTMF {digit}")));
                    }
                    EndpointEvent::RegistrationChanged(info) => {
                        let _ = event_tx.send(UiEvent::Registration(format_registration(&info)));
                    }
                    EndpointEvent::NetworkError { error, .. } => {
                        let _ = event_tx.send(UiEvent::Error(format!("network error: {error}")));
                    }
                    EndpointEvent::Info { call_id, message } => {
                        if let Some(call_id) = call_id {
                            let _ = event_tx.send(UiEvent::Log(format!("{call_id}: {message}")));
                        }
                    }
                }
            }
        }
    }

    let _ = event_tx.send(UiEvent::State(AppState::ShuttingDown));
    running_audio = None;
    let _ = event_tx.send(UiEvent::AudioStopped);
    if let Some(call) = active_call.take() {
        let _ = call.hangup_and_wait(Some(Duration::from_secs(3))).await;
    }
    if let Some(incoming) = pending_incoming.take() {
        let _ = incoming.busy().await;
    }
    if options.register_on_start {
        let _ = control
            .unregister_and_wait(Some(Duration::from_secs(3)))
            .await;
    }
    control.shutdown().await?;
    drop(running_audio);
    Ok(())
}

async fn run_smoke(role: TestRole, options: RuntimeOptions) -> anyhow::Result<()> {
    let mut endpoint = Endpoint::from_config(options.endpoint.clone()).await?;
    let must_register =
        options.register_on_start || matches!(role, TestRole::PbxCaller | TestRole::PbxCallee);
    if must_register {
        let info = endpoint
            .register_and_wait(Some(options.test_timeout))
            .await?;
        println!("{}", format_registration(&info));
        if info.status != EndpointRegistrationStatus::Registered {
            anyhow::bail!("registration did not complete: {:?}", info.status);
        }
    }
    let (control, events) = endpoint.split();
    let result = match role {
        TestRole::Caller | TestRole::PbxCaller => smoke_caller(&options, control, events).await,
        TestRole::Callee | TestRole::PbxCallee => smoke_callee(&options, control, events).await,
    };
    result
}

async fn smoke_caller(
    options: &RuntimeOptions,
    control: EndpointControl,
    mut events: EndpointEvents,
) -> anyhow::Result<()> {
    let target = options
        .dial
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("caller smoke mode requires --dial"))?;
    let call = control.call(target).await?;
    println!("calling {target} ({})", call.id());
    let call = wait_for_answered(&mut events, call.id(), options.test_timeout).await?;
    println!("answered {}", call.id());

    let audio = start_test_audio(call.clone(), options).await?;
    call.send_dtmf(options.test_dtmf).await?;
    println!("sent DTMF {}", options.test_dtmf);
    call.hold().await?;
    wait_for_call_event(&mut events, call.id(), options.test_timeout, |event| {
        matches!(event, EndpointEvent::LocalHold { .. })
    })
    .await?;
    call.resume().await?;
    wait_for_call_event(&mut events, call.id(), options.test_timeout, |event| {
        matches!(event, EndpointEvent::LocalResume { .. })
    })
    .await?;
    tokio::time::sleep(options.test_duration).await;
    call.hangup_and_wait(Some(options.test_timeout)).await?;
    audio.require_media()?;
    control.shutdown().await?;
    println!("caller smoke passed");
    Ok(())
}

async fn smoke_callee(
    options: &RuntimeOptions,
    control: EndpointControl,
    mut events: EndpointEvents,
) -> anyhow::Result<()> {
    let incoming = wait_for_incoming(&mut events, options.test_timeout).await?;
    println!("answering incoming call from {}", incoming.from());
    let call = incoming.answer().await?;
    let audio = start_test_audio(call.clone(), options).await?;
    let deadline = Instant::now() + options.test_timeout + options.test_duration;
    let mut saw_dtmf = false;
    let mut saw_end = false;
    while Instant::now() < deadline {
        let timeout = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(timeout, events.next()).await {
            Ok(Ok(Some(EndpointEvent::DtmfReceived { digit, .. })))
                if digit == options.test_dtmf =>
            {
                saw_dtmf = true;
                println!("received DTMF {digit}");
            }
            Ok(Ok(Some(EndpointEvent::CallEnded { .. }))) => {
                saw_end = true;
                break;
            }
            Ok(Ok(Some(_))) => {}
            Ok(Ok(None)) => break,
            Ok(Err(err)) => return Err(err.into()),
            Err(_) => break,
        }
    }
    if !saw_dtmf {
        anyhow::bail!("callee did not receive expected DTMF {}", options.test_dtmf);
    }
    if !saw_end {
        anyhow::bail!("callee did not observe call end");
    }
    audio.require_media()?;
    if options.register_on_start {
        let _ = control
            .unregister_and_wait(Some(Duration::from_secs(3)))
            .await;
    }
    control.shutdown().await?;
    println!("callee smoke passed");
    Ok(())
}

async fn wait_for_answered(
    events: &mut EndpointEvents,
    expected: EndpointCallId,
    timeout: Duration,
) -> anyhow::Result<EndpointCall> {
    let fut = async {
        loop {
            match events.next().await? {
                Some(EndpointEvent::CallAnswered { call, .. }) if call.id() == expected => {
                    return Ok(call)
                }
                Some(EndpointEvent::CallFailed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(SessionError::Other(format!(
                        "call failed: {status_code} {reason}"
                    )))
                }
                Some(_) => {}
                None => return Err(SessionError::Other("event stream closed".into())),
            }
        }
    };
    Ok(tokio::time::timeout(timeout, fut).await??)
}

async fn wait_for_incoming(
    events: &mut EndpointEvents,
    timeout: Duration,
) -> anyhow::Result<EndpointIncomingCall> {
    let fut = async {
        loop {
            match events.next().await? {
                Some(EndpointEvent::IncomingCall(incoming)) => return Ok(incoming),
                Some(_) => {}
                None => return Err(SessionError::Other("event stream closed".into())),
            }
        }
    };
    Ok(tokio::time::timeout(timeout, fut).await??)
}

async fn wait_for_call_event(
    events: &mut EndpointEvents,
    call_id: EndpointCallId,
    timeout: Duration,
    mut matches_event: impl FnMut(&EndpointEvent) -> bool,
) -> anyhow::Result<()> {
    let fut = async {
        loop {
            match events.next().await? {
                Some(event) if event_belongs_to(&event, &call_id) && matches_event(&event) => {
                    return Ok(())
                }
                Some(_) => {}
                None => return Err(SessionError::Other("event stream closed".into())),
            }
        }
    };
    Ok(tokio::time::timeout(timeout, fut).await??)
}

fn event_belongs_to(event: &EndpointEvent, call_id: &EndpointCallId) -> bool {
    match event {
        EndpointEvent::CallProgress { call_id: id, .. }
        | EndpointEvent::CallEnded { call_id: id, .. }
        | EndpointEvent::CallFailed { call_id: id, .. }
        | EndpointEvent::CallCancelled { call_id: id }
        | EndpointEvent::LocalHold { call_id: id }
        | EndpointEvent::LocalResume { call_id: id }
        | EndpointEvent::RemoteHold { call_id: id }
        | EndpointEvent::RemoteResume { call_id: id }
        | EndpointEvent::DtmfReceived { call_id: id, .. } => id == call_id,
        EndpointEvent::CallAnswered { call, .. } => call.id() == *call_id,
        EndpointEvent::NetworkError {
            call_id: Some(id), ..
        }
        | EndpointEvent::Info {
            call_id: Some(id), ..
        } => id == call_id,
        _ => false,
    }
}

async fn start_test_audio(
    call: EndpointCall,
    options: &RuntimeOptions,
) -> anyhow::Result<TestAudioRun> {
    match options.test_audio {
        TestAudio::Synthetic => start_synthetic_audio(call).await,
        TestAudio::Cpal => {
            let bridge = AudioBridge::new(
                options.input_device.clone(),
                options.output_device.clone(),
                mpsc::unbounded_channel().0,
            );
            let running = bridge.start(call).await?;
            Ok(TestAudioRun::Cpal(running))
        }
    }
}

async fn start_synthetic_audio(call: EndpointCall) -> anyhow::Result<TestAudioRun> {
    let audio = call.audio().await?;
    let (sender, mut receiver) = audio.split();
    let received = Arc::new(AtomicUsize::new(0));
    let received_for_task = received.clone();
    let send_task = tokio::spawn(async move {
        let mut timestamp = 0u32;
        loop {
            let frame = EndpointAudioFrame::pcmu_sized_mono_8khz(vec![0; FRAME_SAMPLES], timestamp);
            timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);
            if sender.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(FRAME_MS as u64)).await;
        }
    });
    let recv_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if !frame.samples.is_empty() {
                received_for_task.fetch_add(1, Ordering::SeqCst);
            }
        }
    });
    Ok(TestAudioRun::Synthetic {
        received,
        send_task,
        recv_task,
    })
}

enum TestAudioRun {
    Synthetic {
        received: Arc<AtomicUsize>,
        send_task: tokio::task::JoinHandle<()>,
        recv_task: tokio::task::JoinHandle<()>,
    },
    Cpal(RunningAudio),
}

impl TestAudioRun {
    fn require_media(&self) -> anyhow::Result<()> {
        match self {
            Self::Synthetic { received, .. } => {
                if received.load(Ordering::SeqCst) == 0 {
                    anyhow::bail!("no inbound synthetic media frames received");
                }
            }
            Self::Cpal(running) => {
                let _ = running;
            }
        }
        Ok(())
    }
}

impl Drop for TestAudioRun {
    fn drop(&mut self) {
        if let Self::Synthetic {
            send_task,
            recv_task,
            ..
        } = self
        {
            send_task.abort();
            recv_task.abort();
        }
    }
}

async fn start_cpal_audio(
    bridge: &AudioBridge,
    call: EndpointCall,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
) -> Option<RunningAudio> {
    match bridge.start(call).await {
        Ok(audio) => Some(audio),
        Err(err) => {
            let _ = event_tx.send(UiEvent::Error(format!("audio failed: {err}")));
            None
        }
    }
}

#[derive(Clone)]
struct AudioBridge {
    input_device: Option<String>,
    output_device: Option<String>,
    muted: Arc<AtomicBool>,
    event_tx: mpsc::UnboundedSender<UiEvent>,
}

impl AudioBridge {
    fn new(
        input_device: Option<String>,
        output_device: Option<String>,
        event_tx: mpsc::UnboundedSender<UiEvent>,
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

    async fn start(&self, call: EndpointCall) -> anyhow::Result<RunningAudio> {
        let audio = call.audio().await?;
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

        let input_stream =
            build_input_stream(&input, &input_config.into(), input_channels, mic_tx, muted)?;
        let output_stream = build_output_stream(
            &output,
            &output_config.into(),
            output_channels,
            playback_buffer.clone(),
        )?;

        input_stream.play()?;
        output_stream.play()?;

        let input_task = tokio::spawn(async move {
            send_microphone_frames(&mut mic_rx, input_sample_rate, sender).await;
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
                    let _ = event_tx.send(UiEvent::Error("playback buffer poisoned".into()));
                    break;
                }
            }
        });

        let _ = self.event_tx.send(UiEvent::AudioStarted(format!(
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

async fn send_microphone_frames(
    mic_rx: &mut mpsc::UnboundedReceiver<Vec<f32>>,
    input_sample_rate: u32,
    sender: EndpointAudioSender,
) {
    let mut mono_buffer = Vec::<f32>::new();
    let mut timestamp = 0u32;
    while let Some(samples) = mic_rx.recv().await {
        let resampled = resample_linear(&samples, input_sample_rate, SAMPLE_RATE);
        mono_buffer.extend(resampled);
        while mono_buffer.len() >= FRAME_SAMPLES {
            let chunk = mono_buffer.drain(..FRAME_SAMPLES).collect::<Vec<_>>();
            let pcm = chunk.into_iter().map(float_to_i16).collect::<Vec<_>>();
            let frame = EndpointAudioFrame::pcmu_sized_mono_8khz(pcm, timestamp);
            timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);
            if sender.send(frame).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(FRAME_MS as u64)).await;
        }
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
                send_input_samples(data, channels, &tx, &muted);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _| {
                let converted = data
                    .iter()
                    .map(|sample| *sample as f32 / i16::MAX as f32)
                    .collect::<Vec<_>>();
                send_input_samples(&converted, channels, &tx, &muted);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            config,
            move |data: &[u16], _| {
                let converted = data
                    .iter()
                    .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0)
                    .collect::<Vec<_>>();
                send_input_samples(&converted, channels, &tx, &muted);
            },
            err_fn,
            None,
        )?,
        other => anyhow::bail!("unsupported input sample format {other:?}"),
    };
    Ok(stream)
}

fn send_input_samples(
    data: &[f32],
    channels: usize,
    tx: &mpsc::UnboundedSender<Vec<f32>>,
    muted: &AtomicBool,
) {
    if muted.load(Ordering::SeqCst) {
        let _ = tx.send(vec![0.0; data.len() / channels.max(1)]);
    } else {
        let _ = tx.send(mix_to_mono(data, channels));
    }
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
        Line::from(format!("State: {}", app.state.label())),
        Line::from(format!("Registration: {}", app.registration)),
        Line::from(format!("Local: {}", app.local)),
        Line::from(format!(
            "Register on start: {}",
            app.options.register_on_start
        )),
        Line::from(format!(
            "Dial target: {}",
            app.options.dial.as_deref().unwrap_or("-")
        )),
    ];
    frame.render_widget(
        Paragraph::new(account)
            .block(Block::default().title("Endpoint").borders(Borders::ALL))
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

fn build_runtime_options(cli: &Cli) -> anyhow::Result<RuntimeOptions> {
    let mut endpoint = load_endpoint_config(cli.config.clone())?;
    apply_cli_overrides(&mut endpoint, cli);
    let register_on_start = cli.register
        || endpoint.register_on_start.unwrap_or(false)
        || matches!(cli.test, Some(TestRole::PbxCaller | TestRole::PbxCallee));
    Ok(RuntimeOptions {
        endpoint,
        register_on_start,
        dial: cli.dial.clone(),
        input_device: cli.input_device.clone(),
        output_device: cli.output_device.clone(),
        test_duration: Duration::from_secs(cli.test_duration),
        test_timeout: Duration::from_secs(cli.test_timeout),
        test_dtmf: cli.test_dtmf,
        test_audio: cli.test_audio,
    })
}

fn load_endpoint_config(path: Option<PathBuf>) -> anyhow::Result<EndpointConfig> {
    let Some(path) = path else {
        return Ok(EndpointConfig::default());
    };
    let text = fs::read_to_string(&path)?;
    Ok(serde_json::from_str::<EndpointConfig>(&text)?)
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

fn local_label(config: &EndpointConfig) -> String {
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

fn format_registration(info: &EndpointRegistrationInfo) -> String {
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
