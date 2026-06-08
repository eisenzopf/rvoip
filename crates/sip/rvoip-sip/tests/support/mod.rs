//! Shared two-coordinator scaffolding for §10 verification tests.
//!
//! Tests under `crates/sip/rvoip-sip/tests/` opt in via `mod support;` at
//! the top of each file. Cargo compiles this directory once per test
//! binary that imports it; the `#[allow(dead_code)]` on each submodule
//! suppresses the per-binary unused-helper warnings.

#![allow(dead_code, unused_imports)]

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
    boot_callback_receiver, boot_unified_caller, boot_unified_caller_with_config, establish_call,
    wait_for_call_answered, CallbackReceiver, EstablishedCall,
};
pub use handlers::{AutoAccept, B2buaCarryThrough};
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
