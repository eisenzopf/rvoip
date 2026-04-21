//! Multi-binary audio-roundtrip regression test.
//!
//! Drives the `streampeer_audio_alice` / `streampeer_audio_bob` example
//! binaries with non-default ports + a scratch output directory, then
//! reads each peer's saved WAV and asserts that the received audio
//! actually carries the *other* peer's tone (Alice's file has strong
//! 880 Hz energy; Bob's has strong 440 Hz). Tone detection uses a
//! hand-rolled Goertzel filter so no new dependency is required.
//!
//! This locks in the full media path — RTP send, RTP receive, SDP port
//! exchange, PCMU encode/decode, and `AudioStream` frame delivery — as
//! a CI-verifiable invariant. Without it the pre-existing `run.sh`
//! would pass even if the received stream were silence, self-loopback,
//! or mostly-dropped.

use std::env;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ALICE_SIP_PORT: u16 = 35090;
const BOB_SIP_PORT: u16 = 35091;
const ALICE_MEDIA_START: u16 = 35200;
const ALICE_MEDIA_END: u16 = 35250;
const BOB_MEDIA_START: u16 = 35260;
const BOB_MEDIA_END: u16 = 35310;

const SAMPLE_RATE: f32 = 8000.0;
const ALICE_TONE_HZ: f32 = 440.0;
const BOB_TONE_HZ: f32 = 880.0;

/// 3 s call at 8 kHz = 24 000 samples; allow up to a 1 s slack for
/// first-frame RTP startup losses.
const MIN_RECEIVED_SAMPLES: usize = 16_000;
/// Peer-tone energy must dominate self-tone energy by this factor.
/// The example cleanly loops back the peer's tone through PCMU; without
/// a regression this ratio is typically an order of magnitude higher.
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

fn build_examples() {
    let build_status = Command::new(cargo_bin())
        .args([
            "build",
            "--quiet",
            "-p",
            "rvoip-session-core",
            "--example",
            "streampeer_audio_alice",
            "--example",
            "streampeer_audio_bob",
        ])
        .status()
        .expect("failed to invoke cargo build");
    assert!(build_status.success(), "cargo build failed");
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

/// Goertzel magnitude at `target_hz` for a block of i16 samples. Cheaper
/// than a full FFT when we're only looking at two known frequencies.
/// Returns sqrt(q1² + q2² − q1·q2·coeff) — the standard Goertzel power
/// formulation.
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

fn read_wav(path: &Path) -> Vec<i16> {
    let mut reader = hound::WavReader::open(path)
        .unwrap_or_else(|e| panic!("failed to open {}: {}", path.display(), e));
    reader
        .samples::<i16>()
        .map(|s| s.unwrap_or_else(|e| panic!("bad sample in {}: {}", path.display(), e)))
        .collect()
}

#[test]
fn audio_roundtrip_delivers_peer_tone() {
    build_examples();

    let tmp = tempfile::tempdir().expect("tempdir");
    let out_dir = tmp.path().to_string_lossy().to_string();

    let env_vars: Vec<(&str, String)> = vec![
        ("ALICE_SIP_PORT", ALICE_SIP_PORT.to_string()),
        ("BOB_SIP_PORT", BOB_SIP_PORT.to_string()),
        ("ALICE_MEDIA_PORT_START", ALICE_MEDIA_START.to_string()),
        ("ALICE_MEDIA_PORT_END", ALICE_MEDIA_END.to_string()),
        ("BOB_MEDIA_PORT_START", BOB_MEDIA_START.to_string()),
        ("BOB_MEDIA_PORT_END", BOB_MEDIA_END.to_string()),
        ("AUDIO_OUTPUT_DIR", out_dir.clone()),
    ];

    let mut bob = spawn_example("streampeer_audio_bob", &env_vars);
    std::thread::sleep(Duration::from_millis(800));
    let mut alice = spawn_example("streampeer_audio_alice", &env_vars);

    let deadline = Instant::now() + Duration::from_secs(45);
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
    let alice_status = alice_status.expect("Alice did not finish within 45s");
    assert!(
        alice_status.success(),
        "Alice exited with {:?}",
        alice_status.code()
    );

    // Bob terminates after Alice hangs up, but give him a brief window
    // to flush the WAV to disk before we read it.
    let bob_deadline = Instant::now() + Duration::from_secs(10);
    let bob_status = loop {
        match bob.0.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if Instant::now() >= bob_deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("failed to poll Bob: {}", e),
        }
    };
    let bob_status = bob_status.expect("Bob did not finish within 10s of Alice");
    assert!(bob_status.success(), "Bob exited with {:?}", bob_status.code());

    let alice_wav = tmp.path().join("alice_received.wav");
    let bob_wav = tmp.path().join("bob_received.wav");

    let alice_samples = read_wav(&alice_wav);
    let bob_samples = read_wav(&bob_wav);

    assert!(
        alice_samples.len() >= MIN_RECEIVED_SAMPLES,
        "alice_received.wav too short: {} samples (expected ≥ {})",
        alice_samples.len(),
        MIN_RECEIVED_SAMPLES
    );
    assert!(
        bob_samples.len() >= MIN_RECEIVED_SAMPLES,
        "bob_received.wav too short: {} samples (expected ≥ {})",
        bob_samples.len(),
        MIN_RECEIVED_SAMPLES
    );

    // Alice's received buffer should carry Bob's 880 Hz tone and very
    // little of her own 440 Hz.
    let alice_bob_mag = goertzel_magnitude(&alice_samples, SAMPLE_RATE, BOB_TONE_HZ);
    let alice_self_mag = goertzel_magnitude(&alice_samples, SAMPLE_RATE, ALICE_TONE_HZ);
    let alice_ratio = if alice_self_mag > 1.0 {
        alice_bob_mag / alice_self_mag
    } else {
        f32::INFINITY
    };
    assert!(
        alice_ratio >= DOMINANCE_RATIO,
        "alice_received: 880Hz energy {:.1} vs 440Hz energy {:.1} (ratio {:.2}, want ≥ {:.2})",
        alice_bob_mag,
        alice_self_mag,
        alice_ratio,
        DOMINANCE_RATIO
    );

    // Bob's received buffer should carry Alice's 440 Hz tone.
    let bob_alice_mag = goertzel_magnitude(&bob_samples, SAMPLE_RATE, ALICE_TONE_HZ);
    let bob_self_mag = goertzel_magnitude(&bob_samples, SAMPLE_RATE, BOB_TONE_HZ);
    let bob_ratio = if bob_self_mag > 1.0 {
        bob_alice_mag / bob_self_mag
    } else {
        f32::INFINITY
    };
    assert!(
        bob_ratio >= DOMINANCE_RATIO,
        "bob_received: 440Hz energy {:.1} vs 880Hz energy {:.1} (ratio {:.2}, want ≥ {:.2})",
        bob_alice_mag,
        bob_self_mag,
        bob_ratio,
        DOMINANCE_RATIO
    );
}
