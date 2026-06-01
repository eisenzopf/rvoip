//! PBX interop runner that drives the unified scenario harness through the
//! reactive [`CallbackPeer::builder`](rvoip_sip::CallbackPeerBuilder) closure
//! callback API surface.
//!
//! Scenario logic lives in [`common`]; this binary is a thin entry point so
//! the same suite is also exercised through `Endpoint` and `StreamPeer`
//! (sibling files in this directory).
//!
//! Behaviour is controlled by `PBX_PROVIDER`, `PBX_SCENARIO`,
//! `PBX_TRANSPORT`, and `PBX_ROLE`; the orchestrator that sets them is
//! `examples/pbx/run.sh`.
//!
//! Run directly:
//!
//! ```sh
//! cargo run -p rvoip-sip --features dev-insecure-tls --example pbx_callback_builder
//! ```
//!
//! See `examples/pbx/README.md` for the full scenario matrix.

mod common;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    common::run_callback_builder_surface().await
}
