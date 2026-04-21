//! Multi-binary integration test for outbound NOTIFY (RFC 6665).
//!
//! Alice calls Bob, Bob accepts, Alice calls
//! `SessionHandle::send_notify("dialog", body, Some("active;expires=3600"))`.
//! Bob's session-event stream must surface `Event::NotifyReceived` with
//! the expected event package + subscription-state fields — proving:
//!
//! 1. `SessionHandle::send_notify` / `UnifiedCoordinator::send_notify`
//!    reach `DialogAdapter::send_notify` with the right arguments.
//! 2. dialog-core builds and sends the NOTIFY with a proper Event:
//!    header, Subscription-State: header, and body.
//! 3. dialog-core's inbound NOTIFY handler publishes the new
//!    `DialogToSessionEvent::NotifyReceived` cross-crate event.
//! 4. session-core's event handler dispatches that to a public
//!    `Event::NotifyReceived` on the session event stream.

use std::env;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ALICE_PORT: u16 = 35091;
const BOB_PORT: u16 = 35092;

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
    let status = Command::new(cargo_bin())
        .args([
            "build",
            "--quiet",
            "-p",
            "rvoip-session-core",
            "--example",
            "streampeer_notify_send_alice",
            "--example",
            "streampeer_notify_send_bob",
        ])
        .status()
        .expect("failed to invoke cargo build");
    assert!(status.success(), "cargo build failed");
}

#[test]
fn send_notify_surfaces_as_notify_received_on_peer() {
    build_examples();

    let envs: Vec<(&str, String)> = vec![
        ("ALICE_PORT", ALICE_PORT.to_string()),
        ("BOB_PORT", BOB_PORT.to_string()),
    ];

    let mut bob = spawn_example("streampeer_notify_send_bob", &envs);
    std::thread::sleep(Duration::from_millis(800));
    let mut alice = spawn_example("streampeer_notify_send_alice", &envs);

    let deadline = Instant::now() + Duration::from_secs(30);

    let poll = |child: &mut std::process::Child| -> Option<std::process::ExitStatus> {
        loop {
            match child.try_wait() {
                Ok(Some(s)) => return Some(s),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => panic!("failed to poll child: {}", e),
            }
        }
    };

    let bob_exit = poll(&mut bob.0);
    let alice_exit = poll(&mut alice.0);

    let bob_status =
        bob_exit.unwrap_or_else(|| panic!("Bob did not finish within 30s"));
    let alice_status =
        alice_exit.unwrap_or_else(|| panic!("Alice did not finish within 30s"));

    assert!(
        bob_status.success(),
        "Bob exited with {:?} (expected 0 = saw expected Event::NotifyReceived)",
        bob_status.code()
    );
    assert!(
        alice_status.success(),
        "Alice exited with {:?} (expected 0 = NOTIFY sent + call torn down cleanly)",
        alice_status.code()
    );
}
