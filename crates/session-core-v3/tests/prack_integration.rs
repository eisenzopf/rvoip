//! Multi-binary integration test for RFC 3262 PRACK policy.
//!
//! Covers the 420 Bad Extension negative case: Alice advertises no 100rel
//! support while Bob requires it, so Bob MUST reject with 420 per RFC 3262
//! §4. Alice exits 0 only when she receives `CallFailed { status_code: 420 }`.
//!
//! The positive reliable-provisional flow (PRACK round-trip against a real
//! reliable 183 with SDP) is validated by wire-level unit tests in
//! `dialog-core/tests/prack_test.rs`. A full session-core-v3 positive
//! integration test requires a `send_early_media` API that isn't wired into
//! session-core-v3 yet — tracked as follow-on work.

use std::env;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ALICE_PORT: u16 = 35063;
const BOB_PORT: u16 = 35064;

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

#[test]
fn prack_policy_mismatch_returns_420() {
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

    let env_vars: Vec<(&str, String)> = vec![
        ("ALICE_PORT", ALICE_PORT.to_string()),
        ("BOB_PORT", BOB_PORT.to_string()),
    ];

    // Bob listens; give him a moment to bind before Alice INVITEs.
    let _bob = spawn_example("streampeer_prack_bob", &env_vars);
    std::thread::sleep(Duration::from_millis(800));

    let mut alice = spawn_example("streampeer_prack_alice", &env_vars);

    let deadline = Instant::now() + Duration::from_secs(20);
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

    let status = match exit {
        Some(s) => s,
        None => panic!("Alice did not finish within 20s"),
    };

    assert!(
        status.success(),
        "Alice exited with {:?} (expected success — received 420)",
        status.code()
    );
}
