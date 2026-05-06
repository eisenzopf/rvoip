//! Command-level regression for the Endpoint audio roundtrip example.
//!
//! The example itself performs the tone analysis. This test gives it isolated
//! SIP/media ports and asserts both verification lines are printed.

use std::env;
use std::process::Command;

fn cargo_bin() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

#[test]
fn endpoint_audio_roundtrip_verifies_tones() {
    let output = Command::new(cargo_bin())
        .args([
            "run",
            "--quiet",
            "-p",
            "rvoip-session-core",
            "--example",
            "endpoint_audio_roundtrip",
        ])
        .env("ALICE_SIP_PORT", "35420")
        .env("BOB_SIP_PORT", "35421")
        .env("ALICE_MEDIA_PORT_START", "35440")
        .env("ALICE_MEDIA_PORT_END", "35489")
        .env("BOB_MEDIA_PORT_START", "35490")
        .env("BOB_MEDIA_PORT_END", "35539")
        .output()
        .expect("failed to run endpoint_audio_roundtrip example");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "endpoint_audio_roundtrip failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        stdout.contains("Alice received Bob's 880 Hz tone"),
        "missing Alice verification line\nstdout:\n{}",
        stdout
    );
    assert!(
        stdout.contains("Bob received Alice's 440 Hz tone"),
        "missing Bob verification line\nstdout:\n{}",
        stdout
    );
}
