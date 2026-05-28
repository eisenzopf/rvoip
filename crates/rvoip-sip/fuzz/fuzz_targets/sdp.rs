#![no_main]

use bytes::Bytes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    let bytes = Bytes::copy_from_slice(data);
    let _ = rvoip_sip_core::parse_sdp(&bytes);
});
