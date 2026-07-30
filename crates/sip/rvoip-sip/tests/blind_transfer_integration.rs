//! Multi-binary integration test for RFC 3515 blind transfer.
//!
//! Blind transfer cannot be tested reliably in-process — two StreamPeers sharing
//! one Tokio runtime have repeatedly produced socket / state collisions. Instead
//! we drive the three peers of the scenario (Alice, Bob, Charlie) as separate
//! child processes, mirroring `examples/stream_peer/05_blind_transfer/run.sh`.
//!
//! Topology:
//!   Alice   → calls → Bob
//!   Bob     → REFER → Alice (target: Charlie)
//!   Alice   → calls → Charlie
//!
//! Each peer exits 0 on success. The test succeeds if Alice exits cleanly
//! within the deadline; Bob and Charlie are then cleaned up.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use support::{build_examples, example_binary};

#[cfg(unix)]
fn exit_detail(status: &std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    format!(
        "code={:?}, signal={:?}, core_dumped={}",
        status.code(),
        status.signal(),
        status.core_dumped()
    )
}

#[cfg(not(unix))]
fn exit_detail(status: &std::process::ExitStatus) -> String {
    format!("code={:?}", status.code())
}

/// Port set chosen to avoid collisions with the shell-script example
/// (which uses 5060-5062).
const ALICE_PORT: u16 = 35060;
const BOB_PORT: u16 = 35061;
const CHARLIE_PORT: u16 = 35062;

/// Kill-guard that reaps a child on drop — keeps stray processes from piling
/// up when the test fails partway through.
struct ChildGuard {
    child: std::process::Child,
    log_path: PathBuf,
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_example(name: &str, envs: &[(&str, String)], log_dir: &Path) -> ChildGuard {
    // The examples are built before launch. Executing them directly avoids
    // serializing the long-lived Bob/Charlie peers behind Cargo's artifact
    // lock, which would prevent Alice from starting until they exit.
    let mut cmd = Command::new(example_binary(name));
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let log_path = log_dir.join(format!("{name}.log"));
    let log = std::fs::File::create(&log_path)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", log_path.display()));
    let stderr = log
        .try_clone()
        .unwrap_or_else(|e| panic!("failed to clone {}: {e}", log_path.display()));
    cmd.stdout(Stdio::from(log)).stderr(Stdio::from(stderr));
    let child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {}", name, e));
    ChildGuard { child, log_path }
}

fn log_tail(path: &Path) -> String {
    let log = std::fs::read_to_string(path)
        .unwrap_or_else(|error| format!("<unable to read {}: {error}>", path.display()));
    let lines: Vec<_> = log.lines().collect();
    lines[lines.len().saturating_sub(120)..].join("\n")
}

#[test]
fn blind_transfer_end_to_end() {
    build_examples(&[
        "stream_peer_blind_transfer_alice",
        "stream_peer_blind_transfer_bob",
        "stream_peer_blind_transfer_charlie",
    ]);

    let env_vars: Vec<(&str, String)> = vec![
        ("ALICE_PORT", ALICE_PORT.to_string()),
        ("BOB_PORT", BOB_PORT.to_string()),
        ("CHARLIE_PORT", CHARLIE_PORT.to_string()),
    ];
    let logs = tempfile::tempdir().expect("blind-transfer child log directory");

    // Charlie first so he's ready to accept the transferred call.
    let _charlie = spawn_example("stream_peer_blind_transfer_charlie", &env_vars, logs.path());
    std::thread::sleep(Duration::from_millis(800));

    // Bob next — he waits on an incoming INVITE and issues a REFER after accept.
    let _bob = spawn_example("stream_peer_blind_transfer_bob", &env_vars, logs.path());
    std::thread::sleep(Duration::from_millis(800));

    // Alice starts the flow. Her exit status is our verdict.
    let mut alice = spawn_example("stream_peer_blind_transfer_alice", &env_vars, logs.path());

    let deadline = Instant::now() + Duration::from_secs(30);
    let exit = loop {
        match alice.child.try_wait() {
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
        None => panic!(
            "Alice did not finish within 30s\n--- Alice log tail ---\n{}\n--- Bob log tail ---\n{}\n--- Charlie log tail ---\n{}",
            log_tail(&alice.log_path),
            log_tail(&_bob.log_path),
            log_tail(&_charlie.log_path)
        ),
    };

    if !status.success() {
        panic!(
            "Alice exited unsuccessfully ({})\n--- Alice log tail ---\n{}\n--- Bob log tail ---\n{}\n--- Charlie log tail ---\n{}",
            exit_detail(&status),
            log_tail(&alice.log_path),
            log_tail(&_bob.log_path),
            log_tail(&_charlie.log_path)
        );
    }
    // _bob and _charlie are dropped here; ChildGuard kills them.
}
