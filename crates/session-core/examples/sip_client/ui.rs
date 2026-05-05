use std::collections::VecDeque;
use std::io::{self, Stdout};

use crossterm::{
    event::{KeyCode, KeyEvent},
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

use rvoip_session_core::{EndpointProfileName, EndpointTransport};

use crate::config::RuntimeOptions;
use crate::runtime::RuntimeCommand;

const MAX_LOGS: usize = 200;

pub(crate) enum UiEvent {
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
pub(crate) enum AppState {
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
pub(crate) enum InputMode {
    Normal,
    Dial,
    Transfer,
}

pub(crate) struct TuiApp {
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
    pub(crate) should_quit: bool,
}

impl TuiApp {
    pub(crate) fn new(options: RuntimeOptions) -> Self {
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

    pub(crate) fn push_log(&mut self, message: impl Into<String>) {
        if self.logs.len() >= MAX_LOGS {
            self.logs.pop_front();
        }
        self.logs.push_back(message.into());
    }

    pub(crate) fn apply_event(&mut self, event: UiEvent) {
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

pub(crate) fn handle_key(
    key: KeyEvent,
    app: &mut TuiApp,
    command_tx: &mpsc::UnboundedSender<RuntimeCommand>,
) {
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

pub(crate) fn draw_ui(frame: &mut Frame<'_>, app: &TuiApp) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(7),
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.size());

    let header = Line::from(vec![
        Span::styled(
            "RVoIP Softphone",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  Local: "),
        Span::styled(app.local.clone(), Style::default().fg(Color::White)),
        Span::raw("  Profile: "),
        Span::styled(profile_label(app), Style::default().fg(Color::White)),
        Span::raw("  Transport: "),
        Span::styled(transport_label(app), Style::default().fg(Color::White)),
    ]);
    frame.render_widget(
        Paragraph::new(header)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        root[0],
    );

    let status = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(root[1]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("Call: "),
            Span::styled(app.state.label(), state_style(&app.state)),
        ]))
        .block(Block::default().borders(Borders::ALL)),
        status[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("Registration: "),
            Span::styled(
                app.registration.clone(),
                registration_style(&app.registration),
            ),
        ]))
        .block(Block::default().borders(Borders::ALL))
        .wrap(Wrap { trim: true }),
        status[1],
    );

    let (active_call_id, active_peer) = app
        .active_call
        .as_ref()
        .map(|(id, peer)| (id.as_str(), peer.as_str()))
        .unwrap_or(("-", "-"));
    let incoming = app
        .incoming
        .as_ref()
        .map(|(id, from, to)| format!("{from} -> {to} ({id})"))
        .unwrap_or_else(|| "-".into());
    let call = vec![
        Line::from(vec![
            Span::raw("Peer: "),
            Span::styled(active_peer.to_string(), Style::default().fg(Color::White)),
            Span::raw("  ID: "),
            Span::raw(active_call_id.to_string()),
        ]),
        Line::from(format!("Incoming: {incoming}")),
        Line::from(format!(
            "Startup dial: {}",
            app.options.dial.as_deref().unwrap_or("-")
        )),
        Line::from(format!(
            "Audio: {}",
            if app.audio.is_empty() {
                "-"
            } else {
                &app.audio
            }
        )),
        Line::from(format!(
            "Mic: {}  Hold: {}",
            if app.muted { "muted" } else { "live" },
            if matches!(app.state, AppState::OnHold) {
                "on"
            } else {
                "off"
            }
        )),
    ];
    frame.render_widget(
        Paragraph::new(call)
            .block(Block::default().title("Call").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        root[2],
    );

    let (input_title, input_line) = input_prompt(app);
    frame.render_widget(
        Paragraph::new(input_line).block(Block::default().title(input_title).borders(Borders::ALL)),
        root[3],
    );

    let logs = app
        .logs
        .iter()
        .rev()
        .take(root[4].height.saturating_sub(2) as usize)
        .rev()
        .map(|line| ListItem::new(Line::from(Span::styled(line.clone(), log_style(line)))))
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(logs).block(Block::default().title("Events").borders(Borders::ALL)),
        root[4],
    );

    let help = Line::from(vec![
        Span::styled("d", key_style()),
        Span::raw(" Dial "),
        Span::styled("a", key_style()),
        Span::raw(" Ans "),
        Span::styled("r", key_style()),
        Span::raw(" Rej "),
        Span::styled("h", key_style()),
        Span::raw(" End "),
        Span::styled("m", key_style()),
        Span::raw(" Mute "),
        Span::styled("o", key_style()),
        Span::raw(" Hold "),
        Span::styled("0-9*#", key_style()),
        Span::raw(" DTMF "),
        Span::styled("t", key_style()),
        Span::raw(" Xfer "),
        Span::styled("q", key_style()),
        Span::raw(" Quit"),
    ]);
    frame.render_widget(
        Paragraph::new(help)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        root[5],
    );
}

fn key_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn state_style(state: &AppState) -> Style {
    let color = match state {
        AppState::Idle => Color::Gray,
        AppState::Registering | AppState::Calling | AppState::Transferring => Color::Blue,
        AppState::Registered | AppState::InCall => Color::Green,
        AppState::Incoming | AppState::OnHold => Color::Yellow,
        AppState::ShuttingDown => Color::Gray,
        AppState::Error => Color::Red,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn registration_style(status: &str) -> Style {
    let normalized = status.to_ascii_lowercase();
    let color = if normalized.contains("failed") {
        Color::Red
    } else if normalized.starts_with("registered") {
        Color::Green
    } else if normalized.contains("registering") || normalized.contains("unregistering") {
        Color::Blue
    } else {
        Color::Gray
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn input_prompt(app: &TuiApp) -> (&'static str, Line<'static>) {
    match app.input_mode {
        InputMode::Normal => (
            "Command",
            Line::from(vec![
                Span::styled("Ready", Style::default().fg(Color::Gray)),
                Span::raw("  Press "),
                Span::styled("d", key_style()),
                Span::raw(" to dial or "),
                Span::styled("q", key_style()),
                Span::raw(" to quit"),
            ]),
        ),
        InputMode::Dial => (
            "Dial",
            Line::from(vec![
                Span::raw("Dial: "),
                Span::styled(app.input.clone(), Style::default().fg(Color::White)),
                Span::raw("  Enter confirm | Esc cancel"),
            ]),
        ),
        InputMode::Transfer => (
            "Transfer",
            Line::from(vec![
                Span::raw("Transfer to: "),
                Span::styled(app.input.clone(), Style::default().fg(Color::White)),
                Span::raw("  Enter confirm | Esc cancel"),
            ]),
        ),
    }
}

fn log_style(line: &str) -> Style {
    let normalized = line.to_ascii_lowercase();
    if normalized.contains("error") || normalized.contains("failed") {
        Style::default().fg(Color::Red)
    } else if normalized.contains("incoming") || normalized.contains("hold") {
        Style::default().fg(Color::Yellow)
    } else if normalized.contains("registered") || normalized.contains("answered") {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Gray)
    }
}

fn profile_label(app: &TuiApp) -> &'static str {
    match app.options.endpoint.profile {
        Some(EndpointProfileName::Local) => "local",
        Some(EndpointProfileName::LanPbx) => "lan-pbx",
        Some(EndpointProfileName::AsteriskUdp) => "asterisk-udp",
        Some(EndpointProfileName::AsteriskTlsSrtp) => "asterisk-tls-srtp",
        Some(EndpointProfileName::FreeswitchInternal) => "freeswitch-internal",
        Some(EndpointProfileName::FreeswitchTlsSrtp) => "freeswitch-tls-srtp",
        Some(EndpointProfileName::CarrierSbc) => "carrier-sbc",
        None => "default",
    }
}

fn transport_label(app: &TuiApp) -> &'static str {
    match app
        .options
        .endpoint
        .network
        .as_ref()
        .and_then(|network| network.transport)
    {
        Some(EndpointTransport::Udp) => "udp",
        Some(EndpointTransport::Tcp) => "tcp",
        Some(EndpointTransport::Tls) => "tls",
        None => "default",
    }
}

pub(crate) struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    pub(crate) fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub(crate) fn draw<F>(&mut self, f: F) -> io::Result<()>
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
