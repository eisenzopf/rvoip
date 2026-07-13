//! Multi-binary integration test for RFC 4028 session timers.
//!
//! Alice and Bob negotiate a 10 second `Session-Expires`. Alice (as
//! refresher) must send an UPDATE at half-expiry and emit a
//! `SessionRefreshed` app-level event. Alice exits 0 iff she observed
//! the refresh within 12 seconds.
//!
//! Failure-case testing (refresher swallows UPDATE → 408 BYE) is deferred
//! — it requires session-core wiring to drop incoming UPDATEs, which
//! is not currently exposed by the public API.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ALICE_PORT: u16 = 35065;
const BOB_PORT: u16 = 35066;

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

fn spawn_example(name: &str, envs: &[(&str, String)]) -> ChildGuard {
    // Build the pair once, then launch the executables directly. Nested
    // `cargo run` peers can serialize behind Cargo's artifact lock and leave
    // Alice unable to start before the outer deadline.
    let mut cmd = Command::new(example_binary(name));
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
fn session_timer_refresh_emits_event() {
    let build_status = Command::new(cargo_bin())
        .args([
            "build",
            "--quiet",
            "-p",
            "rvoip-sip",
            "--example",
            "regression_session_timer_alice",
            "--example",
            "regression_session_timer_bob",
        ])
        .status()
        .expect("failed to invoke cargo build");
    assert!(build_status.success(), "cargo build failed");

    let env_vars: Vec<(&str, String)> = vec![
        ("ALICE_PORT", ALICE_PORT.to_string()),
        ("BOB_PORT", BOB_PORT.to_string()),
    ];

    let _bob = spawn_example("regression_session_timer_bob", &env_vars);
    std::thread::sleep(Duration::from_millis(800));

    let mut alice = spawn_example("regression_session_timer_alice", &env_vars);

    // 45s: 30s was too tight when the full cargo-test suite runs many
    // integration test binaries in parallel and Alice's startup is slow.
    // Alice's own inner assertion (SessionRefreshed within 12s of
    // Connected) is what keeps this meaningful; the outer deadline is
    // just a safety net against hangs.
    let deadline = Instant::now() + Duration::from_secs(45);
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
        None => panic!("Alice did not finish within 45s"),
    };

    assert!(
        status.success(),
        "Alice exited with {:?} (expected success — SessionRefreshed observed)",
        status.code()
    );
}
