//! Generic CallbackPeer endpoint for the Asterisk callback release-gate suite.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    call_with_answer_retry, callback_runtime, expect_remote_hold_events, load_env,
    post_register_settle_duration, register_callback_endpoint, remote_test_digits,
    remote_test_timeout, send_tone_segment, start_tone_recorder, unregister_callback_endpoint,
    wait_for_call_failed, wait_for_cancel_cleanup, wait_for_cancelled, wait_for_dtmf_sequence,
    wait_for_local_hold_resume, wait_for_next_established, wait_for_remote_hold_resume,
    wait_for_transfer_completion, CallbackEvent, ExampleResult, IncomingMode,
    ENDPOINT_1001_TONE_HZ, ENDPOINT_1002_TONE_HZ, ENDPOINT_1003_TONE_HZ, ENDPOINT_2002_TONE_HZ,
};
use rvoip_session_core::SessionHandle;
use tokio::time::sleep;

const PRE_HOLD_TONE_HZ: f32 = ENDPOINT_1001_TONE_HZ;
const DURING_HOLD_TONE_HZ: f32 = 550.0;
const POST_RESUME_TONE_HZ: f32 = 660.0;
const TONE_FRAMES_PER_PHASE: usize = 100;
const HOLD_TONE_FRAMES: usize = 50;

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    common::init_tracing();

    match std::env::var("CALLBACK_SCENARIO")?.as_str() {
        "registration_tls" => run_registration("1001", 5070, 16000, 16100).await,
        "registration_udp" => run_registration("2001", 5080, 17000, 17100).await,
        "tls_hold_caller" => run_hold_caller("1001", 5070, 16000, 16100, "1002", true).await,
        "tls_hold_callee" => run_hold_callee("1002", 5072, 16120, 16220, true).await,
        "udp_hold_caller" => run_hold_caller("2001", 5080, 17000, 17100, "2002", false).await,
        "udp_hold_callee" => run_hold_callee("2002", 5082, 17120, 17220, false).await,
        "tls_ring_caller" => run_ring_caller("1001", 5070, 16000, 16100).await,
        "tls_ring_target" => run_ring_target("1003", 5074, 16240, 16340).await,
        "udp_ring_caller" => run_ring_caller("2001", 5080, 17000, 17100).await,
        "udp_ring_target" => run_ring_target("2003", 5084, 17240, 17340).await,
        "tls_dtmf_caller" => run_dtmf_caller("1001", 5070, 16000, 16100, "1002", true).await,
        "tls_dtmf_callee" => run_dtmf_callee("1002", 5072, 16120, 16220, true).await,
        "udp_dtmf_caller" => run_dtmf_caller("2001", 5080, 17000, 17100, "2002", false).await,
        "udp_dtmf_callee" => run_dtmf_callee("2002", 5082, 17120, 17220, false).await,
        "tls_transferor" => run_transferor("1001", 5070, 16000, 16100, "1002", true).await,
        "tls_transferee" => run_transferee("1002", 5072, 16120, 16220, true).await,
        "tls_transfer_target" => run_transfer_target("1003", 5074, 16240, 16340, true).await,
        "udp_transferor" => run_transferor("2001", 5080, 17000, 17100, "2002", false).await,
        "udp_transferee" => run_transferee("2002", 5082, 17120, 17220, false).await,
        "udp_transfer_target" => run_transfer_target("2003", 5084, 17240, 17340, false).await,
        "tls_reject_caller" => run_reject_caller("1001", 5070, 16000, 16100).await,
        "tls_reject_callee" => run_reject_callee("1002", 5072, 16120, 16220).await,
        "udp_reject_caller" => run_reject_caller("2001", 5080, 17000, 17100).await,
        "udp_reject_callee" => run_reject_callee("2002", 5082, 17120, 17220).await,
        other => Err(format!("unknown CALLBACK_SCENARIO '{}'", other).into()),
    }
}

async fn run_registration(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::RejectBusy).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    sleep(Duration::from_secs(2)).await;
    unregister_callback_endpoint(&mut runtime, &registration).await?;
    runtime.shutdown().await
}

async fn run_hold_caller(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
    target_user: &str,
    tls: bool,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::RejectBusy).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    settle_after_register().await?;

    let target = runtime.cfg.outbound_call_uri(target_user);
    println!("[{}] Calling {} for callback hold/resume.", user, target);
    let handle = call_with_answer_retry(&mut runtime, &target, remote_test_timeout()?).await?;
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
    send_tone_segment(
        &sender,
        PRE_HOLD_TONE_HZ,
        TONE_FRAMES_PER_PHASE,
        &mut frame_index,
    )
    .await?;
    handle.hold().await?;
    wait_for_hold_state(&handle).await?;
    send_tone_segment(
        &sender,
        DURING_HOLD_TONE_HZ,
        HOLD_TONE_FRAMES,
        &mut frame_index,
    )
    .await?;
    sleep(Duration::from_millis(500)).await;
    handle.resume().await?;
    wait_for_active_state(&handle).await?;
    send_tone_segment(
        &sender,
        POST_RESUME_TONE_HZ,
        TONE_FRAMES_PER_PHASE,
        &mut frame_index,
    )
    .await?;
    wait_for_local_hold_resume(&mut runtime.events, Duration::from_secs(15)).await?;

    drop(sender);
    handle.hangup().await?;
    sleep(Duration::from_secs(1)).await;
    stop_recv_task(recv_task).await;
    let received = received_buf.lock().map(|g| g.clone()).unwrap_or_default();
    let name = if tls {
        "tls_srtp_hold_resume_1001_received.wav"
    } else {
        "hold_resume_2001_received.wav"
    };
    common::save_wav(&runtime.cfg.output_dir, name, &received)?;
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_hold_callee(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
    tls: bool,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::Accept).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    println!("[{}] Waiting for callback hold/resume call.", user);
    let handle = wait_for_next_established(&mut runtime.events, remote_test_timeout()?).await?;
    let tone = if tls {
        ENDPOINT_1002_TONE_HZ
    } else {
        ENDPOINT_2002_TONE_HZ
    };
    let recorder = start_tone_recorder(&handle, tone).await?;
    if expect_remote_hold_events()? {
        wait_for_remote_hold_resume(&mut runtime.events, Duration::from_secs(20)).await?;
    }
    wait_for_callback_end(&mut runtime.events, Duration::from_secs(45)).await?;
    let name = if tls {
        "tls_srtp_hold_resume_1002_received.wav"
    } else {
        "hold_resume_2002_received.wav"
    };
    recorder
        .stop_and_save(&runtime.cfg.output_dir, name)
        .await?;
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_ring_caller(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::RejectBusy).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    settle_after_register().await?;
    let target = runtime.cfg.remote_call_uri();
    println!("[{}] Calling callback ring target {}.", user, target);
    let handle = runtime.control.call(&target).await?;
    wait_for_ringing_state(&handle, remote_test_timeout()?).await?;
    runtime.control.hangup(&handle).await?;
    wait_for_cancel_cleanup(&handle, Duration::from_secs(12)).await?;
    wait_for_cancelled(&mut runtime.events, Duration::from_secs(12)).await?;
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_ring_target(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
) -> ExampleResult<()> {
    let mut runtime = callback_runtime(
        user,
        port,
        media_start,
        media_end,
        IncomingMode::Defer(Duration::from_secs(30)),
    )
    .await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    wait_for_incoming_notice(&mut runtime.events, remote_test_timeout()?).await?;
    match wait_for_cancelled(&mut runtime.events, Duration::from_secs(12)).await {
        Ok(()) => println!("[{}] Observed callback cancellation on ringing target.", user),
        Err(e) => println!(
            "[{}] No endpoint CANCEL callback observed before timeout ({e}); caller-side callback cancellation remains the required assertion for this Asterisk profile.",
            user
        ),
    }
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_dtmf_caller(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
    target_user: &str,
    tls: bool,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::RejectBusy).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    settle_after_register().await?;
    let target = runtime.cfg.outbound_call_uri(target_user);
    let handle = call_with_answer_retry(&mut runtime, &target, remote_test_timeout()?).await?;
    let recorder = if tls {
        Some(start_tone_recorder(&handle, ENDPOINT_1001_TONE_HZ).await?)
    } else {
        None
    };
    for digit in remote_test_digits() {
        sleep(Duration::from_millis(500)).await;
        handle.send_dtmf(digit).await?;
    }
    sleep(Duration::from_secs(1)).await;
    handle.hangup().await?;
    if let Some(recorder) = recorder {
        recorder
            .stop_and_save(&runtime.cfg.output_dir, "tls_srtp_dtmf_1001_received.wav")
            .await?;
    }
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_dtmf_callee(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
    tls: bool,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::Accept).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    let handle = wait_for_next_established(&mut runtime.events, remote_test_timeout()?).await?;
    let recorder = if tls {
        Some(start_tone_recorder(&handle, ENDPOINT_1002_TONE_HZ).await?)
    } else {
        None
    };
    wait_for_dtmf_sequence(
        &mut runtime.events,
        &remote_test_digits(),
        remote_test_timeout()?,
    )
    .await?;
    wait_for_callback_end(&mut runtime.events, Duration::from_secs(15))
        .await
        .ok();
    if let Some(recorder) = recorder {
        recorder
            .stop_and_save(&runtime.cfg.output_dir, "tls_srtp_dtmf_1002_received.wav")
            .await?;
    }
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_reject_caller(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::RejectBusy).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    settle_after_register().await?;
    let target = if runtime.cfg.transport.eq_ignore_ascii_case("tls") {
        runtime.cfg.outbound_call_uri("1002")
    } else {
        runtime.cfg.outbound_call_uri("2002")
    };
    let handle = runtime.control.call(&target).await?;
    wait_for_call_failed(
        &mut runtime.events,
        handle.id(),
        486,
        remote_test_timeout()?,
    )
    .await?;
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_reject_callee(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::RejectBusy).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    wait_for_incoming_notice(&mut runtime.events, remote_test_timeout()?).await?;
    sleep(Duration::from_secs(1)).await;
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_transferor(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
    target_user: &str,
    tls: bool,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::RejectBusy).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    settle_after_register().await?;
    let target = runtime.cfg.outbound_call_uri(target_user);
    let transfer_target = runtime.cfg.remote_call_uri();
    let handle = call_with_answer_retry(&mut runtime, &target, remote_test_timeout()?).await?;
    let recorder = if tls {
        Some(start_tone_recorder(&handle, ENDPOINT_1001_TONE_HZ).await?)
    } else {
        None
    };
    sleep(Duration::from_secs(3)).await;
    handle.transfer_blind(&transfer_target).await?;
    wait_for_transfer_completion(&mut runtime.events, remote_test_timeout()?).await?;
    if let Some(recorder) = recorder {
        recorder
            .stop_and_save(
                &runtime.cfg.output_dir,
                "tls_srtp_blind_transfer_1001_received.wav",
            )
            .await?;
    }
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_transferee(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
    tls: bool,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::Accept).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    let handle = wait_for_next_established(&mut runtime.events, remote_test_timeout()?).await?;
    let recorder = if tls {
        Some(start_tone_recorder(&handle, ENDPOINT_1002_TONE_HZ).await?)
    } else {
        None
    };
    sleep(Duration::from_secs(12)).await;
    handle.hangup().await.ok();
    if let Some(recorder) = recorder {
        recorder
            .stop_and_save(
                &runtime.cfg.output_dir,
                "tls_srtp_blind_transfer_1002_received.wav",
            )
            .await?;
    }
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn run_transfer_target(
    user: &str,
    port: u16,
    media_start: u16,
    media_end: u16,
    tls: bool,
) -> ExampleResult<()> {
    let mut runtime =
        callback_runtime(user, port, media_start, media_end, IncomingMode::Accept).await?;
    let registration = register_callback_endpoint(&mut runtime).await?;
    let handle = wait_for_next_established(&mut runtime.events, Duration::from_secs(90)).await?;
    let recorder = if tls {
        Some(start_tone_recorder(&handle, ENDPOINT_1003_TONE_HZ).await?)
    } else {
        None
    };
    sleep(Duration::from_secs(4)).await;
    handle.hangup().await.ok();
    if let Some(recorder) = recorder {
        recorder
            .stop_and_save(
                &runtime.cfg.output_dir,
                "tls_srtp_blind_transfer_1003_received.wav",
            )
            .await?;
    }
    unregister_callback_endpoint(&mut runtime, &registration)
        .await
        .ok();
    runtime.shutdown().await
}

async fn wait_for_incoming_notice(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    tokio::time::timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Incoming { from, to, .. }) => {
                    println!("[callback] incoming call {} -> {}", from, to);
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
            "timed out after {:?} waiting for incoming call",
            timeout_duration
        )
    })?
}

async fn wait_for_callback_end(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<CallbackEvent>,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    tokio::time::timeout(timeout_duration, async {
        loop {
            match events.recv().await {
                Some(CallbackEvent::Ended { reason, .. }) => {
                    println!("[callback] call ended: {}", reason);
                    return Ok(());
                }
                Some(CallbackEvent::Failed {
                    status_code,
                    reason,
                    ..
                }) => return Err(format!("call failed: {} {}", status_code, reason).into()),
                Some(_) => {}
                None => return Err("callback event channel closed".into()),
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {:?} waiting for call end",
            timeout_duration
        )
    })?
}

async fn wait_for_ringing_state(
    handle: &SessionHandle,
    timeout_duration: Duration,
) -> ExampleResult<()> {
    tokio::time::timeout(timeout_duration, async {
        loop {
            match handle.state().await {
                Ok(rvoip_session_core::CallState::Ringing) => return Ok(()),
                Ok(rvoip_session_core::CallState::Active) => {
                    return Err("call answered before ringing could be asserted".into())
                }
                Ok(_) => sleep(Duration::from_millis(100)).await,
                Err(e) => return Err(format!("failed to read call state: {}", e).into()),
            }
        }
    })
    .await
    .map_err(|_| format!("timed out after {:?} waiting for ringing", timeout_duration))?
}

async fn wait_for_hold_state(handle: &SessionHandle) -> ExampleResult<()> {
    for _ in 0..30 {
        if handle.is_on_hold().await {
            return Ok(());
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err("call did not reach OnHold within 6s".into())
}

async fn wait_for_active_state(handle: &SessionHandle) -> ExampleResult<()> {
    for _ in 0..30 {
        if handle.is_active().await {
            return Ok(());
        }
        sleep(Duration::from_millis(200)).await;
    }
    Err("call did not return to Active within 6s".into())
}

async fn settle_after_register() -> ExampleResult<()> {
    let settle = post_register_settle_duration()?;
    if !settle.is_zero() {
        sleep(settle).await;
    }
    Ok(())
}

async fn stop_recv_task(task: tokio::task::JoinHandle<()>) {
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
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
