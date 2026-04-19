//! Multi-binary integration test for CANCEL / 487 Request Terminated
//! (RFC 3261 §9).
//!
//! Alice INVITEs Bob. Bob never accepts — dialog-core auto-replies 180
//! Ringing. Alice observes `CallStateChanged(Ringing)`, calls
//! `handle.hangup()` (which routes to CANCEL since the call isn't
//! answered yet), and asserts she receives `Event::CallCancelled` — the
//! distinct "missed call" event that session-core-v3 emits on 487 (not
//! the generic `CallFailed`).

use std::env;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ALICE_PORT: u16 = 35071;
const BOB_PORT: u16 = 35072;

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
    cmd.spawn()
        .map(ChildGuard)
        .unwrap_or_else(|e| panic!("failed to spawn {}: {}", name, e))
}

fn build_examples() {
    let status = Command::new(cargo_bin())
        .args([
            "build",
            "--quiet",
            "-p",
            "rvoip-session-core-v3",
            "--example",
            "streampeer_cancel_alice",
            "--example",
            "streampeer_cancel_bob",
        ])
        .status()
        .expect("failed to invoke cargo build");
    assert!(status.success(), "cargo build failed");
}

#[test]
fn cancel_emits_callcancelled_event() {
    build_examples();

    let envs: Vec<(&str, String)> = vec![
        ("ALICE_PORT", ALICE_PORT.to_string()),
        ("BOB_PORT", BOB_PORT.to_string()),
    ];

    let _bob = spawn_example("streampeer_cancel_bob", &envs);
    std::thread::sleep(Duration::from_millis(800));
    let mut alice = spawn_example("streampeer_cancel_alice", &envs);

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

    let status = exit.unwrap_or_else(|| panic!("Alice did not finish within 20s"));
    assert!(
        status.success(),
        "Alice exited with {:?} (expected 0 = saw CallCancelled)",
        status.code()
    );
}
