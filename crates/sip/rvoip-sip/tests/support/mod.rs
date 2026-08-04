//! Shared two-coordinator scaffolding for §10 verification tests.
//!
//! Tests under `crates/sip/rvoip-sip/tests/` opt in via `mod support;` at
//! the top of each file. Cargo compiles this directory once per test
//! binary that imports it; the `#[allow(dead_code)]` on each submodule
//! suppresses the per-binary unused-helper warnings.

#![allow(dead_code, unused_imports)]

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod auth_uas;
pub mod established;
pub mod handlers;
#[cfg(feature = "perf-tests")]
pub mod invariants;
pub mod registrar;
pub mod ringing_uas;
pub mod traces;

pub use auth_uas::{boot_auth_uas, AuthUas, CapturedAuthRequest, ChallengeReply};
pub use established::{
    boot_callback_receiver, boot_callback_receiver_with_handler, boot_unified_caller,
    boot_unified_caller_with_config, establish_call, establish_call_with_handler,
    wait_for_call_answered, CallbackReceiver, EstablishedCall,
};
pub use handlers::{AutoAccept, AutoAcceptUnsupportedInfo, B2buaCarryThrough};
#[cfg(feature = "perf-tests")]
pub use invariants::{
    assert_no_watchdog_fallback, assert_pair_released, assert_single_endpoint_released,
    watchdog_counters, watchdog_counters_from_snapshot, WatchdogCounters,
};
pub use registrar::{boot_mock_registrar, CapturedRegister, MockRegistrar, RegistrarReply};
pub use ringing_uas::{boot_ringing_uas, CapturedRequest, RingingUas};
pub use traces::{
    assert_header_on_wire, receiver_config, wait_for_inbound_method, SMOKE_HEADER_NAME,
    SMOKE_HEADER_VALUE,
};

const ISOLATED_EXAMPLE_TARGET: &str = "rvoip-sip-integration-examples";

/// Reserve a UDP port the kernel says is free right now.
///
/// Hard-coded port numbers make a test depend on nothing else on the machine
/// having claimed them, which is not a property any developer box or CI runner
/// offers. `wsdd`, avahi, a softphone and a local SIP service all bind ports in
/// the ranges these tests used to assume, and 5060 in particular is the one
/// port a SIP developer's machine is most likely to have taken.
///
/// The socket is dropped before the port is returned, so this is advisory: the
/// port can in principle be claimed between the probe and the bind. In practice
/// that window is microseconds, against a collision risk that is otherwise
/// permanent.
pub fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .expect("bind an ephemeral UDP port")
        .local_addr()
        .expect("read the ephemeral port")
        .port()
}

/// Reserve `count` distinct free UDP ports at once.
///
/// All probe sockets stay open until every port has been picked, so the ports
/// are distinct from each other.
pub fn free_udp_ports<const N: usize>() -> [u16; N] {
    let sockets: Vec<std::net::UdpSocket> = (0..N)
        .map(|_| std::net::UdpSocket::bind("127.0.0.1:0").expect("bind an ephemeral UDP port"))
        .collect();
    let mut ports = [0u16; N];
    for (slot, socket) in ports.iter_mut().zip(sockets.iter()) {
        *slot = socket.local_addr().expect("read the ephemeral port").port();
    }
    ports
}

fn cargo_bin() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

/// Return a Cargo target directory that is a sibling of the outer test
/// target, rather than the target whose artifact lock is held by
/// `cargo test --tests`.
pub fn isolated_example_target_dir() -> PathBuf {
    let test_binary = env::current_exe().expect("current integration-test binary");
    let profile_dir = test_binary
        .parent()
        .and_then(Path::parent)
        .expect("integration test runs from a Cargo target profile");
    let outer_target_dir = profile_dir
        .parent()
        .expect("Cargo target profile has a target directory");
    outer_target_dir.join(ISOLATED_EXAMPLE_TARGET)
}

/// Build the named process-fixture examples without contending on the outer
/// `cargo test` target lock.
pub fn build_examples(names: &[&str]) {
    assert!(!names.is_empty(), "at least one example must be requested");

    let target_dir = isolated_example_target_dir();
    let mut command = Command::new(cargo_bin());
    command.args(["build", "--quiet", "-p", "rvoip-sip"]);
    for name in names {
        command.args(["--example", name]);
    }

    let status = command
        .env("CARGO_TARGET_DIR", &target_dir)
        .status()
        .expect("failed to invoke cargo build");
    assert!(
        status.success(),
        "cargo build failed (exit={:?}, target={})",
        status.code(),
        target_dir.display()
    );
}

/// Resolve one example produced by [`build_examples`] for direct execution.
pub fn example_binary(name: &str) -> PathBuf {
    let binary = isolated_example_target_dir()
        .join("debug")
        .join("examples")
        .join(format!("{name}{}", env::consts::EXE_SUFFIX));
    assert!(
        binary.is_file(),
        "built example binary is missing: {}",
        binary.display()
    );
    binary
}
