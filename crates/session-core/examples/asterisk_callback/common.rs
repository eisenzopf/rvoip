#![allow(dead_code)]

#[path = "../asterisk/common.rs"]
pub mod asterisk;

use std::time::Duration;

pub use asterisk::*;
use async_trait::async_trait;
use rvoip_session_core::{
    CallHandler, CallHandlerDecision, CallId, CallbackPeer, CallbackPeerControl, IncomingCall,
    RegistrationHandle, SessionHandle,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

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
    TransferProgress {
        call_id: CallId,
        status_code: u16,
        reason: String,
    },
    TransferCompleted {
        old_call_id: CallId,
        new_call_id: CallId,
        target: String,
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

pub struct EventQueueHandler {
    mode: IncomingMode,
    tx: mpsc::UnboundedSender<CallbackEvent>,
}

impl EventQueueHandler {
    pub fn new(mode: IncomingMode, tx: mpsc::UnboundedSender<CallbackEvent>) -> Self {
        Self { mode, tx }
    }

    fn send(&self, event: CallbackEvent) {
        let _ = self.tx.send(event);
    }
}

#[async_trait]
impl CallHandler for EventQueueHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallHandlerDecision {
        println!("[callback] incoming call {} -> {}", call.from, call.to);
        self.send(CallbackEvent::Incoming {
            call_id: call.call_id.clone(),
            from: call.from.clone(),
            to: call.to.clone(),
        });
        match self.mode {
            IncomingMode::Accept => CallHandlerDecision::Accept,
            IncomingMode::RejectBusy => CallHandlerDecision::Reject {
                status: 486,
                reason: "Busy Here".into(),
            },
            IncomingMode::Defer(duration) => CallHandlerDecision::Defer(call.defer(duration)),
        }
    }

    async fn on_call_established(&self, handle: SessionHandle) {
        println!("[callback] call {} established", handle.id());
        self.send(CallbackEvent::Established(handle));
    }

    async fn on_call_ended(&self, call_id: CallId, reason: rvoip_session_core::EndReason) {
        println!("[callback] call {} ended: {:?}", call_id, reason);
        self.send(CallbackEvent::Ended {
            call_id,
            reason: format!("{reason:?}"),
        });
    }

    async fn on_call_failed(&self, call_id: CallId, status_code: u16, reason: String) {
        println!(
            "[callback] call {} failed: {} {}",
            call_id, status_code, reason
        );
        self.send(CallbackEvent::Failed {
            call_id,
            status_code,
            reason,
        });
    }

    async fn on_call_cancelled(&self, call_id: CallId) {
        println!("[callback] call {} cancelled", call_id);
        self.send(CallbackEvent::Cancelled { call_id });
    }

    async fn on_dtmf(&self, handle: SessionHandle, digit: char) {
        self.send(CallbackEvent::Dtmf {
            call_id: handle.id().clone(),
            digit,
        });
    }

    async fn on_call_on_hold(&self, handle: SessionHandle) {
        self.send(CallbackEvent::LocalHold {
            call_id: handle.id().clone(),
        });
    }

    async fn on_call_resumed(&self, handle: SessionHandle) {
        self.send(CallbackEvent::LocalResume {
            call_id: handle.id().clone(),
        });
    }

    async fn on_remote_call_on_hold(&self, handle: SessionHandle) {
        self.send(CallbackEvent::RemoteHold {
            call_id: handle.id().clone(),
        });
    }

    async fn on_remote_call_resumed(&self, handle: SessionHandle) {
        self.send(CallbackEvent::RemoteResume {
            call_id: handle.id().clone(),
        });
    }

    async fn on_transfer_accepted(&self, handle: SessionHandle, refer_to: String) {
        self.send(CallbackEvent::TransferAccepted {
            call_id: handle.id().clone(),
            refer_to,
        });
    }

    async fn on_transfer_progress(&self, handle: SessionHandle, status_code: u16, reason: String) {
        self.send(CallbackEvent::TransferProgress {
            call_id: handle.id().clone(),
            status_code,
            reason,
        });
    }

    async fn on_transfer_completed(
        &self,
        old_call_id: CallId,
        new_call_id: CallId,
        target: String,
    ) {
        self.send(CallbackEvent::TransferCompleted {
            old_call_id,
            new_call_id,
            target,
        });
    }

    async fn on_transfer_failed(&self, handle: SessionHandle, status_code: u16, reason: String) {
        self.send(CallbackEvent::TransferFailed {
            call_id: handle.id().clone(),
            status_code,
            reason,
        });
    }

    async fn on_registration_success(&self, registrar: String, expires: u32, contact: String) {
        self.send(CallbackEvent::RegistrationSuccess {
            registrar,
            expires,
            contact,
        });
    }

    async fn on_unregistration_success(&self, registrar: String) {
        self.send(CallbackEvent::UnregistrationSuccess { registrar });
    }
}

pub struct CallbackRuntime {
    pub cfg: EndpointConfig,
    pub control: CallbackPeerControl,
    pub events: mpsc::UnboundedReceiver<CallbackEvent>,
    run_task: JoinHandle<rvoip_session_core::Result<()>>,
}

impl CallbackRuntime {
    pub async fn shutdown(self) -> ExampleResult<()> {
        self.control.shutdown();
        let _ = timeout(Duration::from_secs(3), self.run_task).await;
        Ok(())
    }
}

pub async fn callback_runtime(
    username: &str,
    default_local_port: u16,
    default_media_start: u16,
    default_media_end: u16,
    mode: IncomingMode,
) -> ExampleResult<CallbackRuntime> {
    let cfg = endpoint_config(
        username,
        default_local_port,
        default_media_start,
        default_media_end,
    )?;
    let config = if cfg.transport.eq_ignore_ascii_case("tls") {
        cfg.tls_srtp_stream_config()?
    } else {
        cfg.stream_config()
    };
    let (tx, events) = mpsc::unbounded_channel();
    let peer = CallbackPeer::new(EventQueueHandler::new(mode, tx), config).await?;
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

pub async fn register_callback_endpoint(
    runtime: &mut CallbackRuntime,
) -> ExampleResult<RegistrationHandle> {
    let cfg = &runtime.cfg;
    println!("[{}] AOR:        {}", cfg.username, cfg.aor_uri());
    println!("[{}] Contact:    {}", cfg.username, cfg.contact_uri());
    println!("[{}] Registrar:  {}", cfg.username, cfg.registrar_uri());
    println!("[{}] Registering with CallbackPeer...", cfg.username);
    let handle = runtime.control.register_with(cfg.registration()).await?;
    wait_for_callback_registration(&runtime.control, &handle, &cfg.username).await?;
    wait_for_registration_success(&mut runtime.events, Duration::from_secs(10)).await?;
    println!("[{}] Registered.", cfg.username);
    Ok(handle)
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

async fn wait_for_callback_registration(
    control: &CallbackPeerControl,
    handle: &RegistrationHandle,
    username: &str,
) -> ExampleResult<()> {
    for _ in 0..50 {
        if control.is_registered(handle).await? {
            return Ok(());
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err(format!("callback endpoint {} did not register within 10s", username).into())
}

pub async fn call_with_answer_retry(
    runtime: &mut CallbackRuntime,
    target: &str,
    timeout_duration: Duration,
) -> ExampleResult<SessionHandle> {
    let attempts = call_retry_attempts()?.max(1);
    let mut last_error: Option<String> = None;

    for attempt in 1..=attempts {
        let handle = runtime.control.call(target).await?;
        match wait_for_established(&mut runtime.events, handle.id(), timeout_duration).await {
            Ok(answered) => return Ok(answered),
            Err(e) => {
                println!(
                    "[call] Attempt {}/{} to {} was not answered: {}",
                    attempt, attempts, target, e
                );
                last_error = Some(e.to_string());
                if attempt < attempts {
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| "call was not answered and no retry error was captured".into())
        .into())
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
                    return Err(format!("call failed with {} {}", status_code, reason).into())
                }
                Some(CallbackEvent::Cancelled {
                    call_id: cancelled_id,
                }) if &cancelled_id == call_id => return Err("call cancelled before answer".into()),
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
                }) => return Err(format!("call failed with {} {}", status_code, reason).into()),
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
                        println!(
                            "[callback] call failed as expected: {} {}",
                            status_code, reason
                        );
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
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Cancelled { .. }) => return Ok(()),
                Some(CallbackEvent::Failed {
                    status_code,
                    reason,
                    ..
                }) => {
                    return Err(format!(
                        "call failed while waiting for cancellation: {} {}",
                        status_code, reason
                    )
                    .into())
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
                Some(CallbackEvent::Dtmf { digit, .. }) if digit == expected[index] => {
                    println!("[callback] received DTMF '{}'", digit);
                    index += 1;
                }
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

pub async fn wait_for_transfer_completion(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::TransferAccepted { refer_to, .. }) => {
                    println!("[callback-transfer] REFER accepted for {}", refer_to);
                }
                Some(CallbackEvent::TransferProgress {
                    status_code,
                    reason,
                    ..
                }) => {
                    println!("[callback-transfer] progress: {} {}", status_code, reason);
                }
                Some(CallbackEvent::TransferCompleted { target, .. }) => {
                    println!("[callback-transfer] completed to {}", target);
                    return Ok(());
                }
                Some(CallbackEvent::TransferFailed {
                    status_code,
                    reason,
                    ..
                }) => return Err(format!("transfer failed: {} {}", status_code, reason).into()),
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for transfer",
            timeout_duration
        )
    })?
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

pub async fn wait_for_remote_hold_resume(
    events: &mut mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    timeout(timeout_duration, async {
        let mut saw_hold = false;
        loop {
            match events.recv().await {
                Some(CallbackEvent::RemoteHold { .. }) => saw_hold = true,
                Some(CallbackEvent::RemoteResume { .. }) if saw_hold => return Ok(()),
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for remote hold/resume",
            timeout_duration
        )
    })?
}
