#![no_main]

use libfuzzer_sys::fuzz_target;
use rvoip_sip_core::{Header, HeaderName, TypedHeader};
use std::convert::TryFrom;
use std::str::FromStr;

fuzz_target!(|data: &[u8]| {
    if data.len() > 4096 {
        return;
    }
    let Ok(input) = std::str::from_utf8(data) else {
        return;
    };

    if let Some((name, value)) = input.split_once(':') {
        if let Ok(name) = HeaderName::from_str(name.trim()) {
            let header = Header::text(name, value.trim());
            let _ = TypedHeader::try_from(&header);
        }
    } else {
        let _ = HeaderName::from_str(input.trim());
    }
});
