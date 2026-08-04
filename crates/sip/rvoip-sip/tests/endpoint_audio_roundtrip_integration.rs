//! Command-level regression for the Endpoint audio roundtrip example.
//!
//! The example itself performs the tone analysis. This test gives it isolated
//! SIP/media ports and asserts both verification lines are printed.

mod support;

use std::process::Command;

use support::{build_examples, example_binary, free_udp_ports};

#[test]
fn endpoint_audio_roundtrip_verifies_tones() {
    build_examples(&["endpoint_audio_roundtrip"]);

    // Ports come from the kernel rather than from constants. The fixed pair
    // this used to hard-code collides with anything else on the box that binds
    // in the same range, and daemons like wsdd do exactly that.
    let [alice_sip, bob_sip] = free_udp_ports::<2>();
    let [alice_media_start, bob_media_start] = free_udp_ports::<2>();

    let output = Command::new(example_binary("endpoint_audio_roundtrip"))
        .env("ALICE_SIP_PORT", alice_sip.to_string())
        .env("BOB_SIP_PORT", bob_sip.to_string())
        .env("ALICE_MEDIA_PORT_START", alice_media_start.to_string())
        .env("ALICE_MEDIA_PORT_END", (alice_media_start + 49).to_string())
        .env("BOB_MEDIA_PORT_START", bob_media_start.to_string())
        .env("BOB_MEDIA_PORT_END", (bob_media_start + 49).to_string())
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
