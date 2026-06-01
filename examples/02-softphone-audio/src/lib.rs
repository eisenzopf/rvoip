//! Shared helpers for the two-process softphone-audio demo.
//!
//! Both `caller` and `callee` build an [`Endpoint`], then push a PCMU tone over
//! the call while recording what they receive. The receiver verifies the
//! incoming audio with a Goertzel filter (correct dominant frequency, enough
//! energy, low noise) so the demo proves real media flowed — no audio hardware
//! required. PCMU (G.711 µ-law) is the beta full-media codec.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rvoip_sip::{Config, Endpoint, EndpointAudioFrame, EndpointCall, EndpointProfile};
use tokio::time::sleep;

pub const SAMPLE_RATE: u32 = 8_000;
pub const FRAME_SAMPLES: usize = 160;
pub const FRAME_MS: u64 = 20;
pub const TONE_FRAMES: usize = 150;
pub const TONE_AMPLITUDE: f32 = 0.30;

const MEDIA_SETTLE_DELAY: Duration = Duration::from_millis(150);
const RECEIVE_DRAIN_DELAY: Duration = Duration::from_millis(800);
const MIN_RECEIVED_SAMPLES: usize = SAMPLE_RATE as usize * 2;
const MIN_RMS: f32 = 0.02;
const MIN_EXPECTED_POWER: f32 = 0.001;
const MIN_DOMINANCE: f32 = 3.0;
const MAX_DOMINANT_ERROR_HZ: f32 = 40.0;

/// Which tone a side sends and which it expects to hear back.
#[derive(Clone, Copy)]
pub struct AudioPlan {
    pub role: &'static str,
    pub remote: &'static str,
    pub send_hz: f32,
    pub expect_hz: f32,
    pub reject_hz: f32,
}

/// Result of verifying the received tone.
pub struct ToneReport {
    pub role: &'static str,
    pub remote: &'static str,
    pub expected_hz: f32,
    pub samples: usize,
    pub dominant_hz: f32,
    pub expected_power: f32,
    pub rejected_power: f32,
    pub dominance: f32,
    pub rms: f32,
}

/// Build an `Endpoint` bound to `sip_port` with a dedicated media port range
/// (distinct per side so two processes can share one loopback host).
pub async fn build_endpoint(
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

/// Send `plan.send_hz` over the call while recording the far end, then verify
/// the received audio matches `plan.expect_hz`.
pub async fn exchange_audio(call: EndpointCall, plan: AudioPlan) -> anyhow::Result<ToneReport> {
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

    analyze_tone(&samples, plan)
}

pub fn print_report(report: &ToneReport) {
    println!(
        "✅ {} received {}'s {:.0} Hz tone: samples={}, dominant={:.0} Hz, expected-energy={:.6}, self-energy={:.6}, dominance={:.2}x, rms={:.3}",
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
