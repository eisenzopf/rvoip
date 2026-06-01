//! Tight parse + serialize loop for CPU profiling.
//!
//! Run under `samply`:
//!
//! ```bash
//! cargo build --profile flamegraph -p rvoip-sip --example profiling_parser_loop
//! samply record target/flamegraph/examples/profiling_parser_loop
//! ```
//!
//! Stops after `DURATION_SECS` so the profile has a clean end. To run
//! indefinitely (e.g. for live attach with `samply` on macOS), set the
//! env var `RVOIP_PROFILE_DURATION=inf`.

use rvoip_sip_core::parse_message;
use std::env;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const DURATION_SECS: u64 = 30;

const REGISTER: &[u8] = b"REGISTER sip:registrar.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc.example.com:5060;branch=z9hG4bKpprof\r\n\
Max-Forwards: 70\r\n\
From: Alice <sip:alice@example.com>;tag=pprof\r\n\
To: Alice <sip:alice@example.com>\r\n\
Call-ID: pprof-reg@pc.example.com\r\n\
CSeq: 1 REGISTER\r\n\
Contact: <sip:alice@pc.example.com:5060>\r\n\
Expires: 3600\r\n\
Content-Length: 0\r\n\r\n";

const INVITE: &[u8] = b"INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bKpprof2\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=pprof2\r\n\
Call-ID: pprof-invite@pc33.atlanta.example.com\r\n\
CSeq: 1 INVITE\r\n\
Contact: <sip:alice@pc33.atlanta.example.com>\r\n\
Content-Length: 0\r\n\r\n";

const RESPONSE_200_OK: &[u8] = b"SIP/2.0 200 OK\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bKpprof2\r\n\
To: Bob <sip:bob@biloxi.example.com>;tag=pprof-resp\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=pprof2\r\n\
Call-ID: pprof-invite@pc33.atlanta.example.com\r\n\
CSeq: 1 INVITE\r\n\
Contact: <sip:bob@biloxi.example.com>\r\n\
Content-Length: 0\r\n\r\n";

fn main() {
    let duration = match env::var("RVOIP_PROFILE_DURATION").as_deref() {
        Ok("inf") => None,
        Ok(s) => Some(Duration::from_secs(s.parse().unwrap_or(DURATION_SECS))),
        Err(_) => Some(Duration::from_secs(DURATION_SECS)),
    };
    let corpus: [&[u8]; 3] = [REGISTER, INVITE, RESPONSE_200_OK];
    let count = AtomicU64::new(0);

    let start = Instant::now();
    let report_every = Duration::from_secs(5);
    let mut next_report = start + report_every;
    let mut last_count = 0u64;
    let mut last_at = start;

    println!(
        "[parser_loop] running for {} (set RVOIP_PROFILE_DURATION=inf to run forever)",
        match duration {
            Some(d) => format!("{}s", d.as_secs()),
            None => "ever".into(),
        }
    );

    loop {
        for _ in 0..1024 {
            for msg in &corpus {
                let parsed = parse_message(black_box(msg)).expect("parse");
                let bytes = parsed.to_bytes();
                black_box(bytes);
            }
        }
        let n = count.fetch_add(1024 * corpus.len() as u64, Ordering::Relaxed)
            + 1024 * corpus.len() as u64;

        let now = Instant::now();
        if now >= next_report {
            let elapsed = now.duration_since(last_at).as_secs_f64();
            let rate = (n - last_count) as f64 / elapsed;
            println!(
                "[parser_loop] {:>10} msgs total ({:>9.0} msgs/sec)",
                n, rate
            );
            last_count = n;
            last_at = now;
            next_report = now + report_every;
        }

        if let Some(d) = duration {
            if now.duration_since(start) >= d {
                let total = count.load(Ordering::Relaxed);
                let secs = now.duration_since(start).as_secs_f64();
                println!(
                    "[parser_loop] done: {} msgs in {:.2}s ({:.0} msgs/sec)",
                    total,
                    secs,
                    total as f64 / secs
                );
                break;
            }
        }
    }
}
