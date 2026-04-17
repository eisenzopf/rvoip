//! Multi-binary integration test for RFC 3515 blind transfer.
//!
//! Blind transfer cannot be tested reliably in-process — two StreamPeers sharing
//! one Tokio runtime have repeatedly produced socket / state collisions. Instead
//! we drive the three peers of the scenario (Alice, Bob, Charlie) as separate
//! child processes, mirroring `examples/streampeer/blind_transfer/run.sh`.
//!
//! Topology:
//!   Alice   → calls → Bob
//!   Bob     → REFER → Alice (target: Charlie)
//!   Alice   → calls → Charlie
//!
//! Each peer exits 0 on success. The test succeeds if Alice exits cleanly
//! within the deadline; Bob and Charlie are then cleaned up.

use std::env;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Port set chosen to avoid collisions with the shell-script example
/// (which uses 5060-5062).
const ALICE_PORT: u16 = 35060;
const BOB_PORT: u16 = 35061;
const CHARLIE_PORT: u16 = 35062;

/// Kill-guard that reaps a child on drop — keeps stray processes from piling
/// up when the test fails partway through.
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn cargo_bin() -> String {
    // Honour CARGO if cargo set it (it does when running tests via cargo test).
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
    // Silence the child's stdout/stderr by default; uncomment to debug.
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {}", name, e));
    ChildGuard(child)
}

#[test]
fn blind_transfer_end_to_end() {
    // Build all three examples first so cargo-run invocations below are cheap.
    let build_status = Command::new(cargo_bin())
        .args([
            "build",
            "--quiet",
            "-p",
            "rvoip-session-core-v3",
            "--example",
            "streampeer_blind_transfer_alice",
            "--example",
            "streampeer_blind_transfer_bob",
            "--example",
            "streampeer_blind_transfer_charlie",
        ])
        .status()
        .expect("failed to invoke cargo build");
    assert!(build_status.success(), "cargo build failed");

    let env_vars: Vec<(&str, String)> = vec![
        ("ALICE_PORT", ALICE_PORT.to_string()),
        ("BOB_PORT", BOB_PORT.to_string()),
        ("CHARLIE_PORT", CHARLIE_PORT.to_string()),
    ];

    // Charlie first so he's ready to accept the transferred call.
    let _charlie = spawn_example("streampeer_blind_transfer_charlie", &env_vars);
    std::thread::sleep(Duration::from_millis(800));

    // Bob next — he waits on an incoming INVITE and issues a REFER after accept.
    let _bob = spawn_example("streampeer_blind_transfer_bob", &env_vars);
    std::thread::sleep(Duration::from_millis(800));

    // Alice starts the flow. Her exit status is our verdict.
    let mut alice = spawn_example("streampeer_blind_transfer_alice", &env_vars);

    let deadline = Instant::now() + Duration::from_secs(30);
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
        None => panic!("Alice did not finish within 30s"),
    };

    assert!(
        status.success(),
        "Alice exited with {:?} (expected success)",
        status.code()
    );
    // _bob and _charlie are dropped here; ChildGuard kills them.
}
