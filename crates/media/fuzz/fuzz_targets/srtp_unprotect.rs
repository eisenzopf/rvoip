#![no_main]
//! Fuzz `SrtpContext::unprotect` — the decrypt + auth-tag path applied to
//! attacker-controlled SRTP datagrams. A fixed key/salt is used: we are
//! exercising parse + tag handling (incl. the auth-tag truncation guard),
//! not key management.

use libfuzzer_sys::fuzz_target;
use rvoip_rtp_core::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }
    // AES-128 CM: 16-byte master key + 14-byte salt.
    let key = SrtpCryptoKey::new(vec![0u8; 16], vec![0u8; 14]);
    let mut ctx = match SrtpContext::new(SRTP_AES128_CM_SHA1_80, key) {
        Ok(ctx) => ctx,
        Err(_) => return,
    };
    let _ = ctx.unprotect(data);
});
