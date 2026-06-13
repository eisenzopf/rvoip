//! Shared harness for the `tests/perf/` scenarios.
//!
//! Each scenario builds on these primitives:
//!
//! - [`ports`]: process-wide UDP port allocator so parallel `cargo test`
//!   runs don't collide.
//! - [`env`]: collects host info (CPU model, RAM, rustc, git rev) and
//!   asserts release-mode at startup; debug builds fail loudly.
//! - [`histogram`]: hdrhistogram wrapper exposing
//!   `record(nanos) → p50/p95/p99/p99.9/max` with a JSON-serialisable
//!   shape.
//! - [`load`]: token-bucket load profile (ramp → steady → cooldown).
//! - [`report`]: stdout summary table + JSON file writer to
//!   `target/perf-results/<scenario>.json` matching the canonical schema
//!   in `docs/BENCHMARKING.md`.

// Each `tests/perf/perf_*.rs` is its own crate and includes this
// module via `#[path = "support/mod.rs"] mod support;`. Different test
// binaries use different subsets of the re-exports, so per-crate
// "unused import" warnings would fire for any re-export that one test
// happens not to touch. Silence those at the module boundary.
#![allow(dead_code, unused_imports)]

pub mod burst;
pub mod env;
pub mod histogram;
pub mod load;
pub mod pcap;
pub mod ports;
pub mod report;
pub mod sampler;
pub mod soak;
pub mod sweep;

// `env::EnvironmentBlock` is captured internally by
// `ScenarioReport::new` — scenarios don't need a direct re-export.
pub use histogram::LatencyHistogram;
pub use load::LoadProfile;
pub use report::ScenarioReport;
pub use sampler::{ResourceSample, ResourceSampler, ResourceSummary};
pub use sweep::{parse_sweep_env, SweepRunner};
