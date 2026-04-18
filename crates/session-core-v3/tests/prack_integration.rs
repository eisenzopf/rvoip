//! Multi-binary integration tests for RFC 3262 PRACK / reliable-provisional
//! behaviour.
//!
//! Covers two scenarios, each running Alice and Bob in separate processes
//! with ports selected per-test to avoid collision on shared CI runners:
//!
//! - **Negative (420 Bad Extension)**: Alice advertises no 100rel while Bob
//!   requires it. Bob MUST reject with 420 per RFC 3262 §4. Alice exits 0
//!   when she sees `CallFailed { status_code: 420 }`.
//!
//! - **Positive (reliable 183 → auto-PRACK → 200 OK)**: both peers support
//!   100rel. Bob calls `send_early_media(None)` on the incoming call, which
//!   drives a reliable 183 with auto-negotiated SDP (Phase C.1.3 wire path).
//!   Alice's auto-PRACK (Phase C.1.2) round-trips underneath, Bob accepts,
//!   and Alice exits 0 on `CallAnswered`.

use std::env;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const NEG_ALICE_PORT: u16 = 35063;
const NEG_BOB_PORT: u16 = 35064;
const POS_ALICE_PORT: u16 = 35065;
const POS_BOB_PORT: u16 = 35066;

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
        "rvoip-session-core-v3",
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

fn build_examples() {
    let build_status = Command::new(cargo_bin())
        .args([
            "build",
            "--quiet",
            "-p",
            "rvoip-session-core-v3",
            "--example",
            "streampeer_prack_alice",
            "--example",
            "streampeer_prack_bob",
        ])
        .status()
        .expect("failed to invoke cargo build");
    assert!(build_status.success(), "cargo build failed");
}

fn run_scenario(
    alice_port: u16,
    bob_port: u16,
    mode: &str,
    bob_wait_secs: u64,
    alice_wait_secs: u64,
) {
    let env_vars: Vec<(&str, String)> = vec![
        ("ALICE_PORT", alice_port.to_string()),
        ("BOB_PORT", bob_port.to_string()),
        ("PRACK_MODE", mode.to_string()),
    ];

    // Bob listens; give him a moment to bind before Alice INVITEs.
    let _bob = spawn_example("streampeer_prack_bob", &env_vars);
    std::thread::sleep(Duration::from_millis(800));
    let _ = bob_wait_secs;

    let mut alice = spawn_example("streampeer_prack_alice", &env_vars);

    let deadline = Instant::now() + Duration::from_secs(alice_wait_secs);
    let exit = loop {
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

    let status = exit
        .unwrap_or_else(|| panic!("Alice did not finish within {}s (mode={})", alice_wait_secs, mode));

    assert!(
        status.success(),
        "Alice exited with {:?} (mode={}, expected success)",
        status.code(),
        mode
    );
}

#[test]
fn prack_policy_mismatch_returns_420() {
    build_examples();
    run_scenario(NEG_ALICE_PORT, NEG_BOB_PORT, "negative", 8, 20);
}

#[test]
fn prack_positive_reliable_183_flow() {
    build_examples();
    // Positive path: Bob sends reliable 183, Alice auto-PRACKs, Bob 200s.
    // Alice exits 0 on CallAnswered. Give generous margin for the
    // multi-step dance on a loaded CI box.
    run_scenario(POS_ALICE_PORT, POS_BOB_PORT, "positive", 12, 25);
}
