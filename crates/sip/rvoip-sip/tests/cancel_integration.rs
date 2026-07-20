//! Multi-binary integration test for CANCEL / 487 Request Terminated
//! (RFC 3261 §9).
//!
//! Alice INVITEs Bob. Bob never accepts — dialog-core auto-replies 180
//! Ringing. Alice observes `CallStateChanged(Ringing)`, calls
//! `handle.hangup()` (which routes to CANCEL since the call isn't
//! answered yet), and asserts she receives `Event::CallCancelled` — the
//! distinct "missed call" event that session-core emits on 487 (not
//! the generic `CallFailed`). Dialog-core can report the same terminal 487
//! through two coordination paths, so Alice also asserts exactly one terminal
//! app event is delivered.

use std::env;
use std::path::{Path, PathBuf};
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
    // The examples are built before launch. Running the executables directly
    // avoids serializing Bob and Alice behind Cargo's artifact lock: Bob is a
    // long-lived peer, so a second nested `cargo run` cannot start reliably
    // until the very process it needs to call has exited.
    let mut cmd = Command::new(example_binary(name));
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn()
        .map(ChildGuard)
        .unwrap_or_else(|e| panic!("failed to spawn {}: {}", name, e))
}

fn example_binary(name: &str) -> PathBuf {
    let test_binary = env::current_exe().expect("current integration-test binary");
    let debug_dir = test_binary
        .parent()
        .and_then(Path::parent)
        .expect("integration test runs from target/<profile>/deps");
    let binary = debug_dir
        .join("examples")
        .join(format!("{name}{}", env::consts::EXE_SUFFIX));
    assert!(
        binary.is_file(),
        "built example binary is missing: {}",
        binary.display()
    );
    binary
}

fn build_examples() {
    let status = Command::new(cargo_bin())
        .args([
            "build",
            "--quiet",
            "-p",
            "rvoip-sip",
            "--example",
            "regression_cancel_alice",
            "--example",
            "regression_cancel_bob",
        ])
        .status()
        .expect("failed to invoke cargo build");
    assert!(status.success(), "cargo build failed");
}

#[test]
fn cancel_emits_exactly_one_callcancelled_event() {
    build_examples();

    let envs: Vec<(&str, String)> = vec![
        ("ALICE_PORT", ALICE_PORT.to_string()),
        ("BOB_PORT", BOB_PORT.to_string()),
    ];

    let _bob = spawn_example("regression_cancel_bob", &envs);
    std::thread::sleep(Duration::from_millis(800));
    let mut alice = spawn_example("regression_cancel_alice", &envs);

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
        "Alice exited with {:?} (expected 0 = saw exactly one CallCancelled)",
        status.code()
    );
}
