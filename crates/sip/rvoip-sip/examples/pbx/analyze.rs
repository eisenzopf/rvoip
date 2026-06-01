//! Post-run analyzer for PBX interop captures.
//!
//! Reads the WAV evidence and structured logs produced under
//! `examples/pbx/output/<provider>/<api>/<scenario>/<transport>/` by the
//! interop runners (`pbx_endpoint`, `pbx_stream_peer`,
//! `pbx_callback_builder`) and reports per-scenario pass/fail evidence.
//!
//! Run directly:
//!
//! ```sh
//! cargo run -p rvoip-sip --features dev-insecure-tls --example pbx_analyze
//! ```
//!
//! See `examples/pbx/README.md` for the full output layout.

mod common;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    common::run_analyze().await
}
