//! Regression tests: integers on the wire that overflow their target type
//! must produce a parse error, never a panic. With the workspace release
//! profile's `panic = "abort"`, a panic here kills the process from a single
//! INVITE carrying an SDP offer.

use bytes::Bytes;
use rvoip_sip_core::sdp::attributes::parse_scalability_mode;
use rvoip_sip_core::sdp::parser::time_parser::parse_time_with_unit;
use rvoip_sip_core::sdp::{parse_sdp, parse_sdp_strict, parse_sdp_with_mode, SdpParseMode};

const OVERFLOWING: &str = "99999999999999999999999";

fn sdp_with_repeat(repeat_line: &str) -> Bytes {
    Bytes::from(format!(
        "v=0\r\n\
         o=- 1 1 IN IP4 192.0.2.1\r\n\
         s=-\r\n\
         c=IN IP4 192.0.2.1\r\n\
         t=0 0\r\n\
         {repeat_line}\r\n\
         m=audio 49170 RTP/AVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n"
    ))
}

#[test]
fn repeat_interval_overflowing_u64_is_rejected_not_panicking() {
    let sdp = sdp_with_repeat(&format!("r={OVERFLOWING} 1 0"));
    // Lenient parsing may drop the line or reject the body; either is fine.
    let _ = parse_sdp(&sdp);
    assert!(parse_sdp_strict(&sdp).is_err());
}

#[test]
fn repeat_fields_overflowing_u64_never_panic() {
    for line in [
        format!("r={OVERFLOWING} 1 0"),
        format!("r=7d {OVERFLOWING} 0"),
        format!("r=7d 1h {OVERFLOWING}"),
        format!("r={OVERFLOWING}d 1 0"),
        // Fits in u64 but overflows once multiplied by the unit.
        format!("r={}d 1 0", u64::MAX / 2),
    ] {
        let sdp = sdp_with_repeat(&line);
        for mode in [SdpParseMode::Strict, SdpParseMode::Lenient] {
            let _ = parse_sdp_with_mode(&sdp, mode);
        }
    }
}

#[test]
fn fuzzer_crash_input_sdp_r_overflow() {
    // crashes/sdp-r-overflow.bin from the 2026-11-24 security pass.
    let input = format!("r={}6", "3".repeat(93));
    let _ = parse_sdp(&Bytes::from(input.clone()));
    assert!(parse_time_with_unit(&input[2..]).is_err());
}

#[test]
fn time_with_unit_overflow_is_an_error() {
    assert!(parse_time_with_unit(OVERFLOWING).is_err());
    assert!(parse_time_with_unit(&format!("{OVERFLOWING}d")).is_err());
    assert_eq!(parse_time_with_unit("30").unwrap(), 30);
}

#[test]
fn scalability_mode_overflowing_layer_counts_are_errors() {
    assert!(parse_scalability_mode(&format!("L{OVERFLOWING}")).is_err());
    assert!(parse_scalability_mode(&format!("L{OVERFLOWING}T1")).is_err());
    assert!(parse_scalability_mode(&format!("L1T{OVERFLOWING}")).is_err());
    let (_, spatial, temporal, _) = parse_scalability_mode("L2T3").unwrap();
    assert_eq!((spatial, temporal), (Some(2), Some(3)));
}
