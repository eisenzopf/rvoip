//! Shared harness for the umbrella SIP-stack end-to-end benchmarks.
//!
//! Each bench binary opens its own UDP/SIP listeners on loopback; this
//! module centralises the bits they all need (port allocator, runtime
//! builder) so two benches running back-to-back via `cargo bench -p
//! rvoip-sip` don't collide.

#![allow(dead_code)]

use std::sync::atomic::{AtomicU16, Ordering};
use tokio::runtime::{Builder, Runtime};

/// Starting port for bench-allocated SIP listeners. Picked well above
/// the example range (5060–6999) so re-running benches while an example
/// is still draining sockets doesn't conflict.
const PORT_BASE: u16 = 40000;
const MEDIA_BASE: u16 = 42000;

static NEXT_SIP_PORT: AtomicU16 = AtomicU16::new(PORT_BASE);
static NEXT_MEDIA_PORT: AtomicU16 = AtomicU16::new(MEDIA_BASE);

/// Reserve a unique SIP port for a peer in this process.
pub fn next_sip_port() -> u16 {
    NEXT_SIP_PORT.fetch_add(1, Ordering::Relaxed)
}

/// Reserve a 200-port window for a peer's RTP media.
pub fn next_media_window() -> (u16, u16) {
    let start = NEXT_MEDIA_PORT.fetch_add(200, Ordering::Relaxed);
    (start, start + 199)
}

/// Multi-threaded tokio runtime sized for end-to-end SIP benches.
/// Fewer than 4 workers tends to serialise the server peer's receive
/// loop behind the client peer's send loop, inflating per-call latency.
pub fn build_runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .expect("bench runtime")
}
