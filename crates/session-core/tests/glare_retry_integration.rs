//! Multi-binary integration test for RFC 3261 §14.1 re-INVITE glare.
//!
//! Alice and Bob each reach Active on a call, then simultaneously invoke
//! `hold()` at the same wall-clock instant. Each side's UAS sees an
//! incoming re-INVITE while its own outgoing re-INVITE is pending — the
//! `HasPendingReinvite` guard in the state table fires 491 Request
//! Pending. The `ReinviteGlare` transition schedules a retry with
//! random backoff (2.1–4.0 s), and after the retries resolve both peers
//! settle on OnHold.
//!
//! The test is considered passing iff both child processes exit 0. Alice
//! drives the hangup at the end.

use std::env;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    cmd.spawn()
        .map(ChildGuard)
        .unwrap_or_else(|e| panic!("failed to spawn {}: {}", name, e))
}

fn build_examples() {
    let output = Command::new(cargo_bin())
        .args([
            "build",
            "-p",
            "rvoip-session-core",
            "--example",
            "streampeer_glare_retry_alice",
            "--example",
            "streampeer_glare_retry_bob",
        ])
        .output()
        .expect("failed to invoke cargo build");
    if !output.status.success() {
        let _ = std::io::Write::write_all(
            &mut std::io::stderr(),
            &output.stderr,
        );
        panic!(
            "cargo build failed (exit={:?}); stderr printed above",
            output.status.code()
        );
    }
}

#[test]
fn glare_retry_converges_to_on_hold() {
    build_examples();

    // Both peers sleep until this wall-clock instant before calling hold().
    // Use a generous 8 s lead time: `cargo run --quiet` can take a couple
    // of seconds to resolve dependencies even when the example binary is
    // pre-built, and Alice additionally needs to establish the call and
    // reach Active before the glare window opens.
    let start_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_millis() as u64
        + 8_000;

    let envs: Vec<(&str, String)> = vec![
        ("ALICE_PORT", ALICE_PORT.to_string()),
        ("BOB_PORT", BOB_PORT.to_string()),
        ("RVOIP_TEST_GLARE_START_MS", start_ms.to_string()),
    ];

    let mut bob = spawn_example("streampeer_glare_retry_bob", &envs);
    std::thread::sleep(Duration::from_millis(800));
    let mut alice = spawn_example("streampeer_glare_retry_alice", &envs);

    // The glare path takes longer than a normal call: 8 s synchronize
    // lead, simultaneous hold, 491s, 2.1–4.0 s retry backoff, second
    // attempt, 3 s stability check. 45 s keeps enough headroom without
    // making a hung test drag the suite excessively.
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

    let alice_status =
        alice_status.unwrap_or_else(|| panic!("Alice did not finish within 30s"));
    assert!(
        alice_status.success(),
        "Alice exited with {:?} (expected 0 = saw stable OnHold)",
        alice_status.code()
    );

    // Give Bob up to 10 s to finish on the wait_for_ended path after
    // Alice's hangup.
    let bob_deadline = Instant::now() + Duration::from_secs(10);
    let bob_status = loop {
        match bob.0.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= bob_deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => panic!("failed to poll Bob: {}", e),
        }
    };

    let bob_status = bob_status.unwrap_or_else(|| panic!("Bob did not finish within 10s"));
    assert!(
        bob_status.success(),
        "Bob exited with {:?} (expected 0 = saw stable OnHold)",
        bob_status.code()
    );
}
