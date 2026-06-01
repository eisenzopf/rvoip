//! Endpoint audio roundtrip example.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example endpoint_audio_roundtrip
//!
//! This is intentionally single-process: Bob and Alice are normal Endpoint
//! clients, but one command shows the complete call + audio path.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::sleep;

use rvoip_sip::{Config, Endpoint, EndpointAudioFrame, EndpointCall, EndpointProfile};

const SAMPLE_RATE: u32 = 8_000;
const FRAME_SAMPLES: usize = 160;
const FRAME_MS: u64 = 20;
const TONE_FRAMES: usize = 150;
const TONE_AMPLITUDE: f32 = 0.30;
const MEDIA_SETTLE_DELAY: Duration = Duration::from_millis(150);
const RECEIVE_DRAIN_DELAY: Duration = Duration::from_millis(800);
const MIN_RECEIVED_SAMPLES: usize = SAMPLE_RATE as usize * 2;
const MIN_RMS: f32 = 0.02;
const MIN_EXPECTED_POWER: f32 = 0.001;
const MIN_DOMINANCE: f32 = 3.0;
const MAX_DOMINANT_ERROR_HZ: f32 = 40.0;

#[derive(Clone)]
struct Ports {
    alice_sip: u16,
    bob_sip: u16,
    alice_media_start: u16,
    alice_media_end: u16,
    bob_media_start: u16,
    bob_media_end: u16,
}

#[derive(Clone, Copy)]
struct AudioPlan {
    role: &'static str,
    remote: &'static str,
    send_hz: f32,
    expect_hz: f32,
    reject_hz: f32,
    wav_name: &'static str,
}

struct ToneReport {
    role: &'static str,
    remote: &'static str,
    expected_hz: f32,
    samples: usize,
    dominant_hz: f32,
    expected_power: f32,
    rejected_power: f32,
    dominance: f32,
    rms: f32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ports = Ports::from_env();
    let output_dir = output_dir();

    let (ready_tx, ready_rx) = oneshot::channel();
    let bob_ports = ports.clone();
    let bob_output_dir = output_dir.clone();
    let bob_task = tokio::spawn(async move { run_bob(bob_ports, bob_output_dir, ready_tx).await });

    ready_rx
        .await
        .map_err(|_| anyhow::anyhow!("Bob endpoint failed before it was ready"))?;
    sleep(Duration::from_millis(250)).await;

    let alice = build_endpoint(
        "alice",
        ports.alice_sip,
        ports.alice_media_start,
        ports.alice_media_end,
    )
    .await?;

    println!("[Alice] calling Bob at sip:bob@127.0.0.1:{}", ports.bob_sip);
    let call = alice
        .call_and_wait(
            &format!("sip:bob@127.0.0.1:{}", ports.bob_sip),
            Some(Duration::from_secs(10)),
        )
        .await?;
    println!("[Alice] call answered as {}", call.id());

    let alice_report = exchange_audio(
        call.clone(),
        AudioPlan {
            role: "Alice",
            remote: "Bob",
            send_hz: 440.0,
            expect_hz: 880.0,
            reject_hz: 440.0,
            wav_name: "alice_received.wav",
        },
        output_dir.as_deref(),
    )
    .await?;
    print_report(&alice_report);

    call.hangup_and_wait(Some(Duration::from_secs(5))).await?;
    alice.shutdown().await?;

    let bob_report = bob_task
        .await
        .map_err(|err| anyhow::anyhow!("Bob task failed: {err}"))??;
    print_report(&bob_report);

    println!("Endpoint audio roundtrip verified.");
    Ok(())
}

async fn run_bob(
    ports: Ports,
    output_dir: Option<PathBuf>,
    ready_tx: oneshot::Sender<()>,
) -> anyhow::Result<ToneReport> {
    let mut bob = build_endpoint(
        "bob",
        ports.bob_sip,
        ports.bob_media_start,
        ports.bob_media_end,
    )
    .await?;
    let _ = ready_tx.send(());

    println!("[Bob] waiting on sip:bob@127.0.0.1:{}", ports.bob_sip);
    let incoming = bob.wait_for_incoming().await?;
    println!("[Bob] incoming call from {}", incoming.from());
    let call = incoming.answer().await?;
    println!("[Bob] answered as {}", call.id());

    let report = exchange_audio(
        call.clone(),
        AudioPlan {
            role: "Bob",
            remote: "Alice",
            send_hz: 880.0,
            expect_hz: 440.0,
            reject_hz: 880.0,
            wav_name: "bob_received.wav",
        },
        output_dir.as_deref(),
    )
    .await?;

    call.wait_for_end(Some(Duration::from_secs(10))).await?;
    bob.shutdown().await?;
    Ok(report)
}

async fn build_endpoint(
    name: &str,
    sip_port: u16,
    media_start: u16,
    media_end: u16,
) -> rvoip_sip::Result<Endpoint> {
    let mut config = Config::local(name, sip_port);
    config.media_port_start = media_start;
    config.media_port_end = media_end;

    Endpoint::builder()
        .name(name)
        .profile(EndpointProfile::Custom(config))
        .build()
        .await
}

async fn exchange_audio(
    call: EndpointCall,
    plan: AudioPlan,
    output_dir: Option<&Path>,
) -> anyhow::Result<ToneReport> {
    let audio = call.audio().await?;
    let (sender, mut receiver) = audio.split();
    let received = Arc::new(Mutex::new(Vec::<i16>::new()));
    let received_for_task = received.clone();

    let receive_task = tokio::spawn(async move {
        while let Some(frame) = receiver.recv().await {
            if let Ok(mut samples) = received_for_task.lock() {
                samples.extend_from_slice(&frame.samples);
            }
        }
    });

    sleep(MEDIA_SETTLE_DELAY).await;
    let mut phase = 0.0;
    for frame_index in 0..TONE_FRAMES {
        let samples = tone_frame(plan.send_hz, &mut phase);
        let timestamp = (frame_index * FRAME_SAMPLES) as u32;
        let frame = EndpointAudioFrame::pcmu_sized_mono_8khz(samples, timestamp);
        sender.send(frame).await?;
        sleep(Duration::from_millis(FRAME_MS)).await;
    }
    drop(sender);

    sleep(RECEIVE_DRAIN_DELAY).await;
    receive_task.abort();
    let _ = receive_task.await;

    let samples = received
        .lock()
        .map_err(|_| anyhow::anyhow!("received audio buffer poisoned"))?
        .clone();

    if let Some(dir) = output_dir {
        save_wav(dir, plan.wav_name, &samples)?;
    }

    analyze_tone(&samples, plan)
}

fn analyze_tone(samples: &[i16], plan: AudioPlan) -> anyhow::Result<ToneReport> {
    if samples.len() < MIN_RECEIVED_SAMPLES {
        anyhow::bail!(
            "{} received only {} samples; need at least {}",
            plan.role,
            samples.len(),
            MIN_RECEIVED_SAMPLES
        );
    }

    let normalized = samples
        .iter()
        .map(|sample| *sample as f32 / i16::MAX as f32)
        .collect::<Vec<_>>();
    let rms = rms(&normalized);
    let expected_power = goertzel_power(&normalized, plan.expect_hz);
    let rejected_power = goertzel_power(&normalized, plan.reject_hz);
    let dominance = expected_power / rejected_power.max(1.0e-9);
    let (dominant_hz, dominant_power) = dominant_tone(&normalized);

    if rms < MIN_RMS {
        anyhow::bail!(
            "{} received audio is too quiet: rms {:.4}, expected {:.0} Hz",
            plan.role,
            rms,
            plan.expect_hz
        );
    }
    if expected_power < MIN_EXPECTED_POWER {
        anyhow::bail!(
            "{} expected {:.0} Hz tone too weak: power {:.6}, rms {:.4}",
            plan.role,
            plan.expect_hz,
            expected_power,
            rms
        );
    }
    if dominance < MIN_DOMINANCE {
        anyhow::bail!(
            "{} received the wrong tone: expected {:.0} Hz power {:.6}, self {:.0} Hz power {:.6}, dominance {:.2}x",
            plan.role,
            plan.expect_hz,
            expected_power,
            plan.reject_hz,
            rejected_power,
            dominance
        );
    }
    if (dominant_hz - plan.expect_hz).abs() > MAX_DOMINANT_ERROR_HZ {
        anyhow::bail!(
            "{} dominant tone was {:.0} Hz, expected {:.0} Hz (power {:.6})",
            plan.role,
            dominant_hz,
            plan.expect_hz,
            dominant_power
        );
    }

    Ok(ToneReport {
        role: plan.role,
        remote: plan.remote,
        expected_hz: plan.expect_hz,
        samples: samples.len(),
        dominant_hz,
        expected_power,
        rejected_power,
        dominance,
        rms,
    })
}

fn print_report(report: &ToneReport) {
    println!(
        "{} received {}'s {:.0} Hz tone: samples={}, dominant={:.0} Hz, expected-tone energy={:.6}, self-tone energy={:.6}, dominance={:.2}x, rms={:.3}",
        report.role,
        report.remote,
        report.expected_hz,
        report.samples,
        report.dominant_hz,
        report.expected_power,
        report.rejected_power,
        report.dominance,
        report.rms
    );
}

fn tone_frame(freq_hz: f32, phase: &mut f32) -> Vec<i16> {
    let phase_step = std::f32::consts::TAU * freq_hz / SAMPLE_RATE as f32;
    let mut samples = Vec::with_capacity(FRAME_SAMPLES);
    for _ in 0..FRAME_SAMPLES {
        samples.push(float_to_i16(TONE_AMPLITUDE * phase.sin()));
        *phase += phase_step;
        if *phase >= std::f32::consts::TAU {
            *phase -= std::f32::consts::TAU;
        }
    }
    samples
}

fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

fn rms(samples: &[f32]) -> f32 {
    let power = samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32;
    power.sqrt()
}

fn dominant_tone(samples: &[f32]) -> (f32, f32) {
    let mut best_hz = 0.0;
    let mut best_power = 0.0;
    for hz in (300..=1_000).step_by(20) {
        let hz = hz as f32;
        let power = goertzel_power(samples, hz);
        if power > best_power {
            best_power = power;
            best_hz = hz;
        }
    }
    (best_hz, best_power)
}

fn goertzel_power(samples: &[f32], freq_hz: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let omega = std::f32::consts::TAU * freq_hz / SAMPLE_RATE as f32;
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

fn save_wav(dir: &Path, name: &str, samples: &[i16]) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(name);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec)?;
    for sample in samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize()?;
    println!("Saved {}", path.display());
    Ok(())
}

fn output_dir() -> Option<PathBuf> {
    std::env::var_os("AUDIO_OUTPUT_DIR").map(PathBuf::from)
}

impl Ports {
    fn from_env() -> Self {
        Self {
            alice_sip: env_u16("ALICE_SIP_PORT", 5072),
            bob_sip: env_u16("BOB_SIP_PORT", 5073),
            alice_media_start: env_u16("ALICE_MEDIA_PORT_START", 17_200),
            alice_media_end: env_u16("ALICE_MEDIA_PORT_END", 17_249),
            bob_media_start: env_u16("BOB_MEDIA_PORT_START", 17_250),
            bob_media_end: env_u16("BOB_MEDIA_PORT_END", 17_299),
        }
    }
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
