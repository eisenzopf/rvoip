//! Multi-binary 3-peer bridge roundtrip regression test.
//!
//! Drives `streampeer_bridge_alice`, `streampeer_bridge_peer`, and
//! `streampeer_bridge_carol` together, then reads each saved WAV and
//! asserts that the received audio actually carries the *other* endpoint's
//! tone (Alice's WAV has strong 880 Hz; Carol's has strong 440 Hz).
//!
//! This is the end-to-end verification for Item 2 (RTP bridge primitive)
//! and exercises Item 1 (per-call `events_for_session` streams) as a side
//! effect. Without both items, the bridge peer wouldn't route media
//! between the two legs or cleanly observe each leg's lifecycle. Tone
//! detection reuses the Goertzel filter pattern from
//! `audio_roundtrip_integration.rs`.

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ALICE_SIP_PORT: u16 = 35590;
const CAROL_SIP_PORT: u16 = 35591;
const BRIDGE_SIP_PORT: u16 = 35592;

const ALICE_MEDIA_START: u16 = 35700;
const ALICE_MEDIA_END: u16 = 35750;
const BRIDGE_MEDIA_START: u16 = 35760;
const BRIDGE_MEDIA_END: u16 = 35810;
const CAROL_MEDIA_START: u16 = 35820;
const CAROL_MEDIA_END: u16 = 35870;

const SAMPLE_RATE: f32 = 8000.0;
const ALICE_TONE_HZ: f32 = 440.0;
const CAROL_TONE_HZ: f32 = 880.0;

/// 3 s call at 8 kHz = 24 000 samples; allow slack for RTP startup +
/// the extra bridge-peer hop. Cap at 12 000 so slower CI machines still
/// pass as long as at least ~1.5 s of audio made it through.
const MIN_RECEIVED_SAMPLES: usize = 12_000;
/// Peer-tone energy must dominate self-tone energy by this factor.
const DOMINANCE_RATIO: f32 = 5.0;

struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn cargo_bin() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn spawn_example(name: &str, envs: &[(&str, String)]) -> ChildGuard {
    let mut cmd = Command::new(cargo_bin());
    cmd.args([
        "run",
        "--quiet",
        "-p",
        "rvoip-session-core",
        "--example",
        name,
    ]);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {}", name, e));
    ChildGuard(child)
}

fn goertzel_magnitude(samples: &[i16], sample_rate: f32, target_hz: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let n = samples.len() as f32;
    let k = (0.5 + (n * target_hz) / sample_rate).floor();
    let omega = (2.0 * std::f32::consts::PI * k) / n;
    let coeff = 2.0 * omega.cos();
    let (mut q1, mut q2) = (0.0f32, 0.0f32);
    for &s in samples {
        let q0 = coeff * q1 - q2 + (s as f32);
        q2 = q1;
        q1 = q0;
    }
    (q1 * q1 + q2 * q2 - q1 * q2 * coeff).sqrt()
}

fn read_wav(path: &PathBuf) -> Vec<i16> {
    let mut reader = hound::WavReader::open(path)
        .unwrap_or_else(|e| panic!("failed to open {}: {}", path.display(), e));
    reader
        .samples::<i16>()
        .map(|s| s.unwrap_or_else(|e| panic!("bad sample in {}: {}", path.display(), e)))
        .collect()
}

#[test]
fn bridge_roundtrip_relays_tones_between_legs() {
    let build_status = Command::new(cargo_bin())
        .args([
            "build",
            "--quiet",
            "-p",
            "rvoip-session-core",
            "--example",
            "streampeer_bridge_alice",
            "--example",
            "streampeer_bridge_peer",
            "--example",
            "streampeer_bridge_carol",
        ])
        .status()
        .expect("failed to invoke cargo build");
    assert!(build_status.success(), "cargo build failed");

    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp.path().to_string_lossy().to_string();

    let env_vars: Vec<(&str, String)> = vec![
        ("ALICE_SIP_PORT", ALICE_SIP_PORT.to_string()),
        ("CAROL_SIP_PORT", CAROL_SIP_PORT.to_string()),
        ("BRIDGE_SIP_PORT", BRIDGE_SIP_PORT.to_string()),
        ("ALICE_MEDIA_PORT_START", ALICE_MEDIA_START.to_string()),
        ("ALICE_MEDIA_PORT_END", ALICE_MEDIA_END.to_string()),
        ("BRIDGE_MEDIA_PORT_START", BRIDGE_MEDIA_START.to_string()),
        ("BRIDGE_MEDIA_PORT_END", BRIDGE_MEDIA_END.to_string()),
        ("CAROL_MEDIA_PORT_START", CAROL_MEDIA_START.to_string()),
        ("CAROL_MEDIA_PORT_END", CAROL_MEDIA_END.to_string()),
        ("AUDIO_OUTPUT_DIR", out_dir.clone()),
        // Keep this larger than the 3-second tone emission so both sides
        // finish transmitting before the bridge tears down.
        ("BRIDGE_CALL_DURATION_SECS", "4".to_string()),
    ];

    // Start the callee and bridge first so Alice's INVITE has somewhere
    // to land. Matches the audio_roundtrip ordering.
    let mut carol = spawn_example("streampeer_bridge_carol", &env_vars);
    std::thread::sleep(Duration::from_millis(800));
    let mut bridge = spawn_example("streampeer_bridge_peer", &env_vars);
    std::thread::sleep(Duration::from_millis(800));
    let mut alice = spawn_example("streampeer_bridge_alice", &env_vars);

    let deadline = Instant::now() + Duration::from_secs(60);
    let alice_status = loop {
        match alice.0.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => panic!("failed to poll Alice: {}", e),
        }
    };
    let alice_status = alice_status.expect("Alice did not finish within 60s");
    assert!(
        alice_status.success(),
        "Alice exited with {:?}",
        alice_status.code()
    );

    // Let the bridge tear down both legs, then let Carol flush her WAV.
    let tail_deadline = Instant::now() + Duration::from_secs(15);
    for (name, child) in [("bridge", &mut bridge), ("carol", &mut carol)] {
        loop {
            match child.0.try_wait() {
                Ok(Some(s)) => {
                    assert!(
                        s.success() || s.code() == Some(0),
                        "{} exited with {:?}",
                        name,
                        s.code()
                    );
                    break;
                }
                Ok(None) => {
                    if Instant::now() >= tail_deadline {
                        panic!("{} did not finish within 15s of Alice", name);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => panic!("failed to poll {}: {}", name, e),
            }
        }
    }

    let alice_wav = tmp.path().join("alice_received.wav");
    let carol_wav = tmp.path().join("carol_received.wav");

    let alice_samples = read_wav(&alice_wav);
    let carol_samples = read_wav(&carol_wav);

    assert!(
        alice_samples.len() >= MIN_RECEIVED_SAMPLES,
        "alice_received.wav too short: {} samples (expected ≥ {})",
        alice_samples.len(),
        MIN_RECEIVED_SAMPLES
    );
    assert!(
        carol_samples.len() >= MIN_RECEIVED_SAMPLES,
        "carol_received.wav too short: {} samples (expected ≥ {})",
        carol_samples.len(),
        MIN_RECEIVED_SAMPLES
    );

    // Alice's received buffer should carry Carol's 880 Hz tone (forwarded
    // across the bridge) and very little of her own 440 Hz.
    let alice_peer_mag = goertzel_magnitude(&alice_samples, SAMPLE_RATE, CAROL_TONE_HZ);
    let alice_self_mag = goertzel_magnitude(&alice_samples, SAMPLE_RATE, ALICE_TONE_HZ);
    let alice_ratio = if alice_self_mag > 1.0 {
        alice_peer_mag / alice_self_mag
    } else {
        f32::INFINITY
    };
    assert!(
        alice_ratio >= DOMINANCE_RATIO,
        "alice_received: 880Hz energy {:.1} vs 440Hz energy {:.1} (ratio {:.2}, want ≥ {:.2}) — bridge didn't forward Carol's tone",
        alice_peer_mag,
        alice_self_mag,
        alice_ratio,
        DOMINANCE_RATIO
    );

    // Carol's received buffer should carry Alice's 440 Hz tone.
    let carol_peer_mag = goertzel_magnitude(&carol_samples, SAMPLE_RATE, ALICE_TONE_HZ);
    let carol_self_mag = goertzel_magnitude(&carol_samples, SAMPLE_RATE, CAROL_TONE_HZ);
    let carol_ratio = if carol_self_mag > 1.0 {
        carol_peer_mag / carol_self_mag
    } else {
        f32::INFINITY
    };
    assert!(
        carol_ratio >= DOMINANCE_RATIO,
        "carol_received: 440Hz energy {:.1} vs 880Hz energy {:.1} (ratio {:.2}, want ≥ {:.2}) — bridge didn't forward Alice's tone",
        carol_peer_mag,
        carol_self_mag,
        carol_ratio,
        DOMINANCE_RATIO
    );
}
