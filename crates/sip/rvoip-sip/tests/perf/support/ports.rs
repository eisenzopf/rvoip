//! Process-wide port allocator for the perf suite.
//!
//! Modelled on `benches/common/mod.rs::next_sip_port`; the SIP range
//! starts at 41000 (well above the bench range 40000–40999 and the
//! example range 5060–6999) so re-running benches and perf tests
//! back-to-back doesn't conflict.

use std::sync::atomic::{AtomicU16, Ordering};

const SIP_BASE: u16 = 41000;
const SIP_SPAN: u16 = 10_000;
const MEDIA_BASE: u16 = 43000;

static NEXT_SIP_PORT: AtomicU16 = AtomicU16::new(SIP_BASE);
static NEXT_MEDIA_PORT: AtomicU16 = AtomicU16::new(MEDIA_BASE);

/// Reserve a unique SIP port for a peer in this process.
pub fn next_sip_port() -> u16 {
    let process_offset = (std::process::id() as u16) % SIP_SPAN;
    for _ in 0..SIP_SPAN {
        let sequence = NEXT_SIP_PORT
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_sub(SIP_BASE);
        let port = SIP_BASE + ((process_offset + sequence) % SIP_SPAN);
        if sip_port_is_available(port) {
            return port;
        }
    }
    panic!(
        "no available SIP port found in {SIP_BASE}-{}",
        SIP_BASE + SIP_SPAN - 1
    );
}

/// Reserve a 200-port window for a peer's RTP media. Returns
/// `(start_inclusive, end_inclusive)`.
pub fn next_media_window() -> (u16, u16) {
    let start = NEXT_MEDIA_PORT.fetch_add(200, Ordering::Relaxed);
    (start, start + 199)
}

fn sip_port_is_available(port: u16) -> bool {
    let addr = ("127.0.0.1", port);
    std::net::UdpSocket::bind(addr).is_ok() && std::net::TcpListener::bind(addr).is_ok()
}
