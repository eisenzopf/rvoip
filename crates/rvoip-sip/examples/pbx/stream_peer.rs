//! PBX interop runner that drives the unified scenario harness through the
//! [`StreamPeer`](rvoip_sip::StreamPeer) event-stream API surface.
//!
//! Scenario logic lives in [`common`]; this binary is a thin entry point so
//! the same suite is also exercised through `Endpoint` and
//! `CallbackPeer::builder` (sibling files in this directory).
//!
//! Behaviour is controlled by `PBX_PROVIDER`, `PBX_SCENARIO`,
//! `PBX_TRANSPORT`, and `PBX_ROLE`; the orchestrator that sets them is
//! `examples/pbx/run.sh`.
//!
//! Run directly:
//!
//! ```sh
//! cargo run -p rvoip-sip --features dev-insecure-tls --example pbx_stream_peer
//! ```
//!
//! See `examples/pbx/README.md` for the full scenario matrix.

mod common;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    common::run_stream_peer_surface().await
}
