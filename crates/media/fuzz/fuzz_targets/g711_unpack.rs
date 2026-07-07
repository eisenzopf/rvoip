#![no_main]
//! Fuzz G.711 (µ-law) depacketization — the RTP payload -> PCM path exercised
//! by the transcoder. `unpack` has an infallible signature (returns `Bytes`),
//! so this target asserts it never panics on a malformed/oversized payload.

use libfuzzer_sys::fuzz_target;
use rvoip_media_core::rtp_processing::payload::{G711UPayloadFormat, PayloadFormat};

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let fmt = G711UPayloadFormat::new(8000);
    let _ = fmt.unpack(data, 0);
});
