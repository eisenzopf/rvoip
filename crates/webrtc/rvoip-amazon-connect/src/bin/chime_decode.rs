//! `chime-decode` — decode base64 Amazon Chime `SdkSignalFrame`s and print their
//! fields, using the same `prost` types our gateway speaks.
//!
//! Feed it either the browser widget's captured frames (from the Playwright
//! capture harness in `tools/capture-chime/`) or the `connect-probe`
//! `--dump-frames` output. Diffing the two reveals exactly where our
//! reconstructed wire format diverges from the working widget — see the plan's
//! "diff loop".
//!
//! Input: one base64-encoded frame per line (blank lines / `#` comments
//! ignored). A line may be prefixed `tx:`/`rx:` (the harness/probe annotate
//! direction); the prefix is stripped before decoding.
//!
//! ```bash
//! cargo run --bin chime-decode -- capture.b64
//! cargo run --bin chime-decode < frames.b64
//! ```

use std::io::Read;

use base64::Engine as _;
use prost::Message as _;
use rvoip_amazon_connect::signaling::proto::{sdk_signal_frame::Type as FrameType, SdkSignalFrame};

fn main() {
    let input = match std::env::args().nth(1) {
        Some(path) => {
            std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("read {path}: {e}")))
        }
        None => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .unwrap_or_else(|e| fail(&format!("read stdin: {e}")));
            s
        }
    };

    let mut n = 0usize;
    for (lineno, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Strip an optional direction prefix like "tx:" / "rx:".
        let b64 = line
            .split_once(':')
            .filter(|(p, _)| matches!(p.trim(), "tx" | "rx" | "TX" | "RX"))
            .map(|(_, rest)| rest.trim())
            .unwrap_or(line);
        let dir = line
            .split_once(':')
            .map(|(p, _)| p.trim())
            .filter(|p| matches!(*p, "tx" | "rx" | "TX" | "RX"))
            .unwrap_or("--");

        let bytes = match base64::engine::general_purpose::STANDARD.decode(b64) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("line {}: base64 decode error: {e}", lineno + 1);
                continue;
            }
        };
        match SdkSignalFrame::decode(&bytes[..]) {
            Ok(frame) => {
                n += 1;
                print_frame(dir, &frame, bytes.len());
            }
            Err(e) => eprintln!("line {}: protobuf decode error: {e}", lineno + 1),
        }
    }
    eprintln!("\ndecoded {n} frame(s)");
}

fn print_frame(dir: &str, f: &SdkSignalFrame, byte_len: usize) {
    let ty = FrameType::try_from(f.r#type)
        .map(|t| format!("{t:?}"))
        .unwrap_or_else(|_| format!("Unknown({})", f.r#type));
    println!("[{dir}] {ty}  ({byte_len} bytes, ts={})", f.timestamp_ms);

    if let Some(j) = &f.join {
        println!(
            "      JOIN: protocol_version={:?} flags={:?} wants_compressed_sdp={:?} audio_session_id={:?}",
            j.protocol_version, j.flags, j.wants_compressed_sdp, j.audio_session_id
        );
        if let Some(cd) = &j.client_details {
            println!(
                "        client: app_name={:?} client_source={:?} chime_sdk_version={:?} platform={:?}",
                cd.app_name, cd.client_source, cd.chime_sdk_version, cd.platform_name
            );
        }
    }
    if let Some(a) = &f.joinack {
        let turn = a.turn_credentials.as_ref().map(|t| {
            format!(
                "username={:?} ttl={:?} uris={:?}",
                t.username, t.ttl, t.uris
            )
        });
        println!(
            "      JOIN_ACK: turn=[{}] video_subscription_limit={:?} wants_compressed_sdp={:?}",
            turn.unwrap_or_else(|| "none".into()),
            a.video_subscription_limit,
            a.wants_compressed_sdp
        );
    }
    if let Some(s) = &f.sub {
        println!(
            "      SUBSCRIBE: duplex={:?} audio_host={:?} audio_muted={:?} audio_checkin={:?} sdp_offer={} compressed_sdp={}",
            s.duplex,
            s.audio_host,
            s.audio_muted,
            s.audio_checkin,
            s.sdp_offer.as_ref().map(|o| format!("{} chars", o.len())).unwrap_or_else(|| "none".into()),
            s.compressed_sdp_offer.as_ref().map(|c| format!("{} bytes", c.len())).unwrap_or_else(|| "none".into()),
        );
    }
    if let Some(s) = &f.suback {
        println!(
            "      SUBSCRIBE_ACK: duplex={:?} sdp_answer={} compressed_sdp={} tracks={}",
            s.duplex,
            s.sdp_answer
                .as_ref()
                .map(|a| format!("{} chars", a.len()))
                .unwrap_or_else(|| "none".into()),
            s.compressed_sdp_answer
                .as_ref()
                .map(|c| format!("{} bytes", c.len()))
                .unwrap_or_else(|| "none".into()),
            s.tracks.len(),
        );
    }
    if let Some(e) = &f.error {
        println!(
            "      ERROR: status={:?} description={:?}",
            e.status, e.description
        );
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("chime-decode: {msg}");
    std::process::exit(1);
}
