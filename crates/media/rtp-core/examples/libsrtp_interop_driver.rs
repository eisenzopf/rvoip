//! Deterministic SRTP/SRTCP driver used by `scripts/test_libsrtp_interop.sh`.
//!
//! The packet, key, and expected ciphertext values come from libSRTP's
//! `srtp_validate` test at the commit pinned by that script. Keeping this
//! driver dependency-free makes the external interoperability gate usable in
//! release qualification without adding libSRTP to the Rust dependency graph.

use rvoip_rtp_core::packet::RtpPacket;
use rvoip_rtp_core::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80};
use std::error::Error;

const MASTER_KEY: [u8; 16] = [
    0xe1, 0xf9, 0x7a, 0x0d, 0x3e, 0x01, 0x8b, 0xe0, 0xd6, 0x4f, 0xa3, 0x2c, 0x06, 0xde, 0x41, 0x39,
];
const MASTER_SALT: [u8; 14] = [
    0x0e, 0xc6, 0x75, 0xad, 0x49, 0x8a, 0xfe, 0xeb, 0xb6, 0x96, 0x0b, 0x3a, 0xab, 0xe6,
];

const RTP_PLAINTEXT: [u8; 28] = [
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad, 0xca, 0xfe, 0xba, 0xbe, 0xab, 0xab, 0xab, 0xab,
    0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
];
const SRTP_CIPHERTEXT: [u8; 38] = [
    0x80, 0x0f, 0x12, 0x34, 0xde, 0xca, 0xfb, 0xad, 0xca, 0xfe, 0xba, 0xbe, 0x4e, 0x55, 0xdc, 0x4c,
    0xe7, 0x99, 0x78, 0xd8, 0x8c, 0xa4, 0xd2, 0x15, 0x94, 0x9d, 0x24, 0x02, 0xb7, 0x8d, 0x6a, 0xcc,
    0x99, 0xea, 0x17, 0x9b, 0x8d, 0xbb,
];

const RTCP_PLAINTEXT: [u8; 24] = [
    0x81, 0xc8, 0x00, 0x0b, 0xca, 0xfe, 0xba, 0xbe, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
    0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab, 0xab,
];
const SRTCP_CIPHERTEXT: [u8; 38] = [
    0x81, 0xc8, 0x00, 0x0b, 0xca, 0xfe, 0xba, 0xbe, 0x71, 0x28, 0x03, 0x5b, 0xe4, 0x87, 0xb9, 0xbd,
    0xbe, 0xf8, 0x90, 0x41, 0xf9, 0x77, 0xa5, 0xa8, 0x80, 0x00, 0x00, 0x01, 0x99, 0x3e, 0x08, 0xcd,
    0x54, 0xd6, 0xc1, 0x23, 0x07, 0x98,
];

fn context() -> Result<SrtpContext, rvoip_rtp_core::Error> {
    SrtpContext::new(
        SRTP_AES128_CM_SHA1_80,
        SrtpCryptoKey::new(MASTER_KEY.to_vec(), MASTER_SALT.to_vec()),
    )
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut result, "{byte:02x}").expect("writing to String cannot fail");
    }
    result
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if value.len() % 2 != 0 {
        return Err("hex input must contain an even number of characters".to_string());
    }

    value
        .as_bytes()
        .chunks_exact(2)
        .enumerate()
        .map(|(index, pair)| {
            let digits = std::str::from_utf8(pair)
                .map_err(|_| format!("invalid UTF-8 at hex byte {index}"))?;
            u8::from_str_radix(digits, 16)
                .map_err(|_| format!("invalid hex byte `{digits}` at offset {}", index * 2))
        })
        .collect()
}

fn require_expected(label: &str, actual: &[u8], expected: &[u8]) -> Result<(), String> {
    if actual == expected {
        return Ok(());
    }

    Err(format!(
        "{label} did not match libSRTP v2.8.0 known answer\nexpected: {}\nactual:   {}",
        encode_hex(expected),
        encode_hex(actual)
    ))
}

fn protect_rtp() -> Result<(), Box<dyn Error>> {
    let packet = RtpPacket::parse(&RTP_PLAINTEXT)?;
    let wire = context()?.protect(&packet)?.serialize()?;
    require_expected("SRTP ciphertext", &wire, &SRTP_CIPHERTEXT)?;
    println!("{}", encode_hex(&wire));
    Ok(())
}

fn unprotect_rtp(input: &str) -> Result<(), Box<dyn Error>> {
    let wire = decode_hex(input)?;
    let plaintext = context()?.unprotect(&wire)?.serialize()?;
    require_expected("RTP plaintext", &plaintext, &RTP_PLAINTEXT)?;
    println!("ok");
    Ok(())
}

fn protect_rtcp() -> Result<(), Box<dyn Error>> {
    let wire = context()?.protect_rtcp(&RTCP_PLAINTEXT)?;
    require_expected("SRTCP ciphertext", &wire, &SRTCP_CIPHERTEXT)?;
    println!("{}", encode_hex(&wire));
    Ok(())
}

fn unprotect_rtcp(input: &str) -> Result<(), Box<dyn Error>> {
    let wire = decode_hex(input)?;
    let plaintext = context()?.unprotect_rtcp(&wire)?;
    require_expected("RTCP plaintext", &plaintext, &RTCP_PLAINTEXT)?;
    println!("ok");
    Ok(())
}

fn usage() -> &'static str {
    "usage: libsrtp_interop_driver <protect-rtp|unprotect-rtp|protect-rtcp|unprotect-rtcp> [hex-packet]"
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("protect-rtp"), None, None) => protect_rtp(),
        (Some("unprotect-rtp"), Some(input), None) => unprotect_rtp(&input),
        (Some("protect-rtcp"), None, None) => protect_rtcp(),
        (Some("unprotect-rtcp"), Some(input), None) => unprotect_rtcp(&input),
        _ => Err(usage().into()),
    }
}
