#![no_main]
//! Fuzz `RtcpPacket::parse` — inbound RTCP (incl. compound) parse entry point.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let _ = rvoip_rtp_core::RtcpPacket::parse(data);
});
