use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use rvoip_sip::{
    Endpoint, EndpointAudioFrame, EndpointCall, EndpointCallId, EndpointControl, EndpointEvent,
    EndpointEvents, EndpointIncomingCall, EndpointRegistrationStatus, EndpointSipTrace,
    SessionError,
};

use crate::audio::{
    float_to_i16, resample_linear, AudioBridge, RunningAudio, FRAME_MS, FRAME_SAMPLES, SAMPLE_RATE,
};
use crate::config::{format_registration, RuntimeOptions, TestAudio, TestRole};
use crate::runtime::TraceFile;

const CALLER_TEST_TONE_HZ: f32 = 440.0;
const CALLEE_TEST_TONE_HZ: f32 = 660.0;
const TEST_TONE_AMPLITUDE: f32 = 0.30;
const TEST_TONE_MAX_SAMPLES: usize = SAMPLE_RATE as usize * 10;
const TEST_TONE_MIN_SAMPLES: usize = SAMPLE_RATE as usize / 2;
const TEST_TONE_MIN_RMS: f32 = 0.02;
const TEST_TONE_MIN_POWER: f32 = 0.001;
const TEST_TONE_MIN_DOMINANCE: f32 = 3.0;
const TEST_TONE_MAX_FREQ_ERROR_HZ: f32 = 40.0;

pub(crate) async fn run_smoke(role: TestRole, options: RuntimeOptions) -> anyhow::Result<()> {
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
    let mut trace_file = match options.sip_trace.file.as_ref() {
        Some(path) => {
            println!("writing SIP trace to {}", path.display());
            Some(TraceFile::open(path).map_err(|err| {
                anyhow::anyhow!("failed to open SIP trace file {}: {err}", path.display())
            })?)
        }
        None => None,
    };
    let result = match role {
        TestRole::Caller | TestRole::PbxCaller => {
            smoke_caller(&options, control, events, &mut trace_file).await
        }
        TestRole::Callee | TestRole::PbxCallee => {
            smoke_callee(&options, control, events, &mut trace_file).await
        }
    };
    result
}

async fn smoke_caller(
    options: &RuntimeOptions,
    control: EndpointControl,
    mut events: EndpointEvents,
    trace_file: &mut Option<TraceFile>,
) -> anyhow::Result<()> {
    let target = options
        .dial
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("caller smoke mode requires --dial"))?;
    let call = control.call(target).await?;
    println!("calling {target} ({})", call.id());
    let call = wait_for_answered(&mut events, call.id(), options.test_timeout, trace_file).await?;
    println!("answered {}", call.id());

    let audio = start_test_audio(
        call.clone(),
        options,
        TestTonePlan::for_role(TestRole::Caller),
    )
    .await?;
    call.send_dtmf(options.test_dtmf).await?;
    println!("sent DTMF {}", options.test_dtmf);
    call.hold().await?;
    wait_for_call_event(
        &mut events,
        call.id(),
        options.test_timeout,
        trace_file,
        |event| matches!(event, EndpointEvent::LocalHold { .. }),
    )
    .await?;
    call.resume().await?;
    wait_for_call_event(
        &mut events,
        call.id(),
        options.test_timeout,
        trace_file,
        |event| matches!(event, EndpointEvent::LocalResume { .. }),
    )
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
    trace_file: &mut Option<TraceFile>,
) -> anyhow::Result<()> {
    let incoming = wait_for_incoming(&mut events, options.test_timeout, trace_file).await?;
    println!("answering incoming call from {}", incoming.from());
    let call = incoming.answer().await?;
    let audio = start_test_audio(
        call.clone(),
        options,
        TestTonePlan::for_role(TestRole::Callee),
    )
    .await?;
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
            Ok(Ok(Some(EndpointEvent::SipTrace(trace)))) => {
                write_trace_event(trace_file, &trace)?;
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
    trace_file: &mut Option<TraceFile>,
) -> anyhow::Result<EndpointCall> {
    let fut = async {
        loop {
            match events.next().await? {
                Some(EndpointEvent::CallAnswered { call, .. }) if call.id() == expected => {
                    return Ok(call)
                }
                Some(EndpointEvent::SipTrace(trace)) => {
                    write_trace_event(trace_file, &trace)?;
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
    trace_file: &mut Option<TraceFile>,
) -> anyhow::Result<EndpointIncomingCall> {
    let fut = async {
        loop {
            match events.next().await? {
                Some(EndpointEvent::IncomingCall(incoming)) => return Ok(incoming),
                Some(EndpointEvent::SipTrace(trace)) => {
                    write_trace_event(trace_file, &trace)?;
                }
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
    trace_file: &mut Option<TraceFile>,
    mut matches_event: impl FnMut(&EndpointEvent) -> bool,
) -> anyhow::Result<()> {
    let fut = async {
        loop {
            match events.next().await? {
                Some(event) if event_belongs_to(&event, &call_id) && matches_event(&event) => {
                    return Ok(())
                }
                Some(EndpointEvent::SipTrace(trace)) => {
                    write_trace_event(trace_file, &trace)?;
                }
                Some(_) => {}
                None => return Err(SessionError::Other("event stream closed".into())),
            }
        }
    };
    Ok(tokio::time::timeout(timeout, fut).await??)
}

fn write_trace_event(
    trace_file: &mut Option<TraceFile>,
    trace: &EndpointSipTrace,
) -> Result<(), SessionError> {
    if let Some(file) = trace_file.as_mut() {
        file.write(trace)?;
    }
    Ok(())
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
    tone: TestTonePlan,
) -> anyhow::Result<TestAudioRun> {
    match options.test_audio {
        TestAudio::Synthetic => start_synthetic_audio(call, tone).await,
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

#[derive(Debug, Clone, Copy)]
struct TestTonePlan {
    send_hz: f32,
    expect_hz: f32,
    reject_hz: f32,
}

impl TestTonePlan {
    fn for_role(role: TestRole) -> Self {
        match role {
            TestRole::Caller | TestRole::PbxCaller => Self {
                send_hz: CALLER_TEST_TONE_HZ,
                expect_hz: CALLEE_TEST_TONE_HZ,
                reject_hz: CALLER_TEST_TONE_HZ,
            },
            TestRole::Callee | TestRole::PbxCallee => Self {
                send_hz: CALLEE_TEST_TONE_HZ,
                expect_hz: CALLER_TEST_TONE_HZ,
                reject_hz: CALLEE_TEST_TONE_HZ,
            },
        }
    }
}

async fn start_synthetic_audio(
    call: EndpointCall,
    tone: TestTonePlan,
) -> anyhow::Result<TestAudioRun> {
    let audio = call.audio().await?;
    let (sender, mut receiver) = audio.split();
    let capture = Arc::new(ToneCapture::default());
    let capture_for_task = capture.clone();
    let send_task = tokio::spawn(async move {
        let mut timestamp = 0u32;
        let mut phase = 0.0f32;
        loop {
            let pcm = tone_frame(tone.send_hz, &mut phase);
            let frame = EndpointAudioFrame::pcmu_sized_mono_8khz(pcm, timestamp);
            timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);
            if sender.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(FRAME_MS as u64)).await;
        }
    });
    let recv_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            capture_for_task.record(&frame);
        }
    });
    Ok(TestAudioRun::Synthetic {
        capture,
        tone,
        send_task,
        recv_task,
    })
}

enum TestAudioRun {
    Synthetic {
        capture: Arc<ToneCapture>,
        tone: TestTonePlan,
        send_task: tokio::task::JoinHandle<()>,
        recv_task: tokio::task::JoinHandle<()>,
    },
    Cpal(RunningAudio),
}

impl TestAudioRun {
    fn require_media(&self) -> anyhow::Result<()> {
        match self {
            Self::Synthetic { capture, tone, .. } => {
                let analysis = capture.analyze(*tone)?;
                println!(
                    "detected {:.0} Hz audio tone (dominant {:.0} Hz, rms {:.3}, frames {})",
                    tone.expect_hz, analysis.dominant_hz, analysis.rms, analysis.frames
                );
            }
            Self::Cpal(running) => {
                let _ = running;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct ToneCapture {
    received: AtomicUsize,
    samples: Mutex<Vec<f32>>,
}

impl ToneCapture {
    fn record(&self, frame: &EndpointAudioFrame) {
        if frame.samples.is_empty() {
            return;
        }
        self.received.fetch_add(1, Ordering::SeqCst);

        let mut samples = frame
            .samples
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect::<Vec<_>>();
        if frame.sample_rate != SAMPLE_RATE {
            samples = resample_linear(&samples, frame.sample_rate, SAMPLE_RATE);
        }

        if let Ok(mut captured) = self.samples.lock() {
            let remaining = TEST_TONE_MAX_SAMPLES.saturating_sub(captured.len());
            captured.extend(samples.into_iter().take(remaining));
        }
    }

    fn analyze(&self, tone: TestTonePlan) -> anyhow::Result<ToneAnalysis> {
        let frames = self.received.load(Ordering::SeqCst);
        if frames == 0 {
            anyhow::bail!("no inbound synthetic media frames received");
        }

        let samples = self
            .samples
            .lock()
            .map_err(|_| anyhow::anyhow!("tone capture buffer poisoned"))?;
        if samples.len() < TEST_TONE_MIN_SAMPLES {
            anyhow::bail!(
                "not enough inbound audio for tone analysis: {} samples, need at least {}",
                samples.len(),
                TEST_TONE_MIN_SAMPLES
            );
        }

        let rms = rms(&samples);
        let expected_power = goertzel_power(&samples, SAMPLE_RATE, tone.expect_hz);
        let rejected_power = goertzel_power(&samples, SAMPLE_RATE, tone.reject_hz);
        let (dominant_hz, dominant_power) = dominant_tone(&samples);
        let dominance = expected_power / rejected_power.max(1.0e-9);

        if rms < TEST_TONE_MIN_RMS {
            anyhow::bail!(
                "remote audio tone too quiet: rms {:.4}, expected {:.0} Hz",
                rms,
                tone.expect_hz
            );
        }
        if expected_power < TEST_TONE_MIN_POWER {
            anyhow::bail!(
                "expected {:.0} Hz audio tone too weak: power {:.6}, rms {:.4}, frames {}",
                tone.expect_hz,
                expected_power,
                rms,
                frames
            );
        }
        if dominance < TEST_TONE_MIN_DOMINANCE {
            anyhow::bail!(
                "wrong remote audio tone: expected {:.0} Hz power {:.6}, local {:.0} Hz power {:.6}, dominance {:.2}",
                tone.expect_hz,
                expected_power,
                tone.reject_hz,
                rejected_power,
                dominance
            );
        }
        if (dominant_hz - tone.expect_hz).abs() > TEST_TONE_MAX_FREQ_ERROR_HZ {
            anyhow::bail!(
                "dominant remote audio tone was {:.0} Hz, expected {:.0} Hz (power {:.6})",
                dominant_hz,
                tone.expect_hz,
                dominant_power
            );
        }

        Ok(ToneAnalysis {
            frames,
            rms,
            dominant_hz,
        })
    }
}

#[derive(Debug)]
struct ToneAnalysis {
    frames: usize,
    rms: f32,
    dominant_hz: f32,
}

fn tone_frame(freq_hz: f32, phase: &mut f32) -> Vec<i16> {
    let phase_step = std::f32::consts::TAU * freq_hz / SAMPLE_RATE as f32;
    let mut pcm = Vec::with_capacity(FRAME_SAMPLES);
    for _ in 0..FRAME_SAMPLES {
        pcm.push(float_to_i16(TEST_TONE_AMPLITUDE * phase.sin()));
        *phase += phase_step;
        if *phase >= std::f32::consts::TAU {
            *phase -= std::f32::consts::TAU;
        }
    }
    pcm
}

fn rms(samples: &[f32]) -> f32 {
    let power = samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32;
    power.sqrt()
}

fn dominant_tone(samples: &[f32]) -> (f32, f32) {
    let mut best_hz = 0.0;
    let mut best_power = 0.0;
    for hz in (300..=900).step_by(20) {
        let hz = hz as f32;
        let power = goertzel_power(samples, SAMPLE_RATE, hz);
        if power > best_power {
            best_power = power;
            best_hz = hz;
        }
    }
    (best_hz, best_power)
}

fn goertzel_power(samples: &[f32], sample_rate: u32, freq_hz: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let omega = std::f32::consts::TAU * freq_hz / sample_rate as f32;
    let coeff = 2.0 * omega.cos();
    let mut q1 = 0.0;
    let mut q2 = 0.0;
    for sample in samples {
        let q0 = coeff * q1 - q2 + *sample;
        q2 = q1;
        q1 = q0;
    }
    let power = q1 * q1 + q2 * q2 - coeff * q1 * q2;
    power / (samples.len() * samples.len()) as f32
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
