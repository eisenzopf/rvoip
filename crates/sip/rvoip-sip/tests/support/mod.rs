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
pub mod sdp;
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
pub use sdp::{attach_pcmu_sdp_answer, fixture_media_port};
pub use traces::{
    assert_header_on_wire, receiver_config, wait_for_inbound_method, SMOKE_HEADER_NAME,
    SMOKE_HEADER_VALUE,
};

const ISOLATED_EXAMPLE_TARGET: &str = "rvoip-sip-integration-examples";

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
