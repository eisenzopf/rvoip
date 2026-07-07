#![no_main]
//! Fuzz `Record::parse_multiple` — the outermost DTLS record-layer parse that
//! runs on every inbound datagram before the handshake state machine. Handshake
//! message bodies are reached transitively through this entry point.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let _ = rvoip_rtp_core::dtls::record::Record::parse_multiple(data);
});
