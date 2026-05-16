//! Interactive TUI event loop: wires terminal key events, periodic redraws,
//! audio bridge ticks, and [`EndpointEvent`](rvoip_sip::EndpointEvent) streams
//! together so an operator can place, answer, hold, resume, transfer, and tear
//! down calls from the keyboard.
//!
//! Implementation detail of the `sip_client` example; see [`super::main`].

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event as TerminalEvent, KeyEventKind};
use tokio::sync::mpsc;

use rvoip_sip::{
    Endpoint, EndpointCall, EndpointEvent, EndpointIncomingCall, EndpointSipTrace,
};

use crate::audio::{start_cpal_audio, AudioBridge, RunningAudio};
use crate::config::{format_registration, local_label, RuntimeOptions};
use crate::ui::{draw_ui, handle_key, AppState, TerminalSession, TuiApp, UiEvent};

pub(crate) enum RuntimeCommand {
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

pub(crate) async fn run_tui(options: RuntimeOptions) -> anyhow::Result<()> {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let runtime_options = options.clone();
    let runtime = std::thread::Builder::new()
        .name("sip-client-runtime".into())
        .spawn(move || {
            match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt.block_on(run_runtime(runtime_options, command_rx, event_tx)),
                Err(err) => eprintln!("failed to start SIP runtime: {err}"),
            }
        })?;

    let mut terminal = TerminalSession::enter()?;
    let mut app = TuiApp::new(options);
    app.push_log("select an action with Up/Down, then Enter");

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
) -> anyhow::Result<()> {
    let mut trace_file = match options.sip_trace.file.as_ref() {
        Some(path) => {
            let file = TraceFile::open(path).map_err(|err| {
                anyhow::anyhow!("failed to open SIP trace file {}: {err}", path.display())
            })?;
            let _ = event_tx.send(UiEvent::Log(format!(
                "writing SIP trace to {}",
                path.display()
            )));
            Some(file)
        }
        None => None,
    };
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
                return Err(err.into());
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
        match control.invite(target).map(|b| b.send()) {
            Ok(send) => match send.await.map(|cid| control.wrap_call(cid)) {
                Ok(call) => {
                    let _ = event_tx.send(UiEvent::Calling {
                        id: call.id().to_string(),
                        target: target.clone(),
                    });
                    let _ = event_tx.send(UiEvent::Log(format!(
                        "calling {target} ({})",
                        call.id()
                    )));
                    active_call = Some(call);
                }
                Err(err) => {
                    let _ = event_tx.send(UiEvent::Error(format!("dial failed: {err}")));
                }
            },
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
                        match control.invite(&target).map(|b| b.send()) {
                            Ok(send) => match send.await.map(|cid| control.wrap_call(cid)) {
                                Ok(call) => {
                                    let _ = event_tx.send(UiEvent::Calling {
                                        id: call.id().to_string(),
                                        target: target.clone(),
                                    });
                                    let _ = event_tx.send(UiEvent::Log(format!(
                                        "calling {target} ({})",
                                        call.id()
                                    )));
                                    active_call = Some(call);
                                    on_hold = false;
                                }
                                Err(err) => {
                                    let _ = event_tx.send(UiEvent::Error(format!("dial failed: {err}")));
                                }
                            },
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
                    EndpointEvent::SipTrace(trace) => {
                        if let Some(file) = trace_file.as_mut() {
                            if let Err(err) = file.write(&trace) {
                                let _ = event_tx.send(UiEvent::Error(format!(
                                    "failed to write SIP trace: {err}"
                                )));
                            }
                        }
                        let _ = event_tx.send(UiEvent::SipTrace(trace));
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

pub(crate) struct TraceFile {
    writer: BufWriter<File>,
}

impl TraceFile {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            writer: BufWriter::new(file),
        })
    }

    pub(crate) fn write(&mut self, trace: &EndpointSipTrace) -> io::Result<()> {
        writeln!(
            self.writer,
            "===== {} {} {} {} -> {} =====",
            trace.timestamp_unix_millis,
            trace.direction.arrow(),
            trace.transport,
            trace.local_addr,
            trace.remote_addr
        )?;
        writeln!(self.writer, "Start-Line: {}", trace.start_line)?;
        if let Some(call_id) = trace.sip_call_id.as_ref() {
            writeln!(self.writer, "SIP Call-ID: {call_id}")?;
        }
        if let Some(session_id) = trace.session_id.as_ref() {
            writeln!(self.writer, "Session ID: {session_id}")?;
        }
        if trace.redacted {
            writeln!(self.writer, "Redacted: yes")?;
        }
        if trace.truncated {
            writeln!(
                self.writer,
                "Truncated: yes, original bytes {}",
                trace.original_len
            )?;
        }
        writeln!(self.writer)?;
        writeln!(self.writer, "{}", trace.raw_message)?;
        writeln!(self.writer)?;
        self.writer.flush()
    }
}
