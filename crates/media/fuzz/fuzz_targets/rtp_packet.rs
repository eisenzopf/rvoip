#![no_main]
//! Fuzz `RtpPacket::parse` — the primary inbound RTP parse entry point.
//! Must return `Err` (never panic) on any malformed/truncated datagram.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let _ = rvoip_rtp_core::RtpPacket::parse(data);
});
