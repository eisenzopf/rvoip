#![no_main]

use libfuzzer_sys::fuzz_target;
use rvoip_sip_core::Uri;
use std::str::FromStr;

fuzz_target!(|data: &[u8]| {
    if data.len() > 4096 {
        return;
    }
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };
    let _ = Uri::from_str(input.trim());
});
