#![no_main]
//! Fuzz `decode_binding_response` — the STUN parse used during ICE / NAT
//! discovery, exposed to unauthenticated network responses.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    // Fixed expected transaction id (12 bytes per RFC 5389); fuzz the response.
    let txn = [0u8; 12];
    let _ = rvoip_rtp_core::network::stun::decode_binding_response(data, &txn);
});
