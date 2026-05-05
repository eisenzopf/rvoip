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

use crate::audio::audio_device_summary;
use crate::config::RuntimeOptions;
use crate::runtime::RuntimeCommand;

const MAX_LOGS: usize = 200;
const MAX_DTMF_HISTORY: usize = 24;

pub(crate) enum UiEvent {
    Ready {
        local: String,
    },
    State(AppState),
    Registration(String),
    Calling {
        id: String,
        target: String,
    },
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
enum Action {
    Dial,
    Answer,
    Reject,
    CancelCall,
    HangUp,
    Mute,
    Unmute,
    Hold,
    Resume,
    SendDtmf,
    Transfer,
    AudioDevices,
    Quit,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Self::Dial => "Dial",
            Self::Answer => "Answer",
            Self::Reject => "Reject",
            Self::CancelCall => "Cancel Call",
            Self::HangUp => "Hang Up",
            Self::Mute => "Mute",
            Self::Unmute => "Unmute",
            Self::Hold => "Hold",
            Self::Resume => "Resume",
            Self::SendDtmf => "Send DTMF",
            Self::Transfer => "Transfer",
            Self::AudioDevices => "Audio Devices",
            Self::Quit => "Quit",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Dial => "Dial Target",
            Self::Transfer => "Transfer Target",
            Self::SendDtmf => "Send DTMF",
            Self::Reject => "Reject Call",
            Self::CancelCall => "Cancel Call",
            Self::HangUp => "Hang Up",
            Self::Quit => "Quit",
            Self::AudioDevices => "Audio Devices",
            _ => self.label(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Menu,
    Prompt(Action),
    Confirm(Action),
    Detail,
}

pub(crate) struct TuiApp {
    options: RuntimeOptions,
    state: AppState,
    registration: String,
    local: String,
    calling: Option<(String, String)>,
    incoming: Option<(String, String, String)>,
    active_call: Option<(String, String)>,
    audio: String,
    muted: bool,
    selected_action: usize,
    view: ViewMode,
    input: String,
    dtmf_history: String,
    audio_detail: Vec<String>,
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
            calling: None,
            incoming: None,
            active_call: None,
            audio: "stopped".into(),
            muted: false,
            selected_action: 0,
            view: ViewMode::Menu,
            input: String::new(),
            dtmf_history: String::new(),
            audio_detail: Vec::new(),
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
            UiEvent::State(state) => self.set_state(state),
            UiEvent::Registration(status) => {
                self.registration = status.clone();
                self.push_log(status);
            }
            UiEvent::Calling { id, target } => {
                self.calling = Some((id, target.clone()));
                self.incoming = None;
                self.set_state(AppState::Calling);
                self.push_log(format!("calling {target}"));
            }
            UiEvent::Incoming { id, from, to } => {
                self.incoming = Some((id, from.clone(), to));
                self.calling = None;
                self.set_state(AppState::Incoming);
                self.push_log(format!("incoming call from {from}"));
            }
            UiEvent::ActiveCall { id, peer } => {
                self.incoming = None;
                self.calling = None;
                self.active_call = Some((id, peer));
                self.set_state(AppState::InCall);
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
                self.clamp_selected_action();
                self.push_log(if muted {
                    "microphone muted"
                } else {
                    "microphone unmuted"
                });
            }
            UiEvent::Log(message) => self.push_log(message),
            UiEvent::Error(message) => {
                self.set_state(AppState::Error);
                self.push_log(format!("error: {message}"));
            }
            UiEvent::ShutdownComplete => {
                self.should_quit = true;
            }
        }
    }

    fn set_state(&mut self, state: AppState) {
        if self.state != state {
            self.view = ViewMode::Menu;
            self.input.clear();
            self.dtmf_history.clear();
            self.selected_action = 0;
        }

        self.state = state;
        if matches!(
            self.state,
            AppState::Idle | AppState::Registered | AppState::Error | AppState::ShuttingDown
        ) {
            self.calling = None;
            self.incoming = None;
            self.active_call = None;
        }
        self.clamp_selected_action();
    }

    fn selected_action(&self) -> Option<Action> {
        available_actions(self).get(self.selected_action).copied()
    }

    fn clamp_selected_action(&mut self) {
        let actions = available_actions(self);
        if actions.is_empty() {
            self.selected_action = 0;
        } else if self.selected_action >= actions.len() {
            self.selected_action = actions.len() - 1;
        }
    }
}

pub(crate) fn handle_key(
    key: KeyEvent,
    app: &mut TuiApp,
    command_tx: &mpsc::UnboundedSender<RuntimeCommand>,
) {
    match app.view {
        ViewMode::Menu => handle_menu_key(key, app, command_tx),
        ViewMode::Prompt(action) => handle_prompt_key(key, action, app, command_tx),
        ViewMode::Confirm(action) => handle_confirm_key(key, action, app, command_tx),
        ViewMode::Detail => handle_detail_key(key, app),
    }
}

fn handle_menu_key(
    key: KeyEvent,
    app: &mut TuiApp,
    command_tx: &mpsc::UnboundedSender<RuntimeCommand>,
) {
    match key.code {
        KeyCode::Up => move_selection(app, -1),
        KeyCode::Down => move_selection(app, 1),
        KeyCode::Enter => {
            if let Some(action) = app.selected_action() {
                open_action(action, app, command_tx);
            }
        }
        KeyCode::Char(ch) => {
            if let Some(action) = shortcut_action(ch, app) {
                open_action(action, app, command_tx);
            }
        }
        _ => {}
    }
}

fn handle_prompt_key(
    key: KeyEvent,
    action: Action,
    app: &mut TuiApp,
    command_tx: &mpsc::UnboundedSender<RuntimeCommand>,
) {
    match key.code {
        KeyCode::Esc => return_to_menu(app),
        KeyCode::Enter => match action {
            Action::Dial | Action::Transfer => {
                let value = app.input.trim().to_string();
                if !value.is_empty() {
                    let command = if action == Action::Dial {
                        RuntimeCommand::Dial(value)
                    } else {
                        RuntimeCommand::Transfer(value)
                    };
                    let _ = command_tx.send(command);
                }
                return_to_menu(app);
            }
            Action::SendDtmf => return_to_menu(app),
            _ => {}
        },
        KeyCode::Backspace if action != Action::SendDtmf => {
            app.input.pop();
        }
        KeyCode::Char(ch) => match action {
            Action::Dial | Action::Transfer => app.input.push(ch),
            Action::SendDtmf if ch.is_ascii_digit() || ch == '*' || ch == '#' => {
                let _ = command_tx.send(RuntimeCommand::SendDtmf(ch));
                app.dtmf_history.push(ch);
                if app.dtmf_history.len() > MAX_DTMF_HISTORY {
                    let excess = app.dtmf_history.len() - MAX_DTMF_HISTORY;
                    app.dtmf_history.drain(..excess);
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn handle_confirm_key(
    key: KeyEvent,
    action: Action,
    app: &mut TuiApp,
    command_tx: &mpsc::UnboundedSender<RuntimeCommand>,
) {
    match key.code {
        KeyCode::Esc | KeyCode::Char('n') => return_to_menu(app),
        KeyCode::Enter | KeyCode::Char('y') => {
            match action {
                Action::Reject => {
                    let _ = command_tx.send(RuntimeCommand::Reject);
                }
                Action::CancelCall | Action::HangUp => {
                    let _ = command_tx.send(RuntimeCommand::Hangup);
                }
                Action::Quit => {
                    app.set_state(AppState::ShuttingDown);
                    let _ = command_tx.send(RuntimeCommand::Shutdown);
                }
                _ => {}
            }
            return_to_menu(app);
        }
        _ => {}
    }
}

fn handle_detail_key(key: KeyEvent, app: &mut TuiApp) {
    if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
        return_to_menu(app);
    }
}

fn move_selection(app: &mut TuiApp, delta: isize) {
    let len = available_actions(app).len();
    if len == 0 {
        app.selected_action = 0;
        return;
    }
    app.selected_action = if delta.is_negative() {
        (app.selected_action + len - 1) % len
    } else {
        (app.selected_action + 1) % len
    };
}

fn open_action(
    action: Action,
    app: &mut TuiApp,
    command_tx: &mpsc::UnboundedSender<RuntimeCommand>,
) {
    match action {
        Action::Dial | Action::Transfer | Action::SendDtmf => {
            app.view = ViewMode::Prompt(action);
            app.input.clear();
            app.dtmf_history.clear();
        }
        Action::Answer => {
            let _ = command_tx.send(RuntimeCommand::Answer);
        }
        Action::Mute | Action::Unmute => {
            let _ = command_tx.send(RuntimeCommand::ToggleMute);
        }
        Action::Hold | Action::Resume => {
            let _ = command_tx.send(RuntimeCommand::HoldResume);
        }
        Action::Reject | Action::CancelCall | Action::HangUp | Action::Quit => {
            app.view = ViewMode::Confirm(action);
        }
        Action::AudioDevices => {
            app.audio_detail = audio_device_summary(
                app.options.input_device.as_deref(),
                app.options.output_device.as_deref(),
                &app.audio,
            );
            app.view = ViewMode::Detail;
        }
    }
}

fn shortcut_action(ch: char, app: &TuiApp) -> Option<Action> {
    let target = match ch {
        'd' => Action::Dial,
        'a' => Action::Answer,
        'r' => Action::Reject,
        'h' => {
            if matches!(app.state, AppState::Calling) {
                Action::CancelCall
            } else {
                Action::HangUp
            }
        }
        'm' => {
            if app.muted {
                Action::Unmute
            } else {
                Action::Mute
            }
        }
        'o' => {
            if matches!(app.state, AppState::OnHold) {
                Action::Resume
            } else {
                Action::Hold
            }
        }
        't' => Action::Transfer,
        'q' => Action::Quit,
        _ => return None,
    };

    available_actions(app)
        .into_iter()
        .find(|action| *action == target)
}

fn return_to_menu(app: &mut TuiApp) {
    app.view = ViewMode::Menu;
    app.input.clear();
    app.dtmf_history.clear();
    app.clamp_selected_action();
}

fn available_actions(app: &TuiApp) -> Vec<Action> {
    match app.state {
        AppState::Idle | AppState::Registered => {
            vec![Action::Dial, Action::AudioDevices, Action::Quit]
        }
        AppState::Registering => vec![Action::AudioDevices, Action::Quit],
        AppState::Incoming => vec![
            Action::Answer,
            Action::Reject,
            Action::AudioDevices,
            Action::Quit,
        ],
        AppState::Calling => vec![Action::CancelCall, Action::AudioDevices, Action::Quit],
        AppState::InCall => vec![
            Action::HangUp,
            if app.muted {
                Action::Unmute
            } else {
                Action::Mute
            },
            Action::Hold,
            Action::SendDtmf,
            Action::Transfer,
            Action::AudioDevices,
            Action::Quit,
        ],
        AppState::OnHold => vec![
            Action::HangUp,
            if app.muted {
                Action::Unmute
            } else {
                Action::Mute
            },
            Action::Resume,
            Action::SendDtmf,
            Action::Transfer,
            Action::AudioDevices,
            Action::Quit,
        ],
        AppState::Transferring => vec![
            Action::HangUp,
            if app.muted {
                Action::Unmute
            } else {
                Action::Mute
            },
            Action::SendDtmf,
            Action::AudioDevices,
            Action::Quit,
        ],
        AppState::Error | AppState::ShuttingDown => vec![Action::Quit],
    }
}

pub(crate) fn draw_ui(frame: &mut Frame<'_>, app: &TuiApp) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(5),
        ])
        .split(frame.size());

    draw_header(frame, app, root[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(40)])
        .split(root[1]);
    draw_actions(frame, app, body[0]);
    draw_detail(frame, app, body[1]);
    draw_events(frame, app, root[2]);
}

fn draw_header(frame: &mut Frame<'_>, app: &TuiApp, area: ratatui::layout::Rect) {
    let header = Line::from(vec![
        Span::styled(
            "RVoIP Softphone",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(app.local.clone(), Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled(
            app.registration.clone(),
            registration_style(&app.registration),
        ),
        Span::raw(" / "),
        Span::styled(app.state.label(), state_style(&app.state)),
    ]);
    clear_area(frame, area);
    frame.render_widget(
        Paragraph::new(header)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn clear_area(frame: &mut Frame<'_>, area: ratatui::layout::Rect) {
    let width = area.width as usize;
    let lines = (0..area.height)
        .map(|_| Line::from(" ".repeat(width)))
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_actions(frame: &mut Frame<'_>, app: &TuiApp, area: ratatui::layout::Rect) {
    let actions = available_actions(app);
    let items = actions
        .iter()
        .enumerate()
        .map(|(idx, action)| {
            let selected = idx == app.selected_action && matches!(app.view, ViewMode::Menu);
            let marker = if selected { "> " } else { "  " };
            let style = if selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, style),
                Span::styled(action.label(), style),
            ]))
        })
        .collect::<Vec<_>>();

    clear_area(frame, area);
    frame.render_widget(
        List::new(items).block(Block::default().title("Actions").borders(Borders::ALL)),
        area,
    );
}

fn draw_detail(frame: &mut Frame<'_>, app: &TuiApp, area: ratatui::layout::Rect) {
    match app.view {
        ViewMode::Menu => draw_state_detail(frame, app, area),
        ViewMode::Prompt(action) => draw_prompt(frame, app, action, area),
        ViewMode::Confirm(action) => draw_confirm(frame, app, action, area),
        ViewMode::Detail => draw_audio_detail(frame, app, area),
    }
}

fn draw_state_detail(frame: &mut Frame<'_>, app: &TuiApp, area: ratatui::layout::Rect) {
    let lines = match app.state {
        AppState::Idle | AppState::Registered | AppState::Registering => ready_lines(app),
        AppState::Incoming => incoming_lines(app),
        AppState::Calling => calling_lines(app),
        AppState::InCall | AppState::OnHold | AppState::Transferring => active_call_lines(app),
        AppState::Error => vec![
            Line::from(Span::styled(
                "Error",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from("Check Events for details."),
        ],
        AppState::ShuttingDown => vec![Line::from("Shutting down...")],
    };

    clear_area(frame, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title("Current").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn ready_lines(app: &TuiApp) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Ready",
            Style::default()
                .fg(if matches!(app.state, AppState::Registering) {
                    Color::Yellow
                } else {
                    Color::Green
                })
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("Local: {}", app.local)),
        Line::from(format!("Registration: {}", app.registration)),
        Line::from(format!(
            "Startup dial: {}",
            app.options.dial.as_deref().unwrap_or("-")
        )),
        Line::from(format!(
            "Profile: {}    Transport: {}",
            profile_label(app),
            transport_label(app)
        )),
    ]
}

fn incoming_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let (id, from, to) = app
        .incoming
        .as_ref()
        .map(|(id, from, to)| (id.as_str(), from.as_str(), to.as_str()))
        .unwrap_or(("-", "-", "-"));
    vec![
        Line::from(Span::styled(
            "Incoming Call",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("From: {from}")),
        Line::from(format!("To: {to}")),
        Line::from(format!("Call ID: {id}")),
        Line::from("Select Answer or Reject."),
    ]
}

fn calling_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let (id, target) = app
        .calling
        .as_ref()
        .map(|(id, target)| (id.as_str(), target.as_str()))
        .unwrap_or(("-", app.options.dial.as_deref().unwrap_or("-")));
    vec![
        Line::from(Span::styled(
            "Calling",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("Target: {target}")),
        Line::from(format!("Call ID: {id}")),
        Line::from("Select Cancel Call to stop ringing."),
    ]
}

fn active_call_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let (id, peer) = app
        .active_call
        .as_ref()
        .map(|(id, peer)| (id.as_str(), peer.as_str()))
        .unwrap_or(("-", "-"));
    vec![
        Line::from(Span::styled(
            match app.state {
                AppState::OnHold => "On Hold",
                AppState::Transferring => "Transferring",
                _ => "In Call",
            },
            state_style(&app.state),
        )),
        Line::from(format!("Peer: {peer}")),
        Line::from(format!("Call ID: {id}")),
        Line::from(format!("Audio: {}", app.audio)),
        Line::from(format!(
            "Mic: {}    Hold: {}",
            if app.muted { "muted" } else { "live" },
            if matches!(app.state, AppState::OnHold) {
                "on"
            } else {
                "off"
            }
        )),
    ]
}

fn draw_prompt(frame: &mut Frame<'_>, app: &TuiApp, action: Action, area: ratatui::layout::Rect) {
    let lines = match action {
        Action::Dial => vec![
            Line::from("Enter a SIP URI, extension, or reachable target."),
            Line::from(vec![
                Span::raw("Dial: "),
                Span::styled(app.input.clone(), Style::default().fg(Color::White)),
            ]),
            Line::from("Enter confirms. Esc cancels."),
        ],
        Action::Transfer => vec![
            Line::from("Enter a transfer target."),
            Line::from(vec![
                Span::raw("Transfer to: "),
                Span::styled(app.input.clone(), Style::default().fg(Color::White)),
            ]),
            Line::from("Enter confirms. Esc cancels."),
        ],
        Action::SendDtmf => vec![
            Line::from("Type 0-9, *, or #. Each digit sends immediately."),
            Line::from(vec![
                Span::raw("Sent: "),
                Span::styled(
                    if app.dtmf_history.is_empty() {
                        "-".into()
                    } else {
                        app.dtmf_history.clone()
                    },
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from("Esc returns to call controls."),
        ],
        _ => vec![Line::from("Unsupported prompt.")],
    };

    clear_area(frame, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(action.title()).borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_confirm(frame: &mut Frame<'_>, app: &TuiApp, action: Action, area: ratatui::layout::Rect) {
    let prompt = match action {
        Action::Reject => "Reject this incoming call?",
        Action::CancelCall => "Cancel this outgoing call?",
        Action::HangUp => "Hang up the current call?",
        Action::Quit => "Quit the softphone?",
        _ => "Confirm action?",
    };
    let lines = vec![
        Line::from(Span::styled(
            prompt,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from("Enter or y confirms. Esc or n cancels."),
        Line::from(String::new()),
        selected_context_line(app),
    ];

    clear_area(frame, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title(action.title()).borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_audio_detail(frame: &mut Frame<'_>, app: &TuiApp, area: ratatui::layout::Rect) {
    let lines = app
        .audio_detail
        .iter()
        .map(|line| {
            if line.ends_with("devices:") {
                Line::from(Span::styled(
                    line.clone(),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(line.clone())
            }
        })
        .chain(std::iter::once(Line::from(String::new())))
        .chain(std::iter::once(Line::from("Esc returns to actions.")))
        .collect::<Vec<_>>();

    clear_area(frame, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title("Audio Devices")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_events(frame: &mut Frame<'_>, app: &TuiApp, area: ratatui::layout::Rect) {
    let mut lines = app
        .logs
        .iter()
        .rev()
        .take(area.height.saturating_sub(3) as usize)
        .rev()
        .map(|line| Line::from(Span::styled(line.clone(), log_style(line))))
        .collect::<Vec<_>>();
    lines.push(Line::from(Span::styled(
        "Up/Down select  Enter choose  Esc back",
        Style::default().fg(Color::Gray),
    )));

    clear_area(frame, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().title("Events").borders(Borders::ALL))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn selected_context_line(app: &TuiApp) -> Line<'static> {
    match app.state {
        AppState::Incoming => {
            let from = app
                .incoming
                .as_ref()
                .map(|(_, from, _)| from.as_str())
                .unwrap_or("-");
            Line::from(format!("Incoming from: {from}"))
        }
        AppState::Calling => {
            let target = app
                .calling
                .as_ref()
                .map(|(_, target)| target.as_str())
                .unwrap_or("-");
            Line::from(format!("Calling: {target}"))
        }
        AppState::InCall | AppState::OnHold | AppState::Transferring => {
            let peer = app
                .active_call
                .as_ref()
                .map(|(_, peer)| peer.as_str())
                .unwrap_or("-");
            Line::from(format!("Peer: {peer}"))
        }
        _ => Line::from(format!("State: {}", app.state.label())),
    }
}

fn state_style(state: &AppState) -> Style {
    let color = match state {
        AppState::Idle => Color::Gray,
        AppState::Registering | AppState::Calling | AppState::Transferring => Color::Yellow,
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
        Color::Yellow
    } else {
        Color::Gray
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
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
