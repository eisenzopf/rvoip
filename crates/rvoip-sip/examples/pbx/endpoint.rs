//! PBX interop runner that drives the unified scenario harness through the
//! [`Endpoint`](rvoip_sip::Endpoint) account/profile API surface.
//!
//! All scenario logic lives in [`common`]; this binary is a thin entry point
//! so the same suite can also be exercised through `StreamPeer` and
//! `CallbackPeer::builder` (see the sibling files in this directory).
//!
//! Behaviour is controlled by the env vars `PBX_PROVIDER`
//! (`asterisk`|`freeswitch`), `PBX_SCENARIO` (e.g. `registration`,
//! `basic_call`, `hold_resume`, `blind_transfer`), `PBX_TRANSPORT`
//! (`udp`|`tls`), and `PBX_ROLE`. The orchestrator that sets these is
//! `examples/pbx/run.sh`.
//!
//! Run directly:
//!
//! ```sh
//! cargo run -p rvoip-sip --features dev-insecure-tls --example pbx_endpoint
//! ```
//!
//! See `examples/pbx/README.md` for the full scenario matrix and provider
//! setup.

mod common;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    common::run_endpoint_surface().await
}
