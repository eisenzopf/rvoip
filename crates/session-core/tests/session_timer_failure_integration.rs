//! Multi-binary integration test for RFC 4028 §10 session-timer refresh
//! FAILURE.
//!
//! Alice calls Bob with a 4-second `Session-Expires`. Bob accepts the
//! call and then exits its process at t≈1.5 s — before Alice's first
//! refresh at t≈2 s. Alice's UPDATE then lands on a closed UDP port;
//! dialog-core's `session_timer` task awaits the transaction outcome,
//! sees a `TransactionTimeout`, falls back to a re-INVITE which also
//! times out, and tears the dialog down with a `Reason: SIP ;cause=408`
//! BYE. The session layer surfaces `Event::SessionRefreshFailed`.
//!
//! Alice's binary asserts it sees `SessionRefreshFailed` within 15 s.
//! `RVOIP_TEST_TRANSACTION_TIMEOUT_MS=2500` shortens Timer F (default
//! 32 s) so each UPDATE/re-INVITE fails within ~2.5 s — on macOS UDP
//! send-to-dead-port is silent, so we can't rely on ICMP port
//! unreachable and need the transaction-layer timeout to do the work.

use std::env;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ALICE_PORT: u16 = 35073;
const BOB_PORT: u16 = 35074;

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

#[test]
fn session_timer_refresh_failure_emits_event() {
    let build_status = Command::new(cargo_bin())
        .args([
            "build",
            "--quiet",
            "-p",
            "rvoip-session-core",
            "--example",
            "streampeer_session_timer_failure_alice",
            "--example",
            "streampeer_session_timer_failure_bob",
        ])
        .status()
        .expect("failed to invoke cargo build");
    assert!(build_status.success(), "cargo build failed");

    let env_vars: Vec<(&str, String)> = vec![
        ("ALICE_PORT", ALICE_PORT.to_string()),
        ("BOB_PORT", BOB_PORT.to_string()),
        // Shorten Timer F so each dead-peer UPDATE/re-INVITE gives up
        // in ~2.5 s instead of 32 s. Scoped to this test's child procs.
        ("RVOIP_TEST_TRANSACTION_TIMEOUT_MS", "2500".to_string()),
    ];

    let _bob = spawn_example("streampeer_session_timer_failure_bob", &env_vars);
    std::thread::sleep(Duration::from_millis(800));

    let mut alice = spawn_example("streampeer_session_timer_failure_alice", &env_vars);

    // 45 s outer safety net. Alice's internal assertion uses a 15 s
    // budget from CallEstablished — the outer deadline just guards
    // against a hang when the whole integration-test binary runs in
    // parallel with the rest of the suite.
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
        "Alice exited with {:?} (expected success — SessionRefreshFailed observed)",
        status.code()
    );
}
